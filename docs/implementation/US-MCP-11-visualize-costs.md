# US-MCP-11: Visualize Cost Breakdowns

**Epic:** MCP Server for AI Agent Integration
**Category:** Visualization
**Status:** ✅ Implemented
**Implementation Date:** 2026-01-30

---

## User Story

**As an** AI agent
**I want to** generate visual cost breakdown diagrams
**So that** users can quickly see which activities are most expensive

---

## Acceptance Criteria

### AC1: Generate Cost Diagram
- ✅ Creates Mermaid graph showing cost distribution
- ✅ Shows total cost at root
- ✅ Branches to per-activity costs
- ✅ Includes provider information

### AC2: Filtering
- ✅ Only shows activities with non-zero costs
- ✅ Formats costs to 4 decimal places

---

## Implementation

**Tool:** `render_cost_diagram`
**Location:** `kruxiaflow-mcp/src/kruxiaflow_mcp/tools/visualization.py:111-154`
**Utility:** `generate_cost_breakdown_diagram`
**Location:** `kruxiaflow-mcp/src/kruxiaflow_mcp/utils/mermaid.py:155-184`
