"""Tests for activity context."""

import logging
from unittest import mock
from uuid import uuid4

import pytest

from kruxiaflow.worker import ActivityContext


class TestActivityContextBasic:
    """Test ActivityContext basic functionality."""

    def test_create_context(self):
        workflow_id = uuid4()
        activity_id = uuid4()
        ctx = ActivityContext(
            workflow_id=workflow_id,
            activity_id=activity_id,
            activity_key="test_activity",
        )
        assert ctx.workflow_id == workflow_id
        assert ctx.activity_id == activity_id
        assert ctx.activity_key == "test_activity"

    def test_context_ids_are_uuid(self):
        ctx = ActivityContext(
            workflow_id=uuid4(),
            activity_id=uuid4(),
            activity_key="test",
        )
        # UUIDs have specific string representation
        assert len(str(ctx.workflow_id)) == 36
        assert len(str(ctx.activity_id)) == 36


class TestActivityContextLogger:
    """Test ActivityContext logger functionality."""

    def test_logger_property(self):
        ctx = ActivityContext(
            workflow_id=uuid4(),
            activity_id=uuid4(),
            activity_key="my_activity",
        )
        logger = ctx.logger
        assert isinstance(logger, logging.Logger)
        assert logger.name == "kruxiaflow.activity.my_activity"

    def test_logger_is_cached(self):
        ctx = ActivityContext(
            workflow_id=uuid4(),
            activity_id=uuid4(),
            activity_key="test",
        )
        logger1 = ctx.logger
        logger2 = ctx.logger
        assert logger1 is logger2


class TestActivityContextHeartbeat:
    """Test ActivityContext heartbeat functionality."""

    @pytest.mark.asyncio
    async def test_heartbeat_without_client(self):
        ctx = ActivityContext(
            workflow_id=uuid4(),
            activity_id=uuid4(),
            activity_key="test",
        )
        # Should not raise even without client
        await ctx.heartbeat()

    @pytest.mark.asyncio
    async def test_heartbeat_with_client(self):
        activity_id = uuid4()
        ctx = ActivityContext(
            workflow_id=uuid4(),
            activity_id=activity_id,
            activity_key="test",
        )

        # Mock client
        mock_client = mock.AsyncMock()
        ctx._client = mock_client
        ctx._worker_id = "worker_123"

        await ctx.heartbeat()

        mock_client.heartbeat.assert_called_once_with(activity_id, "worker_123")

    @pytest.mark.asyncio
    async def test_heartbeat_without_worker_id(self):
        ctx = ActivityContext(
            workflow_id=uuid4(),
            activity_id=uuid4(),
            activity_key="test",
        )
        mock_client = mock.AsyncMock()
        ctx._client = mock_client
        # No worker_id set

        # Should not call heartbeat
        await ctx.heartbeat()
        mock_client.heartbeat.assert_not_called()


class TestActivityContextFileOperations:
    """Test ActivityContext file operations."""

    @pytest.mark.asyncio
    async def test_download_file_without_executor(self):
        ctx = ActivityContext(
            workflow_id=uuid4(),
            activity_id=uuid4(),
            activity_key="test",
        )
        with pytest.raises(RuntimeError, match="File operations not available"):
            await ctx.download_file("some/path")

    @pytest.mark.asyncio
    async def test_upload_file_without_executor(self):
        ctx = ActivityContext(
            workflow_id=uuid4(),
            activity_id=uuid4(),
            activity_key="test",
        )
        with pytest.raises(RuntimeError, match="File operations not available"):
            await ctx.upload_file("/local/path", "file.txt")

    @pytest.mark.asyncio
    async def test_download_file_with_executor(self):
        ctx = ActivityContext(
            workflow_id=uuid4(),
            activity_id=uuid4(),
            activity_key="test",
        )

        mock_executor = mock.AsyncMock()
        mock_executor.download_file.return_value = "/tmp/downloaded.txt"
        ctx._file_executor = mock_executor

        result = await ctx.download_file("storage://path/to/file.txt")

        mock_executor.download_file.assert_called_once_with(
            "storage://path/to/file.txt"
        )
        assert result == "/tmp/downloaded.txt"

    @pytest.mark.asyncio
    async def test_upload_file_with_executor(self):
        ctx = ActivityContext(
            workflow_id=uuid4(),
            activity_id=uuid4(),
            activity_key="test",
        )

        mock_executor = mock.AsyncMock()
        mock_executor.upload_file.return_value = (
            "postgres://workflow/activity/uploaded.txt"
        )
        ctx._file_executor = mock_executor

        result = await ctx.upload_file("/local/file.txt", "uploaded.txt")

        mock_executor.upload_file.assert_called_once_with(
            "/local/file.txt", "uploaded.txt"
        )
        assert result == "postgres://workflow/activity/uploaded.txt"
