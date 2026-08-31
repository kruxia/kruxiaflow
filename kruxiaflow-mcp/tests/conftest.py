"""Pytest configuration and fixtures."""

from unittest.mock import AsyncMock, Mock

import pytest

from kruxiaflow_mcp.client import KruxiaFlowClient
from kruxiaflow_mcp.config import Settings


@pytest.fixture
def mock_settings() -> Settings:
    """Create mock settings for testing."""
    return Settings(
        kruxiaflow_url="http://localhost:8080",
        kruxiaflow_token="test_token_12345",
        mcp_transport="stdio",
        mcp_log_level="debug",
    )


@pytest.fixture
async def mock_client() -> AsyncMock:
    """Create mock Kruxia Flow API client."""
    client = AsyncMock(spec=KruxiaFlowClient)
    client.base_url = "http://localhost:8080"
    client.token = "test_token"
    return client


@pytest.fixture
def sample_workflow_definition() -> dict:
    """Sample workflow definition for testing."""
    return {
        "name": "test_workflow",
        "description": "Test workflow for unit tests",
        "activities": [
            {
                "key": "step1",
                "activity_name": "http_request",
                "parameters": {
                    "method": "GET",
                    "url": "https://api.example.com/data",
                },
                "outputs": ["response"],
            },
            {
                "key": "step2",
                "activity_name": "llm_prompt",
                "parameters": {
                    "model": "anthropic/claude-sonnet-4-5-20250929",
                    "prompt": "Analyze: {{step1.response.body}}",
                    "max_tokens": 1000,
                },
                "outputs": ["result"],
                "depends_on": ["step1"],
            },
        ],
    }
