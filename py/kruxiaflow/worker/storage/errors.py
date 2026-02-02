"""Storage error types.

Mirrors Rust StorageError for interface compatibility.
"""


class StorageError(Exception):
    """Base exception for storage operations."""


class FileNotFoundError(StorageError):
    """File not found in storage."""

    def __init__(self, file_ref: str):
        super().__init__(f"File not found: {file_ref}")
        self.file_ref = file_ref


class UploadFailedError(StorageError):
    """File upload failed."""

    def __init__(self, message: str):
        super().__init__(f"Upload failed: {message}")


class DownloadFailedError(StorageError):
    """File download failed."""

    def __init__(self, message: str):
        super().__init__(f"Download failed: {message}")


class InvalidFileReferenceError(StorageError):
    """Invalid file reference format."""

    def __init__(self, reference: str):
        super().__init__(f"Invalid file reference: {reference}")
        self.reference = reference
