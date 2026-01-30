"""Tests for Kruxia Flow API client."""

from datetime import timedelta
from unittest.mock import AsyncMock, MagicMock, patch

import httpx
import pytest

from kruxiaflow_mcp.client import KruxiaFlowClient


@pytest.mark.asyncio
async def test_client_initialization() -> None:
    """Test client initialization."""
    client = KruxiaFlowClient(
        base_url="http://localhost:8080",
        token="test_token",
    )

    assert client.base_url == "http://localhost:8080"
    assert client.token == "test_token"
    assert "Bearer test_token" in client.client.headers["Authorization"]

    await client.close()


@pytest.mark.asyncio
async def test_client_get_request() -> None:
    """Test GET request."""
    client = KruxiaFlowClient(
        base_url="http://localhost:8080",
        token="test_token",
    )

    with patch.object(client.client, "request", new_callable=AsyncMock) as mock_request:
        mock_response = MagicMock()
        mock_response.status_code = 200
        mock_response.json.return_value = {"result": "success"}
        mock_response.elapsed = timedelta(seconds=0.123)
        mock_response.raise_for_status = MagicMock()
        mock_request.return_value = mock_response

        result = await client.get("/api/v1/test")

        assert result == {"result": "success"}
        mock_request.assert_called_once()

    await client.close()


@pytest.mark.asyncio
async def test_client_handles_errors() -> None:
    """Test error handling."""
    client = KruxiaFlowClient(
        base_url="http://localhost:8080",
        token="test_token",
    )

    with patch.object(client.client, "request", new_callable=AsyncMock) as mock_request:
        mock_request.side_effect = httpx.HTTPStatusError(
            "Not found",
            request=AsyncMock(),
            response=AsyncMock(status_code=404, text="Not found"),
        )

        with pytest.raises(httpx.HTTPStatusError):
            await client.get("/api/v1/nonexistent")

    await client.close()
