# Development Plan: Kruxia Flow MCP Server

**Epic**: MCP Server for AI Agent Integration
**Status**: 🔮 Planned
**Priority**: P1 (High)
**Author**: Engineering Team
**Last Updated**: 2026-01-30

---

## 1. Project Overview

### 1.1 Summary

Build an MCP (Model Context Protocol) server that exposes Kruxia Flow's workflow orchestration capabilities to AI agents. The server implements the MCP specification to enable Claude, IDE extensions, and custom AI agents to:

- Discover and validate workflow definitions
- Submit and monitor workflow executions
- Query costs and make budget-aware decisions
- Generate visual workflow diagrams
- Interact with human-in-the-loop workflows

### 1.2 Architecture Overview

```mermaid
flowchart TB
    subgraph Agents["AI Agent Layer"]
        CC[Claude Code]
        CD[Claude Desktop]
        IDE[IDE Extensions]
        Custom[Custom Agents]
    end

    subgraph MCP["MCP Server Layer"]
        direction TB
        Transport[Transport Layer<br/>stdio / HTTP+SSE]
        Protocol[Protocol Handler<br/>JSON-RPC 2.0]
        Tools[Tool Registry<br/>13 MCP Tools]
        Client[Kruxia API Client<br/>HTTP + WebSocket]
    end

    subgraph KF["Kruxia Flow Layer"]
        API[API Server<br/>:8080]
        Orch[Orchestrator]
        Worker[Workers]
        PG[(PostgreSQL)]
    end

    Agents -->|MCP Protocol| Transport
    Transport --> Protocol
    Protocol --> Tools
    Tools --> Client
    Client -->|REST API| API
    Client -->|WebSocket| API
    API --> PG
    Orch --> PG
    Worker --> PG
```

---

## 2. Implementation Options

The MCP server can be implemented in either Python or Rust. This section provides specifications for both approaches.

### 2.1 Option A: Python with FastMCP (Recommended for MVP)

**Pros**:
- Rapid development with FastMCP library
- Rich ecosystem for HTTP clients, JSON handling
- Easy debugging and iteration
- Matches MCP reference implementations

**Cons**:
- Additional runtime dependency (Python 3.11+)
- Separate process from Kruxia Flow binary

#### Project Structure (Python)

```
kruxiaflow-mcp/
├── pyproject.toml
├── README.md
├── src/
│   └── kruxiaflow_mcp/
│       ├── __init__.py
│       ├── __main__.py           # Entry point
│       ├── server.py             # MCP server setup
│       ├── config.py             # Configuration management
│       ├── client.py             # Kruxia Flow API client
│       ├── tools/
│       │   ├── __init__.py
│       │   ├── discovery.py      # list_*, get_* tools
│       │   ├── execution.py      # submit_*, cancel_* tools
│       │   ├── observability.py  # status, output, cost tools
│       │   ├── visualization.py  # render_workflow_diagram
│       │   └── control.py        # signal, human-in-loop tools
│       ├── schemas/
│       │   ├── __init__.py
│       │   ├── workflow.py       # Workflow validation schemas
│       │   └── responses.py      # Response type definitions
│       └── utils/
│           ├── __init__.py
│           ├── mermaid.py        # Mermaid diagram generator
│           └── cost.py           # Cost estimation utilities
├── tests/
│   ├── conftest.py
│   ├── test_protocol.py
│   ├── test_tools/
│   │   ├── test_discovery.py
│   │   ├── test_execution.py
│   │   └── test_observability.py
│   └── fixtures/
│       └── workflows/
└── docker/
    └── Dockerfile
```

#### Core Implementation (Python)

**pyproject.toml**:
```toml
[project]
name = "kruxiaflow-mcp"
version = "0.1.0"
description = "MCP Server for Kruxia Flow Workflow Orchestration"
requires-python = ">=3.11"
dependencies = [
    "fastmcp>=0.4.0",
    "httpx>=0.27.0",
    "pydantic>=2.0.0",
    "pyyaml>=6.0.0",
    "websockets>=12.0",
]

[project.optional-dependencies]
dev = [
    "pytest>=8.0.0",
    "pytest-asyncio>=0.23.0",
    "pytest-httpx>=0.30.0",
    "ruff>=0.3.0",
    "mypy>=1.9.0",
]

[project.scripts]
kruxiaflow-mcp = "kruxiaflow_mcp.__main__:main"

[tool.ruff]
line-length = 100
target-version = "py311"
```

**src/kruxiaflow_mcp/server.py**:
```python
"""Kruxia Flow MCP Server implementation."""

from fastmcp import FastMCP
from fastmcp.tools import tool

from .client import KruxiaFlowClient
from .config import Settings
from .tools.discovery import register_discovery_tools
from .tools.execution import register_execution_tools
from .tools.observability import register_observability_tools
from .tools.visualization import register_visualization_tools
from .tools.control import register_control_tools

def create_server(settings: Settings) -> FastMCP:
    """Create and configure the MCP server."""

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

    # Register tool groups
    register_discovery_tools(mcp, client)
    register_execution_tools(mcp, client)
    register_observability_tools(mcp, client)
    register_visualization_tools(mcp, client)
    register_control_tools(mcp, client)

    return mcp
```

**src/kruxiaflow_mcp/tools/discovery.py**:
```python
"""Discovery tools for listing and inspecting workflow definitions."""

from typing import Optional
from fastmcp import FastMCP
from fastmcp.tools import tool
from pydantic import BaseModel, Field

from ..client import KruxiaFlowClient

class ListWorkflowDefinitionsInput(BaseModel):
    """Input schema for list_workflow_definitions."""
    namespace: Optional[str] = Field(None, description="Filter by namespace")
    limit: int = Field(20, ge=1, le=100, description="Maximum results")
    offset: int = Field(0, ge=0, description="Pagination offset")

class WorkflowDefinitionSummary(BaseModel):
    """Summary of a workflow definition."""
    name: str
    namespace: Optional[str]
    description: Optional[str]
    version: str
    activity_count: int
    created_at: str

def register_discovery_tools(mcp: FastMCP, client: KruxiaFlowClient):
    """Register discovery tools with the MCP server."""

    @mcp.tool()
    async def list_workflow_definitions(
        namespace: Optional[str] = None,
        limit: int = 20,
        offset: int = 0,
    ) -> dict:
        """
        List available workflow definitions.

        Returns workflow definitions that can be executed. Use this to discover
        what workflows are available before submitting executions.

        Args:
            namespace: Optional namespace filter
            limit: Maximum number of results (1-100, default 20)
            offset: Pagination offset

        Returns:
            List of workflow definition summaries with names, descriptions,
            and activity counts.
        """
        response = await client.get(
            "/api/v1/workflow_definitions",
            params={"namespace": namespace, "limit": limit, "offset": offset}
        )
        return {
            "definitions": response["definitions"],
            "total_count": response["total_count"],
            "has_more": response["total_count"] > offset + limit,
        }

    @mcp.tool()
    async def get_workflow_definition(name: str) -> dict:
        """
        Get detailed information about a workflow definition.

        Returns the full workflow definition including all activities,
        dependencies, and configuration.

        Args:
            name: The workflow definition name

        Returns:
            Complete workflow definition with activities, dependencies,
            and settings.
        """
        response = await client.get(f"/api/v1/workflow_definitions/{name}")
        return response

    @mcp.tool()
    async def list_activities() -> dict:
        """
        List available activity types for building workflows.

        Returns all built-in activity types with their parameter schemas
        and descriptions.

        Returns:
            List of activity types including http_request, llm_prompt,
            postgres_query, embedding, send_email, etc.
        """
        # Built-in activities are well-known
        return {
            "activities": [
                {
                    "name": "http_request",
                    "description": "Make HTTP requests (GET, POST, PUT, DELETE)",
                    "parameters": ["method", "url", "headers", "body", "query_params"],
                },
                {
                    "name": "llm_prompt",
                    "description": "Send prompts to LLM providers with automatic fallback",
                    "parameters": ["model", "prompt", "system_prompt", "max_tokens", "temperature"],
                },
                {
                    "name": "postgres_query",
                    "description": "Execute PostgreSQL queries",
                    "parameters": ["query", "params", "db_url"],
                },
                {
                    "name": "embedding",
                    "description": "Generate vector embeddings",
                    "parameters": ["model", "input"],
                },
                {
                    "name": "send_email",
                    "description": "Send emails via SMTP",
                    "parameters": ["to", "subject", "text_body", "html_body"],
                },
            ]
        }
```

**src/kruxiaflow_mcp/tools/visualization.py**:
```python
"""Visualization tools for generating workflow diagrams."""

from typing import Optional, Literal
from fastmcp import FastMCP

from ..client import KruxiaFlowClient
from ..utils.mermaid import generate_workflow_diagram

def register_visualization_tools(mcp: FastMCP, client: KruxiaFlowClient):
    """Register visualization tools with the MCP server."""

    @mcp.tool()
    async def render_workflow_diagram(
        workflow_id: Optional[str] = None,
        workflow_definition: Optional[str] = None,
        format: Literal["mermaid"] = "mermaid",
        include_status: bool = False,
    ) -> dict:
        """
        Generate a visual diagram of a workflow.

        Creates a Mermaid flowchart showing the workflow structure,
        activity dependencies, and optionally execution status.

        Args:
            workflow_id: ID of a running/completed workflow (for status)
            workflow_definition: Name of a workflow definition
            format: Output format (currently only 'mermaid')
            include_status: Include execution status colors (requires workflow_id)

        Returns:
            Mermaid diagram code that can be rendered in markdown.

        Example output:
            ```mermaid
            flowchart TB
                step1[Step 1] --> step2[Step 2]
                step2 --> step3[Step 3]
            ```
        """
        if workflow_id:
            # Fetch workflow with execution status
            workflow = await client.get(f"/api/v1/workflows/{workflow_id}")
            definition = workflow.get("definition", {})
            activities_status = {
                a["key"]: a["status"]
                for a in workflow.get("activities", [])
            }
        elif workflow_definition:
            # Fetch definition only
            definition = await client.get(
                f"/api/v1/workflow_definitions/{workflow_definition}"
            )
            activities_status = None
        else:
            return {"error": "Provide either workflow_id or workflow_definition"}

        diagram = generate_workflow_diagram(
            definition=definition,
            activities_status=activities_status if include_status else None,
        )

        return {
            "format": format,
            "diagram": diagram,
            "rendered": f"```mermaid\n{diagram}\n```",
        }
```

**src/kruxiaflow_mcp/utils/mermaid.py**:
```python
"""Mermaid diagram generation utilities."""

from typing import Optional

def generate_workflow_diagram(
    definition: dict,
    activities_status: Optional[dict[str, str]] = None,
) -> str:
    """
    Generate Mermaid flowchart from workflow definition.

    Args:
        definition: Workflow definition dict with activities
        activities_status: Optional dict mapping activity_key to status

    Returns:
        Mermaid flowchart diagram code
    """
    lines = ["flowchart TB"]

    activities = definition.get("activities", [])

    # Build node definitions
    for activity in activities:
        key = activity["key"]
        activity_name = activity.get("activity_name", "unknown")

        # Determine status indicator
        status_icon = ""
        if activities_status:
            status = activities_status.get(key, "pending")
            status_icon = {
                "completed": "✅ ",
                "running": "🔄 ",
                "failed": "❌ ",
                "pending": "⏳ ",
                "waiting": "⏸️ ",
            }.get(status, "")

        # Build node label
        label = f"{status_icon}{key}<br/>{activity_name}"

        # Check for special properties
        if activity.get("iteration_scoped"):
            label += "<br/>🔁 loop"

        lines.append(f'    {key}["{label}"]')

    # Build edges from depends_on
    for activity in activities:
        key = activity["key"]
        depends_on = activity.get("depends_on", [])

        for dep in depends_on:
            # Handle simple string or dict dependency
            if isinstance(dep, str):
                dep_key = dep
                condition = None
            else:
                dep_key = dep.get("activity_key", dep.get("key"))
                condition = dep.get("conditions", dep.get("condition"))

            if condition:
                # Conditional edge
                lines.append(f'    {dep_key} -->|"condition"| {key}')
            else:
                lines.append(f"    {dep_key} --> {key}")

    # Add status-based styling
    if activities_status:
        lines.append("")
        for key, status in activities_status.items():
            color = {
                "completed": "#90EE90",  # Light green
                "running": "#FFD700",    # Gold
                "failed": "#FF6B6B",     # Red
                "pending": "#D3D3D3",    # Light gray
                "waiting": "#87CEEB",    # Sky blue
            }.get(status)
            if color:
                lines.append(f"    style {key} fill:{color}")

    return "\n".join(lines)
```

---

### 2.2 Option B: Rust with MCP SDK

**Pros**:
- Single binary deployment with Kruxia Flow
- Native performance
- No additional runtime dependencies

**Cons**:
- More development effort
- MCP Rust SDK is less mature
- Longer iteration cycles

#### Project Structure (Rust)

```
kruxiaflow/
├── Cargo.toml
├── src/
│   ├── main.rs
│   ├── lib.rs
│   └── mcp/                      # MCP server module
│       ├── mod.rs
│       ├── server.rs             # MCP server implementation
│       ├── transport.rs          # stdio/HTTP transport
│       ├── protocol.rs           # JSON-RPC 2.0 handling
│       ├── tools/
│       │   ├── mod.rs
│       │   ├── discovery.rs
│       │   ├── execution.rs
│       │   ├── observability.rs
│       │   ├── visualization.rs
│       │   └── control.rs
│       └── utils/
│           ├── mod.rs
│           └── mermaid.rs
```

#### Core Implementation (Rust)

**Cargo.toml additions**:
```toml
[features]
default = ["api", "orchestrator", "worker"]
mcp = ["dep:mcp-sdk"]

[dependencies]
mcp-sdk = { version = "0.1", optional = true }
```

**src/mcp/server.rs**:
```rust
//! MCP Server implementation for Kruxia Flow.

use mcp_sdk::{McpServer, Tool, ToolResult};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::api::client::ApiClient;

/// Kruxia Flow MCP Server
pub struct KruxiaFlowMcpServer {
    client: Arc<ApiClient>,
}

impl KruxiaFlowMcpServer {
    pub fn new(api_url: String, token: String) -> Self {
        Self {
            client: Arc::new(ApiClient::new(api_url, token)),
        }
    }

    pub fn register_tools(&self, server: &mut McpServer) {
        // Discovery tools
        server.register_tool(self.list_workflow_definitions_tool());
        server.register_tool(self.get_workflow_definition_tool());
        server.register_tool(self.list_activities_tool());

        // Execution tools
        server.register_tool(self.validate_workflow_tool());
        server.register_tool(self.submit_workflow_tool());
        server.register_tool(self.cancel_workflow_tool());

        // Observability tools
        server.register_tool(self.get_workflow_status_tool());
        server.register_tool(self.list_workflows_tool());
        server.register_tool(self.get_activity_output_tool());
        server.register_tool(self.get_workflow_cost_tool());
        server.register_tool(self.estimate_workflow_cost_tool());

        // Visualization tools
        server.register_tool(self.render_workflow_diagram_tool());

        // Control tools
        server.register_tool(self.send_workflow_signal_tool());
    }

    fn list_workflow_definitions_tool(&self) -> Tool {
        Tool::new(
            "list_workflow_definitions",
            "List available workflow definitions",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "namespace": {"type": "string"},
                    "limit": {"type": "integer", "default": 20},
                    "offset": {"type": "integer", "default": 0}
                }
            }),
        )
        .with_handler({
            let client = self.client.clone();
            move |args| {
                let client = client.clone();
                Box::pin(async move {
                    let namespace = args.get("namespace").and_then(|v| v.as_str());
                    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20);
                    let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0);

                    let response = client
                        .get_workflow_definitions(namespace, limit as usize, offset as usize)
                        .await?;

                    Ok(ToolResult::success(serde_json::to_value(response)?))
                })
            }
        })
    }

    // ... additional tool implementations
}
```

**src/mcp/utils/mermaid.rs**:
```rust
//! Mermaid diagram generation for workflow visualization.

use std::collections::HashMap;

use crate::workflow::definition::WorkflowDefinition;

/// Generate Mermaid flowchart from workflow definition.
pub fn generate_workflow_diagram(
    definition: &WorkflowDefinition,
    activities_status: Option<&HashMap<String, String>>,
) -> String {
    let mut lines = vec!["flowchart TB".to_string()];

    // Build node definitions
    for activity in &definition.activities {
        let status_icon = activities_status
            .and_then(|s| s.get(&activity.key))
            .map(|status| match status.as_str() {
                "completed" => "✅ ",
                "running" => "🔄 ",
                "failed" => "❌ ",
                "pending" => "⏳ ",
                "waiting" => "⏸️ ",
                _ => "",
            })
            .unwrap_or("");

        let label = format!(
            "{}{}<br/>{}",
            status_icon,
            activity.key,
            activity.activity_name
        );

        lines.push(format!("    {}[\"{}\"]", activity.key, label));
    }

    // Build edges from depends_on
    for activity in &definition.activities {
        for dep in &activity.depends_on {
            match dep {
                Dependency::Simple(key) => {
                    lines.push(format!("    {} --> {}", key, activity.key));
                }
                Dependency::Conditional { activity_key, conditions } => {
                    lines.push(format!(
                        "    {} -->|\"condition\"| {}",
                        activity_key, activity.key
                    ));
                }
            }
        }
    }

    // Add status-based styling
    if let Some(status_map) = activities_status {
        lines.push(String::new());
        for (key, status) in status_map {
            let color = match status.as_str() {
                "completed" => "#90EE90",
                "running" => "#FFD700",
                "failed" => "#FF6B6B",
                "pending" => "#D3D3D3",
                "waiting" => "#87CEEB",
                _ => continue,
            };
            lines.push(format!("    style {} fill:{}", key, color));
        }
    }

    lines.join("\n")
}
```

---

## 3. Development Tasks

### 3.1 Task Breakdown

| ID    | Task                                          | Est. | Priority | Dependencies |
|-------|-----------------------------------------------|------|----------|--------------|
| MCP-1 | Project Setup & Configuration                 | 2d   | P0       | -            |
| MCP-2 | Kruxia Flow API Client                        | 3d   | P0       | MCP-1        |
| MCP-3 | Discovery Tools (list/get definitions)        | 2d   | P0       | MCP-2        |
| MCP-4 | Execution Tools (validate/submit/cancel)      | 3d   | P0       | MCP-2        |
| MCP-5 | Observability Tools (status/output/cost)      | 3d   | P0       | MCP-2        |
| MCP-6 | Visualization Tools (Mermaid diagrams)        | 2d   | P1       | MCP-3        |
| MCP-7 | Control Tools (signals, human-in-loop)        | 2d   | P1       | MCP-4        |
| MCP-8 | Error Handling & Validation                   | 2d   | P0       | MCP-3,4,5    |
| MCP-9 | Authentication & Security                     | 2d   | P0       | MCP-2        |
| MCP-10| Unit Tests                                    | 3d   | P0       | MCP-3,4,5    |
| MCP-11| Integration Tests                             | 3d   | P0       | MCP-10       |
| MCP-12| Documentation                                 | 2d   | P1       | MCP-11       |
| MCP-13| CI/CD Pipeline                                | 1d   | P1       | MCP-11       |
| MCP-14| Claude Code Integration Testing               | 2d   | P1       | MCP-11       |
| MCP-15| IDE Extension Testing                         | 2d   | P2       | MCP-11       |
| MCP-16| Audit Logging Infrastructure                  | 3d   | P0       | MCP-2        |
| MCP-17| Prompt & Workflow Provenance                  | 2d   | P0       | MCP-16       |
| MCP-18| OpenTelemetry Integration                     | 2d   | P1       | MCP-2        |
| MCP-19| Prometheus Metrics Endpoint                   | 2d   | P1       | MCP-18       |
| MCP-20| Observability Dashboard                       | 3d   | P2       | MCP-19       |
| MCP-21| Post-Test Analysis Pipeline                   | 2d   | P1       | MCP-16       |

### 3.2 Detailed Task Specifications

#### MCP-1: Project Setup & Configuration

**Objective**: Initialize project structure and configuration management.

**Deliverables**:
- [ ] Project directory structure created
- [ ] `pyproject.toml` / `Cargo.toml` configured
- [ ] Configuration loading from environment variables
- [ ] Logging setup with configurable levels
- [ ] Development environment documentation

**Configuration Variables**:
```bash
# Required
KRUXIAFLOW_URL=http://localhost:8080
KRUXIAFLOW_TOKEN=<jwt_token>

# Optional
MCP_LOG_LEVEL=info
MCP_TRANSPORT=stdio  # or http
MCP_HTTP_PORT=8081
MCP_DEBUG=false
```

---

#### MCP-2: Kruxia Flow API Client

**Objective**: Build HTTP client wrapper for Kruxia Flow API.

**Deliverables**:
- [ ] Async HTTP client with connection pooling
- [ ] JWT token authentication
- [ ] Automatic token refresh
- [ ] Request/response logging
- [ ] Error handling and retries
- [ ] WebSocket client for streaming (future)

**API Endpoints to Support**:
| Endpoint                                        | Method | Used By             |
|-------------------------------------------------|--------|---------------------|
| `/api/v1/workflow_definitions`                  | GET    | list_definitions    |
| `/api/v1/workflow_definitions`                  | POST   | validate, submit    |
| `/api/v1/workflow_definitions/:name`            | GET    | get_definition      |
| `/api/v1/workflows`                             | GET    | list_workflows      |
| `/api/v1/workflows`                             | POST   | submit_workflow     |
| `/api/v1/workflows/:id`                         | GET    | get_status          |
| `/api/v1/workflows/:id/cost`                    | GET    | get_cost            |
| `/api/v1/workflows/:id/output`                  | GET    | get_output          |
| `/api/v1/workflows/:id/activities/:key/output`  | GET    | get_activity_output |
| `/api/v1/workflows/:id/signal`                  | POST   | send_signal         |

---

#### MCP-3: Discovery Tools

**Objective**: Implement tools for discovering workflow definitions and activities.

**Tools to Implement**:

```python
@mcp.tool()
async def list_workflow_definitions(
    namespace: str | None = None,
    limit: int = 20,
    offset: int = 0,
) -> dict:
    """List available workflow definitions."""

@mcp.tool()
async def get_workflow_definition(name: str) -> dict:
    """Get detailed workflow definition."""

@mcp.tool()
async def list_activities() -> dict:
    """List available activity types."""
```

---

#### MCP-4: Execution Tools

**Objective**: Implement tools for workflow validation and execution.

**Tools to Implement**:

```python
@mcp.tool()
async def validate_workflow(
    workflow_yaml: str,
) -> dict:
    """Validate workflow YAML without submitting."""

@mcp.tool()
async def submit_workflow(
    workflow_yaml: str,
    input: dict,
    budget_limit_usd: float | None = None,
) -> dict:
    """Submit workflow for execution."""

@mcp.tool()
async def cancel_workflow(
    workflow_id: str,
    reason: str | None = None,
) -> dict:
    """Cancel a running workflow."""
```

---

#### MCP-5: Observability Tools

**Objective**: Implement tools for monitoring workflow status and costs.

**Tools to Implement**:

```python
@mcp.tool()
async def get_workflow_status(
    workflow_id: str,
    include_activities: bool = False,
) -> dict:
    """Get current workflow status."""

@mcp.tool()
async def list_workflows(
    status: str | None = None,
    limit: int = 20,
) -> dict:
    """List workflows with filters."""

@mcp.tool()
async def get_activity_output(
    workflow_id: str,
    activity_key: str,
) -> dict:
    """Get output from specific activity."""

@mcp.tool()
async def get_workflow_cost(
    workflow_id: str,
) -> dict:
    """Get workflow cost breakdown."""

@mcp.tool()
async def estimate_workflow_cost(
    workflow_yaml: str,
    input: dict,
) -> dict:
    """Estimate cost before execution."""
```

---

#### MCP-6: Visualization Tools

**Objective**: Implement Mermaid diagram generation.

**Tools to Implement**:

```python
@mcp.tool()
async def render_workflow_diagram(
    workflow_id: str | None = None,
    workflow_definition: str | None = None,
    format: Literal["mermaid"] = "mermaid",
    include_status: bool = False,
) -> dict:
    """Generate visual workflow diagram."""
```

**Output Example**:
```json
{
  "format": "mermaid",
  "diagram": "flowchart TB\n    step1[...] --> step2[...]\n    ...",
  "rendered": "```mermaid\nflowchart TB\n...\n```"
}
```

---

#### MCP-7: Control Tools

**Objective**: Implement human-in-the-loop and control tools.

**Tools to Implement**:

```python
@mcp.tool()
async def send_workflow_signal(
    workflow_id: str,
    signal_name: str,
    data: dict | None = None,
) -> dict:
    """Send signal to waiting workflow."""

@mcp.tool()
async def list_waiting_workflows() -> dict:
    """List workflows waiting for signals."""
```

---

## 4. Testing Instructions

### 4.1 MCP Server Test Harness

The MCP Inspector is the official tool for testing MCP servers.

#### Installation

```bash
# Install MCP Inspector globally
npm install -g @anthropic/mcp-inspector

# Or run with npx
npx @anthropic/mcp-inspector
```

#### Basic Testing

```bash
# Start MCP server
cd kruxiaflow-mcp
python -m kruxiaflow_mcp --debug

# In another terminal, connect MCP Inspector
mcp-inspector stdio python -m kruxiaflow_mcp

# Or for HTTP transport
mcp-inspector http://localhost:8081
```

#### Inspector Commands

```
# List available tools
> tools

# Call a tool
> call list_workflow_definitions {"limit": 5}

# Call with complex arguments
> call submit_workflow {"workflow_yaml": "name: test\nactivities: []", "input": {}}

# View tool schema
> schema list_workflow_definitions

# Test error handling
> call get_workflow_status {"workflow_id": "invalid"}
```

#### Automated Test Script

```bash
#!/bin/bash
# tests/harness/run_mcp_tests.sh

set -e

echo "Starting Kruxia Flow..."
KRUXIAFLOW_DATABASE_URL="postgres://test:test@localhost:5432/kruxiaflow_test" \
  ./target/release/kruxiaflow &
KRUXIA_PID=$!
sleep 3

echo "Starting MCP Server..."
cd kruxiaflow-mcp
python -m kruxiaflow_mcp &
MCP_PID=$!
sleep 2

echo "Running MCP Inspector validation..."
npx @anthropic/mcp-inspector --validate stdio python -m kruxiaflow_mcp

echo "Running tool tests..."
npx @anthropic/mcp-inspector --test tests/harness/tool_tests.json stdio python -m kruxiaflow_mcp

# Cleanup
kill $MCP_PID $KRUXIA_PID
```

---

### 4.2 Claude Code Integration Testing

#### Setup

1. **Configure Claude Code MCP Settings**:

```json
// ~/.config/claude-code/mcp.json (Linux/Mac)
// %APPDATA%\claude-code\mcp.json (Windows)
{
  "servers": {
    "kruxiaflow": {
      "command": "python",
      "args": ["-m", "kruxiaflow_mcp"],
      "env": {
        "KRUXIAFLOW_URL": "http://localhost:8080",
        "KRUXIAFLOW_TOKEN": "<your_jwt_token>",
        "MCP_LOG_LEVEL": "debug"
      }
    }
  }
}
```

2. **Restart Claude Code**:
```bash
claude --restart
```

3. **Verify Connection**:
```
> /tools
# Should show kruxiaflow tools listed
```

#### Test Scenarios for Claude Code

**Scenario 1: Discovery**
```
You: "What workflow definitions are available in Kruxia Flow?"

Expected: Claude calls list_workflow_definitions and presents results
```

**Scenario 2: Workflow Creation**
```
You: "Create a workflow that fetches weather data and sends me an email summary"

Expected: Claude drafts YAML, calls validate_workflow, then submit_workflow
```

**Scenario 3: Monitoring**
```
You: "What's the status of workflow abc123?"

Expected: Claude calls get_workflow_status with include_activities=true
```

**Scenario 4: Cost Awareness**
```
You: "How much did workflow abc123 cost?"

Expected: Claude calls get_workflow_cost and presents breakdown
```

**Scenario 5: Visualization**
```
You: "Show me a diagram of the research workflow"

Expected: Claude calls render_workflow_diagram and displays Mermaid
```

#### Verbose Logging for Claude Code

Enable detailed logging to capture Claude's interactions:

```bash
# Set environment before starting Claude Code
export MCP_LOG_LEVEL=debug
export MCP_LOG_FILE=/tmp/kruxiaflow-mcp-claude.log

# Start Claude Code
claude

# Monitor logs in real-time
tail -f /tmp/kruxiaflow-mcp-claude.log
```

**Log Format**:
```
2026-01-30T10:30:00.123Z [DEBUG] MCP_TOOL_CALL tool=list_workflow_definitions args={"limit":20}
2026-01-30T10:30:00.234Z [DEBUG] API_REQUEST method=GET url=/api/v1/workflow_definitions
2026-01-30T10:30:00.345Z [DEBUG] API_RESPONSE status=200 duration_ms=111
2026-01-30T10:30:00.346Z [DEBUG] MCP_TOOL_RESULT success=true items=5
```

---

### 4.3 IDE Extension Testing

#### VS Code Extension

1. **Install MCP Extension**:
```bash
code --install-extension anthropic.mcp-tools
```

2. **Configure Workspace Settings**:
```json
// .vscode/settings.json
{
  "mcp.servers": {
    "kruxiaflow": {
      "command": "python",
      "args": ["-m", "kruxiaflow_mcp"],
      "env": {
        "KRUXIAFLOW_URL": "http://localhost:8080",
        "KRUXIAFLOW_TOKEN": "${env:KRUXIAFLOW_TOKEN}"
      }
    }
  },
  "mcp.logLevel": "debug",
  "mcp.showNotifications": true
}
```

3. **Test via Command Palette**:
- `Cmd/Ctrl+Shift+P` → "MCP: List Tools"
- `Cmd/Ctrl+Shift+P` → "MCP: Call Tool" → Select "list_workflow_definitions"

4. **View Logs**:
- `Cmd/Ctrl+Shift+P` → "MCP: Show Logs"
- Or open Output panel → Select "MCP Server"

#### JetBrains IDE (IntelliJ, PyCharm)

1. **Install MCP Plugin**:
- Settings → Plugins → Marketplace → Search "MCP Tools"

2. **Configure Server**:
- Settings → Tools → MCP Servers → Add
- Command: `python -m kruxiaflow_mcp`
- Environment: Set `KRUXIAFLOW_URL` and `KRUXIAFLOW_TOKEN`

3. **Test via Tools Window**:
- View → Tool Windows → MCP
- Expand "kruxiaflow" server
- Right-click tool → "Invoke"

---

### 4.4 Verbose Logging Configuration

#### Python MCP Server Logging

```python
# src/kruxiaflow_mcp/config.py

import logging
import os
import sys
from datetime import datetime

def setup_logging():
    """Configure comprehensive logging for debugging."""

    log_level = os.getenv("MCP_LOG_LEVEL", "info").upper()
    log_file = os.getenv("MCP_LOG_FILE")

    # Create formatters
    detailed_formatter = logging.Formatter(
        "%(asctime)s.%(msecs)03d [%(levelname)s] %(name)s: %(message)s",
        datefmt="%Y-%m-%dT%H:%M:%S"
    )

    # Root logger
    root_logger = logging.getLogger()
    root_logger.setLevel(getattr(logging, log_level))

    # Console handler (stderr for MCP compatibility)
    console_handler = logging.StreamHandler(sys.stderr)
    console_handler.setFormatter(detailed_formatter)
    root_logger.addHandler(console_handler)

    # File handler (optional)
    if log_file:
        file_handler = logging.FileHandler(log_file)
        file_handler.setFormatter(detailed_formatter)
        root_logger.addHandler(file_handler)

    # MCP-specific loggers
    logging.getLogger("mcp.protocol").setLevel(logging.DEBUG)
    logging.getLogger("mcp.tools").setLevel(logging.DEBUG)
    logging.getLogger("httpx").setLevel(logging.DEBUG if log_level == "DEBUG" else logging.WARNING)

# Custom log events
class MCPLogger:
    def __init__(self):
        self.logger = logging.getLogger("kruxiaflow_mcp")

    def tool_call(self, tool_name: str, args: dict):
        self.logger.debug(f"MCP_TOOL_CALL tool={tool_name} args={args}")

    def tool_result(self, tool_name: str, success: bool, duration_ms: float):
        self.logger.debug(f"MCP_TOOL_RESULT tool={tool_name} success={success} duration_ms={duration_ms:.1f}")

    def api_request(self, method: str, url: str):
        self.logger.debug(f"API_REQUEST method={method} url={url}")

    def api_response(self, status: int, duration_ms: float):
        self.logger.debug(f"API_RESPONSE status={status} duration_ms={duration_ms:.1f}")

    def validation_error(self, error: str, context: dict):
        self.logger.warning(f"VALIDATION_ERROR error={error} context={context}")

    def cost_tracking(self, workflow_id: str, cost_usd: float, budget_usd: float):
        self.logger.info(f"COST_TRACKING workflow={workflow_id} cost={cost_usd:.4f} budget={budget_usd:.2f}")
```

#### Log Analysis Script

```python
#!/usr/bin/env python3
"""Analyze MCP server logs for debugging and metrics."""

import re
import sys
from collections import defaultdict
from datetime import datetime

def analyze_logs(log_file: str):
    """Parse and analyze MCP server logs."""

    tool_calls = defaultdict(list)
    api_requests = defaultdict(list)
    errors = []

    with open(log_file) as f:
        for line in f:
            # Parse tool calls
            if "MCP_TOOL_CALL" in line:
                match = re.search(r"tool=(\w+) args=(\{.*\})", line)
                if match:
                    tool_calls[match.group(1)].append({
                        "timestamp": line[:23],
                        "args": match.group(2),
                    })

            # Parse API requests
            elif "API_REQUEST" in line:
                match = re.search(r"method=(\w+) url=(\S+)", line)
                if match:
                    api_requests[match.group(2)].append({
                        "timestamp": line[:23],
                        "method": match.group(1),
                    })

            # Collect errors
            elif "[ERROR]" in line or "[WARNING]" in line:
                errors.append(line.strip())

    # Print summary
    print("=" * 60)
    print("MCP SERVER LOG ANALYSIS")
    print("=" * 60)

    print("\n📊 Tool Call Summary:")
    for tool, calls in sorted(tool_calls.items(), key=lambda x: -len(x[1])):
        print(f"  {tool}: {len(calls)} calls")

    print("\n🌐 API Request Summary:")
    for endpoint, requests in sorted(api_requests.items(), key=lambda x: -len(x[1])):
        print(f"  {endpoint}: {len(requests)} requests")

    if errors:
        print(f"\n⚠️ Errors/Warnings: {len(errors)}")
        for error in errors[:10]:  # Show first 10
            print(f"  {error[:100]}...")

    print("\n" + "=" * 60)

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: analyze_logs.py <log_file>")
        sys.exit(1)
    analyze_logs(sys.argv[1])
```

#### Real-Time Log Monitoring

```bash
#!/bin/bash
# scripts/watch_mcp_logs.sh

LOG_FILE=${MCP_LOG_FILE:-/tmp/kruxiaflow-mcp.log}

echo "Watching MCP logs: $LOG_FILE"
echo "Filters: TOOL_CALL, API_REQUEST, ERROR, COST"
echo "Press Ctrl+C to stop"
echo ""

tail -f "$LOG_FILE" | grep --line-buffered -E "(TOOL_CALL|API_REQUEST|ERROR|COST|WARNING)"
```

---

#### MCP-16: Audit Logging Infrastructure

**Objective**: Implement comprehensive audit logging for all MCP tool invocations.

**Deliverables**:
- [ ] Database schema for `mcp_audit_log` table
- [ ] Audit writer service with async write
- [ ] Middleware to capture all tool invocations
- [ ] Agent identity extraction from JWT/session
- [ ] Configurable retention policies

**Database Schema**:
```sql
CREATE TABLE mcp_audit_log (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    timestamp TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Agent identification
    agent_id TEXT NOT NULL,
    agent_type TEXT NOT NULL,
    session_id UUID,

    -- Request context
    tool_name TEXT NOT NULL,
    tool_arguments JSONB NOT NULL,

    -- For workflow creation
    natural_language_prompt TEXT,
    generated_workflow_yaml TEXT,
    workflow_definition_name TEXT,
    workflow_id UUID,

    -- Validation
    validation_passed BOOLEAN,
    validation_warnings JSONB,

    -- Results
    result_status TEXT NOT NULL,
    error_message TEXT,
    duration_ms INTEGER,

    -- Cost
    estimated_cost_usd DECIMAL(10,6),
    actual_cost_usd DECIMAL(10,6),

    -- Tracing
    trace_id TEXT,
    span_id TEXT,

    -- Metadata
    metadata JSONB
);

CREATE INDEX idx_mcp_audit_agent ON mcp_audit_log(agent_id, timestamp DESC);
CREATE INDEX idx_mcp_audit_session ON mcp_audit_log(session_id, timestamp DESC);
CREATE INDEX idx_mcp_audit_workflow ON mcp_audit_log(workflow_id);
CREATE INDEX idx_mcp_audit_tool ON mcp_audit_log(tool_name, timestamp DESC);
CREATE INDEX idx_mcp_audit_trace ON mcp_audit_log(trace_id);
```

---

#### MCP-17: Prompt & Workflow Provenance

**Objective**: Capture and store the complete provenance of AI-generated workflows.

**Deliverables**:
- [ ] Natural language prompt capture from tool arguments
- [ ] Pre-validation YAML storage
- [ ] Post-validation YAML storage
- [ ] Modification tracking
- [ ] Human approval status tracking
- [ ] Execution outcome linking

**Implementation**:

```python
class ProvenanceTracker:
    """Track complete workflow provenance."""

    async def record_workflow_creation(
        self,
        agent_id: str,
        session_id: str,
        natural_language_prompt: str | None,
        generated_yaml: str,
        validation_result: ValidationResult,
        workflow_id: str | None = None,
    ) -> str:
        """Record workflow creation with full provenance."""

        return await self.db.execute("""
            INSERT INTO mcp_audit_log (
                agent_id, session_id, tool_name, tool_arguments,
                natural_language_prompt, generated_workflow_yaml,
                validation_passed, validation_warnings,
                workflow_id, result_status
            ) VALUES ($1, $2, 'submit_workflow', $3, $4, $5, $6, $7, $8, $9)
            RETURNING id
        """, ...)
```

---

#### MCP-18: OpenTelemetry Integration

**Objective**: Add distributed tracing support for debugging and observability.

**Deliverables**:
- [ ] OpenTelemetry SDK integration
- [ ] Span creation for each tool invocation
- [ ] Context propagation to Kruxia Flow API
- [ ] Trace ID inclusion in audit logs
- [ ] Configurable exporters (Jaeger, OTLP)

**Implementation**:

```python
from opentelemetry import trace
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.exporter.otlp.proto.grpc.trace_exporter import OTLPSpanExporter

tracer = trace.get_tracer("kruxiaflow-mcp")

@mcp.tool()
async def submit_workflow(workflow_yaml: str, input: dict) -> dict:
    with tracer.start_as_current_span("submit_workflow") as span:
        span.set_attribute("agent.id", get_current_agent_id())
        span.set_attribute("workflow.yaml_length", len(workflow_yaml))

        # Propagate trace context to API calls
        headers = inject_trace_context({})

        result = await api_client.post(
            "/api/v1/workflows",
            headers=headers,
            ...
        )

        span.set_attribute("workflow.id", result.get("workflow_id"))
        return result
```

---

#### MCP-19: Prometheus Metrics Endpoint

**Objective**: Expose Prometheus-compatible metrics for monitoring.

**Deliverables**:
- [ ] `/metrics` HTTP endpoint
- [ ] Tool invocation counters
- [ ] Latency histograms
- [ ] Active session gauges
- [ ] Cost tracking metrics
- [ ] Error rate metrics

**Metrics to Expose**:

```python
from prometheus_client import Counter, Histogram, Gauge

# Counters
tool_calls_total = Counter(
    'mcp_tool_calls_total',
    'Total MCP tool invocations',
    ['tool_name', 'status', 'agent_type']
)

workflows_created_total = Counter(
    'mcp_workflows_created_total',
    'Workflows created via MCP',
    ['agent_type', 'validation_status']
)

# Histograms
tool_duration_seconds = Histogram(
    'mcp_tool_duration_seconds',
    'Tool invocation latency',
    ['tool_name', 'agent_type'],
    buckets=[0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5]
)

# Gauges
active_sessions = Gauge(
    'mcp_active_sessions',
    'Currently active agent sessions',
    ['agent_type']
)

workflow_cost_usd = Gauge(
    'mcp_workflow_cost_usd',
    'Cost of workflows',
    ['workflow_definition']
)
```

---

#### MCP-20: Observability Dashboard

**Objective**: Create Grafana dashboards for MCP server monitoring.

**Deliverables**:
- [ ] Grafana dashboard JSON exports
- [ ] Overview dashboard (health, errors, latency)
- [ ] Agent activity dashboard
- [ ] Cost tracking dashboard
- [ ] Alerting rules configuration
- [ ] Documentation for dashboard setup

**Dashboard Panels**:

See Section 7.3 for complete dashboard specifications.

---

#### MCP-21: Post-Test Analysis Pipeline

**Objective**: Build automated analysis pipeline for test results.

**Deliverables**:
- [ ] Test data collection scripts
- [ ] Analysis queries (SQL)
- [ ] Regression detection algorithms
- [ ] Report generation templates
- [ ] AI-assisted log analysis integration
- [ ] CI/CD integration for automated analysis

**Implementation**:

```python
class TestAnalysisPipeline:
    """Automated test result analysis."""

    async def collect_test_data(self, run_id: str) -> TestRunData:
        """Collect all data from a test run."""
        return TestRunData(
            audit_logs=await self.fetch_audit_logs(run_id),
            metrics=await self.fetch_prometheus_snapshot(),
            generated_workflows=await self.fetch_workflow_artifacts(run_id),
        )

    async def analyze(self, data: TestRunData) -> TestAnalysis:
        """Run analysis on collected data."""
        return TestAnalysis(
            consistency=self.analyze_consistency(data),
            cost_accuracy=self.analyze_cost_accuracy(data),
            error_patterns=self.analyze_error_patterns(data),
            regressions=self.detect_regressions(data),
        )

    async def generate_report(self, analysis: TestAnalysis) -> str:
        """Generate markdown test report."""
        ...
```

---

## 5. Implementation Checklist

### Phase 1: Foundation (Week 1)

- [ ] **MCP-1**: Project setup complete
- [ ] **MCP-2**: API client working
- [ ] **MCP-9**: Authentication implemented
- [ ] Basic stdio transport working
- [ ] Can connect via MCP Inspector

### Phase 2: Core Tools (Week 2)

- [ ] **MCP-3**: Discovery tools complete
- [ ] **MCP-4**: Execution tools complete
- [ ] **MCP-5**: Observability tools complete
- [ ] **MCP-8**: Error handling complete
- [ ] All tools testable via MCP Inspector

### Phase 3: Advanced Features (Week 3)

- [ ] **MCP-6**: Visualization tools complete
- [ ] **MCP-7**: Control tools complete
- [ ] **MCP-10**: Unit tests passing
- [ ] **MCP-11**: Integration tests passing

### Phase 4: Audit & Observability (Week 4)

- [ ] **MCP-16**: Audit logging infrastructure complete
- [ ] **MCP-17**: Prompt & workflow provenance working
- [ ] **MCP-18**: OpenTelemetry integration complete
- [ ] **MCP-19**: Prometheus metrics endpoint working

### Phase 5: Polish & Release (Week 5)

- [ ] **MCP-12**: Documentation complete
- [ ] **MCP-13**: CI/CD pipeline working
- [ ] **MCP-14**: Claude Code testing complete
- [ ] **MCP-15**: IDE extension testing complete
- [ ] **MCP-20**: Observability dashboard deployed
- [ ] **MCP-21**: Post-test analysis pipeline working
- [ ] Release candidate ready

---

## 6. Success Criteria

### Functional Requirements

| Requirement                                      | Verification Method          |
|--------------------------------------------------|------------------------------|
| All 13 MCP tools implemented and documented      | MCP Inspector validation     |
| Tools return correct JSON schemas                | Schema validation tests      |
| Error messages are helpful and actionable        | Manual review                |
| Mermaid diagrams render correctly                | Visual inspection            |
| Cost tracking accurate to 4 decimal places       | Comparison with API          |

### Non-Functional Requirements

| Requirement                                      | Target         | Verification          |
|--------------------------------------------------|----------------|----------------------|
| Tool invocation latency                          | <100ms p95     | Load testing         |
| Memory usage (idle)                              | <50MB          | Process monitoring   |
| Startup time                                     | <2s            | Timing measurement   |
| Error rate                                       | <0.1%          | Integration tests    |
| Documentation coverage                           | 100%           | Doc review           |

### Integration Requirements

| Integration Point                                | Status Required            |
|--------------------------------------------------|----------------------------|
| MCP Inspector                                    | All tools pass validation  |
| Claude Code                                      | End-to-end scenarios work  |
| VS Code Extension                                | Tools accessible           |
| JetBrains Plugin                                 | Tools accessible           |

---

## 7. Observability Infrastructure

### 7.1 Architecture Overview

```mermaid
flowchart TB
    subgraph MCP["MCP Server"]
        App[Application Code]
        Audit[Audit Logger]
        Metrics[Metrics Collector]
        Tracer[OpenTelemetry Tracer]
    end

    subgraph Collection["Data Collection"]
        Prom[Prometheus]
        OTEL[OTEL Collector]
        PG[(PostgreSQL<br/>mcp_audit_log)]
    end

    subgraph Visualization["Dashboards & Alerts"]
        Grafana[Grafana]
        AM[Alertmanager]
    end

    subgraph Analysis["Analysis Tools"]
        Jaeger[Jaeger/Tempo]
        Analytics[Test Analytics]
        AIAnalysis[AI Log Analysis]
    end

    App --> Audit --> PG
    App --> Metrics --> Prom
    App --> Tracer --> OTEL

    Prom --> Grafana
    Prom --> AM
    OTEL --> Jaeger
    PG --> Analytics
    PG --> AIAnalysis
```

### 7.2 Metrics Specification

**Required Metrics**:

| Metric Name                        | Type      | Labels                               | Description                           |
|------------------------------------|-----------|--------------------------------------|---------------------------------------|
| `mcp_tool_calls_total`             | Counter   | tool_name, status, agent_type        | Total tool invocations                |
| `mcp_tool_duration_seconds`        | Histogram | tool_name, agent_type                | Tool invocation latency               |
| `mcp_api_requests_total`           | Counter   | endpoint, method, status             | Kruxia Flow API calls                 |
| `mcp_api_duration_seconds`         | Histogram | endpoint, method                     | API call latency                      |
| `mcp_workflows_created_total`      | Counter   | agent_type, validation_status        | Workflows created via MCP             |
| `mcp_workflows_cost_usd`           | Gauge     | workflow_definition                  | Cost of workflows                     |
| `mcp_active_sessions`              | Gauge     | agent_type                           | Currently active agent sessions       |
| `mcp_validation_errors_total`      | Counter   | error_type                           | Workflow validation failures          |
| `mcp_budget_exceeded_total`        | Counter   | workflow_definition                  | Budget limit exceeded events          |
| `mcp_audit_log_writes_total`       | Counter   | status                               | Audit log write operations            |
| `mcp_prompt_logged_total`          | Counter   | has_prompt                           | Prompts captured for provenance       |

**Histogram Buckets**:
```python
# Tool invocation latency buckets (seconds)
TOOL_LATENCY_BUCKETS = [0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]

# API call latency buckets (seconds)
API_LATENCY_BUCKETS = [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0]
```

### 7.3 Dashboard Specifications

#### Dashboard 1: MCP Server Overview

**Grafana Dashboard JSON ID**: `mcp-overview`

| Panel                      | Type       | Query/Data Source                              |
|----------------------------|------------|------------------------------------------------|
| Active Agents              | Stat       | `sum(mcp_active_sessions)`                     |
| Tool Calls (24h)           | Stat       | `sum(increase(mcp_tool_calls_total[24h]))`     |
| Error Rate (5m)            | Gauge      | `sum(rate(mcp_tool_calls_total{status="error"}[5m])) / sum(rate(mcp_tool_calls_total[5m]))` |
| Workflows Created (24h)    | Stat       | `sum(increase(mcp_workflows_created_total[24h]))` |
| Total Cost (24h)           | Stat       | `sum(mcp_workflows_cost_usd)`                  |
| Tool Latency p95           | Time series| `histogram_quantile(0.95, rate(mcp_tool_duration_seconds_bucket[5m]))` |
| Tool Calls by Type         | Pie chart  | `sum by (tool_name) (increase(mcp_tool_calls_total[24h]))` |
| Error Timeline             | Time series| `sum(rate(mcp_tool_calls_total{status="error"}[5m]))` |

#### Dashboard 2: Agent Activity

**Grafana Dashboard JSON ID**: `mcp-agents`

| Panel                      | Type       | Query/Data Source                              |
|----------------------------|------------|------------------------------------------------|
| Tool Usage by Agent Type   | Pie chart  | `sum by (agent_type) (mcp_tool_calls_total)`   |
| Top Agents by Activity     | Table      | SQL: `SELECT agent_id, COUNT(*) FROM mcp_audit_log GROUP BY agent_id ORDER BY 2 DESC LIMIT 10` |
| Session Duration Histogram | Histogram  | Custom session tracking                        |
| Workflows per Agent Type   | Bar chart  | `sum by (agent_type) (mcp_workflows_created_total)` |
| Agent Error Rates          | Time series| `sum by (agent_type) (rate(mcp_tool_calls_total{status="error"}[5m]))` |

#### Dashboard 3: Cost & Budget

**Grafana Dashboard JSON ID**: `mcp-costs`

| Panel                      | Type       | Query/Data Source                              |
|----------------------------|------------|------------------------------------------------|
| Cost Trend (7d)            | Time series| SQL: `SELECT DATE_TRUNC('hour', timestamp) as time, SUM(actual_cost_usd) FROM mcp_audit_log GROUP BY 1` |
| Cost by Workflow Type      | Pie chart  | SQL: `SELECT workflow_definition_name, SUM(actual_cost_usd) FROM mcp_audit_log GROUP BY 1` |
| Budget Utilization         | Gauge      | Actual vs budget limits                        |
| Budget Exceeded Events     | Table      | `sum by (workflow_definition) (mcp_budget_exceeded_total)` |
| Cost Estimation Accuracy   | Time series| SQL comparing estimated vs actual              |

#### Dashboard 4: Audit & Compliance

**Grafana Dashboard JSON ID**: `mcp-audit`

| Panel                      | Type       | Query/Data Source                              |
|----------------------------|------------|------------------------------------------------|
| Audit Log Volume           | Time series| `sum(rate(mcp_audit_log_writes_total[5m]))`    |
| Prompts Captured Rate      | Gauge      | `sum(mcp_prompt_logged_total{has_prompt="true"}) / sum(mcp_prompt_logged_total)` |
| Recent Tool Calls          | Table      | SQL: Last 100 audit log entries                |
| Validation Failures        | Time series| `sum(rate(mcp_validation_errors_total[5m]))`   |
| Agent Session Timeline     | Gantt      | Session start/end times                        |

### 7.4 Alerting Rules

**Prometheus Alerting Rules** (`mcp-alerts.yml`):

```yaml
groups:
  - name: mcp-server
    rules:
      - alert: MCPHighErrorRate
        expr: |
          sum(rate(mcp_tool_calls_total{status="error"}[5m])) /
          sum(rate(mcp_tool_calls_total[5m])) > 0.05
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "MCP Server error rate above 5%"
          description: "Error rate is {{ $value | humanizePercentage }}"

      - alert: MCPToolLatencyHigh
        expr: |
          histogram_quantile(0.95, rate(mcp_tool_duration_seconds_bucket[10m])) > 0.5
        for: 10m
        labels:
          severity: warning
        annotations:
          summary: "MCP tool p95 latency above 500ms"
          description: "p95 latency is {{ $value | humanizeDuration }}"

      - alert: MCPBudgetExceeded
        expr: increase(mcp_budget_exceeded_total[1m]) > 0
        for: 0m
        labels:
          severity: info
        annotations:
          summary: "Workflow budget exceeded"
          description: "Budget exceeded for {{ $labels.workflow_definition }}"

      - alert: MCPAgentAbuseDetected
        expr: |
          sum by (agent_id) (rate(mcp_tool_calls_total[1m])) > 16.67
        for: 1m
        labels:
          severity: critical
        annotations:
          summary: "Agent making >1000 calls/minute"
          description: "Agent {{ $labels.agent_id }} rate: {{ $value | humanize }}/sec"

      - alert: MCPValidationErrorSpike
        expr: |
          rate(mcp_validation_errors_total[5m]) >
          10 * avg_over_time(rate(mcp_validation_errors_total[1h])[24h:1h])
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "Validation error rate 10x above baseline"

      - alert: MCPAuditLogFailure
        expr: |
          rate(mcp_audit_log_writes_total{status="error"}[5m]) > 0
        for: 1m
        labels:
          severity: critical
        annotations:
          summary: "Audit log writes failing"
          description: "Audit logging is critical for compliance"

      - alert: MCPNoPromptsCaptured
        expr: |
          sum(rate(mcp_prompt_logged_total{has_prompt="true"}[5m])) /
          sum(rate(mcp_prompt_logged_total[5m])) < 0.9
        for: 10m
        labels:
          severity: warning
        annotations:
          summary: "Less than 90% of workflow submissions have prompts logged"
```

### 7.5 Tracing Configuration

**OpenTelemetry Setup**:

```python
# src/kruxiaflow_mcp/observability/tracing.py

from opentelemetry import trace
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import BatchSpanProcessor
from opentelemetry.exporter.otlp.proto.grpc.trace_exporter import OTLPSpanExporter
from opentelemetry.sdk.resources import Resource, SERVICE_NAME

def setup_tracing(service_name: str = "kruxiaflow-mcp"):
    """Configure OpenTelemetry tracing."""

    resource = Resource(attributes={
        SERVICE_NAME: service_name,
    })

    provider = TracerProvider(resource=resource)

    # Export to OTLP collector (Jaeger, Tempo, etc.)
    otlp_exporter = OTLPSpanExporter(
        endpoint=os.getenv("OTEL_EXPORTER_OTLP_ENDPOINT", "localhost:4317"),
        insecure=True,
    )
    provider.add_span_processor(BatchSpanProcessor(otlp_exporter))

    trace.set_tracer_provider(provider)

    return trace.get_tracer(service_name)


# Trace context propagation for API calls
from opentelemetry.propagate import inject

def inject_trace_context(headers: dict) -> dict:
    """Inject trace context into HTTP headers."""
    inject(headers)
    return headers
```

**Span Conventions**:

| Span Name              | Attributes                                              |
|------------------------|---------------------------------------------------------|
| `mcp.tool.{name}`      | agent.id, agent.type, tool.arguments_size               |
| `mcp.api.request`      | http.method, http.url, http.status_code                 |
| `mcp.validation`       | workflow.name, validation.passed, validation.warnings   |
| `mcp.audit.write`      | audit.entry_id, audit.tool_name                         |

### 7.6 Log Aggregation

**Structured Log Format**:

```json
{
  "timestamp": "2026-01-30T10:30:00.123Z",
  "level": "INFO",
  "logger": "kruxiaflow_mcp.tools.execution",
  "message": "Workflow submitted successfully",
  "trace_id": "abc123def456",
  "span_id": "789xyz",
  "agent_id": "claude-code-session-123",
  "tool_name": "submit_workflow",
  "workflow_id": "019353a1-b0c1-7000-8000-000000000001",
  "duration_ms": 234,
  "cost_usd": 0.0,
  "extra": {
    "workflow_name": "research_assistant",
    "activity_count": 4
  }
}
```

**Log Shipping Configuration** (Vector/Fluentd):

```yaml
# vector.toml
[sources.mcp_logs]
type = "file"
include = ["/var/log/kruxiaflow-mcp/*.log"]

[transforms.parse_json]
type = "remap"
inputs = ["mcp_logs"]
source = '''
. = parse_json!(.message)
'''

[sinks.elasticsearch]
type = "elasticsearch"
inputs = ["parse_json"]
endpoint = "http://elasticsearch:9200"
index = "mcp-logs-%Y.%m.%d"
```

---

## Appendix A: MCP Protocol Reference

### JSON-RPC 2.0 Message Format

**Request**:
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "list_workflow_definitions",
    "arguments": {"limit": 10}
  }
}
```

**Response**:
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"definitions\": [...], \"total_count\": 5}"
      }
    ]
  }
}
```

### Transport Options

| Transport | Use Case                        | Configuration                |
|-----------|--------------------------------|------------------------------|
| stdio     | Claude Code, CLI tools          | Default, no config needed    |
| HTTP+SSE  | Web integrations, IDE plugins   | `--transport http --port 8081`|

---

## Appendix B: Related Documentation

- [PRD: Kruxia Flow MCP Server](./mcp-server-prd.md)
- [Test Plan: Kruxia Flow MCP Server](./mcp-server-test-plan.md)
- [Kruxia Flow API Reference](../api-reference.md)
- [Kruxia Flow Architecture](../architecture.md)
- [MCP Specification](https://modelcontextprotocol.io/specification)
- [FastMCP Documentation](https://github.com/jlowin/fastmcp)
