"""File upload/download handler.

Mirrors Rust FileExecutor for interface compatibility.

Uses the Kruxia Flow API for file storage operations (not direct database access).
"""

from __future__ import annotations

import shutil
import tempfile
from collections.abc import AsyncIterator
from pathlib import Path
from typing import TYPE_CHECKING
from uuid import UUID

from .storage import FileReference

if TYPE_CHECKING:
    from .client import WorkerApiClient

# Chunk size for file I/O (8KB, matches API streaming)
CHUNK_SIZE = 8192


class FileExecutor:
    """File upload/download handler for activities.

    Manages a temporary directory for activity file I/O and
    provides methods to download files from storage and upload
    results back via the Kruxia Flow API.

    Mirrors Rust FileExecutor for interface compatibility.
    """

    def __init__(
        self,
        workflow_id: UUID,
        activity_key: str,
        client: WorkerApiClient | None = None,
    ):
        """Create FileExecutor.

        Args:
            workflow_id: Workflow instance ID
            activity_key: Activity key within workflow
            client: WorkerApiClient for API access (optional for testing)
        """
        self._workflow_id = workflow_id
        self._activity_key = activity_key
        self._client = client
        self._temp_dir: Path | None = None

    @property
    def temp_dir(self) -> Path:
        """Get temp directory, creating if needed."""
        if self._temp_dir is None:
            self._temp_dir = Path(tempfile.mkdtemp(prefix="kruxiaflow_"))
        return self._temp_dir

    @property
    def workflow_id(self) -> UUID:
        """Get workflow ID."""
        return self._workflow_id

    @property
    def activity_key(self) -> str:
        """Get activity key."""
        return self._activity_key

    async def download_file(self, file_ref: str) -> str:
        """Download file from storage to temp directory.

        Args:
            file_ref: File reference (e.g., "postgres://uuid/activity/file.txt")

        Returns:
            Local file path

        Raises:
            RuntimeError: If client is not configured
            FileNotFoundError: If file doesn't exist in storage
        """
        if self._client is None:
            raise RuntimeError("Client not configured for file operations")

        # Parse file reference
        ref = FileReference.from_string(file_ref)

        # Determine local path
        local_path = self.temp_dir / ref.filename

        # Stream download to local file
        with open(local_path, "wb") as f:
            async for chunk in self._client.download_file(
                ref.workflow_id,
                ref.activity_key,
                ref.filename,
            ):
                f.write(chunk)

        return str(local_path)

    async def upload_file(
        self,
        local_path: str,
        filename: str,
        content_type: str | None = None,
    ) -> str:
        """Upload file to storage.

        Args:
            local_path: Local file path
            filename: Target filename in storage
            content_type: Optional MIME type

        Returns:
            Storage file reference (e.g., "postgres://uuid/activity/file.txt")

        Raises:
            RuntimeError: If client is not configured
            FileNotFoundError: If local file doesn't exist
        """
        if self._client is None:
            raise RuntimeError("Client not configured for file operations")

        path = Path(local_path)
        if not path.exists():
            raise FileNotFoundError(f"Local file not found: {local_path}")

        # Create async file chunk generator
        async def file_chunks() -> AsyncIterator[bytes]:
            with open(path, "rb") as f:
                while True:
                    chunk = f.read(CHUNK_SIZE)
                    if not chunk:
                        break
                    yield chunk

        # Upload via API
        await self._client.upload_file(
            self._workflow_id,
            self._activity_key,
            filename,
            file_chunks(),
            content_type,
        )

        # Return file reference
        ref = FileReference(
            workflow_id=self._workflow_id,
            activity_key=self._activity_key,
            filename=filename,
        )
        return ref.to_string("postgres")

    async def cleanup(self) -> None:
        """Remove temp directory."""
        if self._temp_dir and self._temp_dir.exists():
            shutil.rmtree(self._temp_dir)
            self._temp_dir = None
