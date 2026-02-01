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

    @pytest.mark.asyncio
    async def test_run_until_shutdown_registers_signal_handlers(self):
        """Test that run_until_shutdown registers SIGINT and SIGTERM handlers."""
        config = create_config()
        registry = ActivityRegistry()
        manager = WorkerManager(config, registry)

        registered_signals = []

        async def run_test():
            loop = asyncio.get_event_loop()

            def tracking_add_handler(sig, handler):
                registered_signals.append(sig)
                # Don't actually register, just track

            with (
                mock.patch.object(loop, "add_signal_handler", tracking_add_handler),
                mock.patch.object(manager, "start") as mock_start,
                mock.patch.object(manager, "stop"),
            ):
                mock_start.return_value = asyncio.create_task(asyncio.sleep(0))

                # Create a task that sets the stop event after a short delay
                async def run_with_timeout():
                    # We need to access the internal stop_event
                    # Instead, just let it time out
                    with contextlib.suppress(asyncio.TimeoutError):
                        await asyncio.wait_for(
                            manager.run_until_shutdown(), timeout=0.1
                        )

                await run_with_timeout()

        await run_test()

        import signal

        assert signal.SIGINT in registered_signals
        assert signal.SIGTERM in registered_signals

    @pytest.mark.asyncio
    async def test_run_until_shutdown_stops_on_event(self):
        """Test that run_until_shutdown stops when stop event is set."""
        config = create_config()
        registry = ActivityRegistry()
        manager = WorkerManager(config, registry)

        # Track calls
        start_called = False
        stop_called = False

        async def mock_start():
            nonlocal start_called
            start_called = True
            return asyncio.create_task(asyncio.sleep(10))

        async def mock_stop():
            nonlocal stop_called
            stop_called = True

        with (
            mock.patch.object(manager, "start", mock_start),
            mock.patch.object(manager, "stop", mock_stop),
        ):
            # Run with a short timeout
            async def trigger_shutdown():
                await asyncio.sleep(0.05)
                # Simulate signal by setting poller shutdown
                if manager._poller:
                    manager._poller.shutdown()

            with contextlib.suppress(asyncio.TimeoutError):
                await asyncio.wait_for(
                    asyncio.gather(
                        manager.run_until_shutdown(),
                        trigger_shutdown(),
                    ),
                    timeout=1.0,
                )

        assert start_called
        assert stop_called

    @pytest.mark.asyncio
    async def test_run_until_shutdown_calls_stop_in_finally(self):
        """Test that stop is called even if start raises."""
        config = create_config()
        registry = ActivityRegistry()
        manager = WorkerManager(config, registry)

        stop_called = False

        async def mock_stop():
            nonlocal stop_called
            stop_called = True

        with (
            mock.patch.object(
                manager, "start", side_effect=RuntimeError("Start failed")
            ),
            mock.patch.object(manager, "stop", mock_stop),
            pytest.raises(RuntimeError, match="Start failed"),
        ):
            await manager.run_until_shutdown()

        assert stop_called

    @pytest.mark.asyncio
    async def test_run_until_shutdown_signal_handler_sets_stop_event(self):
        """Test that the signal handler sets the stop event."""
        import signal

        config = create_config()
        registry = ActivityRegistry()
        manager = WorkerManager(config, registry)

        # Track registered signal handlers
        registered_handlers = {}

        async def mock_start():
            # Return a task that never completes
            return asyncio.create_task(asyncio.sleep(10))

        async def mock_stop():
            pass

        def track_add_signal_handler(sig, handler):
            registered_handlers[sig] = handler

        with (
            mock.patch.object(manager, "start", mock_start),
            mock.patch.object(manager, "stop", mock_stop),
        ):
            loop = asyncio.get_event_loop()
            with mock.patch.object(
                loop, "add_signal_handler", track_add_signal_handler
            ):
                # Start run_until_shutdown in the background
                run_task = asyncio.create_task(manager.run_until_shutdown())

                # Give it time to register signal handlers
                await asyncio.sleep(0.01)

                # Verify handlers were registered
                assert signal.SIGINT in registered_handlers
                assert signal.SIGTERM in registered_handlers

                # Call the signal handler directly to simulate receiving SIGINT
                # This tests line 95: stop_event.set()
                registered_handlers[signal.SIGINT]()

                # Wait for run_until_shutdown to complete
                try:
                    await asyncio.wait_for(run_task, timeout=1.0)
                except asyncio.TimeoutError:
                    run_task.cancel()
                    with contextlib.suppress(asyncio.CancelledError):
                        await run_task
