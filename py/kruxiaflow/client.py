"""API client for Kruxia Flow server."""

import os
from typing import Any

import httpx

from .models import Workflow


class KruxiaFlowError(Exception):
    """Base exception for Kruxia Flow client errors."""


class AuthenticationError(KruxiaFlowError):
    """Authentication failed."""


class DeploymentError(KruxiaFlowError):
    """Workflow deployment failed."""


class WorkflowNotFoundError(KruxiaFlowError):
    """Workflow not found."""


class KruxiaFlow:
    """API client for Kruxia Flow server.

    Provides methods for deploying workflows, checking status, and managing
    workflow execution.

    Example:
        client = KruxiaFlow(
            api_url="http://localhost:8080",
            api_token=os.environ["KRUXIAFLOW_TOKEN"]
        )

        # Deploy a workflow
        result = client.deploy(workflow)
        print(f"Deployed: {result['workflow_id']}")

        # Check status
        status = client.get_workflow(result['workflow_id'])
    """

    def __init__(
        self,
        api_url: str | None = None,
        api_token: str | None = None,
        *,
        timeout: float = 30.0,
    ):
        """Create a new Kruxia Flow client.

        Args:
            api_url: Base URL for the API server (default: KRUXIAFLOW_API_URL env var)
            api_token: Bearer token for authentication (default: KRUXIAFLOW_TOKEN env var)
            timeout: Request timeout in seconds (default: 30.0)

        Raises:
            ValueError: If api_url or api_token is not provided and not in environment
        """
        self._api_url = (api_url or os.environ.get("KRUXIAFLOW_API_URL", "")).rstrip(
            "/"
        )
        self._api_token = api_token or os.environ.get("KRUXIAFLOW_TOKEN", "")

        if not self._api_url:
            raise ValueError(
                "api_url is required. Pass it directly or set KRUXIAFLOW_API_URL environment variable."
            )
        if not self._api_token:
            raise ValueError(
                "api_token is required. Pass it directly or set KRUXIAFLOW_TOKEN environment variable."
            )

        # Resolve token if it's an env var reference
        if self._api_token.startswith("${") and self._api_token.endswith("}"):
            env_var = self._api_token[2:-1]
            self._api_token = os.environ.get(env_var, "")
            if not self._api_token:
                raise ValueError(f"Environment variable {env_var} is not set")

        self._client = httpx.Client(
            base_url=self._api_url,
            headers={"Authorization": f"Bearer {self._api_token}"},
            timeout=timeout,
        )

    def deploy(self, workflow: Workflow) -> dict[str, Any]:
        """Deploy a workflow to the server.

        Args:
            workflow: Workflow to deploy

        Returns:
            Response containing workflow_id and deployment status

        Raises:
            DeploymentError: If deployment fails
        """
        try:
            response = self._client.post(
                "/api/v1/workflows",
                json=workflow.to_dict(),
            )
            response.raise_for_status()
            return response.json()
        except httpx.HTTPStatusError as e:
            if e.response.status_code == 401:
                raise AuthenticationError("Authentication failed") from e
            raise DeploymentError(
                f"Deployment failed: {e.response.status_code} - {e.response.text}"
            ) from e
        except httpx.RequestError as e:
            raise DeploymentError(f"Request failed: {e}") from e

    def start_workflow(
        self,
        workflow_name: str,
        *,
        inputs: dict[str, Any] | None = None,
        version: str | None = None,
    ) -> dict[str, Any]:
        """Start a workflow execution by name.

        Args:
            workflow_name: Name of the workflow to start
            inputs: Input parameters for the workflow
            version: Optional version (default: latest)

        Returns:
            Response containing workflow instance ID and status

        Raises:
            KruxiaFlowError: If starting fails
        """
        body: dict[str, Any] = {"name": workflow_name}
        if inputs:
            body["inputs"] = inputs
        if version:
            body["version"] = version

        try:
            response = self._client.post("/api/v1/workflows/start", json=body)
            response.raise_for_status()
            return response.json()
        except httpx.HTTPStatusError as e:
            if e.response.status_code == 401:
                raise AuthenticationError("Authentication failed") from e
            if e.response.status_code == 404:
                raise WorkflowNotFoundError(
                    f"Workflow '{workflow_name}' not found"
                ) from e
            raise KruxiaFlowError(
                f"Failed to start workflow: {e.response.status_code} - {e.response.text}"
            ) from e
        except httpx.RequestError as e:
            raise KruxiaFlowError(f"Request failed: {e}") from e

    def get_workflow(self, workflow_id: str) -> dict[str, Any]:
        """Get workflow execution status.

        Args:
            workflow_id: Unique workflow execution ID

        Returns:
            Workflow status including state and activity statuses

        Raises:
            WorkflowNotFoundError: If workflow not found
            KruxiaFlowError: If request fails
        """
        try:
            response = self._client.get(f"/api/v1/workflows/{workflow_id}")
            response.raise_for_status()
            return response.json()
        except httpx.HTTPStatusError as e:
            if e.response.status_code == 401:
                raise AuthenticationError("Authentication failed") from e
            if e.response.status_code == 404:
                raise WorkflowNotFoundError(
                    f"Workflow '{workflow_id}' not found"
                ) from e
            raise KruxiaFlowError(
                f"Failed to get workflow: {e.response.status_code} - {e.response.text}"
            ) from e
        except httpx.RequestError as e:
            raise KruxiaFlowError(f"Request failed: {e}") from e

    def get_workflow_output(self, workflow_id: str) -> dict[str, Any]:
        """Get workflow output.

        Returns all activity outputs for a completed workflow.

        Args:
            workflow_id: Unique workflow execution ID

        Returns:
            Dictionary of activity outputs

        Raises:
            WorkflowNotFoundError: If workflow not found
            KruxiaFlowError: If request fails
        """
        try:
            response = self._client.get(f"/api/v1/workflows/{workflow_id}/output")
            response.raise_for_status()
            return response.json()
        except httpx.HTTPStatusError as e:
            if e.response.status_code == 401:
                raise AuthenticationError("Authentication failed") from e
            if e.response.status_code == 404:
                raise WorkflowNotFoundError(
                    f"Workflow '{workflow_id}' not found"
                ) from e
            raise KruxiaFlowError(
                f"Failed to get workflow output: {e.response.status_code} - {e.response.text}"
            ) from e
        except httpx.RequestError as e:
            raise KruxiaFlowError(f"Request failed: {e}") from e

    def cancel_workflow(self, workflow_id: str) -> None:
        """Cancel a running workflow.

        Args:
            workflow_id: Unique workflow execution ID

        Raises:
            WorkflowNotFoundError: If workflow not found
            KruxiaFlowError: If cancellation fails
        """
        try:
            response = self._client.post(f"/api/v1/workflows/{workflow_id}/cancel")
            response.raise_for_status()
        except httpx.HTTPStatusError as e:
            if e.response.status_code == 401:
                raise AuthenticationError("Authentication failed") from e
            if e.response.status_code == 404:
                raise WorkflowNotFoundError(
                    f"Workflow '{workflow_id}' not found"
                ) from e
            raise KruxiaFlowError(
                f"Failed to cancel workflow: {e.response.status_code} - {e.response.text}"
            ) from e
        except httpx.RequestError as e:
            raise KruxiaFlowError(f"Request failed: {e}") from e

    def get_activity_output(
        self, workflow_id: str, activity_key: str
    ) -> dict[str, Any]:
        """Get output for a specific activity.

        Args:
            workflow_id: Unique workflow execution ID
            activity_key: Activity key within the workflow

        Returns:
            Activity output data

        Raises:
            WorkflowNotFoundError: If workflow or activity not found
            KruxiaFlowError: If request fails
        """
        try:
            response = self._client.get(
                f"/api/v1/workflows/{workflow_id}/activities/{activity_key}/output"
            )
            response.raise_for_status()
            return response.json()
        except httpx.HTTPStatusError as e:
            if e.response.status_code == 401:
                raise AuthenticationError("Authentication failed") from e
            if e.response.status_code == 404:
                raise WorkflowNotFoundError(
                    f"Workflow '{workflow_id}' or activity '{activity_key}' not found"
                ) from e
            raise KruxiaFlowError(
                f"Failed to get activity output: {e.response.status_code} - {e.response.text}"
            ) from e
        except httpx.RequestError as e:
            raise KruxiaFlowError(f"Request failed: {e}") from e

    def close(self) -> None:
        """Close the client connection."""
        self._client.close()

    def __enter__(self) -> "KruxiaFlow":
        """Context manager entry."""
        return self

    def __exit__(self, *args: object) -> None:
        """Context manager exit."""
        self.close()


class AsyncKruxiaFlow:
    """Async API client for Kruxia Flow server.

    Provides async methods for deploying workflows, checking status, and managing
    workflow execution.

    Example:
        async with AsyncKruxiaFlow(
            api_url="http://localhost:8080",
            api_token=os.environ["KRUXIAFLOW_TOKEN"]
        ) as client:
            result = await client.deploy(workflow)
    """

    def __init__(
        self,
        api_url: str | None = None,
        api_token: str | None = None,
        *,
        timeout: float = 30.0,
    ):
        """Create a new async Kruxia Flow client.

        Args:
            api_url: Base URL for the API server (default: KRUXIAFLOW_API_URL env var)
            api_token: Bearer token for authentication (default: KRUXIAFLOW_TOKEN env var)
            timeout: Request timeout in seconds (default: 30.0)
        """
        self._api_url = (api_url or os.environ.get("KRUXIAFLOW_API_URL", "")).rstrip(
            "/"
        )
        self._api_token = api_token or os.environ.get("KRUXIAFLOW_TOKEN", "")

        if not self._api_url:
            raise ValueError(
                "api_url is required. Pass it directly or set KRUXIAFLOW_API_URL environment variable."
            )
        if not self._api_token:
            raise ValueError(
                "api_token is required. Pass it directly or set KRUXIAFLOW_TOKEN environment variable."
            )

        # Resolve token if it's an env var reference
        if self._api_token.startswith("${") and self._api_token.endswith("}"):
            env_var = self._api_token[2:-1]
            self._api_token = os.environ.get(env_var, "")
            if not self._api_token:
                raise ValueError(f"Environment variable {env_var} is not set")

        self._client = httpx.AsyncClient(
            base_url=self._api_url,
            headers={"Authorization": f"Bearer {self._api_token}"},
            timeout=timeout,
        )

    async def deploy(self, workflow: Workflow) -> dict[str, Any]:
        """Deploy a workflow to the server.

        Args:
            workflow: Workflow to deploy

        Returns:
            Response containing workflow_id and deployment status
        """
        try:
            response = await self._client.post(
                "/api/v1/workflows",
                json=workflow.to_dict(),
            )
            response.raise_for_status()
            return response.json()
        except httpx.HTTPStatusError as e:
            if e.response.status_code == 401:
                raise AuthenticationError("Authentication failed") from e
            raise DeploymentError(
                f"Deployment failed: {e.response.status_code} - {e.response.text}"
            ) from e
        except httpx.RequestError as e:
            raise DeploymentError(f"Request failed: {e}") from e

    async def get_workflow(self, workflow_id: str) -> dict[str, Any]:
        """Get workflow execution status.

        Args:
            workflow_id: Unique workflow execution ID

        Returns:
            Workflow status including state and activity statuses
        """
        try:
            response = await self._client.get(f"/api/v1/workflows/{workflow_id}")
            response.raise_for_status()
            return response.json()
        except httpx.HTTPStatusError as e:
            if e.response.status_code == 401:
                raise AuthenticationError("Authentication failed") from e
            if e.response.status_code == 404:
                raise WorkflowNotFoundError(
                    f"Workflow '{workflow_id}' not found"
                ) from e
            raise KruxiaFlowError(
                f"Failed to get workflow: {e.response.status_code} - {e.response.text}"
            ) from e
        except httpx.RequestError as e:
            raise KruxiaFlowError(f"Request failed: {e}") from e

    async def cancel_workflow(self, workflow_id: str) -> None:
        """Cancel a running workflow.

        Args:
            workflow_id: Unique workflow execution ID
        """
        try:
            response = await self._client.post(
                f"/api/v1/workflows/{workflow_id}/cancel"
            )
            response.raise_for_status()
        except httpx.HTTPStatusError as e:
            if e.response.status_code == 401:
                raise AuthenticationError("Authentication failed") from e
            if e.response.status_code == 404:
                raise WorkflowNotFoundError(
                    f"Workflow '{workflow_id}' not found"
                ) from e
            raise KruxiaFlowError(
                f"Failed to cancel workflow: {e.response.status_code} - {e.response.text}"
            ) from e
        except httpx.RequestError as e:
            raise KruxiaFlowError(f"Request failed: {e}") from e

    async def close(self) -> None:
        """Close the client connection."""
        await self._client.aclose()

    async def __aenter__(self) -> "AsyncKruxiaFlow":
        """Async context manager entry."""
        return self

    async def __aexit__(self, *args: object) -> None:
        """Async context manager exit."""
        await self.close()
