"""MCP server setup and configuration."""

import logging

from fastmcp import FastMCP

from .client import KruxiaFlowClient
from .config import Settings

logger = logging.getLogger(__name__)


def create_server(settings: Settings) -> FastMCP:
    """Create and configure the MCP server.

    Args:
        settings: Server configuration

    Returns:
        Configured FastMCP server instance
    """
    logger.info("Initializing MCP server")

    # Create MCP server instance
    mcp = FastMCP(
        name="kruxiaflow",
        version="0.1.0",
        description="Kruxia Flow Workflow Orchestration MCP Server",
    )

    # Initialize API client
    client = KruxiaFlowClient(
        base_url=settings.kruxiaflow_url,
        token=settings.kruxiaflow_token,
    )

    logger.info(f"Initialized Kruxia Flow client: {settings.kruxiaflow_url}")

    # Register tool groups
    from .tools.control import register_control_tools
    from .tools.discovery import register_discovery_tools
    from .tools.execution import register_execution_tools
    from .tools.observability import register_observability_tools
    from .tools.visualization import register_visualization_tools

    register_discovery_tools(mcp, client)
    register_execution_tools(mcp, client)
    register_observability_tools(mcp, client)
    register_visualization_tools(mcp, client)
    register_control_tools(mcp, client)

    logger.info(
        "MCP server initialized with all 13 tools (discovery, execution, observability, visualization, control)"
    )

    return mcp
