"""Tests for file executor."""

import tempfile
from collections.abc import AsyncIterator
from pathlib import Path
from unittest import mock
from uuid import uuid4

import pytest

from kruxiaflow.worker import FileExecutor
from kruxiaflow.worker.storage.errors import FileNotFoundError


class MockWorkerApiClient:
    """Mock WorkerApiClient for testing file operations."""

    def __init__(self):
        self.uploaded_files: dict[str, bytes] = {}
        self.download_data: dict[str, bytes] = {}

    async def upload_file(
        self,
        workflow_id,
        activity_key,
        filename,
        data: AsyncIterator[bytes],
        content_type=None,
    ):
        # Consume the async iterator
        chunks = []
        async for chunk in data:
            chunks.append(chunk)
        content = b"".join(chunks)
        key = f"{workflow_id}/{activity_key}/{filename}"
        self.uploaded_files[key] = content
        return mock.MagicMock(size=len(content))

    async def download_file(
        self, workflow_id, activity_key, filename
    ) -> AsyncIterator[bytes]:
        key = f"{workflow_id}/{activity_key}/{filename}"
        if key not in self.download_data:
            raise FileNotFoundError(key)
        data = self.download_data[key]
        # Yield in chunks
        chunk_size = 8192
        for i in range(0, len(data), chunk_size):
            yield data[i : i + chunk_size]


class TestFileExecutorBasic:
    """Test FileExecutor basic functionality."""

    def test_create_executor(self):
        workflow_id = uuid4()
        executor = FileExecutor(
            workflow_id=workflow_id,
            activity_key="test_activity",
        )
        assert executor.workflow_id == workflow_id
        assert executor.activity_key == "test_activity"
        assert executor._client is None

    def test_create_executor_with_client(self):
        client = MockWorkerApiClient()
        executor = FileExecutor(
            workflow_id=uuid4(),
            activity_key="test",
            client=client,
        )
        assert executor._client is client


class TestFileExecutorTempDir:
    """Test FileExecutor temp directory."""

    def test_temp_dir_created_on_access(self):
        executor = FileExecutor(
            workflow_id=uuid4(),
            activity_key="test",
        )
        # Initially None
        assert executor._temp_dir is None

        # Access creates it
        temp_dir = executor.temp_dir
        assert temp_dir.exists()
        assert temp_dir.is_dir()
        assert "kruxiaflow_" in str(temp_dir)

    def test_temp_dir_is_cached(self):
        executor = FileExecutor(
            workflow_id=uuid4(),
            activity_key="test",
        )
        temp_dir1 = executor.temp_dir
        temp_dir2 = executor.temp_dir
        assert temp_dir1 == temp_dir2

    @pytest.mark.asyncio
    async def test_cleanup_removes_temp_dir(self):
        executor = FileExecutor(
            workflow_id=uuid4(),
            activity_key="test",
        )
        temp_dir = executor.temp_dir
        assert temp_dir.exists()

        await executor.cleanup()

        assert not temp_dir.exists()
        assert executor._temp_dir is None

    @pytest.mark.asyncio
    async def test_cleanup_is_idempotent(self):
        executor = FileExecutor(
            workflow_id=uuid4(),
            activity_key="test",
        )
        _ = executor.temp_dir  # Create temp dir

        await executor.cleanup()
        await executor.cleanup()  # Should not raise


class TestFileExecutorDownload:
    """Test FileExecutor download functionality."""

    @pytest.mark.asyncio
    async def test_download_without_client_raises_error(self):
        executor = FileExecutor(
            workflow_id=uuid4(),
            activity_key="test",
        )

        with pytest.raises(RuntimeError, match="Client not configured"):
            await executor.download_file("postgres://uuid/activity/file.txt")

    @pytest.mark.asyncio
    async def test_download_file_from_api(self):
        workflow_id = uuid4()
        client = MockWorkerApiClient()
        client.download_data[f"{workflow_id}/source_activity/input.txt"] = (
            b"test content"
        )

        executor = FileExecutor(
            workflow_id=workflow_id,
            activity_key="test",
            client=client,
        )

        try:
            file_ref = f"postgres://{workflow_id}/source_activity/input.txt"
            local_path = await executor.download_file(file_ref)

            assert Path(local_path).exists()
            assert Path(local_path).name == "input.txt"
            assert Path(local_path).parent == executor.temp_dir

            with open(local_path, "rb") as f:
                content = f.read()
            assert content == b"test content"
        finally:
            await executor.cleanup()

    @pytest.mark.asyncio
    async def test_download_nonexistent_file_raises_error(self):
        client = MockWorkerApiClient()
        executor = FileExecutor(
            workflow_id=uuid4(),
            activity_key="test",
            client=client,
        )

        try:
            with pytest.raises(FileNotFoundError):
                await executor.download_file(
                    "postgres://00000000-0000-0000-0000-000000000000/activity/missing.txt"
                )
        finally:
            await executor.cleanup()


class TestFileExecutorUpload:
    """Test FileExecutor upload functionality."""

    @pytest.mark.asyncio
    async def test_upload_without_client_raises_error(self):
        executor = FileExecutor(
            workflow_id=uuid4(),
            activity_key="test",
        )

        with pytest.raises(RuntimeError, match="Client not configured"):
            await executor.upload_file("/local/path/file.csv", "result.csv")

    @pytest.mark.asyncio
    async def test_upload_file_via_api(self):
        workflow_id = uuid4()
        client = MockWorkerApiClient()
        executor = FileExecutor(
            workflow_id=workflow_id,
            activity_key="test_activity",
            client=client,
        )

        try:
            # Create a local file
            with tempfile.NamedTemporaryFile(
                mode="w", delete=False, suffix=".txt"
            ) as f:
                f.write("upload content")
                local_path = f.name

            storage_url = await executor.upload_file(local_path, "result.txt")

            assert storage_url == f"postgres://{workflow_id}/test_activity/result.txt"

            # Verify file was uploaded via client
            key = f"{workflow_id}/test_activity/result.txt"
            assert key in client.uploaded_files
            assert client.uploaded_files[key] == b"upload content"

            Path(local_path).unlink()
        finally:
            await executor.cleanup()

    @pytest.mark.asyncio
    async def test_upload_nonexistent_file_raises_error(self):
        import builtins

        client = MockWorkerApiClient()
        executor = FileExecutor(
            workflow_id=uuid4(),
            activity_key="test",
            client=client,
        )

        with pytest.raises(builtins.FileNotFoundError, match="Local file not found"):
            await executor.upload_file("/nonexistent/file.txt", "result.txt")

    @pytest.mark.asyncio
    async def test_upload_different_filenames(self):
        workflow_id = uuid4()
        client = MockWorkerApiClient()
        executor = FileExecutor(
            workflow_id=workflow_id,
            activity_key="activity1",
            client=client,
        )

        try:
            # Create local files
            with tempfile.NamedTemporaryFile(
                mode="w", delete=False, suffix=".txt"
            ) as f:
                f.write("content a")
                path_a = f.name
            with tempfile.NamedTemporaryFile(
                mode="w", delete=False, suffix=".txt"
            ) as f:
                f.write("content b")
                path_b = f.name

            url1 = await executor.upload_file(path_a, "file_a.txt")
            url2 = await executor.upload_file(path_b, "file_b.txt")

            assert "file_a.txt" in url1
            assert "file_b.txt" in url2

            Path(path_a).unlink()
            Path(path_b).unlink()
        finally:
            await executor.cleanup()


class TestFileExecutorIntegration:
    """Integration tests for FileExecutor."""

    @pytest.mark.asyncio
    async def test_download_and_upload_workflow(self):
        """Test a typical download -> process -> upload workflow."""
        workflow_id = uuid4()
        client = MockWorkerApiClient()

        # Pre-populate client with input file
        client.download_data[f"{workflow_id}/input_activity/input.txt"] = (
            b"line1\nline2\nline3\n"
        )

        executor = FileExecutor(
            workflow_id=workflow_id,
            activity_key="process_data",
            client=client,
        )

        try:
            # Download
            file_ref = f"postgres://{workflow_id}/input_activity/input.txt"
            local_input = await executor.download_file(file_ref)

            # Process (uppercase each line)
            with open(local_input) as f:
                lines = f.readlines()
            processed = [line.upper() for line in lines]

            output_path = executor.temp_dir / "output.txt"
            with open(output_path, "w") as f:
                f.writelines(processed)

            # Upload
            storage_url = await executor.upload_file(str(output_path), "output.txt")

            assert "output.txt" in storage_url

            # Verify uploaded content
            key = f"{workflow_id}/process_data/output.txt"
            assert key in client.uploaded_files
            assert client.uploaded_files[key] == b"LINE1\nLINE2\nLINE3\n"
        finally:
            await executor.cleanup()
