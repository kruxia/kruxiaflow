# US-MCP-14: Workflow Authoring Guide for Agents

**Epic:** MCP Server for AI Agent Integration
**Category:** Discovery
**Status:** ✅ Implemented
**Implementation Date:** 2026-01-30

---

## User Story

**As an** AI agent
**I want to** access comprehensive workflow authoring documentation
**So that** I can create valid workflow definitions from scratch without trial and error

---

## Problem Statement

The initial MCP server implementation exposed the API well (how to submit, monitor, and control workflows) but lacked sufficient information for agents to **create** workflows. Agents could see:

✅ What activity types exist (`list_activities`)
✅ Basic parameter descriptions
✅ Brief mention of template expressions

But were missing critical information:
❌ Complete YAML structure (top-level fields, activity structure)
❌ Dependency syntax (`depends_on` with conditions)
❌ Template expression details (`{{INPUT.field}}`, `{{activity.output}}`, `{{SECRET.key}}`, `{{WORKFLOW.id}}`)
❌ Settings structure (retry policies, budget limits)
❌ Workflow patterns (sequential, parallel, conditional, fan-in/fan-out)
❌ Complete working examples showing all features

This created a **workflow creation discoverability gap** - agents didn't have enough "teaching material" to confidently author workflows.

---

## Acceptance Criteria

### AC1: Comprehensive YAML Structure Documentation
- ✅ Documents all required fields (`name`, `activities`)
- ✅ Documents all optional fields (`description`, `namespace`, `settings`)
- ✅ Explains activity structure (`key`, `activity_name`, `parameters`, `outputs`, `depends_on`, `settings`)
- ✅ Provides complete annotated example showing all fields

### AC2: Template Expression Reference
- ✅ Explains template syntax: `{{source.field}}` and `{{source.nested.path[0]}}`
- ✅ Documents all sources:
  - `{{INPUT.field}}` - Workflow input parameters
  - `{{activity_key.output.field}}` - Previous activity outputs
  - `{{SECRET.key}}` - Secure environment variables
  - `{{WORKFLOW.id}}`, `{{WORKFLOW.started_at}}` - Workflow metadata
- ✅ Provides examples of nested JSON access and array indexing
- ✅ Includes security notes about SECRET handling

### AC3: Dependency Pattern Examples
- ✅ Simple dependencies: `depends_on: [activity1, activity2]`
- ✅ Conditional dependencies with expressions
- ✅ Parallel execution patterns (activities without dependencies)
- ✅ Fan-out/fan-in patterns (one-to-many, many-to-one)
- ✅ Complete working examples for each pattern

### AC4: Settings Configuration Guide
- ✅ Retry policy structure (max_attempts, strategy, backoff)
- ✅ Budget limit structure (limit_usd, action)
- ✅ Timeout configuration
- ✅ Examples showing settings at activity and workflow level

### AC5: Complete Working Examples
- ✅ Simple sequential workflow (A → B → C)
- ✅ Parallel workflow with fan-in
- ✅ Conditional execution workflow
- ✅ Budget-controlled LLM workflow with model fallback
- ✅ Each example is complete, valid, and runnable

### AC6: Best Practices Documentation
- ✅ Workflow design guidelines (focused workflows, meaningful names)
- ✅ Error handling patterns (retries, conditionals, timeouts, budgets)
- ✅ Security practices (never hardcode secrets, use {{SECRET}})
- ✅ Template expression tips (test paths, check bounds, reference only dependencies)

### AC7: Agent Workflow
- ✅ Provides clear "next steps" for agents creating workflows
- ✅ References other tools to call (list_activities, validate_workflow, submit_workflow)
- ✅ Suggests validation before submission

---

## Implementation

**Tool:** `get_workflow_authoring_guide`
**Location:** `kruxiaflow-mcp/src/kruxiaflow_mcp/tools/discovery.py:100-436`
**Category:** Discovery

## Tool Response Structure

```python
{
    "yaml_structure": {
        "description": "...",
        "required_fields": {...},
        "optional_fields": {...},
        "activity_structure": {...},
        "example": "..."  # Complete YAML example
    },
    "template_expressions": {
        "description": "...",
        "syntax": "...",
        "sources": {
            "INPUT": {...},
            "activity_key": {...},
            "SECRET": {...},
            "WORKFLOW": {...}
        },
        "example": "..."
    },
    "dependency_patterns": {
        "simple_dependency": {...},
        "conditional_dependency": {...},
        "parallel_execution": {...},
        "fan_out_fan_in": {...}
    },
    "settings_configuration": {
        "retry_policy": {...},
        "budget_limit": {...},
        "timeout": {...}
    },
    "complete_examples": {
        "simple_sequential": {...},
        "parallel_with_fanin": {...},
        "conditional_execution": {...},
        "budget_controlled_llm": {...}
    },
    "best_practices": {
        "workflow_design": [...],
        "error_handling": [...],
        "security": [...],
        "template_expressions": [...]
    },
    "next_steps": {
        "description": "...",
        "steps": [...]
    }
}
```

---

## Usage Pattern

### Agent Workflow Creation Process

```python
# Step 1: Get authoring guide (first time creating workflows)
guide = await get_workflow_authoring_guide()

# Review structure, patterns, examples
yaml_structure = guide["yaml_structure"]
templates = guide["template_expressions"]
patterns = guide["dependency_patterns"]
examples = guide["complete_examples"]

# Step 2: Get available activity types
activities = await list_activities()

# Step 3: Author workflow using patterns from guide
workflow_yaml = """
name: my_workflow
description: My custom workflow
activities:
  - key: step1
    activity_name: http_request
    parameters:
      method: GET
      url: "{{INPUT.api_url}}"
    outputs:
      - response

  - key: step2
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-sonnet-4
      prompt: "Analyze: {{step1.response}}"
    depends_on:
      - step1
    settings:
      budget:
        limit_usd: 0.10
        action: abort
"""

# Step 4: Validate before submission
validation = await validate_workflow(workflow_yaml)
if not validation["valid"]:
    print(f"Errors: {validation['errors']}")
    # Fix issues and retry

# Step 5: Submit workflow
result = await submit_workflow(
    definition_name="my_workflow",
    input={"api_url": "https://api.example.com/data"}
)
workflow_id = result["workflow_id"]
```

---

## Design Decisions

### 1. Single Comprehensive Tool vs Multiple Small Tools

**Decision:** Single `get_workflow_authoring_guide()` tool returning complete documentation

**Rationale:**
- Reduces tool call overhead (1 call vs 5+ calls for structure, templates, patterns, etc.)
- Agent receives complete reference in one response
- Easier to maintain consistency across related documentation
- FastMCP supports rich structured responses efficiently

**Alternatives Considered:**
- Separate tools for each section (get_yaml_structure, get_template_guide, etc.)
  - ❌ More tool calls = more latency
  - ❌ Agent might miss related information
- External documentation URL
  - ❌ Requires network access
  - ❌ Not integrated with MCP discovery

### 2. JSON Response vs Markdown String

**Decision:** Structured JSON with nested sections and examples

**Rationale:**
- Agents can parse and navigate structured data programmatically
- Can extract specific sections (e.g., only dependency patterns)
- Examples are preserved as strings within structure
- Consistent with other MCP tools

**Alternatives Considered:**
- Single markdown string
  - ❌ Harder to parse programmatically
  - ✅ Easier for humans to read (not primary audience)
- Mixed (JSON structure with markdown values)
  - ✅ Good middle ground (what we implemented)

### 3. Coverage Scope

**Decision:** Include all workflow authoring concepts in single tool

**Rationale:**
- Covers the complete workflow creation lifecycle
- No need for external documentation or guessing
- Self-contained reference for offline use
- Matches FastMCP's design for comprehensive tool descriptions

**What's Included:**
- ✅ YAML structure and fields
- ✅ All template expression sources
- ✅ All dependency patterns
- ✅ All settings types
- ✅ Complete working examples
- ✅ Security and best practices

**What's NOT Included:**
- Activity-specific parameter details → Use `list_activities()` instead
- Workflow definition storage → Use `list_workflow_definitions()` instead
- Validation logic → Use `validate_workflow()` instead
- API authentication → Handled by MCP client configuration

---

## Impact

### Before US-MCP-14

Agents had to:
- Guess workflow YAML structure from minimal examples
- Trial-and-error with template expression syntax
- Discover dependency patterns through failures
- Miss features like conditional dependencies, SECRET references
- Rely on external documentation or human guidance

### After US-MCP-14

Agents can:
- ✅ Discover complete workflow authoring capabilities through MCP
- ✅ Learn YAML structure, templates, patterns from single tool call
- ✅ Author workflows confidently with full reference
- ✅ Use advanced features (conditionals, secrets, budgets) from day one
- ✅ Follow security best practices (never hardcode secrets)
- ✅ Validate before submission to catch errors early

---

## Testing

### Manual Testing

```python
# Test: Get authoring guide
guide = await get_workflow_authoring_guide()

assert "yaml_structure" in guide
assert "template_expressions" in guide
assert "dependency_patterns" in guide
assert "settings_configuration" in guide
assert "complete_examples" in guide
assert "best_practices" in guide
assert "next_steps" in guide

# Verify complete examples are valid YAML
import yaml
for example_name, example_data in guide["complete_examples"].items():
    workflow_def = yaml.safe_load(example_data["yaml"])
    assert "name" in workflow_def
    assert "activities" in workflow_def
    assert len(workflow_def["activities"]) > 0
```

---

## Documentation Updates

- ✅ Added to MCP User Guide (mcp-userguide.md)
- ✅ Updated tool count from 13 to 14 tools
- ✅ Added complete tool description in Discovery Tools section
- ✅ Cross-referenced with `list_activities()` and `validate_workflow()`

---

## Future Enhancements

### Post-MVP Considerations

1. **Interactive Workflow Builder**
   - Tool that generates workflow YAML through Q&A
   - "What do you want to do?" → Suggests activities and patterns
   - Could reduce need to read full guide for simple cases

2. **Workflow Templates**
   - Pre-built templates for common patterns (RAG, ETL, API orchestration)
   - Agent can customize template instead of authoring from scratch
   - Faster workflow creation for standard use cases

3. **Validation with Suggestions**
   - Enhanced `validate_workflow()` that suggests fixes
   - "Missing depends_on for template reference" → Suggest correct dependency
   - Reduce iteration cycles for workflow authoring

4. **Activity Parameter Schemas**
   - Detailed JSON Schema for each activity's parameters
   - Enable IDE-like autocomplete and validation
   - More structured than current description strings

---

## Related User Stories

- US-MCP-1: List Workflow Definitions (discover existing workflows)
- US-MCP-2: Get Workflow Definition Details (inspect workflow structure)
- US-MCP-3: List Activity Types (discover building blocks)
- US-MCP-4: Validate Workflow Definition (check before submission)
- US-MCP-5: Submit Workflow for Execution (run created workflows)

---

## Success Metrics

- ✅ Agent can create valid workflow on first attempt using guide
- ✅ Agent uses advanced features (conditionals, secrets, budgets) without external help
- ✅ Validation errors reduced by providing complete reference upfront
- ✅ Time to first successful workflow submission decreased
- ✅ No need for external documentation or human guidance for workflow creation
