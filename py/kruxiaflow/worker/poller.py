"""Worker poller - polls for activities and executes them.

Mirrors Rust WorkerPoller for interface compatibility.

CRITICAL IMPLEMENTATION DETAILS (from Rust worker):
1. Semaphore-based concurrency with owned permits
2. Heartbeat spawned only for activities with timeout > 60s
3. Report completion BEFORE canceling heartbeat (race condition prevention)
4. Poll backoff: sleep poll_interval when no work, immediate retry when work found
5. Error recovery: sleep 5s on poll error, then retry
"""

import asyncio
import contextlib
import logging
from uuid import UUID

from .client import PendingActivity, WorkerApiClient
from .config import WorkerConfig
from .context import ActivityContext
from .errors import ActivityTimeoutError
from .file_executor import FileExecutor
from .registry import ActivityRegistry

logger = logging.getLogger(__name__)


class WorkerPoller:
    """
    Worker poller - polls for activities and executes them.

    Mirrors Rust WorkerPoller for interface compatibility.
    """

    def __init__(
        self,
        config: WorkerConfig,
        client: WorkerApiClient,
        registry: ActivityRegistry,
    ):
        self._config = config
        self._client = client
        self._registry = registry
        self._semaphore = asyncio.Semaphore(config.max_concurrent_activities)
        self._shutdown_event = asyncio.Event()
        self._active_count = 0
        self._tasks: set[asyncio.Task[None]] = set()

    async def run(self) -> None:
        """
        Run the poller loop.

        Mirrors Rust WorkerPoller::run()
        """
        logger.info(
            "Starting worker poller",
            extra={
                "worker_id": self._config.worker_id,
                "worker": self._config.worker,
                "max_concurrent": self._config.max_concurrent_activities,
            },
        )

        while not self._shutdown_event.is_set():
            try:
                executed = await self._poll_and_execute()

                if executed == 0:
                    # No activities available, sleep before next poll
                    await asyncio.sleep(self._config.poll_interval)
                # If activities were executed, poll immediately for more

            except Exception as e:
                logger.error(f"Poller error: {e}")
                # Sleep before retry on error
                await asyncio.sleep(5.0)

    def shutdown(self) -> None:
        """Signal shutdown."""
        self._shutdown_event.set()

    async def _poll_and_execute(self) -> int:
        """
        Poll for activities and execute them.

        Mirrors Rust WorkerPoller::poll_and_execute()

        Returns number of activities executed.
        """
        # Wait for at least one semaphore slot
        # This prevents polling when we can't execute anything
        await self._semaphore.acquire()
        self._semaphore.release()

        # Calculate available slots
        available_slots = self._config.max_concurrent_activities - self._active_count

        # Poll for activities (up to available slots or poll_max_activities)
        max_to_poll = min(available_slots, self._config.poll_max_activities)

        activities = await self._client.poll_activities(
            worker=self._config.worker,
            worker_id=self._config.worker_id,
            max_activities=max_to_poll,
        )

        if not activities:
            return 0

        logger.info(
            f"Claimed {len(activities)} activities",
            extra={"worker_id": self._config.worker_id, "count": len(activities)},
        )

        # Spawn task for each activity
        for activity in activities:
            # Acquire semaphore permit for this activity
            await self._semaphore.acquire()
            self._active_count += 1

            # Spawn execution task (permit released when task completes)
            task = asyncio.create_task(self._execute_activity_with_permit(activity))
            self._tasks.add(task)
            task.add_done_callback(self._tasks.discard)

        return len(activities)

    async def _execute_activity_with_permit(self, activity: PendingActivity) -> None:
        """Execute activity and release semaphore permit when done."""
        try:
            await self._execute_activity(activity)
        finally:
            self._semaphore.release()
            self._active_count -= 1

    async def _execute_activity(self, activity: PendingActivity) -> None:
        """
        Execute a single activity.

        Mirrors Rust WorkerPoller::execute_activity()

        CRITICAL: Report completion BEFORE canceling heartbeat to prevent
        race condition where activity could be reclaimed as stale.
        """
        logger.info(
            "Executing activity",
            extra={
                "activity_id": str(activity.activity_id),
                "activity_key": activity.activity_key,
                "worker": activity.worker,
                "activity_name": activity.activity_name,
            },
        )

        # Determine timeout
        timeout = float(
            activity.timeout_seconds
            if activity.timeout_seconds
            else self._config.activity_timeout
        )

        # Spawn heartbeat task for long-running activities (>60s timeout)
        heartbeat_task: asyncio.Task[None] | None = None
        if timeout > 60.0:
            heartbeat_task = asyncio.create_task(
                self._heartbeat_loop(activity.activity_id)
            )

        # Create file executor for this activity
        file_executor = FileExecutor(
            workflow_id=activity.workflow_id,
            activity_key=activity.activity_key,
            client=self._client,
        )

        # Create execution context
        ctx = ActivityContext(
            workflow_id=activity.workflow_id,
            activity_id=activity.activity_id,
            activity_key=activity.activity_key,
            signal=activity.signal_data,
        )
        ctx._client = self._client
        ctx._worker_id = self._config.worker_id
        ctx._file_executor = file_executor

        # Execute activity
        try:
            result = await self._registry.execute(
                worker=activity.worker,
                name=activity.activity_name,
                params=activity.parameters,
                ctx=ctx,
                timeout=timeout,
            )

            # CRITICAL: Report completion BEFORE canceling heartbeat
            if result.is_error:
                await self._client.fail_activity(
                    activity_id=activity.activity_id,
                    worker_id=self._config.worker_id,
                    error_code=result.error_code or "EXECUTION_ERROR",
                    error_message=result.error_message or "Unknown error",
                    retryable=result.retryable,
                )
            else:
                await self._client.complete_activity(
                    activity_id=activity.activity_id,
                    worker_id=self._config.worker_id,
                    output=result.to_output_dict(),
                    cost_usd=result.cost_usd,
                )

        except (asyncio.TimeoutError, ActivityTimeoutError) as e:
            logger.warning(
                f"Activity timed out after {timeout}s",
                extra={"activity_id": str(activity.activity_id)},
            )
            await self._client.fail_activity(
                activity_id=activity.activity_id,
                worker_id=self._config.worker_id,
                error_code="TIMEOUT",
                error_message=str(e)
                if str(e)
                else f"Activity execution timed out after {timeout}s",
                retryable=True,
            )

        except Exception as e:
            logger.error(
                f"Activity execution failed: {e}",
                extra={"activity_id": str(activity.activity_id)},
            )
            await self._client.fail_activity(
                activity_id=activity.activity_id,
                worker_id=self._config.worker_id,
                error_code="EXECUTION_ERROR",
                error_message=str(e),
                retryable=True,
            )

        finally:
            # Cancel heartbeat task AFTER reporting completion
            # This ensures activity is marked completed in database before heartbeats stop
            if heartbeat_task:
                heartbeat_task.cancel()
                with contextlib.suppress(asyncio.CancelledError):
                    await heartbeat_task

            # Cleanup file executor temp directory
            await file_executor.cleanup()

    async def _heartbeat_loop(self, activity_id: UUID) -> None:
        """
        Send periodic heartbeats until cancelled.

        Mirrors Rust spawn_heartbeat_task()
        """
        while True:
            await asyncio.sleep(self._config.heartbeat_interval)
            try:
                await self._client.heartbeat(activity_id, self._config.worker_id)
            except Exception as e:
                logger.warning(
                    f"Failed to send heartbeat: {e}",
                    extra={"activity_id": str(activity_id)},
                )
