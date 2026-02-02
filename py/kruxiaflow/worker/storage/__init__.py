"""Storage models and errors for file operations.

Provides models for file references and metadata used when
activities upload/download files via the Kruxia Flow API.

Note: Workers access file storage through the API, not directly.
PostgresStorage is only used by the API server itself.
"""

from .errors import (
    FileNotFoundError,
    InvalidFileReferenceError,
    StorageError,
    UploadFailedError,
)
from .models import FileMetadata, FileReference

__all__ = [
    "FileMetadata",
    "FileNotFoundError",
    "FileReference",
    "InvalidFileReferenceError",
    "StorageError",
    "UploadFailedError",
]
