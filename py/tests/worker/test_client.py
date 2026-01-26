"""Tests for worker API client."""

from decimal import Decimal
from uuid import uuid4

import httpx
import pytest
from pytest_httpx import HTTPXMock

from kruxiaflow.worker import AuthenticationError, PendingActivity, WorkerApiClient


class TestPendingActivity:
    """Test PendingActivity model."""

    def test_create_pending_activity(self):
        activity_id = uuid4()
        workflow_id = uuid4()

        activity = PendingActivity(
            activity_id=activity_id,
            workflow_id=workflow_id,
            activity_key="step_1",
            worker="python",
            activity_name="process_data",
            parameters={"input": "value"},
        )

        assert activity.activity_id == activity_id
        assert activity.workflow_id == workflow_id
        assert activity.activity_key == "step_1"
        assert activity.worker == "python"
        assert activity.activity_name == "process_data"
        assert activity.parameters == {"input": "value"}

    def test_optional_fields_default_to_none(self):
        activity = PendingActivity(
            activity_id=uuid4(),
            workflow_id=uuid4(),
            activity_key="step_1",
            worker="python",
            activity_name="test",
            parameters={},
        )

        assert activity.settings is None
        assert activity.timeout_seconds is None
        assert activity.output_definitions is None

    def test_optional_fields_with_values(self):
        activity = PendingActivity(
            activity_id=uuid4(),
            workflow_id=uuid4(),
            activity_key="step_1",
            worker="python",
            activity_name="test",
            parameters={},
            settings={"retry": True},
            timeout_seconds=300,
            output_definitions=[{"name": "result", "type": "value"}],
        )

        assert activity.settings == {"retry": True}
        assert activity.timeout_seconds == 300
        assert activity.output_definitions == [{"name": "result", "type": "value"}]


class TestWorkerApiClientToken:
    """Test WorkerApiClient token management."""

    @pytest.mark.asyncio
    async def test_obtain_token(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/oauth/token",
            json={"access_token": "test_token_123"},
        )

        client = WorkerApiClient(
            api_url="http://localhost:8080",
            client_id="test_client",
            client_secret="test_secret",
        )

        try:
            token = await client._obtain_token()
            assert token == "test_token_123"
        finally:
            await client.close()

    @pytest.mark.asyncio
    async def test_obtain_token_failure(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/oauth/token",
            status_code=401,
            text="Invalid credentials",
        )

        client = WorkerApiClient(
            api_url="http://localhost:8080",
            client_id="bad_client",
            client_secret="bad_secret",
        )

        try:
            with pytest.raises(AuthenticationError, match="Token request failed"):
                await client._obtain_token()
        finally:
            await client.close()

    @pytest.mark.asyncio
    async def test_get_token_caches_token(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/oauth/token",
            json={"access_token": "cached_token"},
        )

        client = WorkerApiClient(
            api_url="http://localhost:8080",
            client_id="test",
            client_secret="secret",
        )

        try:
            token1 = await client._get_token()
            token2 = await client._get_token()

            assert token1 == token2 == "cached_token"
            # Should only have made one request
            assert len(httpx_mock.get_requests()) == 1
        finally:
            await client.close()


class TestWorkerApiClientPoll:
    """Test WorkerApiClient poll activities."""

    @pytest.mark.asyncio
    async def test_poll_activities_empty(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/oauth/token",
            json={"access_token": "token"},
        )
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/workers/poll",
            json={"activities": [], "count": 0},
        )

        client = WorkerApiClient(
            api_url="http://localhost:8080",
            client_id="test",
            client_secret="secret",
        )

        try:
            activities = await client.poll_activities(
                worker="python",
                worker_id="worker_123",
                max_activities=10,
            )
            assert activities == []
        finally:
            await client.close()

    @pytest.mark.asyncio
    async def test_poll_activities_returns_activities(self, httpx_mock: HTTPXMock):
        activity_id = str(uuid4())
        workflow_id = str(uuid4())

        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/oauth/token",
            json={"access_token": "token"},
        )
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/workers/poll",
            json={
                "activities": [
                    {
                        "activity_id": activity_id,
                        "workflow_id": workflow_id,
                        "activity_key": "step_1",
                        "worker": "python",
                        "activity_name": "process",
                        "parameters": {"key": "value"},
                    }
                ],
                "count": 1,
            },
        )

        client = WorkerApiClient(
            api_url="http://localhost:8080",
            client_id="test",
            client_secret="secret",
        )

        try:
            activities = await client.poll_activities(
                worker="python",
                worker_id="worker_123",
                max_activities=10,
            )

            assert len(activities) == 1
            assert str(activities[0].activity_id) == activity_id
            assert str(activities[0].workflow_id) == workflow_id
            assert activities[0].activity_key == "step_1"
            assert activities[0].parameters == {"key": "value"}
        finally:
            await client.close()

    @pytest.mark.asyncio
    async def test_poll_activities_refreshes_token_on_401(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/oauth/token",
            json={"access_token": "old_token"},
        )
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/workers/poll",
            status_code=401,
        )
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/oauth/token",
            json={"access_token": "new_token"},
        )
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/workers/poll",
            json={"activities": [], "count": 0},
        )

        client = WorkerApiClient(
            api_url="http://localhost:8080",
            client_id="test",
            client_secret="secret",
        )

        try:
            activities = await client.poll_activities(
                worker="python",
                worker_id="worker_123",
                max_activities=10,
            )
            assert activities == []

            # Verify we got two token requests (original + refresh)
            token_requests = [
                r for r in httpx_mock.get_requests() if "oauth/token" in str(r.url)
            ]
            assert len(token_requests) == 2
        finally:
            await client.close()


class TestWorkerApiClientHeartbeat:
    """Test WorkerApiClient heartbeat."""

    @pytest.mark.asyncio
    async def test_heartbeat_success(self, httpx_mock: HTTPXMock):
        activity_id = uuid4()

        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/oauth/token",
            json={"access_token": "token"},
        )
        httpx_mock.add_response(
            method="POST",
            url=f"http://localhost:8080/api/v1/activities/{activity_id}/heartbeat",
            json={},
        )

        client = WorkerApiClient(
            api_url="http://localhost:8080",
            client_id="test",
            client_secret="secret",
        )

        try:
            await client.heartbeat(activity_id, "worker_123")

            heartbeat_request = next(
                r for r in httpx_mock.get_requests() if "heartbeat" in str(r.url)
            )
            assert heartbeat_request.method == "POST"
        finally:
            await client.close()


class TestWorkerApiClientComplete:
    """Test WorkerApiClient complete activity."""

    @pytest.mark.asyncio
    async def test_complete_activity_success(self, httpx_mock: HTTPXMock):
        activity_id = uuid4()

        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/oauth/token",
            json={"access_token": "token"},
        )
        httpx_mock.add_response(
            method="POST",
            url=f"http://localhost:8080/api/v1/activities/{activity_id}/complete",
            json={},
        )

        client = WorkerApiClient(
            api_url="http://localhost:8080",
            client_id="test",
            client_secret="secret",
        )

        try:
            await client.complete_activity(
                activity_id=activity_id,
                worker_id="worker_123",
                output={"result": "success"},
            )

            complete_request = next(
                r for r in httpx_mock.get_requests() if "complete" in str(r.url)
            )
            assert complete_request.method == "POST"
        finally:
            await client.close()

    @pytest.mark.asyncio
    async def test_complete_activity_with_cost(self, httpx_mock: HTTPXMock):
        activity_id = uuid4()

        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/oauth/token",
            json={"access_token": "token"},
        )
        httpx_mock.add_response(
            method="POST",
            url=f"http://localhost:8080/api/v1/activities/{activity_id}/complete",
            json={},
        )

        client = WorkerApiClient(
            api_url="http://localhost:8080",
            client_id="test",
            client_secret="secret",
        )

        try:
            await client.complete_activity(
                activity_id=activity_id,
                worker_id="worker_123",
                output={"result": "success"},
                cost_usd=Decimal("0.05"),
            )

            complete_request = next(
                r for r in httpx_mock.get_requests() if "complete" in str(r.url)
            )
            import json

            body = json.loads(complete_request.content)
            assert body["cost_usd"] == "0.05"
        finally:
            await client.close()


class TestWorkerApiClientFail:
    """Test WorkerApiClient fail activity."""

    @pytest.mark.asyncio
    async def test_fail_activity_success(self, httpx_mock: HTTPXMock):
        activity_id = uuid4()

        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/oauth/token",
            json={"access_token": "token"},
        )
        httpx_mock.add_response(
            method="POST",
            url=f"http://localhost:8080/api/v1/activities/{activity_id}/fail",
            json={},
        )

        client = WorkerApiClient(
            api_url="http://localhost:8080",
            client_id="test",
            client_secret="secret",
        )

        try:
            await client.fail_activity(
                activity_id=activity_id,
                worker_id="worker_123",
                error_code="TEST_ERROR",
                error_message="Something went wrong",
                retryable=True,
            )

            fail_request = next(
                r for r in httpx_mock.get_requests() if "fail" in str(r.url)
            )
            import json

            body = json.loads(fail_request.content)
            assert body["error"]["code"] == "TEST_ERROR"
            assert body["error"]["message"] == "Something went wrong"
            assert body["error"]["retryable"] is True
        finally:
            await client.close()

    @pytest.mark.asyncio
    async def test_fail_activity_non_retryable(self, httpx_mock: HTTPXMock):
        activity_id = uuid4()

        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/oauth/token",
            json={"access_token": "token"},
        )
        httpx_mock.add_response(
            method="POST",
            url=f"http://localhost:8080/api/v1/activities/{activity_id}/fail",
            json={},
        )

        client = WorkerApiClient(
            api_url="http://localhost:8080",
            client_id="test",
            client_secret="secret",
        )

        try:
            await client.fail_activity(
                activity_id=activity_id,
                worker_id="worker_123",
                error_code="INVALID_INPUT",
                error_message="Bad input",
                retryable=False,
            )

            fail_request = next(
                r for r in httpx_mock.get_requests() if "fail" in str(r.url)
            )
            import json

            body = json.loads(fail_request.content)
            assert body["error"]["retryable"] is False
        finally:
            await client.close()


class TestWorkerApiClientUploadFile:
    """Test WorkerApiClient upload_file."""

    @pytest.mark.asyncio
    async def test_upload_file_success(self, httpx_mock: HTTPXMock):
        workflow_id = uuid4()

        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/oauth/token",
            json={"access_token": "token"},
        )
        httpx_mock.add_response(
            method="POST",
            url=f"http://localhost:8080/api/v1/workflows/{workflow_id}/activities/step_1/files/output.txt",
            json={"size": 11},
        )

        client = WorkerApiClient(
            api_url="http://localhost:8080",
            client_id="test",
            client_secret="secret",
        )

        try:

            async def data_chunks():
                yield b"hello world"

            metadata = await client.upload_file(
                workflow_id=workflow_id,
                activity_key="step_1",
                filename="output.txt",
                data=data_chunks(),
            )

            assert metadata.workflow_id == workflow_id
            assert metadata.activity_key == "step_1"
            assert metadata.filename == "output.txt"
            assert metadata.size == 11
        finally:
            await client.close()

    @pytest.mark.asyncio
    async def test_upload_file_with_content_type(self, httpx_mock: HTTPXMock):
        workflow_id = uuid4()

        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/oauth/token",
            json={"access_token": "token"},
        )
        httpx_mock.add_response(
            method="POST",
            url=f"http://localhost:8080/api/v1/workflows/{workflow_id}/activities/step_1/files/data.json",
            json={"size": 13},
        )

        client = WorkerApiClient(
            api_url="http://localhost:8080",
            client_id="test",
            client_secret="secret",
        )

        try:

            async def data_chunks():
                yield b'{"key":"val"}'

            metadata = await client.upload_file(
                workflow_id=workflow_id,
                activity_key="step_1",
                filename="data.json",
                data=data_chunks(),
                content_type="application/json",
            )

            assert metadata.content_type == "application/json"

            # Verify content type was sent in header
            upload_request = next(
                r for r in httpx_mock.get_requests() if "files" in str(r.url)
            )
            assert upload_request.headers.get("content-type") == "application/json"
        finally:
            await client.close()

    @pytest.mark.asyncio
    async def test_upload_file_refreshes_token_on_401(self, httpx_mock: HTTPXMock):
        workflow_id = uuid4()

        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/oauth/token",
            json={"access_token": "old_token"},
        )
        httpx_mock.add_response(
            method="POST",
            url=f"http://localhost:8080/api/v1/workflows/{workflow_id}/activities/step_1/files/output.txt",
            status_code=401,
        )
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/oauth/token",
            json={"access_token": "new_token"},
        )
        httpx_mock.add_response(
            method="POST",
            url=f"http://localhost:8080/api/v1/workflows/{workflow_id}/activities/step_1/files/output.txt",
            json={"size": 5},
        )

        client = WorkerApiClient(
            api_url="http://localhost:8080",
            client_id="test",
            client_secret="secret",
        )

        try:

            async def data_chunks():
                yield b"hello"

            metadata = await client.upload_file(
                workflow_id=workflow_id,
                activity_key="step_1",
                filename="output.txt",
                data=data_chunks(),
            )

            assert metadata.size == 5

            # Verify two token requests were made
            token_requests = [
                r for r in httpx_mock.get_requests() if "oauth/token" in str(r.url)
            ]
            assert len(token_requests) == 2
        finally:
            await client.close()


class TestWorkerApiClientDownloadFile:
    """Test WorkerApiClient download_file."""

    @pytest.mark.asyncio
    async def test_download_file_success(self, httpx_mock: HTTPXMock):
        workflow_id = uuid4()

        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/oauth/token",
            json={"access_token": "token"},
        )
        httpx_mock.add_response(
            method="GET",
            url=f"http://localhost:8080/api/v1/workflows/{workflow_id}/activities/step_1/files/input.txt",
            content=b"file content here",
        )

        client = WorkerApiClient(
            api_url="http://localhost:8080",
            client_id="test",
            client_secret="secret",
        )

        try:
            chunks = []
            async for chunk in client.download_file(
                workflow_id=workflow_id,
                activity_key="step_1",
                filename="input.txt",
            ):
                chunks.append(chunk)

            assert b"".join(chunks) == b"file content here"
        finally:
            await client.close()

    @pytest.mark.asyncio
    async def test_download_file_not_found(self, httpx_mock: HTTPXMock):
        from kruxiaflow.worker.storage.errors import FileNotFoundError

        workflow_id = uuid4()

        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/oauth/token",
            json={"access_token": "token"},
        )
        httpx_mock.add_response(
            method="GET",
            url=f"http://localhost:8080/api/v1/workflows/{workflow_id}/activities/step_1/files/missing.txt",
            status_code=404,
        )

        client = WorkerApiClient(
            api_url="http://localhost:8080",
            client_id="test",
            client_secret="secret",
        )

        try:
            with pytest.raises(FileNotFoundError):
                async for _ in client.download_file(
                    workflow_id=workflow_id,
                    activity_key="step_1",
                    filename="missing.txt",
                ):
                    pass
        finally:
            await client.close()

    @pytest.mark.asyncio
    async def test_download_file_refreshes_token_on_401(self, httpx_mock: HTTPXMock):
        workflow_id = uuid4()

        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/oauth/token",
            json={"access_token": "old_token"},
        )
        httpx_mock.add_response(
            method="GET",
            url=f"http://localhost:8080/api/v1/workflows/{workflow_id}/activities/step_1/files/input.txt",
            status_code=401,
        )
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/oauth/token",
            json={"access_token": "new_token"},
        )
        httpx_mock.add_response(
            method="GET",
            url=f"http://localhost:8080/api/v1/workflows/{workflow_id}/activities/step_1/files/input.txt",
            content=b"content after retry",
        )

        client = WorkerApiClient(
            api_url="http://localhost:8080",
            client_id="test",
            client_secret="secret",
        )

        try:
            chunks = []
            async for chunk in client.download_file(
                workflow_id=workflow_id,
                activity_key="step_1",
                filename="input.txt",
            ):
                chunks.append(chunk)

            assert b"".join(chunks) == b"content after retry"

            # Verify two token requests were made
            token_requests = [
                r for r in httpx_mock.get_requests() if "oauth/token" in str(r.url)
            ]
            assert len(token_requests) == 2
        finally:
            await client.close()


class TestWorkerApiClientCleanup:
    """Test WorkerApiClient cleanup."""

    @pytest.mark.asyncio
    async def test_close_client(self, httpx_mock: HTTPXMock):
        client = WorkerApiClient(
            api_url="http://localhost:8080",
            client_id="test",
            client_secret="secret",
        )

        await client.close()
        # Should not raise

    @pytest.mark.asyncio
    async def test_url_trailing_slash_stripped(self):
        client = WorkerApiClient(
            api_url="http://localhost:8080/",
            client_id="test",
            client_secret="secret",
        )

        assert client._api_url == "http://localhost:8080"
        await client.close()


class TestWorkerApiClientConcurrentToken:
    """Test WorkerApiClient token acquisition concurrency."""

    @pytest.mark.asyncio
    async def test_concurrent_token_requests_reuse_cached_token(
        self, httpx_mock: HTTPXMock
    ):
        """Test that concurrent requests share a single token fetch."""
        import asyncio

        # Only one token response
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/oauth/token",
            json={"access_token": "shared_token"},
        )
        # Two poll responses
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/workers/poll",
            json={"activities": [], "count": 0},
        )
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/workers/poll",
            json={"activities": [], "count": 0},
        )

        client = WorkerApiClient(
            api_url="http://localhost:8080",
            client_id="test",
            client_secret="secret",
        )

        try:
            # Make concurrent requests that will both try to get a token
            results = await asyncio.gather(
                client.poll_activities("python", "worker1", 10),
                client.poll_activities("python", "worker2", 10),
            )

            assert results[0] == []
            assert results[1] == []

            # Should only have one token request due to double-check locking
            token_requests = [
                r for r in httpx_mock.get_requests() if "oauth/token" in str(r.url)
            ]
            assert len(token_requests) == 1
        finally:
            await client.close()

    @pytest.mark.asyncio
    async def test_double_check_locking_second_check(self, httpx_mock: HTTPXMock):
        """Test that the second token check inside the lock returns cached token.

        This tests line 81: return self._token (inside the lock)
        """
        import asyncio

        # Only need token response - we're calling _get_token directly
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/oauth/token",
            json={"access_token": "token1"},
        )

        client = WorkerApiClient(
            api_url="http://localhost:8080",
            client_id="test",
            client_secret="secret",
        )

        try:
            # To reliably hit the double-check, we need to:
            # 1. Have one request get the token
            # 2. Have another request pass the first check (token=None)
            #    before the first request finishes getting the token
            # 3. Then wait at the lock until the first request releases it
            # 4. When it acquires the lock, it finds token is now set

            # Create an event to synchronize the test
            first_call_started = asyncio.Event()
            original_obtain = client._obtain_token

            async def slow_obtain_token():
                first_call_started.set()
                await asyncio.sleep(0.01)  # Small delay to allow second call to start
                return await original_obtain()

            # Temporarily patch to add delay
            client._obtain_token = slow_obtain_token

            # Now make concurrent calls to _get_token
            async def get_token_1():
                return await client._get_token()

            async def get_token_2():
                await first_call_started.wait()
                # Now the first call has started but not finished
                # Both should get the same token
                return await client._get_token()

            token1, token2 = await asyncio.gather(get_token_1(), get_token_2())

            assert token1 == "token1"
            assert token2 == "token1"

            # Only one token request should have been made
            token_requests = [
                r for r in httpx_mock.get_requests() if "oauth/token" in str(r.url)
            ]
            assert len(token_requests) == 1
        finally:
            await client.close()
