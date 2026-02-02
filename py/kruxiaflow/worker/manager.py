"""Worker manager - high-level API for running workers.

Mirrors Rust WorkerManager for interface compatibility.
"""

import asyncio
import contextlib
import logging
import signal

from .client import WorkerApiClient
from .config import WorkerConfig
from .poller import WorkerPoller
from .registry import ActivityRegistry

logger = logging.getLogger(__name__)


class WorkerManager:
    """
    Worker manager - high-level API for running workers.

    Mirrors Rust WorkerManager for interface compatibility.
    """

    def __init__(
        self,
        config: WorkerConfig,
        registry: ActivityRegistry,
    ):
        self._config = config
        self._registry = registry
        self._poller: WorkerPoller | None = None
        self._poller_task: asyncio.Task[None] | None = None
        self._client: WorkerApiClient | None = None

    async def start(self) -> asyncio.Task[None]:
        """
        Start worker.

        Returns the poller task handle.
        """
        logger.info(
            "Starting worker manager",
            extra={"worker_id": self._config.worker_id},
        )

        # Create API client
        self._client = WorkerApiClient(
            api_url=str(self._config.api_url),
            client_id=self._config.client_id,
            client_secret=self._config.client_secret,
        )

        # Create poller
        self._poller = WorkerPoller(
            config=self._config,
            client=self._client,
            registry=self._registry,
        )

        # Spawn poller task
        self._poller_task = asyncio.create_task(self._poller.run())

        logger.info("Worker manager started")
        return self._poller_task

    async def stop(self) -> None:
        """Gracefully stop worker."""
        logger.info("Stopping worker manager")

        if self._poller:
            self._poller.shutdown()

        if self._poller_task:
            self._poller_task.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                await self._poller_task

        if self._client:
            await self._client.close()

        logger.info("Worker manager stopped")

    async def run_until_shutdown(self) -> None:
        """
        Run worker until SIGINT/SIGTERM.

        Convenience method for standalone worker processes.
        """
        loop = asyncio.get_event_loop()
        stop_event = asyncio.Event()

        def signal_handler() -> None:
            stop_event.set()

        for sig in (signal.SIGINT, signal.SIGTERM):
            loop.add_signal_handler(sig, signal_handler)

        try:
            await self.start()
            await stop_event.wait()
        finally:
            await self.stop()
