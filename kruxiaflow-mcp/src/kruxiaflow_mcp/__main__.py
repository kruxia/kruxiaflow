"""MCP server entry point."""

import asyncio
import logging
import sys

from .config import Settings, TransportType
from .server import create_server

logger = logging.getLogger(__name__)


async def run_stdio() -> None:
    """Run MCP server with stdio transport."""
    settings = Settings.from_env()
    server = create_server(settings)

    logger.info("Starting MCP server with stdio transport")
    logger.info(f"Connecting to Kruxia Flow at {settings.kruxiaflow_url}")

    # FastMCP handles stdio transport automatically
    await server.run()


async def run_http(port: int) -> None:
    """Run MCP server with HTTP transport."""
    settings = Settings.from_env()
    server = create_server(settings)

    logger.info(f"Starting MCP server with HTTP transport on port {port}")
    logger.info(f"Connecting to Kruxia Flow at {settings.kruxiaflow_url}")

    # FastMCP handles HTTP+SSE transport
    await server.run_http(port=port)


def main() -> None:
    """Main entry point."""
    try:
        settings = Settings.from_env()

        if settings.mcp_transport == TransportType.STDIO:
            asyncio.run(run_stdio())
        else:
            asyncio.run(run_http(settings.mcp_http_port))

    except KeyboardInterrupt:
        logger.info("Shutting down gracefully...")
    except Exception as e:
        logger.error(f"Fatal error: {e}", exc_info=True)
        sys.exit(1)


if __name__ == "__main__":
    main()
