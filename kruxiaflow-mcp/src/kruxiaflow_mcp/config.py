"""Configuration management for MCP server."""

import logging
import sys
from enum import Enum
from pathlib import Path

from pydantic import Field
from pydantic_settings import BaseSettings, SettingsConfigDict


class TransportType(str, Enum):
    """MCP transport types."""

    STDIO = "stdio"
    HTTP = "http"


class LogLevel(str, Enum):
    """Logging levels."""

    DEBUG = "debug"
    INFO = "info"
    WARNING = "warning"
    ERROR = "error"


class Settings(BaseSettings):
    """MCP server configuration.

    Configuration is loaded from environment variables with KRUXIAFLOW_ and MCP_ prefixes.
    """

    model_config = SettingsConfigDict(
        env_prefix="",
        env_file=".env",
        env_file_encoding="utf-8",
        case_sensitive=False,
        extra="ignore",
    )

    # Required: Kruxia Flow connection
    kruxiaflow_url: str = Field(
        ...,
        description="Kruxia Flow API base URL",
        examples=["http://localhost:8080"],
    )
    kruxiaflow_token: str = Field(
        ...,
        description="Kruxia Flow JWT authentication token",
    )

    # MCP server configuration
    mcp_transport: TransportType = Field(
        default=TransportType.STDIO,
        description="Transport type: stdio or http",
    )
    mcp_http_port: int = Field(
        default=8081,
        ge=1,
        le=65535,
        description="HTTP port (if transport=http)",
    )
    mcp_debug: bool = Field(
        default=False,
        description="Enable debug mode",
    )
    mcp_log_level: LogLevel = Field(
        default=LogLevel.INFO,
        description="Logging level",
    )
    mcp_log_file: str | None = Field(
        default=None,
        description="Log file path (default: stderr only)",
    )

    # Observability
    otel_exporter_otlp_endpoint: str | None = Field(
        default=None,
        description="OpenTelemetry OTLP endpoint",
        examples=["localhost:4317"],
    )
    prometheus_port: int | None = Field(
        default=None,
        ge=1,
        le=65535,
        description="Prometheus metrics port",
    )

    # Database (for audit logging)
    database_url: str | None = Field(
        default=None,
        description="PostgreSQL connection URL for audit logging",
        examples=["postgresql://user:pass@localhost:5432/kruxiaflow"],
    )

    def setup_logging(self) -> None:
        """Configure logging based on settings."""
        # Map log level enum to logging module level
        level_map = {
            LogLevel.DEBUG: logging.DEBUG,
            LogLevel.INFO: logging.INFO,
            LogLevel.WARNING: logging.WARNING,
            LogLevel.ERROR: logging.ERROR,
        }
        log_level = level_map[self.mcp_log_level]

        # Create formatter with detailed format
        formatter = logging.Formatter(
            "%(asctime)s.%(msecs)03d [%(levelname)s] %(name)s: %(message)s",
            datefmt="%Y-%m-%dT%H:%M:%S",
        )

        # Root logger
        root_logger = logging.getLogger()
        root_logger.setLevel(log_level)

        # Remove existing handlers
        root_logger.handlers.clear()

        # Console handler (stderr for MCP compatibility)
        console_handler = logging.StreamHandler(sys.stderr)
        console_handler.setFormatter(formatter)
        console_handler.setLevel(log_level)
        root_logger.addHandler(console_handler)

        # File handler (optional)
        if self.mcp_log_file:
            log_path = Path(self.mcp_log_file)
            log_path.parent.mkdir(parents=True, exist_ok=True)
            file_handler = logging.FileHandler(log_path)
            file_handler.setFormatter(formatter)
            file_handler.setLevel(log_level)
            root_logger.addHandler(file_handler)

        # Set specific logger levels
        if self.mcp_debug:
            logging.getLogger("kruxiaflow_mcp").setLevel(logging.DEBUG)
            logging.getLogger("httpx").setLevel(logging.DEBUG)
        else:
            logging.getLogger("httpx").setLevel(logging.WARNING)
            logging.getLogger("urllib3").setLevel(logging.WARNING)

        logging.info(
            f"Logging configured: level={self.mcp_log_level.value}, "
            f"file={self.mcp_log_file or 'stderr'}"
        )

    @classmethod
    def from_env(cls) -> "Settings":
        """Load settings from environment variables.

        Returns:
            Configured Settings instance

        Raises:
            ValueError: If required settings are missing
        """
        try:
            settings = cls()
            settings.setup_logging()
            return settings
        except Exception as e:
            print(f"Configuration error: {e}", file=sys.stderr)
            print("\nRequired environment variables:", file=sys.stderr)
            print("  KRUXIAFLOW_URL - Kruxia Flow API URL", file=sys.stderr)
            print("  KRUXIAFLOW_TOKEN - JWT authentication token", file=sys.stderr)
            print("\nOptional environment variables:", file=sys.stderr)
            print("  MCP_TRANSPORT - Transport type (stdio|http, default: stdio)", file=sys.stderr)
            print("  MCP_HTTP_PORT - HTTP port (default: 8081)", file=sys.stderr)
            print(
                "  MCP_LOG_LEVEL - Log level (debug|info|warning|error, default: info)",
                file=sys.stderr,
            )
            print("  MCP_LOG_FILE - Log file path", file=sys.stderr)
            print("  MCP_DEBUG - Enable debug mode (default: false)", file=sys.stderr)
            sys.exit(1)


# Singleton instance
_settings: Settings | None = None


def get_settings() -> Settings:
    """Get or create settings instance."""
    global _settings
    if _settings is None:
        _settings = Settings.from_env()
    return _settings
