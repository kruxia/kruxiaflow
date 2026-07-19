"""Kruxia Flow API client."""

import logging
from typing import Any
from urllib.parse import urljoin

import httpx

logger = logging.getLogger(__name__)


class KruxiaFlowClient:
    """Async HTTP client for Kruxia Flow API.

    Handles authentication, request/response logging, error handling, and retries.
    """

    def __init__(
        self,
        base_url: str,
        token: str,
        timeout: float = 30.0,
    ) -> None:
        """Initialize API client.

        Args:
            base_url: Kruxia Flow API base URL (e.g., http://localhost:8080)
            token: JWT authentication token
            timeout: Request timeout in seconds
        """
        self.base_url = base_url.rstrip("/")
        self.token = token
        self.timeout = timeout

        # Create HTTP client with connection pooling
        self.client = httpx.AsyncClient(
            base_url=self.base_url,
            timeout=httpx.Timeout(timeout),
            follow_redirects=True,
            headers={
                "Authorization": f"Bearer {token}",
                "User-Agent": "kruxiaflow-mcp/0.1.0",
            },
        )

        logger.debug(f"Initialized API client: base_url={self.base_url}")

    async def close(self) -> None:
        """Close the HTTP client and release connections."""
        await self.client.aclose()

    async def request(
        self,
        method: str,
        path: str,
        *,
        params: dict[str, Any] | None = None,
        json: dict[str, Any] | None = None,
        headers: dict[str, str] | None = None,
    ) -> dict[str, Any]:
        """Make an HTTP request to Kruxia Flow API.

        Args:
            method: HTTP method (GET, POST, etc.)
            path: API path (e.g., /api/v1/workflows)
            params: Query parameters
            json: JSON body
            headers: Additional headers

        Returns:
            Response JSON as dictionary

        Raises:
            httpx.HTTPStatusError: For non-2xx responses
            httpx.RequestError: For connection errors
        """
        url = path if path.startswith("http") else urljoin(self.base_url, path)

        logger.debug(f"API_REQUEST method={method} url={url}")

        try:
            response = await self.client.request(
                method=method,
                url=url,
                params=params,
                json=json,
                headers=headers,
            )
            response.raise_for_status()

            logger.debug(
                f"API_RESPONSE status={response.status_code} "
                f"duration_ms={response.elapsed.total_seconds() * 1000:.1f}"
            )

            return response.json()

        except httpx.HTTPStatusError as e:
            logger.error(
                f"API error: {e.response.status_code} {e.response.text}",
                exc_info=True,
            )
            raise
        except httpx.RequestError as e:
            logger.error(f"Request error: {e}", exc_info=True)
            raise

    async def get(
        self,
        path: str,
        *,
        params: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        """Make a GET request."""
        return await self.request("GET", path, params=params)

    async def post(
        self,
        path: str,
        *,
        json: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        """Make a POST request."""
        return await self.request("POST", path, json=json)

    async def put(
        self,
        path: str,
        *,
        json: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        """Make a PUT request."""
        return await self.request("PUT", path, json=json)

    async def delete(
        self,
        path: str,
    ) -> dict[str, Any]:
        """Make a DELETE request."""
        return await self.request("DELETE", path)

    # Convenience methods for common API endpoints

    async def get_workflow_definitions(
        self,
        namespace: str | None = None,
        limit: int = 20,
        offset: int = 0,
    ) -> dict[str, Any]:
        """List workflow definitions."""
        params = {"limit": limit, "offset": offset}
        if namespace:
            params["namespace"] = namespace
        return await self.get("/api/v1/workflow_definitions", params=params)

    async def get_workflow_definition(self, name: str) -> dict[str, Any]:
        """Get workflow definition by name."""
        return await self.get(f"/api/v1/workflow_definitions/{name}")

    async def list_workflows(
        self,
        status: str | None = None,
        limit: int = 20,
        offset: int = 0,
    ) -> dict[str, Any]:
        """List workflows with optional status filter."""
        params = {"limit": limit, "offset": offset}
        if status:
            params["status"] = status
        return await self.get("/api/v1/workflows", params=params)

    async def get_workflow(
        self,
        workflow_id: str,
        include_activities: bool = False,
    ) -> dict[str, Any]:
        """Get workflow by ID."""
        params = {}
        if include_activities:
            params["include_activities"] = "true"
        return await self.get(f"/api/v1/workflows/{workflow_id}", params=params)

    async def submit_workflow(
        self,
        definition_name: str,
        input_data: dict[str, Any],
    ) -> dict[str, Any]:
        """Submit a new workflow for execution."""
        return await self.post(
            "/api/v1/workflows",
            json={
                "workflow_definition": definition_name,
                "input": input_data,
            },
        )

    async def get_workflow_cost(self, workflow_id: str) -> dict[str, Any]:
        """Get workflow cost breakdown."""
        return await self.get(f"/api/v1/workflows/{workflow_id}/cost")

    async def get_activity_output(
        self,
        workflow_id: str,
        activity_key: str,
    ) -> dict[str, Any]:
        """Get activity output."""
        return await self.get(f"/api/v1/workflows/{workflow_id}/activities/{activity_key}/output")
