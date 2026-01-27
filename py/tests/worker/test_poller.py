"""Tests for worker poller."""

import asyncio
from unittest import mock
from uuid import uuid4

import pytest

from kruxiaflow.worker import (
    Activity,
    ActivityContext,
    ActivityRegistry,
    ActivityResult,
    PendingActivity,
    WorkerConfig,
    activity as activity_decorator,
)
from kruxiaflow.worker.poller import WorkerPoller


@activity_decorator(name="success")
async def success_activity(params: dict, ctx: ActivityContext) -> ActivityResult:
    """Activity that succeeds."""
    return ActivityResult.value("result", params.get("value", "done"))


@activity_decorator(name="failing")
async def failing_activity(params: dict, ctx: ActivityContext) -> ActivityResult:
    """Activity that raises an exception."""
    raise ValueError("Intentional failure")


@activity_decorator(name="slow")
async def slow_activity(params: dict, ctx: ActivityContext) -> ActivityResult:
    """Activity that sleeps."""
    await asyncio.sleep(params.get("sleep", 10))
    return ActivityResult.value("result", "done")


@activity_decorator(name="error_result")
async def error_result_activity(params: dict, ctx: ActivityContext) -> ActivityResult:
    """Activity that returns an error result."""
    return ActivityResult.error("Handled error", code="HANDLED_ERROR", retryable=False)


def create_config(**kwargs) -> WorkerConfig:
    """Create test config with defaults."""
    defaults = {
        "api_url": "http://localhost:8080",
        "worker_id": "test_worker",
        "worker": "test",
        "client_id": "test_client",
        "client_secret": "test_secret",
        "poll_interval": 0.01,
        "activity_timeout": 5.0,
        "heartbeat_interval": 1.0,
        "max_concurrent_activities": 4,
        "poll_max_activities": 10,
    }
    defaults.update(kwargs)
    return WorkerConfig(**defaults)


def create_pending_activity(**kwargs) -> PendingActivity:
    """Create test pending activity with defaults."""
    defaults = {
        "activity_id": uuid4(),
        "workflow_id": uuid4(),
        "activity_key": "test_activity",
        "worker": "test",
        "activity_name": "success",
        "parameters": {},
    }
    defaults.update(kwargs)
    return PendingActivity(**defaults)


class TestWorkerPollerInit:
    """Test WorkerPoller initialization."""

    def test_creates_semaphore(self):
        config = create_config(max_concurrent_activities=8)
        client = mock.AsyncMock()
        registry = ActivityRegistry()

        poller = WorkerPoller(config, client, registry)

        # Semaphore should have config.max_concurrent_activities permits
        assert poller._semaphore._value == 8

    def test_shutdown_event_initially_clear(self):
        config = create_config()
        client = mock.AsyncMock()
        registry = ActivityRegistry()

        poller = WorkerPoller(config, client, registry)

        assert not poller._shutdown_event.is_set()


class TestWorkerPollerShutdown:
    """Test WorkerPoller shutdown."""

    def test_shutdown_sets_event(self):
        config = create_config()
        client = mock.AsyncMock()
        registry = ActivityRegistry()

        poller = WorkerPoller(config, client, registry)
        poller.shutdown()

        assert poller._shutdown_event.is_set()


class TestWorkerPollerPollAndExecute:
    """Test WorkerPoller _poll_and_execute."""

    @pytest.mark.asyncio
    async def test_returns_zero_when_no_activities(self):
        config = create_config()
        client = mock.AsyncMock()
        client.poll_activities.return_value = []
        registry = ActivityRegistry()

        poller = WorkerPoller(config, client, registry)
        count = await poller._poll_and_execute()

        assert count == 0
        client.poll_activities.assert_called_once()

    @pytest.mark.asyncio
    async def test_polls_with_correct_parameters(self):
        config = create_config(
            worker="my_worker",
            worker_id="worker_123",
            poll_max_activities=5,
            max_concurrent_activities=10,  # More than poll_max
        )
        client = mock.AsyncMock()
        client.poll_activities.return_value = []
        registry = ActivityRegistry()

        poller = WorkerPoller(config, client, registry)
        await poller._poll_and_execute()

        client.poll_activities.assert_called_once_with(
            worker="my_worker",
            worker_id="worker_123",
            max_activities=5,  # Should be capped by poll_max_activities
        )

    @pytest.mark.asyncio
    async def test_spawns_tasks_for_activities(self):
        config = create_config()
        client = mock.AsyncMock()
        client.poll_activities.return_value = [
            create_pending_activity(),
            create_pending_activity(),
        ]
        registry = ActivityRegistry()
        registry.register(success_activity, "test")

        poller = WorkerPoller(config, client, registry)
        count = await poller._poll_and_execute()

        assert count == 2
        # Give tasks time to run
        await asyncio.sleep(0.1)


class TestWorkerPollerExecuteActivity:
    """Test WorkerPoller _execute_activity."""

    @pytest.mark.asyncio
    async def test_successful_activity_calls_complete(self):
        config = create_config()
        client = mock.AsyncMock()
        registry = ActivityRegistry()
        registry.register(success_activity, "test")

        poller = WorkerPoller(config, client, registry)
        activity = create_pending_activity(parameters={"value": 42})

        await poller._execute_activity(activity)

        client.complete_activity.assert_called_once()
        call_args = client.complete_activity.call_args
        assert call_args.kwargs["activity_id"] == activity.activity_id
        assert call_args.kwargs["output"] == {"result": 42}

    @pytest.mark.asyncio
    async def test_failed_activity_calls_fail(self):
        config = create_config()
        client = mock.AsyncMock()
        registry = ActivityRegistry()
        registry.register(failing_activity, "test")

        poller = WorkerPoller(config, client, registry)
        activity = create_pending_activity(activity_name="failing")

        await poller._execute_activity(activity)

        client.fail_activity.assert_called_once()
        call_args = client.fail_activity.call_args
        assert call_args.kwargs["activity_id"] == activity.activity_id
        assert call_args.kwargs["error_code"] == "EXECUTION_ERROR"
        assert "Intentional failure" in call_args.kwargs["error_message"]

    @pytest.mark.asyncio
    async def test_timeout_calls_fail(self):
        config = create_config(activity_timeout=0.1)
        client = mock.AsyncMock()
        registry = ActivityRegistry()
        registry.register(slow_activity, "test")

        poller = WorkerPoller(config, client, registry)
        activity = create_pending_activity(
            activity_name="slow",
            parameters={"sleep": 10},
        )

        await poller._execute_activity(activity)

        client.fail_activity.assert_called_once()
        call_args = client.fail_activity.call_args
        assert call_args.kwargs["error_code"] == "TIMEOUT"
        assert "timed out" in call_args.kwargs["error_message"]

    @pytest.mark.asyncio
    async def test_uses_activity_timeout_when_specified(self):
        config = create_config(activity_timeout=10.0)
        client = mock.AsyncMock()
        registry = ActivityRegistry()
        registry.register(slow_activity, "test")

        poller = WorkerPoller(config, client, registry)
        # Activity specifies shorter timeout
        activity = create_pending_activity(
            activity_name="slow",
            parameters={"sleep": 10},
            timeout_seconds=1,  # Custom timeout
        )

        # Should use activity's timeout_seconds, not config
        await poller._execute_activity(activity)

        # With 1s timeout and 10s sleep, should timeout
        client.fail_activity.assert_called_once()

    @pytest.mark.asyncio
    async def test_error_result_calls_fail(self):
        config = create_config()
        client = mock.AsyncMock()
        registry = ActivityRegistry()
        registry.register(error_result_activity, "test")

        poller = WorkerPoller(config, client, registry)
        activity = create_pending_activity(activity_name="error_result")

        await poller._execute_activity(activity)

        client.fail_activity.assert_called_once()
        call_args = client.fail_activity.call_args
        assert call_args.kwargs["error_code"] == "HANDLED_ERROR"
        assert call_args.kwargs["error_message"] == "Handled error"
        assert call_args.kwargs["retryable"] is False


class TestWorkerPollerHeartbeat:
    """Test WorkerPoller heartbeat functionality."""

    @pytest.mark.asyncio
    async def test_heartbeat_not_spawned_for_short_timeout(self):
        config = create_config(activity_timeout=30.0)  # < 60s
        client = mock.AsyncMock()
        registry = ActivityRegistry()
        registry.register(success_activity, "test")

        poller = WorkerPoller(config, client, registry)
        activity = create_pending_activity()

        await poller._execute_activity(activity)

        # Heartbeat should not be called for short timeout
        client.heartbeat.assert_not_called()

    @pytest.mark.asyncio
    async def test_heartbeat_spawned_for_long_timeout(self):
        config = create_config(
            activity_timeout=120.0,  # > 60s
            heartbeat_interval=0.05,
        )
        client = mock.AsyncMock()
        registry = ActivityRegistry()

        @activity_decorator(name="medium_slow")
        async def medium_slow(params: dict, ctx: ActivityContext) -> ActivityResult:
            await asyncio.sleep(0.2)
            return ActivityResult.value("result", "done")

        registry.register(medium_slow, "test")

        poller = WorkerPoller(config, client, registry)
        activity = create_pending_activity(activity_name="medium_slow")

        await poller._execute_activity(activity)

        # Heartbeat should have been called at least once
        assert client.heartbeat.call_count >= 1


class TestWorkerPollerSemaphore:
    """Test WorkerPoller semaphore-based concurrency."""

    @pytest.mark.asyncio
    async def test_semaphore_limits_concurrency(self):
        max_concurrent = 2
        config = create_config(max_concurrent_activities=max_concurrent)

        client = mock.AsyncMock()
        client.poll_activities.return_value = []
        registry = ActivityRegistry()

        @activity_decorator(name="tracking")
        async def tracking_activity(
            params: dict, ctx: ActivityContext
        ) -> ActivityResult:
            # Track concurrent execution
            current = params["counter"]
            current["active"] += 1
            current["max_seen"] = max(current["max_seen"], current["active"])
            await asyncio.sleep(0.05)
            current["active"] -= 1
            return ActivityResult.value("result", "done")

        registry.register(tracking_activity, "test")

        poller = WorkerPoller(config, client, registry)

        counter = {"active": 0, "max_seen": 0}

        # Create more activities than max_concurrent
        activities = [
            create_pending_activity(
                activity_name="tracking",
                parameters={"counter": counter},
            )
            for _ in range(5)
        ]

        # Execute activities in parallel via semaphore
        tasks = []
        for act in activities:
            await poller._semaphore.acquire()
            poller._active_count += 1
            task = asyncio.create_task(poller._execute_activity_with_permit(act))
            tasks.append(task)

        await asyncio.gather(*tasks)

        # Should never exceed max_concurrent
        assert counter["max_seen"] <= max_concurrent

    @pytest.mark.asyncio
    async def test_permit_released_on_success(self):
        config = create_config(max_concurrent_activities=2)
        client = mock.AsyncMock()
        registry = ActivityRegistry()
        registry.register(success_activity, "test")

        poller = WorkerPoller(config, client, registry)

        initial_permits = poller._semaphore._value
        assert initial_permits == 2

        # Execute activity with permit management
        await poller._semaphore.acquire()
        poller._active_count += 1
        await poller._execute_activity_with_permit(create_pending_activity())

        # Permit should be released
        assert poller._semaphore._value == 2
        assert poller._active_count == 0

    @pytest.mark.asyncio
    async def test_permit_released_on_failure(self):
        config = create_config(max_concurrent_activities=2)
        client = mock.AsyncMock()
        registry = ActivityRegistry()
        registry.register(failing_activity, "test")

        poller = WorkerPoller(config, client, registry)

        await poller._semaphore.acquire()
        poller._active_count += 1
        await poller._execute_activity_with_permit(
            create_pending_activity(activity_name="failing")
        )

        # Permit should still be released even on failure
        assert poller._semaphore._value == 2
        assert poller._active_count == 0


class TestWorkerPollerRun:
    """Test WorkerPoller run loop."""

    @pytest.mark.asyncio
    async def test_run_sleeps_when_no_activities(self):
        """Test that run() sleeps when poll returns no activities."""
        config = create_config(poll_interval=0.01)
        client = mock.AsyncMock()
        client.poll_activities.return_value = []
        registry = ActivityRegistry()

        poller = WorkerPoller(config, client, registry)

        # Run for a short time then shutdown
        async def shutdown_after_delay():
            await asyncio.sleep(0.05)
            poller.shutdown()

        shutdown_task = asyncio.create_task(shutdown_after_delay())

        await poller.run()

        await shutdown_task

        # Should have polled multiple times due to sleep interval
        assert client.poll_activities.call_count >= 1

    @pytest.mark.asyncio
    async def test_run_handles_poll_error(self):
        """Test that run() handles errors during polling."""
        config = create_config(poll_interval=0.01)
        client = mock.AsyncMock()
        # First call raises, subsequent calls return empty
        client.poll_activities.side_effect = [
            RuntimeError("Connection error"),
            [],
            [],
        ]
        registry = ActivityRegistry()

        poller = WorkerPoller(config, client, registry)

        # Run for a short time then shutdown
        async def shutdown_after_delay():
            await asyncio.sleep(0.1)
            poller.shutdown()

        shutdown_task = asyncio.create_task(shutdown_after_delay())

        # Should not raise despite the error
        await poller.run()

        await shutdown_task

    @pytest.mark.asyncio
    async def test_run_polls_immediately_when_work_found(self):
        """Test that run() polls immediately when activities are found."""
        config = create_config(poll_interval=1.0)  # Long interval
        client = mock.AsyncMock()
        # Return one activity, then empty
        activity = create_pending_activity()
        client.poll_activities.side_effect = [
            [activity],
            [],
            [],
        ]
        registry = ActivityRegistry()
        registry.register(success_activity, "test")

        poller = WorkerPoller(config, client, registry)

        # Run for a short time then shutdown
        async def shutdown_after_delay():
            await asyncio.sleep(0.15)
            poller.shutdown()

        shutdown_task = asyncio.create_task(shutdown_after_delay())

        await poller.run()

        await shutdown_task

        # Should have polled at least twice quickly (not waiting for 1s interval)
        assert client.poll_activities.call_count >= 2


class TestWorkerPollerHeartbeatFailure:
    """Test WorkerPoller heartbeat failure handling."""

    @pytest.mark.asyncio
    async def test_heartbeat_failure_is_logged_not_raised(self):
        """Test that heartbeat failure is logged but doesn't crash the activity."""
        config = create_config(
            activity_timeout=120.0,  # > 60s to spawn heartbeat
            heartbeat_interval=0.02,  # Short interval for testing
        )
        client = mock.AsyncMock()
        # Heartbeat will fail
        client.heartbeat.side_effect = RuntimeError("Connection lost")
        registry = ActivityRegistry()

        @activity_decorator(name="medium_activity")
        async def medium_activity(params: dict, ctx: ActivityContext) -> ActivityResult:
            await asyncio.sleep(0.1)  # Long enough for heartbeat to be attempted
            return ActivityResult.value("result", "done")

        registry.register(medium_activity, "test")

        poller = WorkerPoller(config, client, registry)
        activity = create_pending_activity(activity_name="medium_activity")

        # Should complete despite heartbeat failures
        await poller._execute_activity(activity)

        # Heartbeat should have been attempted and failed
        assert client.heartbeat.call_count >= 1
        # Activity should still complete
        client.complete_activity.assert_called_once()
