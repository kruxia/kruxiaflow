"""Tests for worker manager."""

import asyncio
import contextlib
from unittest import mock

import pytest

from kruxiaflow.worker import ActivityRegistry, WorkerConfig, WorkerManager


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


class TestWorkerManagerInit:
    """Test WorkerManager initialization."""

    def test_creates_manager(self):
        config = create_config()
        registry = ActivityRegistry()

        manager = WorkerManager(config, registry)

        assert manager._config is config
        assert manager._registry is registry
        assert manager._poller is None
        assert manager._poller_task is None
        assert manager._client is None

    def test_accepts_only_config_and_registry(self):
        config = create_config()
        registry = ActivityRegistry()

        manager = WorkerManager(config, registry)

        assert manager._config is config
        assert manager._registry is registry


class TestWorkerManagerStart:
    """Test WorkerManager start."""

    @pytest.mark.asyncio
    async def test_start_creates_client(self):
        config = create_config()
        registry = ActivityRegistry()
        manager = WorkerManager(config, registry)

        with mock.patch.object(manager, "_config") as mock_config:
            mock_config.validate.return_value = None
            mock_config.api_url = "http://localhost:8080"
            mock_config.client_id = "test"
            mock_config.client_secret = "secret"
            mock_config.worker_id = "worker_123"
            mock_config.worker = "test"
            mock_config.max_concurrent_activities = 4
            mock_config.poll_interval = 0.1
            mock_config.poll_max_activities = 10
            mock_config.activity_timeout = 300.0
            mock_config.heartbeat_interval = 30.0

            task = await manager.start()

            assert manager._client is not None
            assert manager._poller is not None
            assert manager._poller_task is not None
            assert isinstance(task, asyncio.Task)

            await manager.stop()

    @pytest.mark.asyncio
    async def test_start_returns_poller_task(self):
        config = create_config()
        registry = ActivityRegistry()
        manager = WorkerManager(config, registry)

        task = await manager.start()

        assert isinstance(task, asyncio.Task)
        assert manager._poller_task is task

        await manager.stop()


class TestWorkerManagerStop:
    """Test WorkerManager stop."""

    @pytest.mark.asyncio
    async def test_stop_cancels_poller_task(self):
        config = create_config()
        registry = ActivityRegistry()
        manager = WorkerManager(config, registry)

        await manager.start()
        assert manager._poller_task is not None
        assert not manager._poller_task.done()

        await manager.stop()

        assert manager._poller_task.done()

    @pytest.mark.asyncio
    async def test_stop_closes_client(self):
        config = create_config()
        registry = ActivityRegistry()
        manager = WorkerManager(config, registry)

        await manager.start()
        client = manager._client

        with mock.patch.object(client, "close", wraps=client.close) as mock_close:
            await manager.stop()
            mock_close.assert_called_once()

    @pytest.mark.asyncio
    async def test_stop_signals_poller_shutdown(self):
        config = create_config()
        registry = ActivityRegistry()
        manager = WorkerManager(config, registry)

        await manager.start()

        await manager.stop()

        assert manager._poller._shutdown_event.is_set()

    @pytest.mark.asyncio
    async def test_stop_is_safe_without_start(self):
        config = create_config()
        registry = ActivityRegistry()
        manager = WorkerManager(config, registry)

        # Should not raise
        await manager.stop()


class TestWorkerManagerRunUntilShutdown:
    """Test WorkerManager run_until_shutdown."""

    @pytest.mark.asyncio
    async def test_run_until_shutdown_starts_manager(self):
        config = create_config()
        registry = ActivityRegistry()
        manager = WorkerManager(config, registry)

        # Create a task that will send a signal shortly
        async def send_signal():
            await asyncio.sleep(0.1)
            manager._poller.shutdown()

        signal_task = asyncio.create_task(send_signal())

        # Mock the signal handler setup since we can't use real signals in tests
        with mock.patch("asyncio.get_event_loop") as mock_loop:
            mock_loop_instance = mock.MagicMock()
            mock_loop.return_value = mock_loop_instance

            # Start and immediately stop
            start_task = asyncio.create_task(manager.start())
            await asyncio.sleep(0.05)
            await manager.stop()

            await start_task
            signal_task.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                await signal_task


class TestWorkerManagerIntegration:
    """Integration tests for WorkerManager."""

    @pytest.mark.asyncio
    async def test_start_stop_cycle(self):
        """Test a complete start/stop cycle."""
        config = create_config()
        registry = ActivityRegistry()
        manager = WorkerManager(config, registry)

        # Start
        task = await manager.start()
        assert not task.done()

        # Worker should be running
        assert manager._client is not None
        assert manager._poller is not None

        # Stop
        await manager.stop()
        assert task.done()

    @pytest.mark.asyncio
    async def test_multiple_start_stop_cycles(self):
        """Test multiple start/stop cycles."""
        config = create_config()
        registry = ActivityRegistry()
        manager = WorkerManager(config, registry)

        for _ in range(3):
            await manager.start()
            await asyncio.sleep(0.05)
            await manager.stop()
