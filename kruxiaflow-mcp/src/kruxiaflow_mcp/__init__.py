"""Kruxia Flow MCP Server.

Model Context Protocol server for Kruxia Flow workflow orchestration.
"""

__version__ = "0.1.0"

from .config import Settings
from .server import create_server

__all__ = ["Settings", "__version__", "create_server"]
