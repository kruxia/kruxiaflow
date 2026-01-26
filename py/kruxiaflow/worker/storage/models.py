"""Storage data models.

Mirrors Rust FileMetadata and FileReference for interface compatibility.
"""

import re
from datetime import datetime
from uuid import UUID

from pydantic import BaseModel

from ...types import ActivityKey, Filename
from .errors import InvalidFileReferenceError


class FileMetadata(BaseModel):
    """Metadata for a stored file."""

    workflow_id: UUID
    activity_key: ActivityKey
    filename: Filename
    size: int
    content_type: str | None
    created_at: datetime


class FileReference(BaseModel):
    """Reference to a file in storage.

    Format: {provider}://{workflow_id}/{activity_key}/{filename}
    Example: postgres://019353a1-b0c1-7000-8000-000000000001/step1/result.txt
    """

    workflow_id: UUID
    activity_key: ActivityKey
    filename: Filename

    def to_string(self, provider: str = "postgres") -> str:
        """Convert to string reference.

        Args:
            provider: Storage provider name (e.g., "postgres", "s3")

        Returns:
            File reference string
        """
        return f"{provider}://{self.workflow_id}/{self.activity_key}/{self.filename}"

    @classmethod
    def from_string(cls, reference: str) -> "FileReference":
        """Parse a file reference string.

        Args:
            reference: Reference string like "postgres://uuid/activity/filename"

        Returns:
            FileReference instance

        Raises:
            InvalidFileReferenceError: If reference format is invalid
        """
        # Pattern: provider://workflow_id/activity_key/filename
        pattern = r"^(\w+)://([0-9a-f-]+)/([^/]+)/(.+)$"
        match = re.match(pattern, reference, re.IGNORECASE)

        if not match:
            raise InvalidFileReferenceError(reference)

        try:
            workflow_id = UUID(match.group(2))
        except ValueError as e:
            raise InvalidFileReferenceError(reference) from e

        return cls(
            workflow_id=workflow_id,
            activity_key=match.group(3),
            filename=match.group(4),
        )

    @classmethod
    def parse_provider(cls, reference: str) -> str:
        """Extract provider from a file reference string.

        Args:
            reference: Reference string

        Returns:
            Provider name (e.g., "postgres", "s3")

        Raises:
            InvalidFileReferenceError: If reference format is invalid
        """
        pattern = r"^(\w+)://"
        match = re.match(pattern, reference)

        if not match:
            raise InvalidFileReferenceError(reference)

        return match.group(1)
