# Kruxia Flow MCP Server

Model Context Protocol (MCP) server for Kruxia Flow workflow orchestration.

## Overview

This MCP server enables AI agents (Claude Code, IDE extensions, custom agents) to interact with Kruxia Flow through a standardized protocol. Agents can:

- Discover and validate workflow definitions
- Submit and monitor workflow executions
- Query costs and make budget-aware decisions
- Generate visual workflow diagrams
- Interact with human-in-the-loop workflows

## Installation

### Development

```bash
# Create virtual environment
python -m venv venv
source venv/bin/activate  # On Windows: venv\Scripts\activate

# Install in development mode
pip install -e ".[dev]"

# Run tests
pytest

# Run linting
ruff check .
mypy src/
```

### Production

```bash
pip install kruxiaflow-mcp
```

## Configuration

Configuration via environment variables:

```bash
# Required
export KRUXIAFLOW_URL=http://localhost:8080
export KRUXIAFLOW_TOKEN=<jwt_token>

# Optional
export MCP_LOG_LEVEL=info           # debug, info, warning, error
export MCP_TRANSPORT=stdio          # stdio or http
export MCP_HTTP_PORT=8081           # if http transport
export MCP_DEBUG=false              # enable debug mode
export MCP_LOG_FILE=/tmp/mcp.log   # log file path

# Observability
export OTEL_EXPORTER_OTLP_ENDPOINT=localhost:4317
export PROMETHEUS_PORT=9090
```

## Usage

### Standalone

```bash
# Start MCP server (stdio transport for Claude Code)
kruxiaflow-mcp

# Start with HTTP transport
MCP_TRANSPORT=http kruxiaflow-mcp
```

### Claude Code Integration

Add to `~/.config/claude-code/mcp.json`:

```json
{
  "servers": {
    "kruxiaflow": {
      "command": "kruxiaflow-mcp",
      "env": {
        "KRUXIAFLOW_URL": "http://localhost:8080",
        "KRUXIAFLOW_TOKEN": "your_jwt_token",
        "MCP_LOG_LEVEL": "debug"
      }
    }
  }
}
```

### VS Code Extension

Add to `.vscode/settings.json`:

```json
{
  "mcp.servers": {
    "kruxiaflow": {
      "command": "kruxiaflow-mcp",
      "env": {
        "KRUXIAFLOW_URL": "http://localhost:8080",
        "KRUXIAFLOW_TOKEN": "${env:KRUXIAFLOW_TOKEN}"
      }
    }
  }
}
```

## Development

### Project Structure

```
kruxiaflow-mcp/
├── src/kruxiaflow_mcp/
│   ├── __init__.py
│   ├── __main__.py          # Entry point
│   ├── server.py            # MCP server setup
│   ├── config.py            # Configuration management
│   ├── client.py            # Kruxia Flow API client
│   ├── tools/               # MCP tool implementations
│   │   ├── discovery.py     # list_*, get_* tools
│   │   ├── execution.py     # submit_*, cancel_* tools
│   │   ├── observability.py # status, cost tools
│   │   ├── visualization.py # render_workflow_diagram
│   │   └── control.py       # signal, human-in-loop
│   ├── schemas/             # Validation schemas
│   │   ├── workflow.py
│   │   └── responses.py
│   └── utils/               # Utilities
│       ├── mermaid.py       # Diagram generator
│       └── cost.py          # Cost estimation
└── tests/
    ├── test_tools/          # Tool tests
    └── fixtures/            # Test data
```

### Running Tests

```bash
# All tests
pytest

# Specific test file
pytest tests/test_tools/test_discovery.py

# With coverage
pytest --cov=kruxiaflow_mcp --cov-report=html

# Integration tests (requires running Kruxia Flow)
pytest tests/integration/
```

### Code Quality

```bash
# Format code
black src/ tests/

# Lint
ruff check src/ tests/

# Type checking
mypy src/

# All checks
black src/ tests/ && ruff check src/ tests/ && mypy src/ && pytest
```

## MCP Tools

The server exposes 13 MCP tools:

### Discovery
- `list_workflow_definitions` - List available workflow definitions
- `get_workflow_definition` - Get workflow definition details
- `list_activities` - List available activity types

### Execution
- `validate_workflow` - Validate workflow YAML
- `submit_workflow` - Submit workflow for execution
- `cancel_workflow` - Cancel running workflow

### Observability
- `get_workflow_status` - Get workflow status
- `list_workflows` - List workflows with filters
- `get_activity_output` - Get activity output
- `get_workflow_cost` - Get workflow cost breakdown
- `estimate_workflow_cost` - Estimate cost before execution

### Visualization
- `render_workflow_diagram` - Generate Mermaid diagram

### Control
- `send_workflow_signal` - Send signal to waiting workflow

## Architecture

See [Development Plan](../docs/implementation/mcp-server-development-plan.md) for complete architecture documentation.

## License

MIT
