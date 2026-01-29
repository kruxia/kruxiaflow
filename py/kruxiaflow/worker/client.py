"""Worker API client.

Mirrors Rust WorkerApiClient for interface compatibility.
"""

from __future__ import annotations

import asyncio
from collections.abc import AsyncIterator
from datetime import datetime, timezone
from decimal import Decimal
from typing import TYPE_CHECKING, Any
from uuid import UUID

import httpx
from pydantic import BaseModel

from ..types import ActivityKey, ActivityName, WorkerNameRequired
from .errors import AuthenticationError

if TYPE_CHECKING:
    from .storage import FileMetadata


class PendingActivity(BaseModel):
    """Activity claimed from the queue. Mirrors Rust PendingActivity."""

    activity_id: UUID
    workflow_id: UUID
    activity_key: ActivityKey
    worker: WorkerNameRequired
    activity_name: ActivityName
    parameters: dict[str, Any]
    settings: dict[str, Any] | None = None
    timeout_seconds: int | None = None
    output_definitions: list[dict[str, Any]] | None = None
    signal_data: dict[str, Any] | None = None
    """Signal data if activity was waiting for an external signal."""


class WorkerApiClient:
    """
    HTTP client for Worker Activity APIs.

    Mirrors Rust WorkerApiClient for interface compatibility.
    Implements OAuth token management with automatic refresh on 401.
    """

    def __init__(self, api_url: str, client_id: str, client_secret: str):
        self._api_url = api_url.rstrip("/")
        self._client_id = client_id
        self._client_secret = client_secret
        self._token: str | None = None
        self._lock = asyncio.Lock()
        self._http = httpx.AsyncClient(timeout=30.0)

    async def close(self) -> None:
        """Close HTTP client."""
        await self._http.aclose()

    async def _obtain_token(self) -> str:
        """Obtain access token via OAuth client credentials flow."""
        response = await self._http.post(
            f"{self._api_url}/api/v1/oauth/token",
            json={
                "grant_type": "client_credentials",
                "client_id": self._client_id,
                "client_secret": self._client_secret,
            },
        )
        if response.status_code != 200:
            raise AuthenticationError(
                f"Token request failed: {response.status_code} - {response.text}"
            )
        return response.json()["access_token"]

    async def _get_token(self) -> str:
        """Get current token or obtain new one."""
        if self._token:
            return self._token

        async with self._lock:
            # Double-check after acquiring lock
            if self._token:
                return self._token
            self._token = await self._obtain_token()
            return self._token

    async def _clear_token(self) -> None:
        """Clear cached token (called on 401)."""
        async with self._lock:
            self._token = None

    async def _request_with_retry(
        self,
        method: str,
        url: str,
        json: dict | None = None,
    ) -> httpx.Response:
        """Make request with automatic token refresh on 401."""
        token = await self._get_token()

        response = await self._http.request(
            method,
            url,
            headers={"Authorization": f"Bearer {token}"},
            json=json,
        )

        # Handle 401 by refreshing token and retrying once
        if response.status_code == 401:
            await self._clear_token()
            token = await self._get_token()
            response = await self._http.request(
                method,
                url,
                headers={"Authorization": f"Bearer {token}"},
                json=json,
            )

        return response

    async def poll_activities(
        self,
        worker: str,
        worker_id: str,
        max_activities: int,
    ) -> list[PendingActivity]:
        """
        Poll for activities.

        POST /api/v1/workers/poll
        """
        response = await self._request_with_retry(
            "POST",
            f"{self._api_url}/api/v1/workers/poll",
            json={
                "worker": worker,
                "worker_id": worker_id,
                "max_activities": max_activities,
            },
        )

        response.raise_for_status()
        data = response.json()

        return [PendingActivity.model_validate(a) for a in data["activities"]]

    async def heartbeat(self, activity_id: UUID, worker_id: str) -> None:
        """
        Send heartbeat for activity.

        POST /api/v1/activities/{activity_id}/heartbeat
        """
        response = await self._request_with_retry(
            "POST",
            f"{self._api_url}/api/v1/activities/{activity_id}/heartbeat",
            json={"worker_id": worker_id},
        )
        response.raise_for_status()

    async def complete_activity(
        self,
        activity_id: UUID,
        worker_id: str,
        output: dict,
        cost_usd: Decimal | None = None,
    ) -> None:
        """
        Complete activity successfully.

        POST /api/v1/activities/{activity_id}/complete
        """
        body: dict[str, Any] = {"worker_id": worker_id, "output": output}
        if cost_usd is not None:
            body["cost_usd"] = str(cost_usd)

        response = await self._request_with_retry(
            "POST",
            f"{self._api_url}/api/v1/activities/{activity_id}/complete",
            json=body,
        )
        response.raise_for_status()

    async def fail_activity(
        self,
        activity_id: UUID,
        worker_id: str,
        error_code: str,
        error_message: str,
        retryable: bool = True,
    ) -> None:
        """
        Fail activity.

        POST /api/v1/activities/{activity_id}/fail
        """
        response = await self._request_with_retry(
            "POST",
            f"{self._api_url}/api/v1/activities/{activity_id}/fail",
            json={
                "worker_id": worker_id,
                "error": {
                    "code": error_code,
                    "message": error_message,
                    "retryable": retryable,
                },
            },
        )
        response.raise_for_status()

    async def upload_file(
        self,
        workflow_id: UUID,
        activity_key: str,
        filename: str,
        data: AsyncIterator[bytes],
        content_type: str | None = None,
    ) -> FileMetadata:
        """
        Upload a file to workflow storage.

        POST /api/v1/workflows/{workflow_id}/activities/{activity_key}/files/{filename}

        Args:
            workflow_id: Workflow instance ID
            activity_key: Activity key within workflow
            filename: Target filename
            data: Async iterator yielding file chunks
            content_type: Optional MIME type

        Returns:
            FileMetadata for the uploaded file
        """
        from .storage import FileMetadata

        token = await self._get_token()

        # Collect data into bytes for upload
        # Note: For very large files, we'd want true streaming, but httpx
        # doesn't easily support async iterators as request content
        chunks = []
        async for chunk in data:
            chunks.append(chunk)
        content = b"".join(chunks)

        headers = {"Authorization": f"Bearer {token}"}
        if content_type:
            headers["Content-Type"] = content_type

        url = (
            f"{self._api_url}/api/v1/workflows/{workflow_id}"
            f"/activities/{activity_key}/files/{filename}"
        )

        response = await self._http.post(url, headers=headers, content=content)

        # Handle 401 by refreshing token and retrying once
        if response.status_code == 401:
            await self._clear_token()
            token = await self._get_token()
            headers["Authorization"] = f"Bearer {token}"
            response = await self._http.post(url, headers=headers, content=content)

        response.raise_for_status()

        # Parse response metadata
        data_json = response.json()
        return FileMetadata(
            workflow_id=workflow_id,
            activity_key=activity_key,
            filename=filename,
            size=data_json.get("size", len(content)),
            content_type=content_type,
            created_at=datetime.now(timezone.utc),
        )

    async def download_file(
        self,
        workflow_id: UUID,
        activity_key: str,
        filename: str,
    ) -> AsyncIterator[bytes]:
        """
        Download a file from workflow storage.

        GET /api/v1/workflows/{workflow_id}/activities/{activity_key}/files/{filename}

        Args:
            workflow_id: Workflow instance ID
            activity_key: Activity key within workflow
            filename: Filename to download

        Yields:
            File content in chunks
        """
        from .storage.errors import FileNotFoundError

        token = await self._get_token()

        url = (
            f"{self._api_url}/api/v1/workflows/{workflow_id}"
            f"/activities/{activity_key}/files/{filename}"
        )

        async with self._http.stream(
            "GET",
            url,
            headers={"Authorization": f"Bearer {token}"},
        ) as response:
            # Handle 401 by refreshing token and retrying
            if response.status_code == 401:
                await self._clear_token()
                token = await self._get_token()
                # Fall through to check status and retry below

            if response.status_code == 404:
                raise FileNotFoundError(f"{workflow_id}/{activity_key}/{filename}")

            if response.status_code == 401:
                # Retry with new token
                async with self._http.stream(
                    "GET",
                    url,
                    headers={"Authorization": f"Bearer {token}"},
                ) as retry_response:
                    retry_response.raise_for_status()
                    async for chunk in retry_response.aiter_bytes(chunk_size=8192):
                        yield chunk
                return

            response.raise_for_status()
            async for chunk in response.aiter_bytes(chunk_size=8192):
                yield chunk
