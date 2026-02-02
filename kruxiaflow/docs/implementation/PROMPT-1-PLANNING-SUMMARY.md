# Prompt 1: Planning & Foundation - Summary

## Objective

Plan the implementation of a Rust-based MCP server integrated into the Kruxia Flow binary using the `rust-mcp-sdk` from https://github.com/rust-mcp-stack/rust-mcp-sdk.

## What We Accomplished

### 1. ✅ Researched rust-mcp-sdk

**Key Findings:**
- Uses `ServerHandler` trait with `#[async_trait]` for async methods
- Tools defined with `#[mcp_tool]` macro and registered in `list_tools`
- Server creation: `server_runtime::create_server()` for stdio, `hyper_server::create_server()` for HTTP
- Supports MCP protocol version 2025-11-25
- Requires derives: `Debug`, `Deserialize`, `Serialize`, `JsonSchema`

**Resources Consulted:**
- [rust-mcp-sdk GitHub](https://github.com/rust-mcp-stack/rust-mcp-sdk)
- [Building MCP Servers in Rust Guide](https://mcpcat.io/guides/building-mcp-server-rust/)
- [Shuttle.dev MCP Tutorial](https://www.shuttle.dev/blog/2025/07/18/how-to-build-a-stdio-mcp-server-in-rust)
- [MCP Protocol Documentation](https://modelcontextprotocol.io/)

### 2. ✅ Created User Story

**File:** `docs/implementation/US-MCP-15-rust-mcp-server.md`

**Summary:**
- **As a** DevOps engineer deploying Kruxia Flow
- **I want** MCP server functionality built into the main binary
- **So that** I can deploy a single executable instead of managing separate services

**Scope:**
- 13 MCP tools across 5 categories
- Stdio and HTTP transport support
- Granular configuration (feature-level, tool-level)
- Security (auth, rate limiting, resource limits)
- Observability (metrics, audit logging)

**Implementation Phases:** 7 prompts (current is Prompt 1)

### 3. ✅ Created Implementation Plan

**File:** `docs/implementation/rust-mcp-implementation-plan.md`

**Architecture:**
```
kruxiaflow/src/mcp/
├── mod.rs                    # Module exports
├── config.rs                 # Configuration (McpConfig)
├── server.rs                 # Server initialization
├── handler.rs                # ServerHandler implementation
├── middleware.rs             # Rate limiting, auth, metrics
├── transport/
│   ├── stdio.rs             # Stdio transport
│   └── http.rs              # HTTP transport
└── tools/
    ├── discovery.rs         # 4 discovery tools
    ├── execution.rs         # 3 execution tools
    ├── observability.rs     # 5 observability tools
    ├── visualization.rs     # 2 visualization tools
    └── control.rs           # 2 control tools
```

**Key Design Decisions:**

1. **Compile-Time Feature Flag** (per your request):
   ```toml
   [features]
   default = ["mcp-server"]
   mcp-server = ["dep:rust-mcp-sdk"]
   ```
   - Build without MCP: `cargo build --release --no-default-features`
   - Reduces binary size by ~2-5MB for edge deployments
   - Completely excludes MCP dependencies when disabled

2. **Configuration Strategy:**
   - CLI flags > Environment variables > Defaults
   - Follows existing Kruxia Flow patterns
   - Feature-level control (discovery, execution, observability, visualization, control)
   - Tool-level granularity (enable/disable individual tools)
   - Rate limiting (global + per-tool)
   - Authentication (optional for stdio, required for HTTP)

3. **Transport Support:**
   - **Stdio**: Default, for Claude Desktop (single client, local)
   - **HTTP**: Network-accessible (multi-client, requires auth)

4. **Security:**
   - Disabled by default (`KRUXIAFLOW_MCP_ENABLED=false`)
   - Auth required for HTTP transport
   - Rate limiting to prevent abuse
   - Resource limits (concurrent requests, timeouts, response size)
   - Audit logging for sensitive operations

### 4. ✅ Created User Guide Addition

**File:** `docs/implementation/mcp-server-user-guide-addition.md`

**Content:**
- Complete configuration reference
- Deployment scenario examples
- How to add custom MCP tools
- Troubleshooting guide
- Monitoring with Prometheus metrics
- Audit logging examples

### 5. ✅ Defined Implementation Phases

**Phase 1 (Prompt 2): Core Infrastructure**
- Create `mcp` module structure
- Implement `McpConfig`
- Basic MCP server with stdio transport
- Integration with `serve` command

**Phase 2 (Prompt 3): Discovery Tools**
- 4 read-only discovery tools
- Database/API integration
- Foundation for other tools

**Phase 3 (Prompt 4): Execution Tools**
- Validation, submit, cancel workflows
- Feature-level controls

**Phase 4 (Prompt 5): Observability Tools**
- Status, list, output, cost tools
- Cost estimation in Rust

**Phase 5 (Prompt 6): Visualization & Control**
- Mermaid diagram generation
- Signal/waiting workflow tools

**Phase 6 (Prompt 7): Production Hardening**
- Rate limiting middleware
- HTTP transport
- Authentication integration
- Metrics and audit logging
- Comprehensive testing

## Configuration Examples (Simplified)

### Build with MCP Support
```bash
cargo build --release --features mcp-server
```

### Development (Stdio for Claude Desktop)
```bash
export DATABASE_URL="postgres://localhost/kruxiaflow"
export KRUXIAFLOW_MCP_ENABLED=true
# That's it! Uses stdio transport by default
kruxiaflow serve --port 8080 --workers 4
```

### Production (HTTP with Auth)
```bash
export KRUXIAFLOW_MCP_ENABLED=true
export KRUXIAFLOW_MCP_TRANSPORT=http
export KRUXIAFLOW_MCP_HTTP_PORT=8081
export KRUXIAFLOW_MCP_JWT_SECRET="<your-secret>"
# Auth required by default for HTTP
kruxiaflow serve
```

### Edge Deployment (No MCP)
```bash
# Build without MCP (default)
cargo build --release

# Binary is ~2-5MB smaller, MCP dependencies excluded
./target/release/kruxiaflow serve
```

## Key Technical Specifications

### Dependencies
```toml
rust-mcp-sdk = { version = "0.8", optional = true }
# Conditional on feature = "mcp-server"
```

### Configuration Hierarchy
```
Global MCP Config
├── Transport (stdio/http)
├── Authentication
├── Rate Limiting (global)
└── Features
    ├── discovery (4 tools)
    ├── execution (3 tools)
    ├── observability (5 tools)
    ├── visualization (2 tools)
    └── control (2 tools)
        └── Per-tool rate limits
```

### 13 Tools to Implement

**Discovery (Read-Only):**
1. list_workflow_definitions
2. get_workflow_definition
3. list_activities
4. get_workflow_authoring_guide

**Execution (Write Operations):**
5. validate_workflow
6. submit_workflow
7. cancel_workflow

**Observability (Monitoring):**
8. get_workflow_status
9. list_workflows
10. get_activity_output
11. get_workflow_cost
12. estimate_workflow_cost

**Visualization (Diagrams):**
13. render_workflow_diagram
14. render_cost_diagram

**Control (Signals):**
15. send_workflow_signal
16. list_waiting_workflows

## ✅ Review Checklist (APPROVED)

- [x] **User Story**: Updated with opt-in, simple config, pinned SDK
- [x] **Module Structure**: Proposed architecture approved
- [x] **Configuration Strategy**: Simplified to 8 env vars
- [x] **Compile-Time Feature**: Opt-in approach (not default)
- [x] **Security Defaults**: Disabled by default, auth required for HTTP
- [x] **Implementation Phases**: 7-prompt breakdown approved
- [x] **SDK Version**: Pinned to 0.8.2

## ✅ Decisions Made

1. **Feature Flag: OPT-IN**
   - MCP **excluded by default** for lean edge binaries
   - Build with: `cargo build --release --features mcp-server`
   - Default binary ~2-5MB smaller

2. **Configuration: SIMPLIFIED**
   - Only 8 environment variables total
   - No feature-level controls (all 13 tools when enabled)
   - No per-tool rate limits (keep it simple)
   - No audit logging (use existing Kruxia Flow logging)

3. **SDK Version: PINNED**
   - `rust-mcp-sdk = "0.8.2"` for stability

4. **Python MCP Server: UNCHANGED**
   - Keep `kruxiaflow-mcp/` as reference
   - No modifications during this work

## Next Steps

**Upon Approval:**
1. Move to **Prompt 2: Core Infrastructure**
2. Implement `kruxiaflow/src/mcp/config.rs`
3. Implement basic MCP server with stdio transport
4. Integrate with `serve` command
5. Add compile-time feature flag to Cargo.toml

**Estimated Deliverables for Prompt 2:**
- Working MCP server (stdio only)
- Configuration parsing and validation
- Integration with serve command
- Basic "hello world" tool to verify communication

---

## ✅ APPROVED - Ready for Prompt 2

**Decisions Made:**
1. ✅ Opt-in compile-time feature (not default)
2. ✅ Simplified configuration (8 env vars)
3. ✅ Pinned SDK version (0.8.2)
4. ✅ Keep Python MCP server unchanged

**Next: Prompt 2 - Core Infrastructure Implementation**
