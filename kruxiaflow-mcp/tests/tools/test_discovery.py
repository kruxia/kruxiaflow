"""Tests for discovery tools."""

from unittest.mock import AsyncMock, MagicMock

import pytest

from kruxiaflow_mcp.client import KruxiaFlowClient


@pytest.fixture
def mock_client() -> KruxiaFlowClient:
    """Create a mock Kruxia Flow client."""
    client = MagicMock(spec=KruxiaFlowClient)
    client.get_workflow_definitions = AsyncMock()
    client.get_workflow_definition = AsyncMock()
    return client


@pytest.mark.asyncio
async def test_client_get_workflow_definitions(mock_client: KruxiaFlowClient) -> None:
    """Test getting workflow definitions through client."""
    mock_client.get_workflow_definitions.return_value = {
        "definitions": [
            {"name": "weather_report", "description": "Fetch weather forecast"},
            {"name": "user_validation", "description": "Validate user data"},
        ],
        "total": 2,
        "limit": 20,
        "offset": 0,
    }

    result = await mock_client.get_workflow_definitions()

    assert result["total"] == 2
    assert len(result["definitions"]) == 2
    assert result["definitions"][0]["name"] == "weather_report"


@pytest.mark.asyncio
async def test_client_get_workflow_definition(mock_client: KruxiaFlowClient) -> None:
    """Test getting a specific workflow definition through client."""
    mock_client.get_workflow_definition.return_value = {
        "name": "weather_report",
        "description": "Fetch weather forecast",
        "activities": [
            {
                "key": "fetch_weather",
                "activity_name": "http_request",
                "parameters": {"url": "https://api.weather.gov"},
            }
        ],
    }

    result = await mock_client.get_workflow_definition("weather_report")

    assert result["name"] == "weather_report"
    assert len(result["activities"]) == 1
    assert result["activities"][0]["key"] == "fetch_weather"
