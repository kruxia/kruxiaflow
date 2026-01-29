"""Worker configuration.

Mirrors Rust WorkerConfig struct for interface compatibility.
"""

from uuid import uuid4

from pydantic import AliasChoices, Field, HttpUrl
from pydantic_settings import BaseSettings, SettingsConfigDict

from ..types import WorkerSlug


def _default_worker_id() -> str:
    return f"worker_{uuid4().hex[:12]}"


class WorkerConfig(BaseSettings):
    """
    Worker configuration.

    Mirrors Rust WorkerConfig struct for interface compatibility.
    All field names match Rust exactly for future PyO3 migration.

    Uses Pydantic BaseSettings for automatic environment variable loading.
    Environment variables are prefixed with KRUXIAFLOW_.

    Required environment variables:
        KRUXIAFLOW_API_URL: API server base URL (e.g., "http://localhost:8080")
        KRUXIAFLOW_CLIENT_ID: OAuth client ID
        KRUXIAFLOW_CLIENT_SECRET: OAuth client secret

    Optional environment variables:
        KRUXIAFLOW_WORKER_ID: Worker unique identifier (default: auto-generated)
        KRUXIAFLOW_WORKER_POLL_MAX_ACTIVITIES: Max activities per poll (default: 10)
        KRUXIAFLOW_WORKER_POLL_INTERVAL: Poll interval in seconds (default: 0.1)
        KRUXIAFLOW_WORKER_MAX_ACTIVITIES: Max concurrent activities (default: 16)
        KRUXIAFLOW_WORKER_ACTIVITY_TIMEOUT: Activity timeout in seconds (default: 300)
        KRUXIAFLOW_WORKER_HEARTBEAT_INTERVAL: Heartbeat interval in seconds (default: 30)
    """

    model_config = SettingsConfigDict(
        env_prefix="KRUXIAFLOW_",
        env_nested_delimiter="__",
        extra="ignore",
    )

    # API server base URL - required, validated as URL
    api_url: HttpUrl

    # Worker unique identifier
    worker_id: str = Field(default_factory=_default_worker_id)

    # Worker type (e.g., "py-std", "py-data") - defaults to "py-std",
    # can be overridden by KRUXIAFLOW_WORKER or WORKER_TYPE env var
    worker: WorkerSlug = "py-std"

    # Maximum activities to poll per request
    poll_max_activities: int = Field(
        default=10,
        ge=1,
        validation_alias=AliasChoices(
            "poll_max_activities",
            "KRUXIAFLOW_WORKER_POLL_MAX_ACTIVITIES",
        ),
    )

    # Polling interval when no work (seconds)
    poll_interval: float = Field(
        default=0.1,
        validation_alias=AliasChoices(
            "poll_interval",
            "KRUXIAFLOW_WORKER_POLL_INTERVAL",
        ),
    )

    # Maximum concurrent activities (semaphore limit)
    max_concurrent_activities: int = Field(
        default=16,
        ge=1,
        validation_alias=AliasChoices(
            "max_concurrent_activities",
            "KRUXIAFLOW_WORKER_MAX_ACTIVITIES",
        ),
    )

    # Default activity timeout (seconds)
    activity_timeout: float = Field(
        default=300.0,
        validation_alias=AliasChoices(
            "activity_timeout",
            "KRUXIAFLOW_WORKER_ACTIVITY_TIMEOUT",
        ),
    )

    # Heartbeat interval for long tasks (seconds)
    heartbeat_interval: float = Field(
        default=30.0,
        validation_alias=AliasChoices(
            "heartbeat_interval",
            "KRUXIAFLOW_WORKER_HEARTBEAT_INTERVAL",
        ),
    )

    # OAuth credentials - required, non-empty
    client_id: str = Field(min_length=1)
    client_secret: str = Field(min_length=1)
