"""
Kruxia Flow Python Worker SDK.

This module provides the worker SDK for implementing custom Python activities
that can be executed by Kruxia Flow workflows.

Example usage:
    from kruxiaflow.worker import (
        WorkerConfig,
        WorkerManager,
        ActivityRegistry,
        Activity,
        ActivityResult,
        ActivityContext,
        activity,
    )

    # Define activity using decorator
    @activity(name="echo")
    async def echo_activity(params: dict, ctx: ActivityContext) -> ActivityResult:
        return ActivityResult.value("output", params.get("input", ""))

    # Or define activity as class
    class MyActivity(Activity):
        @property
        def name(self) -> str:
            return "my_activity"

        async def execute(self, params: dict, ctx: ActivityContext) -> ActivityResult:
            return ActivityResult.value("result", params["value"] * 2)

    # Load config from environment variables and create registry
    config = WorkerConfig()
    registry = ActivityRegistry()

    # Register activities with worker type from config
    registry.register(echo_activity, config.worker)
    registry.register(MyActivity(), config.worker)

    # Create and run worker
    manager = WorkerManager(config, registry)

    import asyncio
    asyncio.run(manager.run_until_shutdown())
"""

from .activity import (
    Activity,
    ActivityOutput,
    ActivityResult,
    OutputType,
    activity,
)
from .client import PendingActivity, WorkerApiClient
from .config import WorkerConfig
from .context import ActivityContext
from .errors import (
    ActivityExecutionError,
    ActivityNotFoundError,
    ActivityTimeoutError,
    AuthenticationError,
    ConfigError,
    FileOperationError,
    WorkerError,
)
from .file_executor import FileExecutor
from .manager import WorkerManager
from .poller import WorkerPoller
from .registry import ActivityRegistry
from .storage import (
    FileMetadata,
    FileNotFoundError,
    FileReference,
    InvalidFileReferenceError,
    StorageError,
    UploadFailedError,
)

__all__ = [
    "Activity",
    "ActivityContext",
    "ActivityExecutionError",
    "ActivityNotFoundError",
    "ActivityOutput",
    "ActivityRegistry",
    "ActivityResult",
    "ActivityTimeoutError",
    "AuthenticationError",
    "ConfigError",
    "FileExecutor",
    "FileMetadata",
    "FileNotFoundError",
    "FileOperationError",
    "FileReference",
    "InvalidFileReferenceError",
    "OutputType",
    "PendingActivity",
    "StorageError",
    "UploadFailedError",
    "WorkerApiClient",
    "WorkerConfig",
    "WorkerError",
    "WorkerManager",
    "WorkerPoller",
    "activity",
]
