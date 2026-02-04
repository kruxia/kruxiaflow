"""Tests for Kruxia Flow API client."""

import os

import httpx
import pytest
from pytest_httpx import HTTPXMock

from kruxiaflow import Activity, Workflow
from kruxiaflow.client import (
    AsyncKruxiaFlow,
    AuthenticationError,
    DeploymentError,
    KruxiaFlow,
    KruxiaFlowError,
    WorkflowNotFoundError,
)

# ============================================================================
# Test Fixtures
# ============================================================================


@pytest.fixture
def simple_workflow() -> Workflow:
    """Create a simple workflow for testing."""
    step1 = Activity(
        key="step1",
        worker="python",
        activity_name="echo",
        parameters={"message": "hello"},
    )
    return Workflow(name="test_workflow", activities=[step1])


@pytest.fixture
def clean_env():
    """Clear Kruxia Flow environment variables."""
    original = {
        k: os.environ.get(k) for k in ["KRUXIAFLOW_API_URL", "KRUXIAFLOW_TOKEN"]
    }
    for k in original:
        os.environ.pop(k, None)
    yield
    for k, v in original.items():
        if v is not None:
            os.environ[k] = v
        else:
            os.environ.pop(k, None)


# ============================================================================
# Sync Client Initialization Tests
# ============================================================================


@pytest.mark.usefixtures("clean_env")
class TestKruxiaFlowInit:
    """Test KruxiaFlow client initialization."""

    def test_init_with_params(self):
        client = KruxiaFlow(
            api_url="http://localhost:8080",
            api_token="test_token",
        )
        assert client._api_url == "http://localhost:8080"
        assert client._api_token == "test_token"
        client.close()

    def test_init_strips_trailing_slash(self):
        client = KruxiaFlow(
            api_url="http://localhost:8080/",
            api_token="test_token",
        )
        assert client._api_url == "http://localhost:8080"
        client.close()

    def test_init_from_env(self):
        os.environ["KRUXIAFLOW_API_URL"] = "http://api.example.com"
        os.environ["KRUXIAFLOW_TOKEN"] = "env_token"

        client = KruxiaFlow()
        assert client._api_url == "http://api.example.com"
        assert client._api_token == "env_token"
        client.close()

    def test_init_raises_without_api_url(self):
        with pytest.raises(ValueError, match="api_url is required"):
            KruxiaFlow(api_token="token")

    def test_init_raises_without_api_token(self):
        with pytest.raises(ValueError, match="api_token is required"):
            KruxiaFlow(api_url="http://localhost:8080")

    def test_init_resolves_env_var_token(self):
        os.environ["MY_TOKEN"] = "resolved_token"

        client = KruxiaFlow(
            api_url="http://localhost:8080",
            api_token="${MY_TOKEN}",
        )
        assert client._api_token == "resolved_token"
        client.close()

        del os.environ["MY_TOKEN"]

    def test_init_raises_for_unset_env_var_token(self):
        with pytest.raises(ValueError, match="MISSING_TOKEN is not set"):
            KruxiaFlow(
                api_url="http://localhost:8080",
                api_token="${MISSING_TOKEN}",
            )

    def test_custom_timeout(self):
        client = KruxiaFlow(
            api_url="http://localhost:8080",
            api_token="token",
            timeout=60.0,
        )
        assert client._client.timeout.read == 60.0
        client.close()


# ============================================================================
# Sync Client Context Manager Tests
# ============================================================================


@pytest.mark.usefixtures("clean_env")
class TestKruxiaFlowContextManager:
    """Test KruxiaFlow context manager."""

    def test_context_manager(self):
        with KruxiaFlow(
            api_url="http://localhost:8080",
            api_token="token",
        ) as client:
            assert isinstance(client, KruxiaFlow)

    def test_context_manager_closes_client(self):
        client = KruxiaFlow(
            api_url="http://localhost:8080",
            api_token="token",
        )
        with client:
            pass
        # Client should be closed (httpx client is closed)
        assert client._client.is_closed


# ============================================================================
# Sync Client Deploy Tests
# ============================================================================


@pytest.mark.usefixtures("clean_env")
class TestKruxiaFlowDeploy:
    """Test KruxiaFlow deploy method."""

    def test_deploy_success(self, httpx_mock: HTTPXMock, simple_workflow: Workflow):
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/workflows",
            json={"workflow_id": "wf-123", "status": "deployed"},
        )

        with KruxiaFlow(
            api_url="http://localhost:8080",
            api_token="token",
        ) as client:
            result = client.deploy(simple_workflow)

        assert result["workflow_id"] == "wf-123"
        assert result["status"] == "deployed"

    def test_deploy_auth_error(self, httpx_mock: HTTPXMock, simple_workflow: Workflow):
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/workflows",
            status_code=401,
            text="Unauthorized",
        )

        with (
            KruxiaFlow(
                api_url="http://localhost:8080",
                api_token="bad_token",
            ) as client,
            pytest.raises(AuthenticationError, match="Authentication failed"),
        ):
            client.deploy(simple_workflow)

    def test_deploy_failure(self, httpx_mock: HTTPXMock, simple_workflow: Workflow):
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/workflows",
            status_code=400,
            text="Invalid workflow",
        )

        with (
            KruxiaFlow(
                api_url="http://localhost:8080",
                api_token="token",
            ) as client,
            pytest.raises(DeploymentError, match="Deployment failed"),
        ):
            client.deploy(simple_workflow)

    def test_deploy_network_error(
        self, httpx_mock: HTTPXMock, simple_workflow: Workflow
    ):
        httpx_mock.add_exception(
            httpx.ConnectError("Connection refused"),
            url="http://localhost:8080/api/v1/workflows",
        )

        with (
            KruxiaFlow(
                api_url="http://localhost:8080",
                api_token="token",
            ) as client,
            pytest.raises(DeploymentError, match="Request failed"),
        ):
            client.deploy(simple_workflow)


# ============================================================================
# Sync Client Start Workflow Tests
# ============================================================================


@pytest.mark.usefixtures("clean_env")
class TestKruxiaFlowStartWorkflow:
    """Test KruxiaFlow start_workflow method."""

    def test_start_workflow_success(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/workflows/start",
            json={"instance_id": "inst-123", "status": "running"},
        )

        with KruxiaFlow(
            api_url="http://localhost:8080",
            api_token="token",
        ) as client:
            result = client.start_workflow("my_workflow")

        assert result["instance_id"] == "inst-123"

    def test_start_workflow_with_inputs(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/workflows/start",
            json={"instance_id": "inst-456"},
        )

        with KruxiaFlow(
            api_url="http://localhost:8080",
            api_token="token",
        ) as client:
            result = client.start_workflow(
                "my_workflow",
                inputs={"param1": "value1"},
                version="2.0.0",
            )

        assert result["instance_id"] == "inst-456"

        # Verify request body
        request = httpx_mock.get_request()
        import json

        body = json.loads(request.content)
        assert body["name"] == "my_workflow"
        assert body["inputs"] == {"param1": "value1"}
        assert body["version"] == "2.0.0"

    def test_start_workflow_not_found(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/workflows/start",
            status_code=404,
            text="Not found",
        )

        with (
            KruxiaFlow(
                api_url="http://localhost:8080",
                api_token="token",
            ) as client,
            pytest.raises(WorkflowNotFoundError, match="not found"),
        ):
            client.start_workflow("nonexistent")

    def test_start_workflow_auth_error(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/workflows/start",
            status_code=401,
        )

        with (
            KruxiaFlow(
                api_url="http://localhost:8080",
                api_token="token",
            ) as client,
            pytest.raises(AuthenticationError),
        ):
            client.start_workflow("my_workflow")

    def test_start_workflow_server_error(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/workflows/start",
            status_code=500,
            text="Internal server error",
        )

        with (
            KruxiaFlow(
                api_url="http://localhost:8080",
                api_token="token",
            ) as client,
            pytest.raises(KruxiaFlowError, match="Failed to start workflow"),
        ):
            client.start_workflow("my_workflow")

    def test_start_workflow_network_error(self, httpx_mock: HTTPXMock):
        httpx_mock.add_exception(
            httpx.ConnectError("Connection refused"),
            url="http://localhost:8080/api/v1/workflows/start",
        )

        with (
            KruxiaFlow(
                api_url="http://localhost:8080",
                api_token="token",
            ) as client,
            pytest.raises(KruxiaFlowError, match="Request failed"),
        ):
            client.start_workflow("my_workflow")


# ============================================================================
# Sync Client Get Workflow Tests
# ============================================================================


@pytest.mark.usefixtures("clean_env")
class TestKruxiaFlowGetWorkflow:
    """Test KruxiaFlow get_workflow method."""

    def test_get_workflow_success(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="GET",
            url="http://localhost:8080/api/v1/workflows/wf-123",
            json={
                "workflow_id": "wf-123",
                "status": "completed",
                "activities": {"step1": {"status": "completed"}},
            },
        )

        with KruxiaFlow(
            api_url="http://localhost:8080",
            api_token="token",
        ) as client:
            result = client.get_workflow("wf-123")

        assert result["workflow_id"] == "wf-123"
        assert result["status"] == "completed"

    def test_get_workflow_not_found(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="GET",
            url="http://localhost:8080/api/v1/workflows/nonexistent",
            status_code=404,
        )

        with (
            KruxiaFlow(
                api_url="http://localhost:8080",
                api_token="token",
            ) as client,
            pytest.raises(WorkflowNotFoundError),
        ):
            client.get_workflow("nonexistent")

    def test_get_workflow_server_error(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="GET",
            url="http://localhost:8080/api/v1/workflows/wf-123",
            status_code=500,
            text="Internal Server Error",
        )

        with (
            KruxiaFlow(
                api_url="http://localhost:8080",
                api_token="token",
            ) as client,
            pytest.raises(KruxiaFlowError, match="Failed to get workflow"),
        ):
            client.get_workflow("wf-123")

    def test_get_workflow_auth_error(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="GET",
            url="http://localhost:8080/api/v1/workflows/wf-123",
            status_code=401,
        )

        with (
            KruxiaFlow(
                api_url="http://localhost:8080",
                api_token="token",
            ) as client,
            pytest.raises(AuthenticationError),
        ):
            client.get_workflow("wf-123")

    def test_get_workflow_network_error(self, httpx_mock: HTTPXMock):
        httpx_mock.add_exception(
            httpx.ConnectError("Connection refused"),
            url="http://localhost:8080/api/v1/workflows/wf-123",
        )

        with (
            KruxiaFlow(
                api_url="http://localhost:8080",
                api_token="token",
            ) as client,
            pytest.raises(KruxiaFlowError, match="Request failed"),
        ):
            client.get_workflow("wf-123")


# ============================================================================
# Sync Client Get Workflow Output Tests
# ============================================================================


@pytest.mark.usefixtures("clean_env")
class TestKruxiaFlowGetWorkflowOutput:
    """Test KruxiaFlow get_workflow_output method."""

    def test_get_workflow_output_success(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="GET",
            url="http://localhost:8080/api/v1/workflows/wf-123/output",
            json={"step1": {"result": "done"}, "step2": {"count": 42}},
        )

        with KruxiaFlow(
            api_url="http://localhost:8080",
            api_token="token",
        ) as client:
            result = client.get_workflow_output("wf-123")

        assert result["step1"]["result"] == "done"
        assert result["step2"]["count"] == 42

    def test_get_workflow_output_not_found(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="GET",
            url="http://localhost:8080/api/v1/workflows/nonexistent/output",
            status_code=404,
        )

        with (
            KruxiaFlow(
                api_url="http://localhost:8080",
                api_token="token",
            ) as client,
            pytest.raises(WorkflowNotFoundError),
        ):
            client.get_workflow_output("nonexistent")

    def test_get_workflow_output_auth_error(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="GET",
            url="http://localhost:8080/api/v1/workflows/wf-123/output",
            status_code=401,
        )

        with (
            KruxiaFlow(
                api_url="http://localhost:8080",
                api_token="token",
            ) as client,
            pytest.raises(AuthenticationError),
        ):
            client.get_workflow_output("wf-123")

    def test_get_workflow_output_server_error(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="GET",
            url="http://localhost:8080/api/v1/workflows/wf-123/output",
            status_code=500,
            text="Internal server error",
        )

        with (
            KruxiaFlow(
                api_url="http://localhost:8080",
                api_token="token",
            ) as client,
            pytest.raises(KruxiaFlowError, match="Failed to get workflow output"),
        ):
            client.get_workflow_output("wf-123")

    def test_get_workflow_output_network_error(self, httpx_mock: HTTPXMock):
        httpx_mock.add_exception(
            httpx.ConnectError("Connection refused"),
            url="http://localhost:8080/api/v1/workflows/wf-123/output",
        )

        with (
            KruxiaFlow(
                api_url="http://localhost:8080",
                api_token="token",
            ) as client,
            pytest.raises(KruxiaFlowError, match="Request failed"),
        ):
            client.get_workflow_output("wf-123")


# ============================================================================
# Sync Client Cancel Workflow Tests
# ============================================================================


@pytest.mark.usefixtures("clean_env")
class TestKruxiaFlowCancelWorkflow:
    """Test KruxiaFlow cancel_workflow method."""

    def test_cancel_workflow_success(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/workflows/wf-123/cancel",
            status_code=200,
        )

        with KruxiaFlow(
            api_url="http://localhost:8080",
            api_token="token",
        ) as client:
            client.cancel_workflow("wf-123")  # Should not raise

    def test_cancel_workflow_not_found(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/workflows/nonexistent/cancel",
            status_code=404,
        )

        with (
            KruxiaFlow(
                api_url="http://localhost:8080",
                api_token="token",
            ) as client,
            pytest.raises(WorkflowNotFoundError),
        ):
            client.cancel_workflow("nonexistent")

    def test_cancel_workflow_auth_error(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/workflows/wf-123/cancel",
            status_code=401,
        )

        with (
            KruxiaFlow(
                api_url="http://localhost:8080",
                api_token="token",
            ) as client,
            pytest.raises(AuthenticationError),
        ):
            client.cancel_workflow("wf-123")

    def test_cancel_workflow_server_error(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/workflows/wf-123/cancel",
            status_code=500,
            text="Internal server error",
        )

        with (
            KruxiaFlow(
                api_url="http://localhost:8080",
                api_token="token",
            ) as client,
            pytest.raises(KruxiaFlowError, match="Failed to cancel workflow"),
        ):
            client.cancel_workflow("wf-123")

    def test_cancel_workflow_network_error(self, httpx_mock: HTTPXMock):
        httpx_mock.add_exception(
            httpx.ConnectError("Connection refused"),
            url="http://localhost:8080/api/v1/workflows/wf-123/cancel",
        )

        with (
            KruxiaFlow(
                api_url="http://localhost:8080",
                api_token="token",
            ) as client,
            pytest.raises(KruxiaFlowError, match="Request failed"),
        ):
            client.cancel_workflow("wf-123")


# ============================================================================
# Sync Client Get Activity Output Tests
# ============================================================================


@pytest.mark.usefixtures("clean_env")
class TestKruxiaFlowGetActivityOutput:
    """Test KruxiaFlow get_activity_output method."""

    def test_get_activity_output_success(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="GET",
            url="http://localhost:8080/api/v1/workflows/wf-123/activities/step1/output",
            json={"result": "success", "data": [1, 2, 3]},
        )

        with KruxiaFlow(
            api_url="http://localhost:8080",
            api_token="token",
        ) as client:
            result = client.get_activity_output("wf-123", "step1")

        assert result["result"] == "success"
        assert result["data"] == [1, 2, 3]

    def test_get_activity_output_not_found(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="GET",
            url="http://localhost:8080/api/v1/workflows/wf-123/activities/missing/output",
            status_code=404,
        )

        with (
            KruxiaFlow(
                api_url="http://localhost:8080",
                api_token="token",
            ) as client,
            pytest.raises(WorkflowNotFoundError, match="activity 'missing' not found"),
        ):
            client.get_activity_output("wf-123", "missing")

    def test_get_activity_output_auth_error(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="GET",
            url="http://localhost:8080/api/v1/workflows/wf-123/activities/step1/output",
            status_code=401,
        )

        with (
            KruxiaFlow(
                api_url="http://localhost:8080",
                api_token="token",
            ) as client,
            pytest.raises(AuthenticationError),
        ):
            client.get_activity_output("wf-123", "step1")

    def test_get_activity_output_server_error(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="GET",
            url="http://localhost:8080/api/v1/workflows/wf-123/activities/step1/output",
            status_code=500,
            text="Internal server error",
        )

        with (
            KruxiaFlow(
                api_url="http://localhost:8080",
                api_token="token",
            ) as client,
            pytest.raises(KruxiaFlowError, match="Failed to get activity output"),
        ):
            client.get_activity_output("wf-123", "step1")

    def test_get_activity_output_network_error(self, httpx_mock: HTTPXMock):
        httpx_mock.add_exception(
            httpx.ConnectError("Connection refused"),
            url="http://localhost:8080/api/v1/workflows/wf-123/activities/step1/output",
        )

        with (
            KruxiaFlow(
                api_url="http://localhost:8080",
                api_token="token",
            ) as client,
            pytest.raises(KruxiaFlowError, match="Request failed"),
        ):
            client.get_activity_output("wf-123", "step1")


# ============================================================================
# Async Client Tests
# ============================================================================


@pytest.mark.usefixtures("clean_env")
class TestAsyncKruxiaFlowInit:
    """Test AsyncKruxiaFlow client initialization."""

    @pytest.mark.asyncio
    async def test_init_with_params(self):
        client = AsyncKruxiaFlow(
            api_url="http://localhost:8080",
            api_token="test_token",
        )
        assert client._api_url == "http://localhost:8080"
        assert client._api_token == "test_token"
        await client.close()

    def test_init_raises_without_api_url(self):
        with pytest.raises(ValueError, match="api_url is required"):
            AsyncKruxiaFlow(api_token="token")

    def test_init_raises_without_api_token(self):
        with pytest.raises(ValueError, match="api_token is required"):
            AsyncKruxiaFlow(api_url="http://localhost:8080")

    @pytest.mark.asyncio
    async def test_async_context_manager(self):
        async with AsyncKruxiaFlow(
            api_url="http://localhost:8080",
            api_token="token",
        ) as client:
            assert isinstance(client, AsyncKruxiaFlow)

    @pytest.mark.asyncio
    async def test_init_resolves_env_var_token(self):
        os.environ["ASYNC_TEST_TOKEN"] = "resolved_async_token"

        client = AsyncKruxiaFlow(
            api_url="http://localhost:8080",
            api_token="${ASYNC_TEST_TOKEN}",
        )
        assert client._api_token == "resolved_async_token"
        await client.close()

        del os.environ["ASYNC_TEST_TOKEN"]

    def test_init_raises_for_unset_env_var_token(self):
        with pytest.raises(ValueError, match="MISSING_ASYNC_TOKEN is not set"):
            AsyncKruxiaFlow(
                api_url="http://localhost:8080",
                api_token="${MISSING_ASYNC_TOKEN}",
            )


@pytest.mark.usefixtures("clean_env")
class TestAsyncKruxiaFlowDeploy:
    """Test AsyncKruxiaFlow deploy method."""

    @pytest.mark.asyncio
    async def test_deploy_success(
        self, httpx_mock: HTTPXMock, simple_workflow: Workflow
    ):
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/workflows",
            json={"workflow_id": "wf-123", "status": "deployed"},
        )

        async with AsyncKruxiaFlow(
            api_url="http://localhost:8080",
            api_token="token",
        ) as client:
            result = await client.deploy(simple_workflow)

        assert result["workflow_id"] == "wf-123"

    @pytest.mark.asyncio
    async def test_deploy_auth_error(
        self, httpx_mock: HTTPXMock, simple_workflow: Workflow
    ):
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/workflows",
            status_code=401,
        )

        async with AsyncKruxiaFlow(
            api_url="http://localhost:8080",
            api_token="token",
        ) as client:
            with pytest.raises(AuthenticationError):
                await client.deploy(simple_workflow)

    @pytest.mark.asyncio
    async def test_deploy_failure(
        self, httpx_mock: HTTPXMock, simple_workflow: Workflow
    ):
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/workflows",
            status_code=400,
            text="Bad request",
        )

        async with AsyncKruxiaFlow(
            api_url="http://localhost:8080",
            api_token="token",
        ) as client:
            with pytest.raises(DeploymentError):
                await client.deploy(simple_workflow)

    @pytest.mark.asyncio
    async def test_deploy_network_error(
        self, httpx_mock: HTTPXMock, simple_workflow: Workflow
    ):
        httpx_mock.add_exception(
            httpx.ConnectError("Connection refused"),
            url="http://localhost:8080/api/v1/workflows",
        )

        async with AsyncKruxiaFlow(
            api_url="http://localhost:8080",
            api_token="token",
        ) as client:
            with pytest.raises(DeploymentError, match="Request failed"):
                await client.deploy(simple_workflow)


@pytest.mark.usefixtures("clean_env")
class TestAsyncKruxiaFlowGetWorkflow:
    """Test AsyncKruxiaFlow get_workflow method."""

    @pytest.mark.asyncio
    async def test_get_workflow_success(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="GET",
            url="http://localhost:8080/api/v1/workflows/wf-123",
            json={"workflow_id": "wf-123", "status": "completed"},
        )

        async with AsyncKruxiaFlow(
            api_url="http://localhost:8080",
            api_token="token",
        ) as client:
            result = await client.get_workflow("wf-123")

        assert result["status"] == "completed"

    @pytest.mark.asyncio
    async def test_get_workflow_not_found(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="GET",
            url="http://localhost:8080/api/v1/workflows/nonexistent",
            status_code=404,
        )

        async with AsyncKruxiaFlow(
            api_url="http://localhost:8080",
            api_token="token",
        ) as client:
            with pytest.raises(WorkflowNotFoundError):
                await client.get_workflow("nonexistent")

    @pytest.mark.asyncio
    async def test_get_workflow_auth_error(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="GET",
            url="http://localhost:8080/api/v1/workflows/wf-123",
            status_code=401,
        )

        async with AsyncKruxiaFlow(
            api_url="http://localhost:8080",
            api_token="token",
        ) as client:
            with pytest.raises(AuthenticationError):
                await client.get_workflow("wf-123")

    @pytest.mark.asyncio
    async def test_get_workflow_server_error(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="GET",
            url="http://localhost:8080/api/v1/workflows/wf-123",
            status_code=500,
            text="Internal error",
        )

        async with AsyncKruxiaFlow(
            api_url="http://localhost:8080",
            api_token="token",
        ) as client:
            with pytest.raises(KruxiaFlowError, match="Failed to get workflow"):
                await client.get_workflow("wf-123")

    @pytest.mark.asyncio
    async def test_get_workflow_network_error(self, httpx_mock: HTTPXMock):
        httpx_mock.add_exception(
            httpx.ConnectError("Connection refused"),
            url="http://localhost:8080/api/v1/workflows/wf-123",
        )

        async with AsyncKruxiaFlow(
            api_url="http://localhost:8080",
            api_token="token",
        ) as client:
            with pytest.raises(KruxiaFlowError, match="Request failed"):
                await client.get_workflow("wf-123")


@pytest.mark.usefixtures("clean_env")
class TestAsyncKruxiaFlowCancelWorkflow:
    """Test AsyncKruxiaFlow cancel_workflow method."""

    @pytest.mark.asyncio
    async def test_cancel_workflow_success(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/workflows/wf-123/cancel",
            status_code=200,
        )

        async with AsyncKruxiaFlow(
            api_url="http://localhost:8080",
            api_token="token",
        ) as client:
            await client.cancel_workflow("wf-123")  # Should not raise

    @pytest.mark.asyncio
    async def test_cancel_workflow_not_found(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/workflows/nonexistent/cancel",
            status_code=404,
        )

        async with AsyncKruxiaFlow(
            api_url="http://localhost:8080",
            api_token="token",
        ) as client:
            with pytest.raises(WorkflowNotFoundError):
                await client.cancel_workflow("nonexistent")

    @pytest.mark.asyncio
    async def test_cancel_workflow_auth_error(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/workflows/wf-123/cancel",
            status_code=401,
        )

        async with AsyncKruxiaFlow(
            api_url="http://localhost:8080",
            api_token="token",
        ) as client:
            with pytest.raises(AuthenticationError):
                await client.cancel_workflow("wf-123")

    @pytest.mark.asyncio
    async def test_cancel_workflow_server_error(self, httpx_mock: HTTPXMock):
        httpx_mock.add_response(
            method="POST",
            url="http://localhost:8080/api/v1/workflows/wf-123/cancel",
            status_code=500,
            text="Internal server error",
        )

        async with AsyncKruxiaFlow(
            api_url="http://localhost:8080",
            api_token="token",
        ) as client:
            with pytest.raises(KruxiaFlowError, match="Failed to cancel workflow"):
                await client.cancel_workflow("wf-123")

    @pytest.mark.asyncio
    async def test_cancel_workflow_network_error(self, httpx_mock: HTTPXMock):
        httpx_mock.add_exception(
            httpx.ConnectError("Connection refused"),
            url="http://localhost:8080/api/v1/workflows/wf-123/cancel",
        )

        async with AsyncKruxiaFlow(
            api_url="http://localhost:8080",
            api_token="token",
        ) as client:
            with pytest.raises(KruxiaFlowError, match="Request failed"):
                await client.cancel_workflow("wf-123")
