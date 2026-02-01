"""Activity execution context.

Mirrors Rust ActivityContext for interface compatibility.
"""

from __future__ import annotations

import logging
from typing import TYPE_CHECKING, Any
from uuid import UUID

from pydantic import BaseModel, ConfigDict, PrivateAttr

from ..types import ActivityKey

if TYPE_CHECKING:
    from .client import WorkerApiClient
    from .file_executor import FileExecutor


class ActivityContext(BaseModel):
    """
    Context passed to activity handlers.

    Mirrors Rust ActivityContext for interface compatibility.
    Provides workflow/activity IDs, logging, heartbeat, and file operations.
    """

    model_config = ConfigDict(arbitrary_types_allowed=True)

    workflow_id: UUID
    activity_id: UUID
    activity_key: ActivityKey

    # Signal data (for activities that waited for an external signal)
    signal: dict[str, Any] | None = None
    """Data received from an external signal (when wait_for_signal is configured)."""

    # Internal references (not part of public interface)
    _client: WorkerApiClient | None = PrivateAttr(default=None)
    _worker_id: str | None = PrivateAttr(default=None)
    _file_executor: FileExecutor | None = PrivateAttr(default=None)
    _logger: logging.Logger | None = PrivateAttr(default=None)

    @property
    def logger(self) -> logging.Logger:
        """Get logger for this activity."""
        if self._logger is None:
            self._logger = logging.getLogger(f"kruxiaflow.activity.{self.activity_key}")
        return self._logger

    async def heartbeat(self) -> None:
        """
        Send heartbeat to prevent timeout.

        Call this periodically during long-running operations.
        The worker automatically sends heartbeats for activities with
        timeout > 60s, but manual heartbeats provide finer control.
        """
        if self._client and self._worker_id:
            await self._client.heartbeat(self.activity_id, self._worker_id)

    async def download_file(self, storage_path: str) -> str:
        """
        Download file from workflow storage to local temp directory.

        Args:
            storage_path: Storage path (e.g., "{workflow_id}/{activity_key}/file.txt")

        Returns:
            Local file path in temp directory
        """
        if not self._file_executor:
            raise RuntimeError("File operations not available (no storage configured)")
        return await self._file_executor.download_file(storage_path)

    async def upload_file(
        self,
        local_path: str,
        filename: str,
    ) -> str:
        """
        Upload file from local path to workflow storage.

        Args:
            local_path: Local file path
            filename: Target filename in storage

        Returns:
            Storage URL for the uploaded file
        """
        if not self._file_executor:
            raise RuntimeError("File operations not available (no storage configured)")
        return await self._file_executor.upload_file(local_path, filename)
