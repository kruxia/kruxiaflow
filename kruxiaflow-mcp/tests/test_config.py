"""Tests for configuration management."""

import pytest

from kruxiaflow_mcp.config import LogLevel, Settings, TransportType


def test_settings_defaults() -> None:
    """Test default settings values."""
    settings = Settings(
        kruxiaflow_url="http://localhost:8080",
        kruxiaflow_token="test_token",
    )

    assert settings.mcp_transport == TransportType.STDIO
    assert settings.mcp_http_port == 8081
    assert settings.mcp_debug is False
    assert settings.mcp_log_level == LogLevel.INFO


def test_settings_from_env(monkeypatch: pytest.MonkeyPatch) -> None:
    """Test loading settings from environment variables."""
    monkeypatch.setenv("KRUXIAFLOW_URL", "http://test:8080")
    monkeypatch.setenv("KRUXIAFLOW_TOKEN", "token123")
    monkeypatch.setenv("MCP_TRANSPORT", "http")
    monkeypatch.setenv("MCP_HTTP_PORT", "9000")
    monkeypatch.setenv("MCP_LOG_LEVEL", "debug")

    settings = Settings()

    assert settings.kruxiaflow_url == "http://test:8080"
    assert settings.kruxiaflow_token == "token123"
    assert settings.mcp_transport == TransportType.HTTP
    assert settings.mcp_http_port == 9000
    assert settings.mcp_log_level == LogLevel.DEBUG
