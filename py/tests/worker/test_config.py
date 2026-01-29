"""Tests for worker configuration."""

import os
from typing import ClassVar
from unittest import mock

import pytest
from pydantic import ValidationError

from kruxiaflow.worker import WorkerConfig

# Environment variables to clear for isolated tests
KRUXIAFLOW_ENV_VARS = [
    "KRUXIAFLOW_API_URL",
    "KRUXIAFLOW_WORKER_ID",
    "KRUXIAFLOW_WORKER",
    "KRUXIAFLOW_WORKER_POLL_MAX_ACTIVITIES",
    "KRUXIAFLOW_WORKER_POLL_INTERVAL",
    "KRUXIAFLOW_WORKER_MAX_ACTIVITIES",
    "KRUXIAFLOW_WORKER_ACTIVITY_TIMEOUT",
    "KRUXIAFLOW_WORKER_HEARTBEAT_INTERVAL",
    "KRUXIAFLOW_CLIENT_ID",
    "KRUXIAFLOW_CLIENT_SECRET",
]


@pytest.fixture
def clean_env():
    """Clear all KRUXIAFLOW_ environment variables for isolated tests."""
    original = {k: os.environ.get(k) for k in KRUXIAFLOW_ENV_VARS}
    for k in KRUXIAFLOW_ENV_VARS:
        os.environ.pop(k, None)
    yield
    # Restore original values
    for k, v in original.items():
        if v is not None:
            os.environ[k] = v
        else:
            os.environ.pop(k, None)


def make_config(**kwargs) -> WorkerConfig:
    """Create a WorkerConfig with test defaults."""
    defaults = {
        "api_url": "http://localhost:8080",
        "worker": "python",
        "client_id": "test_client",
        "client_secret": "test_secret",
    }
    defaults.update(kwargs)
    return WorkerConfig(**defaults)


@pytest.mark.usefixtures("clean_env")
class TestWorkerConfigRequiredFields:
    """Test WorkerConfig required fields."""

    def test_api_url_is_required(self):
        with pytest.raises(ValidationError):
            WorkerConfig(worker="python", client_id="test_client", client_secret="test")

    def test_worker_has_default(self):
        config = WorkerConfig(
            api_url="http://localhost:8080",
            client_id="test_client",
            client_secret="test",
        )
        assert config.worker == "py-std"

    def test_client_id_is_required(self):
        with pytest.raises(ValidationError):
            WorkerConfig(
                api_url="http://localhost:8080", worker="python", client_secret="test"
            )

    def test_client_secret_is_required(self):
        with pytest.raises(ValidationError):
            WorkerConfig(
                api_url="http://localhost:8080",
                worker="python",
                client_id="test_client",
            )


@pytest.mark.usefixtures("clean_env")
class TestWorkerConfigDefaults:
    """Test WorkerConfig default values for optional fields."""

    def test_default_poll_max_activities(self):
        config = make_config()
        assert config.poll_max_activities == 10

    def test_default_poll_interval(self):
        config = make_config()
        assert config.poll_interval == 0.1

    def test_default_max_concurrent_activities(self):
        config = make_config()
        assert config.max_concurrent_activities == 16

    def test_default_activity_timeout(self):
        config = make_config()
        assert config.activity_timeout == 300.0

    def test_default_heartbeat_interval(self):
        config = make_config()
        assert config.heartbeat_interval == 30.0

    def test_worker_id_is_generated(self):
        config = make_config()
        assert config.worker_id.startswith("worker_")
        assert len(config.worker_id) == 19  # "worker_" + 12 hex chars

    def test_worker_id_is_unique(self):
        config1 = make_config()
        config2 = make_config()
        assert config1.worker_id != config2.worker_id


@pytest.mark.usefixtures("clean_env")
class TestWorkerConfigCustomValues:
    """Test WorkerConfig with custom values."""

    def test_custom_api_url(self):
        config = make_config(api_url="http://example.com:9000")
        assert str(config.api_url) == "http://example.com:9000/"

    def test_custom_worker_id(self):
        config = make_config(worker_id="my_worker_123")
        assert config.worker_id == "my_worker_123"

    def test_custom_worker(self):
        config = make_config(worker="custom")
        assert config.worker == "custom"

    def test_custom_poll_max_activities(self):
        config = make_config(poll_max_activities=5)
        assert config.poll_max_activities == 5

    def test_custom_poll_interval(self):
        config = make_config(poll_interval=0.5)
        assert config.poll_interval == 0.5

    def test_custom_max_concurrent_activities(self):
        config = make_config(max_concurrent_activities=32)
        assert config.max_concurrent_activities == 32

    def test_custom_activity_timeout(self):
        config = make_config(activity_timeout=600.0)
        assert config.activity_timeout == 600.0

    def test_custom_heartbeat_interval(self):
        config = make_config(heartbeat_interval=15.0)
        assert config.heartbeat_interval == 15.0

    def test_custom_client_credentials(self):
        config = make_config(client_id="my_client", client_secret="my_secret")
        assert config.client_id == "my_client"
        assert config.client_secret == "my_secret"


@pytest.mark.usefixtures("clean_env")
class TestWorkerConfigValidation:
    """Test WorkerConfig validation."""

    def test_api_url_must_be_valid_url(self):
        with pytest.raises(ValidationError):
            make_config(api_url="not-a-url")

    def test_api_url_must_not_be_empty(self):
        with pytest.raises(ValidationError):
            make_config(api_url="")

    def test_worker_must_not_be_empty(self):
        with pytest.raises(ValidationError):
            make_config(worker="")

    def test_worker_must_be_valid_slug(self):
        # Must start with lowercase letter
        with pytest.raises(ValidationError):
            make_config(worker="123invalid")
        with pytest.raises(ValidationError):
            make_config(worker="Invalid")  # uppercase
        with pytest.raises(ValidationError):
            make_config(worker="-invalid")  # starts with hyphen

    def test_worker_valid_slug_patterns(self):
        # Valid slugs should work
        assert make_config(worker="python").worker == "python"
        assert make_config(worker="my-worker").worker == "my-worker"
        assert make_config(worker="worker_v2").worker == "worker_v2"
        assert make_config(worker="a1b2c3").worker == "a1b2c3"

    def test_client_id_must_not_be_empty(self):
        with pytest.raises(ValidationError):
            make_config(client_id="")

    def test_client_secret_must_not_be_empty(self):
        with pytest.raises(ValidationError):
            make_config(client_secret="")

    def test_max_concurrent_activities_must_be_positive(self):
        with pytest.raises(ValidationError):
            make_config(max_concurrent_activities=0)

    def test_poll_max_activities_must_be_positive(self):
        with pytest.raises(ValidationError):
            make_config(poll_max_activities=0)


class TestWorkerConfigFromEnvironment:
    """Test WorkerConfig loading from environment variables."""

    # All required env vars for a valid config
    REQUIRED_ENV: ClassVar[dict[str, str]] = {
        "KRUXIAFLOW_API_URL": "http://localhost:8080",
        "KRUXIAFLOW_WORKER": "python",
        "KRUXIAFLOW_CLIENT_ID": "test_client",
        "KRUXIAFLOW_CLIENT_SECRET": "secret",
    }

    def test_loads_api_url_from_env(self):
        env = {**self.REQUIRED_ENV, "KRUXIAFLOW_API_URL": "http://api.example.com:8080"}
        with mock.patch.dict(os.environ, env, clear=True):
            config = WorkerConfig()
            assert str(config.api_url) == "http://api.example.com:8080/"

    def test_loads_worker_id_from_env(self):
        env = {**self.REQUIRED_ENV, "KRUXIAFLOW_WORKER_ID": "custom_worker_123"}
        with mock.patch.dict(os.environ, env, clear=True):
            config = WorkerConfig()
            assert config.worker_id == "custom_worker_123"

    def test_loads_worker_from_env(self):
        env = {**self.REQUIRED_ENV, "KRUXIAFLOW_WORKER": "custom"}
        with mock.patch.dict(os.environ, env, clear=True):
            config = WorkerConfig()
            assert config.worker == "custom"

    def test_loads_client_credentials_from_env(self):
        env = {
            **self.REQUIRED_ENV,
            "KRUXIAFLOW_CLIENT_ID": "my_client",
            "KRUXIAFLOW_CLIENT_SECRET": "my_secret",
        }
        with mock.patch.dict(os.environ, env, clear=True):
            config = WorkerConfig()
            assert config.client_id == "my_client"
            assert config.client_secret == "my_secret"

    def test_raises_on_missing_api_url(self):
        env = {k: v for k, v in self.REQUIRED_ENV.items() if k != "KRUXIAFLOW_API_URL"}
        with (
            mock.patch.dict(os.environ, env, clear=True),
            pytest.raises(ValidationError),
        ):
            WorkerConfig()

    def test_defaults_worker_when_missing(self):
        env = {k: v for k, v in self.REQUIRED_ENV.items() if k != "KRUXIAFLOW_WORKER"}
        with mock.patch.dict(os.environ, env, clear=True):
            config = WorkerConfig()
        assert config.worker == "py-std"

    def test_raises_on_missing_client_id(self):
        env = {
            k: v for k, v in self.REQUIRED_ENV.items() if k != "KRUXIAFLOW_CLIENT_ID"
        }
        with (
            mock.patch.dict(os.environ, env, clear=True),
            pytest.raises(ValidationError),
        ):
            WorkerConfig()

    def test_raises_on_missing_client_secret(self):
        env = {
            k: v
            for k, v in self.REQUIRED_ENV.items()
            if k != "KRUXIAFLOW_CLIENT_SECRET"
        }
        with (
            mock.patch.dict(os.environ, env, clear=True),
            pytest.raises(ValidationError),
        ):
            WorkerConfig()
