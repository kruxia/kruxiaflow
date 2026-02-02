"""Tests for workflow storage."""

from datetime import datetime, timezone
from uuid import UUID, uuid4

import pytest

from kruxiaflow.worker.storage import (
    FileMetadata,
    FileReference,
    InvalidFileReferenceError,
    StorageError,
    UploadFailedError,
)
from kruxiaflow.worker.storage.errors import DownloadFailedError, FileNotFoundError


class TestStorageErrors:
    """Test storage error types."""

    def test_storage_error_base(self):
        error = StorageError("base error")
        assert str(error) == "base error"

    def test_file_not_found_error(self):
        error = FileNotFoundError("uuid/activity/file.txt")
        assert "File not found" in str(error)
        assert error.file_ref == "uuid/activity/file.txt"

    def test_upload_failed_error(self):
        error = UploadFailedError("connection failed")
        assert "Upload failed" in str(error)
        assert "connection failed" in str(error)

    def test_download_failed_error(self):
        error = DownloadFailedError("timeout")
        assert "Download failed" in str(error)
        assert "timeout" in str(error)

    def test_invalid_file_reference_error(self):
        error = InvalidFileReferenceError("bad://ref")
        assert "Invalid file reference" in str(error)
        assert error.reference == "bad://ref"


class TestFileMetadata:
    """Test FileMetadata model."""

    def test_create_file_metadata(self):
        workflow_id = uuid4()
        created_at = datetime.now(timezone.utc)

        metadata = FileMetadata(
            workflow_id=workflow_id,
            activity_key="process_step",
            filename="output.txt",
            size=1024,
            content_type="text/plain",
            created_at=created_at,
        )

        assert metadata.workflow_id == workflow_id
        assert metadata.activity_key == "process_step"
        assert metadata.filename == "output.txt"
        assert metadata.size == 1024
        assert metadata.content_type == "text/plain"
        assert metadata.created_at == created_at

    def test_metadata_with_no_content_type(self):
        metadata = FileMetadata(
            workflow_id=uuid4(),
            activity_key="step1",
            filename="data.bin",
            size=500,
            content_type=None,
            created_at=datetime.now(timezone.utc),
        )

        assert metadata.content_type is None


class TestFileReference:
    """Test FileReference model."""

    def test_create_file_reference(self):
        workflow_id = uuid4()

        ref = FileReference(
            workflow_id=workflow_id,
            activity_key="step1",
            filename="output.json",
        )

        assert ref.workflow_id == workflow_id
        assert ref.activity_key == "step1"
        assert ref.filename == "output.json"

    def test_to_string_postgres(self):
        workflow_id = UUID("01234567-89ab-cdef-0123-456789abcdef")

        ref = FileReference(
            workflow_id=workflow_id,
            activity_key="process",
            filename="result.txt",
        )

        result = ref.to_string("postgres")
        assert (
            result
            == "postgres://01234567-89ab-cdef-0123-456789abcdef/process/result.txt"
        )

    def test_to_string_s3(self):
        workflow_id = UUID("01234567-89ab-cdef-0123-456789abcdef")

        ref = FileReference(
            workflow_id=workflow_id,
            activity_key="fetch",
            filename="data.csv",
        )

        result = ref.to_string("s3")
        assert result == "s3://01234567-89ab-cdef-0123-456789abcdef/fetch/data.csv"

    def test_to_string_default_provider(self):
        ref = FileReference(
            workflow_id=uuid4(),
            activity_key="step",
            filename="file.txt",
        )

        result = ref.to_string()
        assert result.startswith("postgres://")

    def test_from_string_valid_postgres(self):
        ref_str = (
            "postgres://01234567-89ab-cdef-0123-456789abcdef/activity_key/filename.txt"
        )

        ref = FileReference.from_string(ref_str)

        assert ref.workflow_id == UUID("01234567-89ab-cdef-0123-456789abcdef")
        assert ref.activity_key == "activity_key"
        assert ref.filename == "filename.txt"

    def test_from_string_valid_s3(self):
        ref_str = "s3://01234567-89ab-cdef-0123-456789abcdef/process/output.json"

        ref = FileReference.from_string(ref_str)

        assert ref.workflow_id == UUID("01234567-89ab-cdef-0123-456789abcdef")
        assert ref.activity_key == "process"
        assert ref.filename == "output.json"

    def test_from_string_filename_with_path(self):
        """Test that filenames can contain path separators."""
        ref_str = "postgres://01234567-89ab-cdef-0123-456789abcdef/step/subdir/file.txt"

        ref = FileReference.from_string(ref_str)

        assert ref.filename == "subdir/file.txt"

    def test_from_string_invalid_no_protocol(self):
        with pytest.raises(InvalidFileReferenceError):
            FileReference.from_string(
                "01234567-89ab-cdef-0123-456789abcdef/activity/file.txt"
            )

    def test_from_string_invalid_bad_uuid(self):
        with pytest.raises(InvalidFileReferenceError):
            FileReference.from_string("postgres://not-a-uuid/activity/file.txt")

    def test_from_string_uuid_matches_pattern_but_invalid(self):
        """Test UUID that matches the hex pattern but is invalid as UUID."""
        # This string matches the regex [0-9a-f-]+ but is not a valid UUID format
        # (too short - UUID requires 32 hex chars plus 4 dashes)
        with pytest.raises(InvalidFileReferenceError):
            FileReference.from_string("postgres://abcdef00-1234/activity/file.txt")

    def test_from_string_invalid_missing_parts(self):
        with pytest.raises(InvalidFileReferenceError):
            FileReference.from_string("postgres://01234567-89ab-cdef-0123-456789abcdef")

    def test_from_string_invalid_empty(self):
        with pytest.raises(InvalidFileReferenceError):
            FileReference.from_string("")

    def test_parse_provider_postgres(self):
        provider = FileReference.parse_provider(
            "postgres://01234567-89ab-cdef-0123-456789abcdef/step/file.txt"
        )
        assert provider == "postgres"

    def test_parse_provider_s3(self):
        provider = FileReference.parse_provider(
            "s3://01234567-89ab-cdef-0123-456789abcdef/step/file.txt"
        )
        assert provider == "s3"

    def test_parse_provider_invalid(self):
        with pytest.raises(InvalidFileReferenceError):
            FileReference.parse_provider("invalid-reference")

    def test_roundtrip(self):
        """Test that to_string -> from_string produces equivalent reference."""
        workflow_id = uuid4()
        original = FileReference(
            workflow_id=workflow_id,
            activity_key="my_activity",
            filename="result.json",
        )

        ref_str = original.to_string("postgres")
        parsed = FileReference.from_string(ref_str)

        assert parsed.workflow_id == original.workflow_id
        assert parsed.activity_key == original.activity_key
        assert parsed.filename == original.filename
