"""Tests for kruxiaflow.workers.__main__ module."""

import contextlib
import os
from unittest import mock

import pytest


def close_coroutine(coro):
    """Close a coroutine to prevent 'coroutine was never awaited' warnings."""
    with contextlib.suppress(Exception):
        coro.close()


class TestMain:
    """Tests for the main() entry point."""

    def test_main_missing_env_vars_exits_with_error(self):
        """Test that main() exits with error when env vars are missing."""
        # Clear any existing KRUXIAFLOW_ env vars
        env = {k: v for k, v in os.environ.items() if not k.startswith("KRUXIAFLOW_")}
        # Also clear WORKER_TYPE
        env.pop("WORKER_TYPE", None)

        with (
            mock.patch.dict(os.environ, env, clear=True),
            # Make sys.exit raise SystemExit to stop execution properly
            mock.patch("sys.exit", side_effect=SystemExit(1)) as mock_exit,
        ):
            from kruxiaflow.workers.__main__ import main

            with pytest.raises(SystemExit):
                main()

            mock_exit.assert_called_once_with(1)

    def test_main_with_valid_env_vars_starts_worker(self):
        """Test that main() starts worker with valid env vars."""
        env = {
            "KRUXIAFLOW_API_URL": "http://localhost:8080",
            "KRUXIAFLOW_CLIENT_ID": "test_client",
            "KRUXIAFLOW_CLIENT_SECRET": "test_secret",
            "KRUXIAFLOW_WORKER": "python",
        }

        with (
            mock.patch.dict(os.environ, env, clear=False),
            mock.patch("asyncio.run", side_effect=close_coroutine) as mock_run,
        ):
            from kruxiaflow.workers.__main__ import main

            main()

            # asyncio.run should be called with manager.run_until_shutdown()
            mock_run.assert_called_once()

    def test_main_uses_worker_type_env_override(self):
        """Test that WORKER_TYPE env var overrides config.worker."""
        env = {
            "KRUXIAFLOW_API_URL": "http://localhost:8080",
            "KRUXIAFLOW_CLIENT_ID": "test_client",
            "KRUXIAFLOW_CLIENT_SECRET": "test_secret",
            "KRUXIAFLOW_WORKER": "python",
            "WORKER_TYPE": "py-data",  # Override
        }

        captured_config = {}

        def capture_manager(config, registry):
            captured_config["worker"] = config.worker
            return mock.MagicMock()

        with (
            mock.patch.dict(os.environ, env, clear=False),
            mock.patch("asyncio.run", side_effect=close_coroutine),
            mock.patch(
                "kruxiaflow.workers.__main__.WorkerManager",
                side_effect=capture_manager,
            ),
        ):
            from kruxiaflow.workers.__main__ import main

            main()

        # Worker type should be overridden to py-data
        assert captured_config["worker"] == "py-data"

    def test_main_defaults_worker_type_to_py_std(self):
        """Test that worker type defaults to python when not specified."""
        env = {
            "KRUXIAFLOW_API_URL": "http://localhost:8080",
            "KRUXIAFLOW_CLIENT_ID": "test_client",
            "KRUXIAFLOW_CLIENT_SECRET": "test_secret",
            # KRUXIAFLOW_WORKER not set - will use default
        }

        # Remove any existing worker-related env vars
        clean_env = dict(os.environ)
        clean_env.pop("KRUXIAFLOW_WORKER", None)
        clean_env.pop("WORKER_TYPE", None)
        clean_env.update(env)

        captured_config = {}

        def capture_manager(config, registry):
            captured_config["worker"] = config.worker
            return mock.MagicMock()

        with (
            mock.patch.dict(os.environ, clean_env, clear=True),
            mock.patch("asyncio.run", side_effect=close_coroutine),
            mock.patch(
                "kruxiaflow.workers.__main__.WorkerManager",
                side_effect=capture_manager,
            ),
            # Mock WorkerConfig to not require KRUXIAFLOW_WORKER
            mock.patch("kruxiaflow.workers.__main__.WorkerConfig") as mock_config_cls,
        ):
            mock_config = mock.MagicMock()
            mock_config.worker = None  # Not set
            mock_config_cls.return_value = mock_config

            from kruxiaflow.workers.__main__ import main

            main()

            # Worker should be set to python as default
            assert mock_config.worker == "py-std"

    def test_main_handles_keyboard_interrupt(self):
        """Test that main() handles KeyboardInterrupt gracefully."""
        env = {
            "KRUXIAFLOW_API_URL": "http://localhost:8080",
            "KRUXIAFLOW_CLIENT_ID": "test_client",
            "KRUXIAFLOW_CLIENT_SECRET": "test_secret",
            "KRUXIAFLOW_WORKER": "python",
        }

        def close_and_raise_keyboard_interrupt(coro):
            close_coroutine(coro)
            raise KeyboardInterrupt()

        with (
            mock.patch.dict(os.environ, env, clear=False),
            mock.patch("asyncio.run", side_effect=close_and_raise_keyboard_interrupt),
        ):
            from kruxiaflow.workers.__main__ import main

            # Should not raise, just log and return
            main()

    def test_main_handles_worker_failure(self):
        """Test that main() handles worker failure with exit code 1."""
        env = {
            "KRUXIAFLOW_API_URL": "http://localhost:8080",
            "KRUXIAFLOW_CLIENT_ID": "test_client",
            "KRUXIAFLOW_CLIENT_SECRET": "test_secret",
            "KRUXIAFLOW_WORKER": "python",
        }

        def close_and_raise_runtime_error(coro):
            close_coroutine(coro)
            raise RuntimeError("Connection failed")

        with (
            mock.patch.dict(os.environ, env, clear=False),
            mock.patch("asyncio.run", side_effect=close_and_raise_runtime_error),
            mock.patch("sys.exit", side_effect=SystemExit(1)) as mock_exit,
        ):
            from kruxiaflow.workers.__main__ import main

            with pytest.raises(SystemExit):
                main()

            mock_exit.assert_called_once_with(1)

    def test_main_registers_script_activity(self):
        """Test that main() registers the script activity."""
        env = {
            "KRUXIAFLOW_API_URL": "http://localhost:8080",
            "KRUXIAFLOW_CLIENT_ID": "test_client",
            "KRUXIAFLOW_CLIENT_SECRET": "test_secret",
            "KRUXIAFLOW_WORKER": "python",
        }

        captured_registry = {}

        def capture_registry():
            registry = mock.MagicMock()
            captured_registry["instance"] = registry
            return registry

        with (
            mock.patch.dict(os.environ, env, clear=False),
            mock.patch("asyncio.run", side_effect=close_coroutine),
            mock.patch(
                "kruxiaflow.workers.__main__.ActivityRegistry",
                side_effect=capture_registry,
            ),
        ):
            from kruxiaflow.workers.__main__ import main

            main()

        # Verify script_activity was registered
        registry = captured_registry["instance"]
        registry.register.assert_called_once()
        call_args = registry.register.call_args
        assert call_args[0][1] == "python"  # worker type


class TestScriptActivityName:
    """Tests for script_activity name."""

    def test_script_activity_has_correct_name(self):
        """Test that script_activity has name 'script'."""
        from kruxiaflow.workers.script_activity import script_activity

        assert script_activity.name == "script"
