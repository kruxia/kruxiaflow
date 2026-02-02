# US-MCP-15: Rust MCP Server Integration

## User Story

**As a** DevOps engineer deploying Kruxia Flow
**I want** MCP server functionality built into the main Kruxia Flow binary (as an opt-in feature)
**So that** I can deploy a single executable with integrated AI agent capabilities when needed, while keeping the default binary lean for edge deployments

## Key Decisions

- **Opt-in at compile time**: MCP excluded by default (`cargo build --release --features mcp-server`)
- **Simple configuration**: Essential options only, avoiding configuration complexity
- **Pinned SDK version**: rust-mcp-sdk = "0.8.2" for stability
- **Python MCP unchanged**: Keep `kruxiaflow-mcp/` as reference, no modifications
- **HTTP-only transport**: Stdio transport NOT supported in integrated MCP server (see rationale below)

### Why No Stdio Transport?

The integrated Rust MCP server **does NOT support stdio transport** for the following reasons:

1. **Multi-service architecture**: The `serve` command runs multiple services simultaneously:
   - API server (logs to stdout/stderr)
   - Orchestrator (logs to stdout/stderr)
   - Workers (log to stdout/stderr)
   - MCP server

2. **Protocol conflict**: MCP stdio transport requires **clean stdin/stdout** with no other output. Any logging corrupts the JSON-RPC message stream.

3. **Solution for stdio users**: Use a **separate process** for stdio MCP:
   - **Python MCP server**: Use the existing `kruxiaflow-mcp/` (already available)
   - **Standalone Rust MCP server**: Create a dedicated binary that runs only MCP with no stdout logging

The integrated HTTP-only MCP server is designed for production deployments where observability (logging/metrics) and multi-client access are priorities.

## Acceptance Criteria

1. **Single Binary Deployment**
   - [ ] MCP server runs within the `kruxiaflow serve` command
   - [ ] No separate Python process required for MCP functionality
   - [ ] All 13 MCP tools implemented in Rust

2. **Configuration Control**
   - [ ] `--mcp-enabled` flag to enable/disable MCP server
   - [ ] Environment variable support for all MCP settings
   - [ ] Feature-level controls (discovery, execution, observability, visualization, control)
   - [ ] Tool-level enable/disable granularity
   - [ ] Rate limiting configuration (global and per-tool)

3. **Transport Support**
   - [x] HTTP transport for network-accessible MCP endpoints (only supported transport)
   - [ ] Transport selection via `KRUXIAFLOW_MCP_TRANSPORT` environment variable (only 'http' accepted)
   - Note: Stdio transport NOT supported (use separate process - see Key Decisions)

4. **Security**
   - [ ] Optional authentication for HTTP transport
   - [ ] Rate limiting to prevent abuse
   - [ ] Resource limits (max concurrent requests, timeouts, response size)
   - [ ] Audit logging for sensitive operations

5. **Observability**
   - [ ] Prometheus metrics for MCP tool usage
   - [ ] Structured logging for all MCP operations
   - [ ] Audit log for write operations (submit, cancel, signal)

6. **Documentation**
   - [ ] User guide updated with MCP server setup instructions
   - [ ] Configuration reference documentation
   - [ ] Examples for common deployment scenarios

## Technical Requirements

### Rust Dependencies
- `rust-mcp-sdk` from https://github.com/rust-mcp-stack/rust-mcp-sdk
- Support for MCP protocol version 2025-11-25
- HTTP transport layer only (stdio not supported - see Key Decisions)

### Tools to Implement (13 total)

**Discovery Tools (4):**
1. `list_workflow_definitions` - List available workflow definitions
2. `get_workflow_definition` - Get detailed workflow structure
3. `list_activities` - List available activity types
4. `get_workflow_authoring_guide` - Comprehensive workflow authoring documentation

**Execution Tools (3):**
5. `validate_workflow` - Validate workflow YAML before submission
6. `submit_workflow` - Submit workflow for execution
7. `cancel_workflow` - Cancel running workflow

**Observability Tools (5):**
8. `get_workflow_status` - Get workflow execution status
9. `list_workflows` - List workflows with filtering
10. `get_activity_output` - Get activity results
11. `get_workflow_cost` - Get cost breakdown
12. `estimate_workflow_cost` - Pre-execution cost estimation

**Visualization Tools (2):**
13. `render_workflow_diagram` - Generate Mermaid flowchart
14. `render_cost_diagram` - Generate cost breakdown diagram

**Control Tools (2):**
15. `send_workflow_signal` - Send signal to waiting workflow
16. `list_waiting_workflows` - Find workflows waiting for signals

### Integration Points

- Reuse existing Kruxia Flow API/database access patterns
- Share authentication infrastructure with API server
- Integrate with existing logging and metrics systems
- Use the same PostgreSQL pool as other services

## Implementation Phases

### Phase 1: Core Infrastructure (Prompt 2)
- Create `kruxiaflow/src/mcp/` module
- Implement configuration (`McpConfig`) - HTTP transport only
- Basic MCP server with HTTP transport
- Integration with `serve` command

### Phase 2: Discovery Tools (Prompt 3)
- Implement 4 discovery tools
- Database/API integration for workflow definitions
- Activity catalog generation

### Phase 3: Execution Tools (Prompt 4)
- Implement validation, submit, cancel
- Feature-level controls
- Integration with workflow submission API

### Phase 4: Observability Tools (Prompt 5)
- Implement status, list, output, cost tools
- Cost estimation logic in Rust
- Tool-level disable controls

### Phase 5: Visualization & Control (Prompt 6)
- Mermaid diagram generation
- Signal/waiting workflow tools

### Phase 6: Production Hardening (Prompt 7)
- Rate limiting middleware
- HTTP transport
- Authentication integration
- Metrics and audit logging
- Comprehensive testing

## Non-Goals (Out of Scope)

- Replacing or modifying the existing Python MCP server (`kruxiaflow-mcp/`)
- Implementing MCP client functionality (server only)
- Custom activity execution via MCP (use standard workflow submission)
- Real-time streaming of workflow events (use existing SSE if needed)

## Success Metrics

- Single binary can serve both API and MCP clients
- MCP server performance meets or exceeds Python implementation
- Configuration provides sufficient control for production deployments
- Zero breaking changes to existing Kruxia Flow functionality
- Comprehensive documentation enables easy adoption

## Dependencies

- US-MCP-1 through US-MCP-13 (MCP feature specifications) - for reference
- Existing Kruxia Flow API and database infrastructure
- rust-mcp-sdk library

## References

- [rust-mcp-sdk GitHub](https://github.com/rust-mcp-stack/rust-mcp-sdk)
- [MCP Protocol Specification](https://modelcontextprotocol.io/)
- [Building MCP Servers in Rust Guide](https://mcpcat.io/guides/building-mcp-server-rust/)
- [Shuttle.dev MCP Tutorial](https://www.shuttle.dev/blog/2025/07/18/how-to-build-a-stdio-mcp-server-in-rust)

## Notes

- The Python MCP server (`kruxiaflow-mcp/`) will remain as a reference implementation
- Rust implementation may reveal opportunities for optimization or API improvements
- Configuration strategy follows existing Kruxia Flow patterns (env vars + CLI flags)
- Phased implementation approach allows for incremental delivery and testing

---

## Appendix: Stdio Transport Removal - Technical Details

### Decision Rationale

The `serve` command runs multiple services simultaneously:
1. **API Server** - logs to stdout/stderr for observability
2. **Orchestrator** - logs to stdout/stderr for debugging
3. **Workers** - log to stdout/stderr for activity tracking
4. **MCP Server** - would need stdin/stdout for protocol

**The Problem:**
- MCP stdio protocol requires **clean stdin/stdout** with no other output
- Any logging output (from API/orchestrator/workers) **corrupts the JSON-RPC message stream**
- This causes MCP protocol errors and connection failures
- No way to segregate stdout in a single process running multiple services

**Attempted Solutions Rejected:**
1. **Redirect logs to file**: Breaks observability for operators
2. **Conditional logging**: Complex, error-prone, defeats purpose of integrated server
3. **Separate threads with stream isolation**: Not possible - all threads share process stdout/stderr

### Implementation Changes

**Code Changes:**

1. **kruxiaflow/src/mcp/config.rs**
   - Removed `McpTransport::Stdio` enum variant
   - Only `McpTransport::Http` remains
   - Added validation that rejects stdio with clear error message
   - Updated tests to verify stdio rejection
   - Default transport changed from `stdio` to `http`
   - Auth required by default changed to `true` (HTTP security)

2. **kruxiaflow/src/mcp/server.rs**
   - Removed `start_stdio()` function
   - Added documentation explaining why stdio is not supported
   - `start()` now only calls `start_http()`

3. **kruxiaflow/src/commands/serve.rs**
   - Updated help text for `--mcp-transport` flag
   - Documents that only 'http' is supported
   - Points users to separate process for stdio

### Testing

**New Test:**
- `test_config_stdio_transport_rejected` - Verifies stdio is properly rejected with helpful error message

**Test Results:**
All 5 MCP configuration tests passing:
```
test mcp::config::tests::test_config_disabled_by_default ... ok
test mcp::config::tests::test_config_stdio_transport_rejected ... ok
test mcp::config::tests::test_config_http_defaults ... ok
test mcp::config::tests::test_config_http_auth_required_by_default ... ok
test mcp::config::tests::test_cli_overrides_env ... ok
```

### User Experience

**Error Message When Attempting Stdio:**
```bash
$ KRUXIAFLOW_MCP_TRANSPORT=stdio cargo run --features mcp-server -- serve

Error: MCP stdio transport is not supported in the integrated MCP server.
Reason: The 'serve' command runs multiple services with logging to stdout/stderr,
which corrupts the MCP stdio protocol (requires clean stdin/stdout).

For stdio MCP support, use a separate process:
- Python MCP server: kruxiaflow-mcp/
- Standalone Rust MCP server: Create a dedicated binary
```

**Configuration Example (HTTP):**
```bash
# Development
export KRUXIAFLOW_MCP_ENABLED=true
export KRUXIAFLOW_MCP_HTTP_PORT=8081
export KRUXIAFLOW_MCP_AUTH_REQUIRED=false  # Dev only!
cargo run --features mcp-server -- serve

# Production
export KRUXIAFLOW_MCP_ENABLED=true
export KRUXIAFLOW_MCP_HTTP_PORT=8081
export KRUXIAFLOW_MCP_AUTH_REQUIRED=true
export KRUXIAFLOW_MCP_JWT_SECRET="<secure-secret>"
cargo run --features mcp-server -- serve
```

### Impact Analysis

**Positive:**
- ✅ Simplified configuration (no stdio/http branching)
- ✅ Clearer separation of concerns
- ✅ Production-first design (HTTP with auth)
- ✅ No stdout corruption issues
- ✅ Full observability maintained

**Neutral:**
- Users needing stdio (e.g., Claude Desktop) use Python MCP server
- Python MCP server already exists and works well
- Clear migration path for future standalone Rust MCP server

**No Negative Impact:**
- Original plan assumed stdio would work - discovered conflict during implementation
- Better to fix architecture now than ship broken stdio support

### Files Modified

1. `kruxiaflow/src/mcp/config.rs` - Remove stdio, update validation
2. `kruxiaflow/src/mcp/server.rs` - Remove start_stdio, add docs
3. `kruxiaflow/src/mcp/tools/mod.rs` - Fix doc comment
4. `kruxiaflow/src/commands/serve.rs` - Update help, add test fields
5. `docs/implementation/US-MCP-15-rust-mcp-server.md` - Document decision
6. `docs/implementation/mcp-server-user-guide-addition.md` - Update all examples
7. `docs/implementation/PROMPT-2-COMPLETE.md` - Update completion status
