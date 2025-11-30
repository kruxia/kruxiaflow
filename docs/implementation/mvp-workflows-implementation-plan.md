# MVP Workflows Implementation Plan

**Version**: 0.3.0
**Date**: 2025-11-27
**Status**: ✅ **MVP COMPLETE** - All Examples (1-10) and Core Activities Implemented
**Test Coverage**: 90.56% (target >90% achieved)

**Recent Updates**:
- US-5.7a (Email Send) ✅ Complete - SMTP email activity with HTML/text support, TLS modes
- Example 10 (Order Processing) ✅ Complete - E-commerce workflow with HTTP, transactions, email
- US-5.6 (Database Operations) ✅ Complete - postgres_query and postgres_transaction with connection pooling
- Example 9 (Token Streaming) ✅ Complete - Two-workflow series (09a, 09b)
- US-7.1 (Token Streaming) ✅ Complete - WebSocket-based real-time LLM streaming
- US-1A.9a (WebSocket Infrastructure) ✅ Complete - Foundation for token streaming
- Example 8 (Activity Scheduling and Delays) ✅ Complete - Three-workflow series (08a, 08b, 08c)
- US-3.7 (Activity Scheduling) ✅ Complete - delay and scheduled_for with template support
- Example 7 (Agentic Research / Iterative Workflows) ✅ Complete - Two-workflow series (07a, 07b)
- Example 6 (Semantic Caching and RAG) ✅ Complete - Three-workflow series (06a, 06b, 06c)
- US-3.4 (Iterative Workflows) ✅ Complete - Example 7 demonstrates loops with simple and complete variants
- US-5.3 (Semantic Caching) ✅ Complete - 100% production ready
- US-5.1 (Multi-Provider LLM) ✅ Phases 1-5 Complete

**MVP Status**: The orchestrator and built-in worker with all core activities are feature-complete. All 10 examples implemented and tested.

---

## Overview

This document provides a example-based implementation plan for Epic 3 (YAML Workflow Definition Language) and Epic 5 (Built-In Activity Library). Rather than implementing these epics separately, we build both incrementally through **realistic workflow examples** that progress from simple to complex.

Each example is defined by an example workflow that demonstrates new capabilities in both YAML features and built-in activities. By the end of all examples, we will have implemented all requirements from both Epic 3 and Epic 5.

### Key Principles

1. **Example-Driven**: Each example is anchored by a concrete, runnable workflow example
2. **Incremental Complexity**: Start simple, add features progressively
3. **YAML + Activities Together**: Can't define YAML features without activities to use them
4. **End-to-End Testing**: Each example should be fully testable and demonstrable
5. **Market-Aligned**: Examples match real user needs (AI cost control, data pipelines, edge)

### Example Workflow Organization

Workflow examples are organized as follows:

**Implemented Examples** (in `examples/`):
```
examples/
├── 01-weather-report.yaml              # 1: Sequential workflow (✅ COMPLETE)
├── 01b-weather-report-dynamic.yaml     # 1b: Dynamic templates variant (✅ COMPLETE)
├── 02-user-validation.yaml             # 2: Conditional branching (✅ COMPLETE)
├── 03-document-processing.yaml         # 3: Parallel execution (✅ COMPLETE)
├── 04-moderate-content.yaml            # 4: LLM with cost tracking (✅ COMPLETE)
├── 05-research-assistant.yaml          # 5: Multi-model LLM fallback (✅ COMPLETE)
├── 05a-research-assistant-anthropic.yaml  # 5a: Anthropic-only variant (✅ COMPLETE)
├── 05b-research-assistant-openai.yaml     # 5b: OpenAI-only variant (✅ COMPLETE)
├── 05c-research-assistant-google.yaml     # 5c: Google-only variant (✅ COMPLETE)
├── 06a-faq-bot-caching.yaml            # 6a: Semantic caching (✅ COMPLETE)
├── 06b-rag-index-builder.yaml          # 6b: RAG index building (✅ COMPLETE)
├── 06c-rag-query.yaml                  # 6c: RAG query pattern (✅ COMPLETE)
├── 07a-agentic-research-simple.yaml    # 7a: Simple iterative loops (✅ COMPLETE)
├── 07b-agentic-research-complete.yaml  # 7b: Complete iterative loops (✅ COMPLETE)
├── 08a-rate-limited-api-calls.yaml     # 8a: Rate limiting with delay (✅ COMPLETE)
├── 08b-scheduled-daily-report.yaml     # 8b: Absolute scheduling with scheduled_for (✅ COMPLETE)
├── 08c-delayed-reminders.yaml          # 8c: Cascading delays (✅ COMPLETE)
├── 09a-streaming-llm.yaml              # 9a: Basic LLM token streaming (✅ COMPLETE)
├── 09b-streaming-research.yaml         # 9b: Selective streaming workflow (✅ COMPLETE)
├── 10-order-processing.yaml            # 10: Order processing with email (✅ COMPLETE)
└── README.md                           # Index of examples with descriptions
```

**Naming Convention**:
- Format: `NN-descriptive-name.yaml`
- One YAML file per example workflow
- File names match the workflow use case, not the feature being demonstrated
- README.md provides an index showing which features each example demonstrates

**Purpose**:
- **Documentation**: Living examples that demonstrate features
- **Testing**: Each example used in end-to-end tests
- **Validation**: Users can run examples to verify their installation
- **Learning**: Progressive complexity for learning the workflow language

**examples/README.md Structure**:
The README.md in the examples directory provides:
- Overview of the examples collection
- Table mapping each example to the features it demonstrates
- Instructions for running examples (`streamflow test examples/XX-*.yaml`)
- Links to relevant documentation sections
- Prerequisites for running examples (e.g., API keys, test services)

Example table format:
| Example                       | Example | Features Demonstrated                                    | Prerequisites     |
|-------------------------------|-------|----------------------------------------------------------|-------------------|
| `01-weather-report.yaml`      | 1     | Sequential workflow, HTTP request (GET/POST), headers, secrets | Webhook URL       |
| `02-user-validation.yaml`     | 2     | Conditional branching, PostgreSQL                        | Database, API key |
| ...                           | ...   | ...                                                      | ...               |

---

## Implementation Examples

### Example 1: Simple Sequential Workflow with HTTP Activity
**Duration**: 3-4 days (✅ **COMPLETED** 2025-11-12)
**Epic 3**: US-3.1 (Sequential Workflows - Basic)
**Epic 5**: US-5.5 (HTTP Operations - Basic)
**Status**: ✅ **FOUNDATION COMPLETE** - Core infrastructure implemented and tested

#### Example Workflow: Weather Report Pipeline
```yaml
name: weather_report
description: Fetch weather data and send to webhook

activities:
  fetch_weather:
    activity: http_request
    # worker: builtin  # Optional - builtin is the default
    parameters:
      method: GET
      url: "https://api.weather.gov/gridpoints/TOP/31,80/forecast"
      headers:
        User-Agent: "StreamFlow/0.2"
    outputs:
      - forecast

  send_notification:
    activity: http_request
    parameters:
      method: POST
      url: "{{INPUT.webhook_url}}"
      headers:
        Content-Type: "application/json"
      body:
        temperature: "{{fetch_weather.forecast.temperature}}"
        conditions: "{{fetch_weather.forecast.conditions}}"
    depends_on:
      - fetch_weather
```

#### YAML Features Implemented
- ✅ Activity definition with `activity` name and optional `worker` (defaults to `builtin`)
- ✅ Activity naming: lowercase alphanumeric with underscores (snake_case, e.g., `http_request`, `postgres_query`)
- ✅ Sequential execution via `depends_on`
- ✅ Template expressions: `{{INPUT.input_name}}`
- ✅ Output access: `{{activity_key.output_name}}`
- ✅ Workflow input parameters

#### Built-in Activities Implemented
- ✅ `http_request` - HTTP request with configurable method (GET, POST, etc.), custom headers (including Authorization), and request body

#### Implementation Tasks
1. ✅ **COMPLETED** YAML parser for workflow definitions (serde_yaml)
   - Added `serde_yaml` dependency to workspace
   - Implemented `WorkflowDefinition::from_yaml()` and `to_yaml()` methods
   - Added comprehensive YAML parsing tests
2. ✅ **COMPLETED** Template expression engine (basic variable substitution)
   - Created `core/src/workflow/template.rs` module
   - Supports `{{INPUT.key}}`, `{{activity_key.output_name}}`, `{{SECRET.key}}`, `{{WORKFLOW.key}}`
   - Preserves JSON types when resolving entire values
   - Handles nested object and array resolution
   - 7 passing unit tests
3. ✅ **COMPLETED** Workflow graph builder (activities → nodes, depends_on/contributes_to → edges)
   - Already exists in `core/src/workflow/definition.rs`
   - Validates graph structure, detects cycles
   - Supports `depends_on` and `dependency_of` relationships
4. ✅ **COMPLETED** HTTP activity executor (reqwest)
   - Created `activity/src/http.rs` module
   - Configurable HTTP method (GET, POST, PUT, DELETE, PATCH)
   - Custom headers (including Authorization: Bearer, Basic auth)
   - Request body (JSON, form data, etc.)
   - Query parameter support
   - Header templating with secrets ({{SECRET.name}})
   - Timeout configuration
   - 3 passing integration tests with httpbin.org
5. ⏭️ **DEFERRED TO INTEGRATION** Activity result storage and retrieval
   - Will be integrated when connecting activities to orchestrator
6. ⏭️ **DEFERRED TO INTEGRATION** End-to-end test: Submit workflow via API, verify execution
   - Will be implemented after activity worker integration

#### Success Criteria
- ✅ **ACHIEVED** Parse YAML workflow definition
  - `WorkflowDefinition::from_yaml()` parses YAML successfully
  - Validation ensures graph integrity (no cycles, valid references)
  - Example workflow `examples/01-weather-report.yaml` created
- ⏭️ **PENDING INTEGRATION** Execute activities in sequence
  - Core workflow graph structures exist
  - Need to integrate HTTP activity executor with orchestrator
- ✅ **ACHIEVED** Template expressions resolve correctly (including SECRET references)
  - Template engine supports all required expression types
  - Type preservation for JSON values
  - Comprehensive test coverage
- ✅ **ACHIEVED** HTTP activities complete successfully with custom headers
  - HTTP executor supports all methods and headers
  - Tested with real HTTP endpoints (httpbin.org)
  - Query parameters and request bodies work correctly
- ✅ **ACHIEVED** Authorization headers (Bearer token, Basic auth) work correctly
  - Custom headers fully supported
  - Template substitution in headers works
- ⏭️ **PENDING INTEGRATION** Workflow completes with final status
  - Will be verified in end-to-end integration tests

#### Implementation Notes

**What was built:**
1. **YAML Support** - Added `serde_yaml` dependency and parsing/serialization methods
2. **Template Engine** - Complete variable substitution system with type preservation
3. **HTTP Activity** - Full-featured HTTP client with reqwest
4. **Example Workflows** - Weather report pipelines demonstrating sequential execution and activity chaining
5. **Test Coverage** - 21 new tests covering YAML parsing (5), templates (10), and HTTP activities (6)

**What's next (integration phase):** *(All completed for Example 1)*
1. ✅ Create built-in worker that executes HTTP activities
2. ✅ Integrate activity executor with orchestrator event flow
3. ✅ Connect template engine to activity parameter resolution
4. ✅ End-to-end workflow execution tests
5. ✅ Unified API endpoint accepting both JSON and YAML workflows (JSON is valid YAML)

**Files Created/Modified:**
- `core/src/workflow/template.rs` - Template expression engine (new)
- `core/src/workflow/definition.rs` - Added YAML parsing methods
- `activity/src/http.rs` - HTTP activity executor (new)
- `worker/src/activities/http.rs` - HTTP activity worker implementation (new)
- `api/src/handlers/workflow_definitions.rs` - Modified to accept both JSON and YAML via unified endpoint
- `api/tests/yaml_workflow_e2e_tests.rs` - End-to-end workflow test (new)
- `examples/01-weather-report.yaml` - Example workflow (new)
- `examples/01b-weather-report-dynamic.yaml` - Advanced sequential workflow with chaining (new)
- `examples/README.md` - Examples documentation (new)
- `core/tests/yaml_workflow_tests.rs` - YAML parsing tests (new)
- Updated Cargo.toml files for dependencies

---

### Example 2: Conditional Branching with Database Storage
**Duration**: 3-4 days (✅ **COMPLETED** 2025-11-15)
**Epic 3**: US-3.2 (Conditional Branching)
**Epic 5**: US-5.6 (Database Operations)
**Status**: ✅ **COMPLETE** - Conditional branching and PostgreSQL integration implemented

#### Example Workflow: User Validation with Audit Trail
```yaml
name: validate_user
description: Validate user email and store result in database

activities:
  check_email:
    activity: http_request
    parameters:
      method: GET
      url: "https://api.emailvalidation.io/validate"
      query:
        email: "{{INPUT.email}}"
        api_key: "{{SECRET.email_validator_key}}"
    outputs:
      - valid
      - reason

  store_valid_user:
    activity: postgres_query
    parameters:
      db_url: "{{SECRET.db_url}}"
      query: "INSERT INTO valid_users (email, validated_at) VALUES ($1, NOW())"
      params:
        - "{{INPUT.email}}"
    depends_on:
      - check_email:
          condition: "{{check_email.valid == true}}"

  store_invalid_user:
    activity: postgres_query
    parameters:
      db_url: "{{SECRET.db_url}}"
      query: "INSERT INTO invalid_users (email, reason, checked_at) VALUES ($1, $2, NOW())"
      params:
        - "{{INPUT.email}}"
        - "{{check_email.reason}}"
    depends_on:
      - check_email:
          condition: "{{check_email.valid == false}}"

  send_notification:
    activity: http_request
    parameters:
      method: POST
      webhook_key: "{{SECRET.notification_webhook_key}}"
      url: "{{INPUT.notification_webhook_url}}"
      body:
        email: "{{INPUT.email}}"
        status: "{{check_email.valid}}"
    depends_on:
      - store_valid_user:
          condition: "{{check_email.valid == true}}"
      - store_invalid_user:
          condition: "{{check_email.valid != true}}"
```

#### YAML Features Implemented
- ✅ Conditional edges with boolean expressions
- ✅ Comparison operators: `==`, `!=` (full MiniJinja expression support)
- ✅ Multiple edges from single activity (fan-out with conditions)
- ✅ Secret references: `{{SECRET.name}}`
- ✅ `depends_on` as alias for `depends_on` (user-friendly YAML syntax)
- ✅ Flexible condition syntax: both `condition` (single) and `conditions` (array)

#### Built-in Activities Implemented
- ✅ `postgres_query` - Execute SQL queries (SELECT, INSERT, UPDATE, DELETE) with parameter binding
  - SELECT queries: Returns result rows in outputs
  - INSERT/UPDATE/DELETE: Returns affected row count in outputs
  - Connection pooling for performance
  - Parameterized queries for SQL injection prevention

#### Implementation Tasks
1. ✅ **COMPLETED** Conditional edge evaluation - MiniJinja template engine evaluates condition expressions to boolean values
2. ✅ **COMPLETED** Edge evaluation engine - Check MiniJinja-evaluated conditions before scheduling dependent activities
3. ✅ **COMPLETED** Secret management - SECRET context already existed in template resolution (from Example 1)
4. ✅ **COMPLETED** PostgreSQL activity executor
   - SQL execution with parameterized queries
   - Query result parsing and output storage
   - Connection pool caching for reuse
5. ✅ **COMPLETED** Branching logic in orchestrator - Already existed in dependency evaluator, enhanced with MiniJinja
6. ✅ **COMPLETED** End-to-end test: Workflow branches based on HTTP response

#### Success Criteria
- ✅ **ACHIEVED** Conditional expressions evaluate correctly (using MiniJinja)
- ✅ **ACHIEVED** Only activities with satisfied conditions execute
- ✅ **ACHIEVED** Secrets resolve from secure storage (pre-existing feature)
- ✅ **ACHIEVED** Database activities complete successfully
- ✅ **ACHIEVED** Multiple paths can converge (fan-in) with conditions

#### Implementation Notes

**What was built:**
1. **Enhanced YAML Deserialization** - Added `depends_on` alias and flexible condition syntax
2. **MiniJinja Conditional Evaluation** - Replaced string-based evaluation with full template engine
3. **PostgreSQL Activity** - Complete query executor with connection pooling
4. **Example Workflow** - `examples/02-user-validation.yaml` demonstrating conditional branching
5. **End-to-End Tests** - Comprehensive test verifying conditional branching with database operations

**Files Created/Modified:**
- `worker/src/activities/postgres.rs` - PostgreSQL activity executor (new)
- `worker/src/activities/mod.rs` - Added postgres module export
- `worker/Cargo.toml` - Added sqlx dependency
- `core/src/workflow/definition.rs` - Added `depends_on` alias and custom ActivityRelationship deserializer
- `core/src/orchestrator/dependency_evaluator.rs` - Updated conditional evaluation to use MiniJinja
- `examples/02-user-validation.yaml` - Example workflow (new)
- `api/tests/yaml_workflow_e2e_tests.rs` - Added conditional branching test (new)

---

### Example 3: Parallel Execution with File Management
**Duration**: 4-5 days (✅ **COMPLETED** 2025-11-18)
**Epic 3**: US-3.3 (Parallel Execution - Fan-Out/Fan-In)
**Epic 5**: US-5.4 (Object Storage and File Management)
**Status**: ✅ **COMPLETE** - Parallel execution and file management implemented

#### Example Workflow: Multi-Document Processing Pipeline
```yaml
name: process_documents
description: Fetch documents, process in parallel, aggregate results

activities:
  fetch_doc1:
    activity: http_request
    parameters:
      method: GET
      url: "{{INPUT.doc1_url}}"
    outputs:
      - name: doc1
        type: file  # Declares this output is a file, not JSON data

  fetch_doc2:
    activity: http_request
    parameters:
      method: GET
      url: "{{INPUT.doc2_url}}"
    outputs:
      - name: doc2
        type: file

  fetch_doc3:
    activity: http_request
    parameters:
      method: GET
      url: "{{INPUT.doc3_url}}"
    outputs:
      - name: doc3
        type: file

  process_doc1:
    activity: http_request
    parameters:
      method: POST
      url: "{{INPUT.processing_service_url}}"
      files:
        input_doc: "{{FILE.fetch_doc1.document}}"  # Reference file from previous activity
    outputs:
      - name: result1
        type: file  # Processed result is also a file
    depends_on:
      - fetch_doc1

  process_doc2:
    activity: http_request
    parameters:
      method: POST
      url: "{{INPUT.processing_service_url}}"
      files:
        input_doc: "{{FILE.fetch_doc2.document}}"
    outputs:
      - name: result2
        type: file
    depends_on:
      - fetch_doc2

  process_doc3:
    activity: http_request
    parameters:
      method: POST
      url: "{{INPUT.processing_service_url}}"
      files:
        input_doc: "{{FILE.fetch_doc3.document}}"
    outputs:
      - name: result3
        type: file
    depends_on:
      - fetch_doc3

  aggregate_results:
    activity: http_request
    parameters:
      method: POST
      url: "{{INPUT.aggregator_url}}"
      files:
        doc1_result: "{{FILE.process_doc1.result}}"
        doc2_result: "{{FILE.process_doc2.result}}"
        doc3_result: "{{FILE.process_doc3.result}}"
    outputs:
      - name: summary
        type: file  # Aggregated summary as file
    depends_on:
      - process_doc1
      - process_doc2
      - process_doc3

  store_summary:
    activity: http_request
    parameters:
      method: POST
      url: "{{INPUT.storage_webhook_url}}"
      files:
        summary: "{{FILE.aggregate_results.summary}}"
      body:
        workflow_id: "{{WORKFLOW.id}}"
        completed_at: "{{WORKFLOW.completed_at}}"
    depends_on:
      - aggregate_results
```

#### YAML Features Implemented
- ✅ Multiple `depends_on` edges (fan-in)
- ✅ Multiple activities contributing to same dependent activity (fan-out)
- ✅ File outputs with `type: file` declaration
- ✅ File references with `{{FILE.activity_key.output_name}}`
- ✅ Parallel file processing without storing content in JSON

#### Built-in Activities Implemented
- ✅ `http_request` - HTTP request with file download/upload support
  - GET method: Download files from HTTP endpoints
  - POST method: Upload files via HTTP multipart/form-data
- ✅ File management framework (object storage backend)

#### Implementation Tasks
1. ✅ **COMPLETED** Parallel activity scheduling (multiple ready activities scheduled simultaneously)
2. ✅ **COMPLETED** Fan-in synchronization (wait for ALL preceding activities)
3. ✅ **COMPLETED** **File management infrastructure**:
   - WorkflowStorage interface with PostgreSQL Large Objects backend (MVP)
   - File upload when activity completes with `type: file` output
   - File download when activity references `{{FILE.activity_key.name}}`
   - Storage in PostgreSQL Large Objects with metadata tracking
   - Streaming upload/download (no full memory load)
4. ✅ **COMPLETED** **HTTP activity file support**:
   - GET: Save response body as file
   - POST: Send files via multipart/form-data
   - File parameter handling in activity executor
5. ✅ **COMPLETED** **Activity interface for files**:
   - Activities receive file paths for reading
   - Framework handles file storage after completion
   - FILE template expressions resolve to file references
6. ✅ **COMPLETED** End-to-end test: Verify parallel execution with file passing, check fan-in waits for all

#### Success Criteria
- ✅ **ACHIEVED** Multiple activities execute in parallel
- ✅ **ACHIEVED** Fan-in waits for ALL preceding activities before executing
- ✅ **ACHIEVED** PostgreSQL Large Objects storage works correctly
- ✅ **ACHIEVED** Large files handled efficiently (streaming, not full memory load)

#### Implementation Notes

**What was built:**
1. **Parallel Execution** - Orchestrator schedules multiple ready activities simultaneously
2. **Fan-Out/Fan-In** - Dependency evaluator correctly handles multiple dependencies
3. **File Management** - Complete WorkflowStorage interface with PostgreSQL backend
4. **HTTP File Support** - File download (GET) and upload (POST multipart/form-data)
5. **Example Workflow** - `examples/03-document-processing.yaml` demonstrating 8-activity pipeline
6. **End-to-End Tests** - Comprehensive test with mock HTTP server verifying all features

**Files Created/Modified:**
- `examples/03-document-processing.yaml` - Parallel file processing workflow (new)
- `examples/README.md` - Added Example 3 documentation (updated)
- `api/tests/example_03_e2e_test.rs` - End-to-end test with mock HTTP server (new)
- `docs/architecture.md` - Added file management section (updated)
- `docs/implementation/mvp-workflows-implementation-plan.md` - Marked Example 3 complete (updated)

**Features Validated:**
- ✅ Multiple activities ready simultaneously (fetch_doc1, fetch_doc2, fetch_doc3)
- ✅ Activities execute in parallel (worker concurrency)
- ✅ Fan-in activity waits for ALL dependencies (aggregate_results)
- ✅ No circular dependencies in workflow graph
- ✅ File outputs declared with `type: file`
- ✅ FILE template expressions parse and resolve correctly
- ✅ HTTP activity handles file downloads (GET)
- ✅ HTTP activity handles file uploads (POST multipart)
- ✅ Files stored in PostgreSQL Large Objects
- ✅ File references passed between activities

---

### Example 4: LLM Activity with Cost Tracking and Retry
**Duration**: 5-6 days
**Epic 3**: US-3.5 (Activity Settings - Retry, Timeout, Budget)
**Epic 5**: US-5.1 (Multi-Provider LLM - Basic), US-5.2 (Cost Tracking)

#### Example Workflow: AI Content Moderation with Fallback
```yaml
name: moderate_content
description: Use LLM to moderate user content with cost control and retry

activities:
  analyze_content:
    activity: llm_prompt
    parameters:
      model: anthropic/haiku-4-5
      messages:
        - role: system
          content: "You are a content moderation assistant. Analyze the following text and determine if it violates community guidelines."
        - role: user
          content: "{{INPUT.user_content}}"
      max_tokens: 500
    outputs:
      - response
      - cost_usd
      - tokens_used
    settings:
      timeout_seconds: 30
      retry:
        max_attempts: 3
        strategy: exponential  # or "fixed"
        base_seconds: 2
        factor: 2
        max_seconds: 60
      budget:
        limit_usd: 0.50
        on_exceeded: abort

  store_moderation_result:
    activity: postgres_query
    parameters:
      query: |
        INSERT INTO moderation_log
        (content_id, decision, cost, tokens, moderated_at)
        VALUES ($1, $2, $3, $4, NOW())
      params:
        - "{{INPUT.content_id}}"
        - "{{analyze_content.response}}"
        - "{{analyze_content.cost_usd}}"
        - "{{analyze_content.tokens_used}}"
    depends_on:
      - analyze_content
```

#### YAML Features Implemented
- ✅ Activity settings: `retry`, `timeout`, `budget`
- ✅ Retry policy with exponential backoff
- ✅ Budget limits per activity
- ✅ Budget exceeded action: `abort` or `continue`

#### Built-in Activities Implemented
- ✅ `llm_prompt` - LLM completion with Anthropic
- ✅ Cost tracking: Token counting and USD calculation
- ✅ Retry logic with exponential backoff

#### Implementation Tasks
1. ✅ **COMPLETED** Activity settings parser (retry, timeout, budget)
2. ✅ **COMPLETED** **Orchestrator retry logic** (NOT in queue or workers):
   - ✅ Add `handle_activity_failed_event()` in orchestrator
   - ✅ Check retry settings from workflow definition
   - ✅ Calculate backoff delay: `base_seconds * factor^(attempt-1)` capped at `max_seconds`
   - ✅ Track attempt count in `workflows.state_data` JSONB
   - ✅ Publish `ActivityScheduled` event with `scheduled_for` = NOW() + backoff
   - ✅ Record attempt history in `workflow_events` (immutable event log)
3. ⏳ Timeout enforcement (tokio timeout) - Deferred to future implementation
4. ✅ **COMPLETED** Budget tracking service
   - ✅ Pre-execution budget check
   - ✅ Post-execution cost recording
   - ✅ Budget exceeded handling
5. ✅ **COMPLETED** Anthropic activity executor
   - ✅ API integration (Anthropic SDK)
   - ✅ Token counting
   - ✅ Cost calculation (tokens × price per model)
6. ✅ **COMPLETED** Cost storage in PostgreSQL
7. ✅ **COMPLETED** End-to-end test: Verify retry on failure, budget enforcement
   - ✅ Test: Activity fails, orchestrator schedules retry with backoff
   - ✅ Test: Max attempts reached, activity fails permanently
   - ✅ Test: Exponential backoff delay increases correctly
   - ✅ Test: Fixed backoff delay remains constant

**Design Note**: Retry logic implemented in orchestrator event handlers (NOT database stored procedures or workers) for clean separation of concerns and horizontal scalability. See absurd analysis Section 3.

#### Success Criteria
- ✅ **ACHIEVED** LLM activity completes successfully with Anthropic
- ✅ **ACHIEVED** Retries occur on transient failures (rate limits, network errors)
- ✅ **ACHIEVED** Activity aborts when budget exceeded
- ✅ **ACHIEVED** Cost tracked accurately in USD
- ⏳ Timeout enforced correctly - Deferred to future implementation

#### Implementation Notes

**What was built:**
1. **Activity Settings Model** - Complete implementation of retry, timeout, and budget settings
2. **Retry Logic** - Orchestrator handles ActivityFailed events with exponential backoff
3. **Budget Tracking** - Pre-execution budget checks and post-execution cost recording
4. **LLM Activity** - Anthropic Claude integration with cost calculation
5. **Multi-Provider Support** - Anthropic, OpenAI, Google, and Ollama providers
6. **Fallback Chains** - Automatic fallback to cheaper/alternative models on budget constraints
7. **Example Workflow** - `examples/04-moderate-content.yaml` demonstrating all features
8. **Comprehensive Tests** - Budget tracking and retry integration tests

**Files Created/Modified:**
- `examples/04-moderate-content.yaml` - Content moderation workflow with retry and budget (new)
- `examples/README.md` - Added Example 4 documentation (updated)
- `worker/src/activities/llm.rs` - LLM activity with budget-aware fallback chains (already implemented)
- `core/src/orchestrator/orchestrator.rs` - Retry logic and budget enrichment (already implemented)
- `core/tests/llm_budget_integration_tests.rs` - Comprehensive budget tests (already implemented)
- `core/tests/retry_integration_tests.rs` - Retry logic tests (already implemented)
- `docs/implementation/mvp-workflows-implementation-plan.md` - Marked Example 4 complete (updated)

**Features Validated:**
- ✅ ActivitySettings parsed from YAML (retry, timeout, budget)
- ✅ Retry logic with exponential backoff (2s, 4s, 8s, etc.)
- ✅ Budget limits enforced before LLM execution
- ✅ Cost tracking per activity attempt
- ✅ Budget exceeded action (abort workflow)
- ✅ Multi-provider LLM support (Anthropic, OpenAI, Google, Ollama)
- ✅ Fallback chains skip expensive models when budget constrained
- ✅ Token usage and USD cost captured in activity results
- ✅ Comprehensive test coverage for all features

---

### Example 5: Multi-Model LLM with Automatic Fallback
**Duration**: 4-5 days (✅ **COMPLETED** 2025-11-19)
**Epic 3**: (No new YAML features - builds on Example 4)
**Epic 5**: US-5.1 (Multi-Model LLM - Complete)
**Status**: ✅ **COMPLETE** - Multi-model fallback fully implemented and documented

#### Example Workflow: AI Research Assistant with Budget-Aware Model Fallback
```yaml
name: research_assistant
description: Ask LLM question with automatic model fallback and budget-aware provider selection

# Budget-Aware Fallback Behavior (with $0.01 budget):
# For typical prompt (100 input tokens + 1000 output tokens):
#
# 1. openai/o1-pro ($150/$600 per M tokens):
#    Estimated cost: ~$0.615 → SKIPPED (exceeds budget)
#
# 2. anthropic/claude-sonnet-4-5-20250929 ($3/$15 per M tokens):
#    Estimated cost: ~$0.0153 → SKIPPED (exceeds $0.01 budget)
#
# 3. google/gemini-2.0-flash-lite ($0.075/$0.30 per M tokens):
#    Estimated cost: ~$0.0003 → USED (well under budget)

activities:
  ask_question:
    activity: llm_prompt
    parameters:
      model: # Automatic fallback chain (budget-aware)
        - openai/o1-pro                          # Expensive (will be skipped)
        - anthropic/claude-sonnet-4-5-20250929   # Moderate (may be skipped)
        - google/gemini-2.0-flash-lite           # Very cheap (will be used)
      prompt: "{{INPUT.question}}"
      max_tokens: 1000
    outputs:
      - result
    settings:
      retry:
        max_attempts: 3
        strategy: exponential
      budget:
        limit_usd: 0.01  # Tight budget to demonstrate budget-aware fallback
        action: abort

  store_response:
    activity: postgres_query
    parameters:
      query: |
        INSERT INTO research_log
        (question, answer, provider, model, cost, created_at)
        VALUES ($1, $2, $3, $4, $5, NOW())
      params:
        - "{{INPUT.question}}"
        - "{{ask_question.result.content}}"
        - "{{ask_question.result.provider}}"
        - "{{ask_question.result.model}}"
        - 0.001  # Placeholder - actual cost calculation
    depends_on:
      - ask_question
```

#### YAML Features Implemented
- ✅ Model fallback configuration (`model:` array syntax)
- ✅ Budget-aware provider selection (skips expensive models automatically)
- ✅ Multi-provider fallback chain

#### Built-in Activities Implemented
- ✅ `llm_prompt` - OpenAI models (o1-pro, GPT-5, etc.)
- ✅ `llm_prompt` - Anthropic models (Claude 4.5 Sonnet, Haiku, Opus)
- ✅ `llm_prompt` - Google models (Gemini 2.5, 2.0, 1.5)
- ✅ `llm_prompt` - Ollama models (Llama, Mistral, etc.)
- ✅ Model fallback logic with budget awareness
- ✅ Cost estimation before execution

#### Implementation Tasks
1. ✅ **COMPLETED** Model abstraction layer (trait for LLM models)
2. ✅ **COMPLETED** OpenAI API integration
3. ✅ **COMPLETED** Gemini API integration
4. ✅ **COMPLETED** Model fallback engine
   - ✅ Try models in order
   - ✅ Record which model succeeded
5. ✅ **COMPLETED** Model-specific cost calculation
6. ✅ **COMPLETED** End-to-end test: Verify fallback to next model on failure

#### Success Criteria
- ✅ **ACHIEVED** Multiple LLM providers supported (OpenAI, Anthropic, Gemini, Ollama)
- ✅ **ACHIEVED** Automatic fallback on model failure or rate limits
- ✅ **ACHIEVED** Track which model was used
- ✅ **ACHIEVED** Cost calculated correctly per model

#### Implementation Notes

**What was already built (from Example 4):**
1. **Model Abstraction Layer** - `LLMProvider` trait in `worker/src/llm/provider.rs`
2. **Provider Implementations** - Complete implementations for all providers:
   - `AnthropicProvider` (worker/src/llm/anthropic.rs)
   - `OpenAIProvider` (worker/src/llm/openai.rs)
   - `GoogleProvider` (worker/src/llm/google.rs)
   - `OllamaProvider` (worker/src/llm/ollama.rs)
3. **Fallback Chain** - `FallbackChain` struct with automatic provider switching
4. **Budget-Aware Fallback** - Skips expensive models when budget constrained
5. **Provider Tracking** - Returns provider/model metadata in response

**What was created for Example 5:**
1. **Example Workflow** - `examples/05-research-assistant.yaml` demonstrating:
   - Multi-model fallback with three different price points
   - Budget-aware provider selection (o1-pro → Sonnet → Gemini Flash Lite)
   - $0.01 budget demonstrating automatic skipping of expensive models
   - Real-world pricing from `config/llm_models.yaml`
2. **Documentation** - Comprehensive documentation in `examples/README.md` with:
   - Detailed explanation of budget-aware fallback behavior
   - Cost calculations for each model in the chain
   - Expected provider selection based on budget constraints
3. **Model Spec** - `ModelSpec` enum supporting both single model and fallback array syntax

**Files Created/Modified:**
- `examples/05-research-assistant.yaml` - Example workflow with 3-provider fallback chain (updated)
- `examples/README.md` - Added Example 5 documentation with usage instructions (updated)
- `worker/src/activities/llm.rs` - Added `cost_usd` to PromptResponse and activity outputs (updated)
- `docs/implementation/mvp-workflows-implementation-plan.md` - Marked Example 5 complete (updated)

**Features Validated:**
- ✅ `ModelSpec::Fallback` supports array of "provider/model" strings
- ✅ `FallbackChain::prompt()` tries each provider in order
- ✅ **Budget-aware provider selection** - automatically skips expensive models
- ✅ Cost estimation before API call (prevents expensive requests)
- ✅ **Actual cost tracking** - `cost_usd` field in activity output available via template expressions
- ✅ Automatic fallback on provider failure (logged and continues)
- ✅ Provider and model name returned in response
- ✅ Budget enforcement across all providers in chain
- ✅ Cost calculation per provider with model-specific pricing
- ✅ Cost accessible in workflows via `{{activity.result.cost_usd}}`
- ✅ Comprehensive unit tests in `worker/src/activities/llm.rs` (lines 567-899)
- ✅ Real-world pricing demonstration using `config/llm_models.yaml`

**Test Coverage:**
- Unit tests for ModelSpec parsing (single and fallback)
- Unit tests for budget-aware fallback logic
- Unit tests for cost estimation and tracking
- Unit tests for pricing lookup by model key
- Unit tests for zero-budget and cached token scenarios
- **Unit tests for cost_usd in PromptResponse** (validates cost calculation)
- **Unit tests for cost_usd = None when no pricing** (validates optional field)
- **Unit tests for cost_usd in activity output JSON** (validates template expression access)

---

### Example 6: Semantic Caching and RAG with Embeddings
**Duration**: 3-4 days (✅ **COMPLETED** 2025-11-22)
**Epic 3**: US-3.5 (Activity Settings - Caching)
**Epic 5**: US-5.1 (Embedding Generation), US-5.3 (Semantic Caching)
**Status**: ✅ **COMPLETE** - Three-workflow series demonstrating caching, embeddings, and RAG
- ✅ 06a-faq-bot-caching.yaml: Semantic caching fundamentals
- ✅ 06b-rag-index-builder.yaml: Embedding generation and vector indexing
- ✅ 06c-rag-query.yaml: Complete RAG pattern (embed → search → augment → generate)

#### Example Workflow 6a: FAQ Bot with Semantic Caching
```yaml
name: faq_bot
description: Answer FAQs using LLM with aggressive caching for cost savings

activities:
  answer_question:
    activity: llm_prompt
    parameters:
      model: anthropic/haiku-4-5
      messages:
        - role: system
          content: "You are a helpful FAQ assistant. Answer questions concisely."
        - role: user
          content: "{{INPUT.question}}"
      max_tokens: 200
    outputs:
      - response
      - cost_usd
      - cache_hit
    settings:
      cache:
        enabled: true
        ttl_seconds: 3600  # Cache for 1 hour
        key:
          - llm_prompt
          - "{{parameters.model}}"
          - "{{parameters.messages}}"
      budget:
        limit_usd: 0.10

  store_answer:
    activity: postgres_query
    parameters:
      query: |
        INSERT INTO faq_log
        (question, answer, cost, cache_hit, created_at)
        VALUES ($1, $2, $3, $4, NOW())
      params:
        - "{{INPUT.question}}"
        - "{{answer_question.response}}"
        - "{{answer_question.cost_usd}}"
        - "{{answer_question.cache_hit}}"
    depends_on:
      - answer_question
```

#### Example Workflow 6b: RAG Index Builder
```yaml
name: rag_index_builder
description: Build a vector search index by embedding document chunks and storing them in PostgreSQL with pgvector

# This workflow demonstrates:
# - Embedding generation using OpenAI
# - Batch embedding processing
# - PostgreSQL vector storage with pgvector extension
# - Creating a searchable knowledge base

# Prerequisites:
# - PostgreSQL with pgvector extension installed
# - OpenAI API key configured
# - Database table created:
#   CREATE EXTENSION IF NOT EXISTS vector;
#   CREATE TABLE document_chunks (
#     id SERIAL PRIMARY KEY,
#     content TEXT NOT NULL,
#     embedding vector(1536),
#     metadata JSONB,
#     created_at TIMESTAMP DEFAULT NOW()
#   );
#   CREATE INDEX ON document_chunks USING ivfflat (embedding vector_cosine_ops);

activities:
  # Generate embeddings for document chunks
  generate_embeddings:
    activity: embedding
    worker: ai
    parameters:
      provider: openai
      model: text-embedding-3-small
      input: "{{INPUT.chunks}}"  # Array of text chunks
    outputs:
      - embeddings

  # Store chunks with embeddings (simplified 3-chunk example)
  store_chunk_1:
    activity: postgres_query
    worker: builtin
    parameters:
      db_url: "{{SECRET.db_url}}"
      query: |
        INSERT INTO document_chunks (content, embedding, metadata)
        VALUES ($1, $2::vector, $3::jsonb)
        RETURNING id
      params:
        - "{{INPUT.chunks[0]}}"
        - "{{generate_embeddings.embeddings.embeddings[0]}}"
        - '{"source": "{{INPUT.source}}", "chunk_index": 0}'
    depends_on:
      - generate_embeddings

  store_chunk_2:
    activity: postgres_query
    worker: builtin
    parameters:
      db_url: "{{SECRET.db_url}}"
      query: |
        INSERT INTO document_chunks (content, embedding, metadata)
        VALUES ($1, $2::vector, $3::jsonb)
        RETURNING id
      params:
        - "{{INPUT.chunks[1]}}"
        - "{{generate_embeddings.embeddings.embeddings[1]}}"
        - '{"source": "{{INPUT.source}}", "chunk_index": 1}'
    depends_on:
      - generate_embeddings

  store_chunk_3:
    activity: postgres_query
    worker: builtin
    parameters:
      db_url: "{{SECRET.db_url}}"
      query: |
        INSERT INTO document_chunks (content, embedding, metadata)
        VALUES ($1, $2::vector, $3::jsonb)
        RETURNING id
      params:
        - "{{INPUT.chunks[2]}}"
        - "{{generate_embeddings.embeddings.embeddings[2]}}"
        - '{"source": "{{INPUT.source}}", "chunk_index": 2}'
    depends_on:
      - generate_embeddings

  # Confirm indexing complete
  confirm_indexing:
    activity: http_request
    worker: builtin
    parameters:
      method: POST
      url: "{{INPUT.notification_webhook_url}}"
      headers:
        Content-Type: "application/json"
      body:
        status: "indexing_complete"
        workflow_id: "{{WORKFLOW.id}}"
        chunks_indexed: 3
        cost_usd: "{{generate_embeddings.cost_usd}}"
        source: "{{INPUT.source}}"
    depends_on:
      - store_chunk_1
      - store_chunk_2
      - store_chunk_3
```

#### Example Workflow 6c: RAG Query and Q&A
```yaml
name: rag_query
description: Answer questions using RAG (Retrieval-Augmented Generation) with embeddings and LLM

# This workflow demonstrates:
# - Embedding a user question
# - Semantic search using pgvector
# - Passing retrieved context to LLM
# - Classic RAG pattern (embed → search → augment → generate)

# Prerequisites:
# - PostgreSQL with pgvector extension and populated document_chunks table
# - OpenAI API key for embeddings
# - Anthropic API key for LLM

activities:
  # Step 1: Generate embedding for user question
  embed_question:
    activity: embedding
    worker: ai
    parameters:
      provider: openai
      model: text-embedding-3-small
      input:
        - "{{INPUT.question}}"
    outputs:
      - embeddings

  # Step 2: Search for similar chunks using vector similarity
  search_similar_chunks:
    activity: postgres_query
    worker: builtin
    parameters:
      db_url: "{{SECRET.db_url}}"
      query: |
        SELECT content, metadata, (embedding <=> $1::vector) AS distance
        FROM document_chunks
        ORDER BY embedding <=> $1::vector
        LIMIT 3
      params:
        - "{{embed_question.embeddings.embeddings[0]}}"
    outputs:
      - rows
    depends_on:
      - embed_question

  # Step 3: Generate answer using LLM with retrieved context
  generate_answer:
    activity: llm_prompt
    worker: ai
    parameters:
      provider: anthropic
      model: claude-3-5-sonnet-20241022
      messages:
        - role: system
          content: |
            You are a helpful assistant. Answer the user's question using ONLY the context provided.
            If the context doesn't contain relevant information, say so.
        - role: user
          content: |
            Context from knowledge base:
            {% for chunk in search_similar_chunks.rows %}
            ---
            {{ chunk.content }}
            (Source: {{ chunk.metadata.source }}, Distance: {{ chunk.distance }})
            {% endfor %}
            ---

            Question: {{INPUT.question}}

            Please answer the question based on the context above.
      max_tokens: 500
    outputs:
      - response
      - cost_usd
    settings:
      budget:
        limit_usd: 0.50
    depends_on:
      - search_similar_chunks

  # Step 4: Store Q&A result for analytics
  store_qa_result:
    activity: postgres_query
    worker: builtin
    parameters:
      db_url: "{{SECRET.db_url}}"
      query: |
        INSERT INTO qa_log
        (question, answer, chunks_used, embedding_cost, llm_cost, total_cost, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, NOW())
      params:
        - "{{INPUT.question}}"
        - "{{generate_answer.response}}"
        - 3
        - "{{embed_question.cost_usd}}"
        - "{{generate_answer.cost_usd}}"
        - "{{embed_question.cost_usd + generate_answer.cost_usd}}"
    depends_on:
      - generate_answer

  # Step 5: Return answer to user
  send_response:
    activity: http_request
    worker: builtin
    parameters:
      method: POST
      url: "{{INPUT.response_webhook_url}}"
      headers:
        Content-Type: "application/json"
      body:
        question: "{{INPUT.question}}"
        answer: "{{generate_answer.response}}"
        sources_count: 3
        total_cost_usd: "{{embed_question.cost_usd + generate_answer.cost_usd}}"
        workflow_id: "{{WORKFLOW.id}}"
    depends_on:
      - store_qa_result
```

#### YAML Features Implemented
- ✅ Cache settings: `enabled`, `ttl_seconds`, `key`
- ✅ Cache hit indicator in output
- ✅ Array template expressions: Access array elements from activity outputs
- ✅ MiniJinja loops in LLM prompts: `{% for item in array %}`
- ✅ Vector type support in PostgreSQL queries: `$1::vector` casting

#### Built-in Activities Implemented
- ✅ `embedding` - Generate text embeddings using OpenAI
- ✅ Caching layer for LLM activities
- ✅ Redis-backed cache (optional dependency)
- ✅ Cache key generation from parameters
- ✅ PostgreSQL pgvector support for vector similarity search

#### Implementation Tasks

**Embedding Generation (RAG Foundation)**:
1. Implement `embedding` activity (from US-5.1 implementation plan)
   - OpenAI embeddings API integration
   - Batch embedding support (array of inputs)
   - Cost tracking for embedding generation
   - Return embeddings as array of vectors
2. PostgreSQL pgvector setup documentation
   - CREATE EXTENSION vector
   - Vector column type definition
   - Vector similarity search operators (<=>)
   - IVFFlat index creation
3. Vector support in postgres_query activity
   - Accept vector arrays as parameters
   - Type casting support: `$1::vector`
   - JSONB metadata handling

**Semantic Caching**:
4. Caching service trait (abstract cache backend)
5. Redis cache implementation (redis crate)
6. Cache key generation (hash of relevant parameters)
7. Cache lookup before activity execution
8. Cache storage after activity completion
9. TTL expiration (handled by Redis)
10. Graceful degradation when Redis unavailable

**Testing**:
11. End-to-end test: FAQ bot with caching (verify cache hit, cost_usd = 0.0)
12. End-to-end test: RAG indexing workflow (6b-rag-indexing.yaml)
13. End-to-end test: RAG query workflow (6c-rag-query.yaml)
14. Integration test: pgvector similarity search

#### Success Criteria

**Embedding Generation and RAG**:
- ✅ `embedding` activity generates embeddings for text arrays
- ✅ Embeddings stored correctly in PostgreSQL with pgvector
- ✅ Vector similarity search returns relevant chunks
- ✅ RAG indexing workflow completes successfully
- ✅ RAG query workflow retrieves context and generates answers
- ✅ Cost tracking works for embeddings
- ✅ MiniJinja loops work in LLM prompts for context aggregation

**Semantic Caching**:
- ✅ Cache hit returns cached result (cost_usd = 0.0)
- ✅ Cache miss executes activity and stores result
- ✅ TTL expiration works correctly
- ✅ Works with Redis when available
- ✅ Gracefully degrades without Redis (no caching, workflow continues)

**Integration**:
- ✅ All three Example 6 workflows execute end-to-end
- ✅ RAG demonstrates practical use of embeddings
- ✅ Caching demonstrates cost optimization

---

### Example 7: Iterative Workflows with Budget-Aware Loops
**Duration**: 5-6 days (✅ **COMPLETED** 2025-11-22)
**Epic 3**: US-3.4 (Iterative Workflows / Loops)
**Epic 5**: (No new activities - combines existing)
**Status**: ✅ **COMPLETE** - Two-workflow series demonstrating loop fundamentals and production patterns
- ✅ 07a-agentic-research-simple.yaml: Simplified LLM-only loops (educational introduction)
- ✅ 07b-agentic-research-complete.yaml: Full specification (HTTP search, file iteration, dual success/failure paths)

#### Example Workflow: Agentic Research with Iteration
```yaml
name: research_agent
description: Iteratively search and evaluate until sufficient information gathered

activities:
  search_information:
    activity: http_request
    parameters:
      method: POST
      url: "https://api.search.com/query"
      body:
        query: "{{INPUT.research_topic}}"
    outputs:
      - name: results
        type: file  # Store as JSON file to handle large results
        iteration_scoped: true  # Each iteration stores a separate file
    depends_on:
      # Loop: search_information can depend on evaluate_sufficiency
      - evaluate_sufficiency:
          condition: |
            {{search_information.iteration}} < 5 AND
            {{evaluate_sufficiency.sufficient}} == false AND
            {{evaluate_sufficiency.remaining_budget_usd}} > 0.10

  evaluate_sufficiency:
    activity: llm_prompt
    parameters:
      model: anthropic/haiku-4-5
      messages:
        - role: system
          content: |
            You are a research assistant. Determine if the gathered information
            is sufficient to answer the research question. Respond with JSON:
            {"sufficient": true/false, "reason": "explanation", "gaps": ["list", "of", "gaps"]}
        - role: user
          content: |
            Research question: {{INPUT.research_topic}}
            Current iteration: {{search_information.iteration}}

            All information gathered across {{search_information.iteration}} iterations:
            {{search_information[*].results}}

            Current iteration's findings:
            {{search_information.results}}
      max_tokens: 200
    outputs:
      - name: sufficient
      - name: reason
      - name: gaps
    settings:
      budget:
        limit_usd: 0.50
    depends_on:
      - search_information

  compile_report:
    activity: llm_prompt
    parameters:
      model: anthropic/sonnet-4-5
      messages:
        - role: system
          content: "Compile a comprehensive research report from all gathered information."
        - role: user
          content: |
            Topic: {{INPUT.research_topic}}
            Total iterations: {{search_information.iteration}}

            All research findings:
            {{search_information[*].results}}

            Evaluation assessments:
            {{evaluate_sufficiency[*].reason}}
      max_tokens: 1000
    outputs:
      - name: report
        type: file  # Store report as file (could be large)
    settings:
      budget:
        limit_usd: 1.00
    depends_on:
      - evaluate_sufficiency:
          condition: "{{evaluate_sufficiency.sufficient == true}}"

  publish_success:
    activity: http_request
    parameters:
      method: POST
      url: "{{INPUT.publish_url}}"
      files:
        report: "{{FILE.compile_report.report}}"
      body:
        status: "success"
        topic: "{{INPUT.research_topic}}"
        iterations: "{{search_information.iteration}}"
        total_cost_usd: "{{search_information.total_cost_usd}}"
    depends_on:
      - compile_report

  publish_failure:
    activity: http_request
    parameters:
      method: POST
      url: "{{INPUT.publish_url}}"
      body:
        status: "failed"
        topic: "{{INPUT.research_topic}}"
        reason: "Insufficient information after {{search_information.iteration}} iterations"
        iterations: "{{search_information.iteration}}"
        total_cost_usd: "{{search_information.total_cost_usd}}"
        last_assessment: "{{evaluate_sufficiency.reason}}"
        gaps: "{{evaluate_sufficiency.gaps}}"
    depends_on:
      - evaluate_sufficiency:
          condition: |
            "{{evaluate_sufficiency.sufficient}} == false" AND (
              "{{evaluate_sufficiency.remaining_budget_usd}} < 0.10" OR
              "{{search_information.iteration}} >= 5"
            )
```

#### YAML Features Implemented
- ✅ Loop edges (activity depends on itself with condition)
- ✅ Iteration-scoped outputs: `iteration_scoped: true`
- ✅ Access all iteration results: `{{activity_key[*].output_name}}` (array of all iterations)
- ✅ Access current iteration: `{{activity_key.output_name}}` (latest iteration)
- ✅ Activity context variables: `{{activity_key.iteration}}` - iteration count for that activity
- ✅ Budget tracking per activity: `{{activity_key.remaining_budget_usd}}`
- ✅ Maximum iteration limits (prevent infinite loops)
- ✅ Complex boolean expressions with `AND`, `<`
- ✅ File outputs with iteration scoping (large data handling)

#### Built-in Activities Implemented
- (No new activities - combines HTTP + LLM from previous examples)

#### Implementation Tasks
1. Loop detection in workflow graph (edge to earlier activity)
2. **Iteration-scoped storage**:
   - When `iteration_scoped: true`, store separate result per iteration
   - Storage structure: `activity_results[activity_key] = [{iteration: 1, ...}, {iteration: 2, ...}]`
   - For files: File path format `{workflow_id}/{activity_key}/iteration-{N}/{filename}`
   - Framework auto-collects iteration results into array
3. **Template access patterns**:
   - `{{activity_key[*].output_name}}` - Returns array of all iteration results (framework-provided)
   - `{{activity_key.output_name}}` - Returns latest iteration result (framework-provided)
   - `{{activity_key.iteration}}` - Current iteration number for that activity
   - Works for both JSON outputs and file references
4. **Per-activity budget tracking**:
   - Track cumulative cost per activity across iterations
   - Calculate `{{activity_key.remaining_budget_usd}}` based on activity's budget setting
   - Make available in conditions and templates
5. Iteration counter tracking per activity (not global workflow counter)
6. Conditional loop exit evaluation
7. Maximum iteration enforcement (prevent infinite loops)
8. Cycle detection and validation (allow loops, disallow invalid cycles)
9. End-to-end test: Verify loop executes, iteration results accumulate, exits on condition or max iterations

#### Success Criteria
- ✅ Workflow loops back to earlier activity
- ✅ Iteration counter increments correctly
- ✅ Loop exits when condition met
- ✅ Loop exits when max iterations reached
- ✅ Budget-aware loop exit works
- ✅ Workflow doesn't run forever (safety mechanisms work)

---

### Example 8: Activity Scheduling and Delays
**Duration**: 2-3 days (✅ **COMPLETED** 2025-11-23)
**Epic 3**: US-3.7 (Activity Scheduling and Delays)
**Epic 5**: (No new activities - exposes existing infrastructure)
**Status**: ✅ **COMPLETE** - All scheduling features implemented and tested
**Implementation Plan**: See `docs/implementation/US-3.7-activity-scheduling.md` for detailed plan
**Example Workflows**:
- 08a-rate-limited-api-calls.yaml: Rate limiting with delay (supports ms/s/m/mi/h/d/w/mo/y) ✅
- 08b-scheduled-daily-report.yaml: Absolute scheduling with scheduled_for (ISO 8601) ✅
- 08c-delayed-reminders.yaml: Cascading delays for reminder system (1h → 4h → 24h) ✅

#### Example Workflow 8a: Rate Limiting with Delays
Demonstrates using `delay` with flexible duration units to respect API rate limits.

```yaml
name: rate_limited_api_calls
description: Make multiple API calls with delays to respect rate limits

activities:
  - key: call_api_1
    activity_name: http_request
    parameters:
      method: GET
      url: "https://api.example.com/data?page=1"
    outputs:
      - result

  - key: call_api_2
    activity_name: http_request
    parameters:
      method: GET
      url: "https://api.example.com/data?page=2"
    outputs:
      - result
    settings:
      delay: "5s"  # Wait 5 seconds (rate limit: 1 req/5sec)
    depends_on:
      - call_api_1

  - key: call_api_3
    activity_name: http_request
    parameters:
      method: GET
      url: "https://api.example.com/data?page=3"
    outputs:
      - result
    settings:
      delay: "5s"  # Wait another 5 seconds
    depends_on:
      - call_api_2
```

**Use Case**: Sequential API calls with mandatory delays to respect rate limits.
**Duration Units**: Supports s/m/h/d/w/y (e.g., `"30m"`, `"2h"`, `"7d"`).

#### Example Workflow 8b: Scheduled Report Generation
Demonstrates using `scheduled_for` with absolute timestamps for scheduled reports.

```yaml
name: scheduled_daily_report
description: Generate and send daily report at specific time

activities:
  - key: generate_report
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-sonnet-4-5-20250929
      prompt: "Generate a daily summary report for {{INPUT.report_date}}..."
      max_tokens: 2000
    outputs:
      - result
    settings:
      # Schedule for specific time (e.g., 9:00 AM Pacific)
      # Input format: "2025-12-01T09:00:00-08:00"
      scheduled_for: "{{INPUT.report_time}}"
      budget:
        limit_usd: 0.10
        action: abort

  - key: send_report
    activity_name: http_request
    parameters:
      method: POST
      url: "{{INPUT.notification_webhook}}"
      body:
        subject: "Daily Report - {{INPUT.report_date}}"
        content: "{{generate_report.result.content}}"
    depends_on:
      - generate_report
```

**Use Case**: Scheduled reports at specific times using dynamic timestamps from workflow input.

#### Example Workflow 8c: Cascading Delayed Reminders
Demonstrates combining delays with conditionals for reminder workflows.

```yaml
name: delayed_reminder_system
description: Send escalating reminders after delays

activities:
  - key: send_initial_notification
    activity_name: http_request
    parameters:
      method: POST
      url: "{{INPUT.user_webhook}}"
      body:
        message: "Task assigned: {{INPUT.task_name}}"
        priority: "normal"

  - key: send_first_reminder
    activity_name: http_request
    parameters:
      method: POST
      url: "{{INPUT.user_webhook}}"
      body:
        message: "Reminder: {{INPUT.task_name}} is still pending"
    settings:
      delay: "1h"  # Wait 1 hour
    depends_on:
      - send_initial_notification

  - key: send_escalated_reminder
    activity_name: http_request
    parameters:
      method: POST
      url: "{{INPUT.manager_webhook}}"
      body:
        message: "ESCALATED: {{INPUT.task_name}} pending for 4 hours"
        priority: "high"
    settings:
      delay: "3h"  # Wait 3 more hours (4 hours total)
    depends_on:
      - send_first_reminder

  - key: send_final_escalation
    activity_name: http_request
    parameters:
      method: POST
      url: "{{INPUT.oncall_webhook}}"
      body:
        message: "CRITICAL: {{INPUT.task_name}} pending for 24 hours"
        priority: "critical"
    settings:
      delay: "20h"  # Wait 20 more hours (24 hours total)
    depends_on:
      - send_escalated_reminder
```

**Use Case**: Escalating reminder system with increasing urgency over time.

#### YAML Features Implemented

**New Features (US-3.7)**:
- ✅ `settings.delay` - Delay activity execution with flexible units (ms/s/m/mi/h/d/w/mo/y)
- ✅ `settings.scheduled_for` - Schedule activity for absolute timestamp (ISO 8601)
- ✅ Template support in both `scheduled_for` and `delay` - Dynamic scheduling from workflow inputs

**Existing Features Used**:
- Sequential dependencies (`depends_on`)
- Conditional execution
- Template expressions (`{{INPUT.*}}`)
- HTTP request activity

#### Built-in Activities Implemented

**No New Activities**:
- Uses existing `http_request` activity
- Scheduling is a YAML feature, not an activity feature

#### Implementation Tasks

**1. Update ActivitySettings Model**
- Add `delay: Option<String>` field (duration string: "500ms", "5s", "30m", "2h", "7d", "1w", "2mo", "1y")
- Add `scheduled_for: Option<String>` field (ISO 8601 timestamp)
- Validation: `scheduled_for` must be valid ISO 8601 or template expression
- Validation: Cannot specify both `delay` and `scheduled_for`
- Duration parsing: Regex `^(\d+)(ms|s|m|mi|h|d|w|mo|y)$` with unit conversion

**2. Update Orchestrator Activity Scheduling**
- When scheduling activity, check `settings.delay`
- If present, parse duration string and calculate `scheduled_for = NOW() + delay`
- If `scheduled_for` present, parse timestamp and use as `scheduled_for`
- Template evaluation: Resolve `{{INPUT.*}}` expressions before parsing timestamp or duration

**3. Database Changes**
- ✅ **No changes needed** - `activity_queue.scheduled_for` already exists
- ✅ Worker polling already filters by `scheduled_for <= NOW()`

**4. Testing**
- Unit tests for ActivitySettings validation
- Integration test: Verify delayed activity doesn't execute early
- Integration test: Verify scheduled activity executes at correct time
- Integration test: Verify template resolution in `scheduled_for`
- End-to-end test with example workflow

**5. Documentation**
- Update workflow definition language docs
- Add scheduling examples to documentation
- Document use cases: rate limiting, delayed retries, scheduled reports

#### Success Criteria

**Functional**:
- ✅ Activities can be delayed with flexible duration units (`delay`: "500ms", "5s", "30m", "2h", "7d", "1w", "2mo", "1y")
- ✅ Both `m` and `mi` accepted for minutes (user preference for clarity)
- ✅ Activities can be scheduled for absolute time (`scheduled_for`: ISO 8601 with timezone)
- ✅ Templates work in `scheduled_for` parameter (e.g., `"{{INPUT.report_time}}"`)
- ✅ Templates work in `delay` parameter (e.g., `"{{INPUT.delay_minutes}}m"`)
- ✅ Workers don't claim activities before `scheduled_for` time
- ✅ Validation prevents both `delay` and `scheduled_for` together
- ✅ Example workflows run successfully:
  - `examples/08a-rate-limited-api-calls.yaml`
  - `examples/08b-scheduled-daily-report.yaml`
  - `examples/08c-delayed-reminders.yaml`

**Non-Functional**:
- ✅ No performance degradation (existing index supports scheduled queries)
- ✅ Scheduled activities don't consume worker resources while waiting
- ✅ Time precision: Activities execute within 1 second of scheduled time (polling interval)

#### Implementation Notes

**Why This is Simple**:
- Database infrastructure already exists (`activity_queue.scheduled_for`)
- Workers already filter by `scheduled_for <= NOW()`
- Just need to expose in YAML and wire up in orchestrator

**Design Decisions**:
1. **Relative vs Absolute Time**:
   - `delay`: Relative to when activity becomes ready, supports ms/s/m/mi/h/d/w/mo/y units (simple, common case)
   - `scheduled_for`: Absolute ISO 8601 timestamp with timezone (for scheduled reports, deadlines)
2. **Mutually Exclusive**: Cannot specify both (validation error at workflow parse time)
3. **Template Support**: Both `delay` and `scheduled_for` can use templates for dynamic scheduling
4. **Calendar Arithmetic**: Months and years use chrono's calendar-aware arithmetic (handles variable month lengths, leap years)
5. **No Event Suspension (Yet)**: This example only covers time-based delays, not external events
6. **Field Naming**: `scheduled_for` matches database column name and is semantically clear

**Post-MVP Enhancement**:
- Event-driven suspension (`wait_for_event` activity) - see absurd analysis Section 2, Phase 2

---

### 🎯 PRIORITY: Token Streaming Implementation (Pre-Launch)

**Strategic Decision**: Before continuing with Examples 9-10, implement US-1A.9a (WebSocket Infrastructure) + US-7.1 (Token Streaming) to deliver on core AI-native value proposition.

**Rationale**: Token streaming is explicitly promised in the Executive Summary as a key differentiator. Delivering this before public launch (Option 1) is strategically justified because:
1. **Core Value Proposition**: Required for production AI workflows with user-facing UX
2. **Unique Differentiator**: No competitor (Temporal, Airflow, Conductor) offers this
3. **AI-Native Positioning**: Validates the "AI-native" claim with concrete capability
4. **User Expectation**: AI startup engineers (primary persona) expect streaming for ChatGPT-style UX
5. **Contained Scope**: 1-1.5 weeks implementation time (US-1A.9a ~15h + US-7.1 ~20-30h)

**Total Duration**: ~35-45 hours (5-7 days)

### US-1A.9a: WebSocket Infrastructure for Token Streaming
**Duration**: ~15 hours (2 days)
**Epic**: Epic 1A (API Server)
**Status**: ✅ **COMPLETE**

#### Overview
Build WebSocket infrastructure to support real-time token streaming from LLM activities. This provides the foundation for US-7.1 token streaming.

#### Acceptance Criteria
- ✅ WebSocket endpoint: `WS /api/v1/activities/{id}/ws`
- ✅ Authentication: Bearer token in query parameter or initial message
- ✅ Connection management: Handle 1,000+ concurrent connections
- ✅ Message format: JSON messages over WebSocket (`{type: "token", ...}`)
- ✅ Backpressure handling to prevent buffer overflow
- ✅ Graceful connection close on activity completion
- ✅ Error handling: Stream errors as messages before closing

#### Implementation Tasks
1. **WebSocket Handler** (4-5 hours)
   - Axum WebSocket route handler
   - Connection upgrade from HTTP
   - Authentication via Bearer token
   - Connection state management

2. **Connection Manager** (3-4 hours)
   - Maintain map of active connections per activity_id
   - Handle concurrent connections to same activity
   - Graceful connection close
   - Cleanup on activity completion

3. **Message Protocol** (2-3 hours)
   - Define message types: `token`, `complete`, `error`
   - JSON serialization/deserialization
   - Message ordering guarantees

4. **Integration with Activity Execution** (3-4 hours)
   - Hook into activity execution flow
   - Publish streaming events to connected WebSocket clients
   - Handle non-streaming activities (graceful fallback)

5. **Testing** (3-4 hours)
   - Unit tests for WebSocket handler
   - Integration tests with mock streaming activity
   - Load test: 1,000 concurrent connections

#### Implementation Plan
See `docs/implementation/US-1A.9a-websocket-infrastructure.md` for detailed implementation plan.

---

### US-7.1: Token Streaming for Real-Time UX
**Duration**: ~20-30 hours (3-4 days)
**Epic**: Epic 7 (AI-Native Features)
**Status**: ✅ **COMPLETE**
**Dependencies**: US-1A.9a (Complete)

#### Overview
Implement token-by-token streaming from LLM activities (Anthropic, OpenAI, Google) to WebSocket clients, enabling ChatGPT-style real-time UX in workflows.

#### Acceptance Criteria
- ✅ LLM providers support streaming (Anthropic SSE, OpenAI SSE, Google SSE)
- ✅ Activity-level streaming events published to WebSocket subscribers
- ✅ Token-by-token delivery: `{type: "token", text: "hello", index: 0}`
- ✅ <10ms P95 token latency (achievable with async streaming)
- ✅ Support 1,000 concurrent streaming connections
- ✅ Graceful fallback: Non-streaming activities complete normally
- ✅ Integration with Example 6 (agentic research) for demonstration
- ✅ Client library examples for JavaScript/Python

#### Implementation Tasks
1. **LLM Provider Streaming Integration** (8-10 hours)
   - Anthropic streaming API integration (SSE)
   - OpenAI streaming API integration (SSE)
   - Google streaming API integration (SSE)
   - Handle streaming responses asynchronously

2. **Activity Streaming Layer** (6-8 hours)
   - Hook LLM streaming into activity execution
   - Publish tokens to WebSocket connections via US-1A.9a infrastructure
   - Handle streaming errors and reconnection
   - Accumulate full response for activity output

3. **Non-Streaming Fallback** (2-3 hours)
   - Detect non-streaming activities
   - Complete normally without WebSocket streaming
   - Log when streaming not available

4. **Example Integration** (3-4 hours)
   - Update Example 6 (agentic research) to demonstrate streaming
   - Add streaming example to `examples/` directory
   - Document streaming usage in examples/README.md

5. **Client Libraries** (4-6 hours)
   - JavaScript WebSocket client example
   - Python WebSocket client example
   - Documentation for client integration

6. **Testing** (5-6 hours)
   - Unit tests for streaming integration
   - Integration test: End-to-end workflow with streaming
   - Load test: 1,000 concurrent streaming connections
   - Test with all three LLM providers

#### Implementation Plan
See `docs/implementation/US-7.1-token-streaming.md` for detailed implementation plan.

---

### Example 9: Token Streaming ✅ **COMPLETE**
**Duration**: Included in US-1A.9a + US-7.1 above
**Epic 3**: US-3.8 (Streaming Activity Flag)
**Epic 7**: US-7.1 (Token Streaming)
**Status**: ✅ **COMPLETE** - Two-workflow series (09a, 09b) implemented

#### Example 9a: Basic LLM Streaming (`examples/09a-streaming-llm.yaml`)

Demonstrates single-activity streaming with provider fallback:

```yaml
name: streaming_llm_example
description: LLM prompt with token streaming over WebSocket

activities:
  - key: generate_story
    worker: builtin
    activity_name: llm_prompt
    streaming: true  # Enable token streaming
    parameters:
      model:
        - anthropic/claude-3-5-haiku-20241022
        - openai/gpt-4o-mini
        - google/gemini-1.5-flash
      prompt: |
        Write a short story about {{INPUT.topic}} in approximately 200 words.
      max_tokens: 500
      temperature: 0.8
    outputs:
      - result
```

**Key Features**:
- `streaming: true` enables token-by-token delivery via WebSocket
- Provider fallback chain (all three providers support streaming)
- Two-level opt-in: activity config + WebSocket subscribers present

**Client Connection Flow**:
1. Submit workflow via `POST /api/v1/workflows`
2. Get activity_id from workflow status (`GET /api/v1/workflows/{id}`)
3. Connect to WebSocket: `ws://host/api/v1/activities/{activity_id}/ws?token=<jwt>`
4. Receive `StreamMessage` events: `token`, `complete`, `error`

#### Example 9b: Selective Streaming (`examples/09b-streaming-research.yaml`)

Demonstrates multi-step workflow with streaming only on the final output:

```yaml
name: streaming_research_workflow
description: Research assistant with streaming analysis output

# Workflow DAG:
#   summarize_topic ──┬──► analyze_findings (STREAMING)
#   gather_sources ───┘

activities:
  # Non-streaming preparation steps (fast, efficient)
  - key: summarize_topic
    activity_name: llm_prompt
    streaming: false  # Non-streaming for speed
    parameters:
      model: anthropic/claude-3-5-haiku-20241022
      prompt: Provide a brief overview of: {{INPUT.topic}}
      max_tokens: 200

  - key: gather_sources
    activity_name: llm_prompt
    streaming: false
    parameters:
      model: anthropic/claude-3-5-haiku-20241022
      prompt: Generate 3 research questions for: {{INPUT.topic}}
      max_tokens: 300

  # Streaming final analysis (user sees tokens as generated)
  - key: analyze_findings
    activity_name: llm_prompt
    streaming: true  # Enable streaming for real-time output
    parameters:
      model:
        - anthropic/claude-3-5-sonnet-20241022
        - anthropic/claude-3-5-haiku-20241022
      prompt: |
        Write a comprehensive research analysis on: {{INPUT.topic}}
        Context: {{summarize_topic.result.content}}
        Questions: {{gather_sources.result.content}}
      max_tokens: 1500
    depends_on:
      - summarize_topic
      - gather_sources
    settings:
      budget:
        limit_usd: 0.10
        action: warn
```

**Key Features**:
- Selective streaming: only the user-facing analysis step streams
- Non-streaming prep steps run efficiently without WebSocket overhead
- Cost budget enforcement on expensive streaming activity
- DAG dependencies ensure context is available before analysis

#### Streaming Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        API Server                                │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │              ConnectionManager                           │    │
│  │  ┌─────────────────────────────────────────────────┐    │    │
│  │  │ activity_id → [WebSocket connections...]        │    │    │
│  │  └─────────────────────────────────────────────────┘    │    │
│  └─────────────────────────────────────────────────────────┘    │
│         ▲                              │                         │
│         │ register/broadcast           │ tokens                  │
│         │                              ▼                         │
│  ┌──────┴──────────────────────────────────────────────────┐    │
│  │ WS /api/v1/activities/{id}/ws                       │    │
│  └─────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────┘
         ▲                              │
         │ connect                      │ StreamMessage
         │                              ▼
    ┌────┴────┐                    ┌─────────┐
    │ Client  │◄───────────────────│ Worker  │
    │ (WS)    │   token/complete   │ (LLM)   │
    └─────────┘                    └─────────┘
```

**Message Types**:
- `{"type": "token", "text": "Hello", "index": 0, "timestamp": "..."}`
- `{"type": "complete", "activity_id": "...", "result": {...}, "timestamp": "..."}`
- `{"type": "error", "activity_id": "...", "error": "...", "timestamp": "..."}`

#### Success Criteria
- ✅ `streaming: true/false` activity flag controls streaming behavior
- ✅ Worker checks for subscribers before streaming (two-level opt-in)
- ✅ Tokens delivered via WebSocket with <10ms P95 latency
- ✅ Non-streaming activities complete efficiently without overhead
- ✅ Provider fallback works with streaming enabled
- ✅ Cost tracking works for streaming activities

---

### Example 10: Order Processing with Email Notification ✅ **COMPLETE**
**Duration**: 3-4 days (✅ **COMPLETED** 2025-11-27)
**Epic 3**: (No new YAML features)
**Epic 5**: US-5.5 (HTTP ✅), US-5.6 (Database ✅), US-5.7a (email_send ✅)
**Status**: ✅ **COMPLETE** - Order processing workflow demonstrating HTTP, postgres_transaction, and email_send

**Note**: This example completes the MVP. The orchestrator and built-in worker with all core activities are now feature-complete.

#### Example Workflow: Order Processing with Email Confirmation
```yaml
name: order_processing
description: Process order with API calls, database transaction, and email confirmation

activities:
  validate_inventory:
    activity: http_request
    parameters:
      method: GET
      url: "https://api.inventory.com/check"
      query:
        product_id: "{{INPUT.product_id}}"
        quantity: "{{INPUT.quantity}}"
      headers:
        Authorization: "Bearer {{SECRET.inventory_api_key}}"
      timeout_seconds: 10
    outputs:
      - available
    settings:
      retry:
        max_attempts: 3
        backoff: exponential

  reserve_inventory:
    activity: http_request
    parameters:
      method: POST
      url: "https://api.inventory.com/reserve"
      body:
        product_id: "{{INPUT.product_id}}"
        quantity: "{{INPUT.quantity}}"
      headers:
        Authorization: "Bearer {{SECRET.inventory_api_key}}"
    outputs:
      - reservation_id
    depends_on:
      - validate_inventory:
          condition: "{{validate_inventory.available == true}}"

  process_payment:
    activity: http_request
    parameters:
      method: POST
      url: "https://api.payment.com/charge"
      body:
        amount: "{{INPUT.amount}}"
        customer_id: "{{INPUT.customer_id}}"
      headers:
        Authorization: "Bearer {{SECRET.payment_api_key}}"
      timeout_seconds: 30
    outputs:
      - transaction_id
    settings:
      retry:
        max_attempts: 2
    depends_on:
      - reserve_inventory

  record_order:
    activity: postgres_transaction
    parameters:
      db_url: "{{SECRET.db_url}}"
      statements:
        - query: |
            INSERT INTO orders
            (customer_id, product_id, quantity, amount, payment_txn_id, created_at)
            VALUES ($1, $2, $3, $4, $5, NOW())
            RETURNING id as order_id
          params:
            - "{{INPUT.customer_id}}"
            - "{{INPUT.product_id}}"
            - "{{INPUT.quantity}}"
            - "{{INPUT.amount}}"
            - "{{process_payment.transaction_id}}"
        - query: |
            UPDATE inventory
            SET reserved = reserved + $1
            WHERE product_id = $2
          params:
            - "{{INPUT.quantity}}"
            - "{{INPUT.product_id}}"
    outputs:
      - order_id
    depends_on:
      - process_payment

  send_confirmation:
    activity: email_send
    parameters:
      smtp_url: "{{SECRET.smtp_url}}"
      from: "orders@example.com"
      to:
        - "{{INPUT.customer_email}}"
      subject: "Order Confirmation - #{{record_order.result.results[0].rows[0].order_id}}"
      content_type: "text/html"
      body: |
        <html>
        <body style="font-family: Arial, sans-serif; max-width: 600px; margin: 0 auto;">
          <h2 style="color: #2e7d32;">Order Confirmed!</h2>
          <p>Thank you for your order. Here are your order details:</p>

          <table style="width: 100%; border-collapse: collapse; margin: 20px 0;">
            <tr style="background-color: #f5f5f5;">
              <td style="padding: 10px; border: 1px solid #ddd;"><strong>Order ID</strong></td>
              <td style="padding: 10px; border: 1px solid #ddd;">#{{record_order.result.results[0].rows[0].order_id}}</td>
            </tr>
            <tr>
              <td style="padding: 10px; border: 1px solid #ddd;"><strong>Product</strong></td>
              <td style="padding: 10px; border: 1px solid #ddd;">{{INPUT.product_id}}</td>
            </tr>
            <tr style="background-color: #f5f5f5;">
              <td style="padding: 10px; border: 1px solid #ddd;"><strong>Quantity</strong></td>
              <td style="padding: 10px; border: 1px solid #ddd;">{{INPUT.quantity}}</td>
            </tr>
            <tr>
              <td style="padding: 10px; border: 1px solid #ddd;"><strong>Amount</strong></td>
              <td style="padding: 10px; border: 1px solid #ddd;">${{INPUT.amount}}</td>
            </tr>
            <tr style="background-color: #f5f5f5;">
              <td style="padding: 10px; border: 1px solid #ddd;"><strong>Payment ID</strong></td>
              <td style="padding: 10px; border: 1px solid #ddd;">{{process_payment.transaction_id}}</td>
            </tr>
          </table>

          <p>If you have any questions, please contact our support team.</p>

          <p style="color: #666; font-size: 12px; margin-top: 30px;">
            This is an automated message from StreamFlow Order Processing.
          </p>
        </body>
        </html>
    depends_on:
      - record_order
```

#### Built-in Activities Demonstrated
- ✅ `http_request` - HTTP requests with auth headers, query params, timeouts
- ✅ `postgres_transaction` - Multi-statement atomic transaction with RETURNING
- ✅ `email_send` - SMTP email with HTML/text content (US-5.7a)

#### Implementation Tasks

**Phase 1: US-5.7a - email_send Activity** (Dependency)
1. Add `lettre` crate to worker/Cargo.toml
2. Create `worker/src/activities/email.rs` with:
   - `SmtpConfig::from_url()` - Parse SMTP connection URL
   - `RateLimiter` - Per-domain rate limiting
   - `EmailExecutor` - SMTP client wrapper
   - `EmailSendActivity` - ActivityImpl implementation
3. Register `email_send` in builtin activities
4. Unit tests for SMTP URL parsing, rate limiter
5. Integration test with mailhog

**Phase 2: Example 10 Workflow**
1. Create `examples/10-order-processing.yaml`
2. Integration test with mock HTTP endpoints and test database
3. Verify email delivery via mailhog in test environment

#### Success Criteria
- [x] `email_send` activity sends HTML/plain text emails via SMTP
- [ ] Rate limiting prevents spam (configurable per-domain limits) - deferred to post-MVP
- [x] Retry behavior correctly classifies transient vs permanent errors
- [x] Example 10 workflow executes end-to-end
- [x] Order confirmation email received with correct order details

#### Implementation Notes (Completed 2025-11-27)
- Created `examples/10-order-processing.yaml` demonstrating:
  - HTTP requests with authorization headers (mock endpoints)
  - `postgres_transaction` with multiple statements and RETURNING clause
  - `email_send` with HTML body via Mailhog SMTP
  - Sequential dependency chain for order processing
- Created `api/tests/example_10_e2e_test.rs` with:
  - Full E2E test using mock HTTP endpoints and test database
  - Verification of database state (orders and inventory tables)
  - Email delivery verification via Mailhog API
- Fixed bug in `postgres_transaction` RETURNING clause detection:
  - Multiline queries with RETURNING on separate line were not detected
  - Added `has_returning_clause()` helper to handle newlines before RETURNING

#### MVP Completion Note
After Example 10 is complete, the following MVP components are feature-complete:
- ✅ Event-driven orchestrator with dependency evaluation
- ✅ Built-in worker with activity execution
- ✅ Complete built-in activity library:
  - `echo` (testing)
  - `http_request` (HTTP operations)
  - `postgres_query` (database queries)
  - `postgres_transaction` (atomic transactions)
  - `llm_prompt` (AI/LLM integration)
  - `embedding` (vector embeddings)
  - `email_send` (notifications)
- ✅ YAML workflow definition language with all planned features
- ✅ Token streaming for LLM activities
- ✅ Semantic caching for AI workloads

---

### Example 11: Advanced File Management Features
**Duration**: 3-4 days
**Epic 3**: (No new YAML features)
**Epic 5**: US-5.4 (Object Storage and File Management - Complete)

#### Example Workflow: ETL Pipeline with File Lifecycle Management
```yaml
name: data_pipeline
description: ETL pipeline demonstrating advanced file management with external storage integration

activities:
  # Fetch raw data from external S3 bucket (not workflow storage)
  fetch_raw_data:
    activity: s3_get
    parameters:
      bucket: "{{INPUT.source_bucket}}"
      key: "raw/data-{{INPUT.date}}.csv"
      region: "us-east-1"
    outputs:
      - name: raw_data
        type: file  # Downloaded and stored in workflow storage

  # Transform the data (reads from workflow storage, writes to workflow storage)
  transform_data:
    activity: python_script
    parameters:
      script: |
        import pandas as pd
        # File path provided by framework
        df = pd.read_csv(input_files['raw_data'])
        df_transformed = df.transform(...)
        df_transformed.to_parquet(output_files['transformed_data'])
      files:
        raw_data: "{{FILE.fetch_raw_data.raw_data}}"
    outputs:
      - name: transformed_data
        type: file

  validate_output:
    activity: python_script
    parameters:
      script: |
        import pandas as pd
        df = pd.read_parquet(input_files['data'])
        assert len(df) > 0
        # Return validation metadata as JSON
        return {"row_count": len(df), "valid": True}
      files:
        data: "{{FILE.transform_data.transformed_data}}"
    outputs:
      - name: validation_result  # JSON output, not a file
    depends_on:
      - transform_data

  # Upload result to external destination S3 bucket
  upload_result:
    activity: s3_put
    parameters:
      bucket: "{{INPUT.dest_bucket}}"
      key: "processed/data-{{INPUT.date}}.parquet"
      region: "us-east-1"
      file: "{{FILE.transform_data.transformed_data}}"
      metadata:
        source: "streamflow-etl"
        row_count: "{{validate_output.validation_result.row_count}}"
    depends_on:
      - validate_output

  # Delete source file from external S3 (not workflow storage)
  cleanup_source:
    activity: s3_delete
    parameters:
      bucket: "{{INPUT.source_bucket}}"
      key: "raw/data-{{INPUT.date}}.csv"
      region: "us-east-1"
    depends_on:
      - upload_result
```

#### YAML Features Implemented
- ✅ `{{FILE.activity_key.output_name}}` - Reference files from previous activities
- ✅ `{{FOLDER.activity_key.folder_name}}` - Reference folders (when needed)
- ✅ Mixed file and JSON outputs in same workflow

#### Built-in Activities Implemented
- ✅ `s3_get` - Fetch file from external S3 bucket into workflow storage
- ✅ `s3_put` - Upload file from workflow storage to external S3 bucket
- ✅ `s3_delete` - Delete file from external S3 bucket
- ✅ `s3_list` - List files in external S3 bucket (for dynamic workflows)
- ✅ `python_script` - Execute Python with file inputs/outputs
- ✅ Multi-cloud support (GCS, Azure Blob, MinIO)

#### Implementation Tasks
1. **External storage integration**:
   - S3 operations (get, put, delete, list) for external buckets
   - GCS, Azure Blob, MinIO provider implementations
   - Authentication per provider
2. **Python activity with file support**:
   - Provide `input_files` dict with local paths
   - Provide `output_files` dict for writing results
   - Automatic upload of file outputs
3. **File lifecycle management**:
   - Automatic cleanup of workflow files based on retention policy
   - Metadata tagging: workflow_id, activity_key, timestamp
   - Storage backend configuration (local, S3, GCS, etc.)
4. **Advanced features**:
   - Signed URL generation for time-limited access
   - Large file streaming (no full memory load)
   - Compression support (gzip, zstd)
5. End-to-end test: ETL pipeline with external S3 integration, verify cleanup

#### Success Criteria
- ✅ Files pass between activities without JSON serialization
- ✅ External S3 operations (get, put, delete, list) work correctly
- ✅ Python activities can read/write files
- ✅ Multi-cloud storage providers supported
- ✅ Workflow files automatically cleaned up after retention period
- ✅ Large files (>100MB) handled efficiently

---

### Infrastructure: Database Cleanup Worker (US-3.8)
**Duration**: 2-3 days
**Epic 3**: US-3.8 (Database Cleanup with TTL)
**Purpose**: Prevent unbounded database growth by cleaning up completed workflows

#### Overview

Automatic cleanup of completed workflows after configurable retention period to prevent unbounded database growth. **Critical**: `workflow_events` table is NEVER cleaned up (audit trail requirement).

#### Implementation Tasks

**1. Stored Procedures**

Create cleanup stored procedures:

```sql
-- Cleanup completed workflows (NOT workflow_events)
CREATE OR REPLACE FUNCTION cleanup_workflows(
    p_ttl_days INTEGER DEFAULT 7,
    p_batch_size INTEGER DEFAULT 1000
) RETURNS INTEGER AS $$
DECLARE
    v_deleted INTEGER := 0;
    v_workflow_ids UUID[];
BEGIN
    -- Find workflows to delete
    SELECT ARRAY_AGG(id) INTO v_workflow_ids
    FROM workflows
    WHERE status IN ('Completed', 'Failed', 'Cancelled')
      AND updated_at < NOW() - (p_ttl_days || ' days')::INTERVAL
    LIMIT p_batch_size;

    IF v_workflow_ids IS NULL THEN
        RETURN 0;
    END IF;

    -- Delete in dependency order
    DELETE FROM activity_queue WHERE workflow_id = ANY(v_workflow_ids);
    -- NOTE: workflow_events NOT deleted - kept forever for audit trail
    DELETE FROM workflows WHERE id = ANY(v_workflow_ids);

    GET DIAGNOSTICS v_deleted = ROW_COUNT;
    RETURN v_deleted;
END;
$$ LANGUAGE plpgsql;
```

**2. Background Cleanup Worker**

Implement tokio background task:

```rust
pub struct CleanupWorker {
    pool: PgPool,
    config: CleanupConfig,
}

pub struct CleanupConfig {
    pub enabled: bool,
    pub workflow_ttl_days: i32,
    pub interval_hours: u64,
    pub batch_size: i32,
}

impl CleanupWorker {
    pub async fn run(self) {
        let mut interval = tokio::time::interval(
            Duration::from_secs(self.config.interval_hours * 3600)
        );

        loop {
            interval.tick().await;

            if !self.config.enabled {
                continue;
            }

            let deleted = sqlx::query_scalar!(
                "SELECT cleanup_workflows($1, $2)",
                self.config.workflow_ttl_days,
                self.config.batch_size
            )
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);

            if deleted > 0 {
                info!("Cleaned up {} workflows", deleted);
            }
        }
    }
}
```

**3. Configuration (Environment Variables)**

```bash
STREAMFLOW_CLEANUP_ENABLED=true
STREAMFLOW_CLEANUP_WORKFLOW_TTL_DAYS=7
STREAMFLOW_CLEANUP_INTERVAL_HOURS=1
STREAMFLOW_CLEANUP_BATCH_SIZE=1000
```

**4. Integration with `streamflow serve`**

Launch cleanup worker alongside orchestrator and API server:

```rust
tokio::spawn(cleanup_worker.run());
```

**5. Observability**

- Log cleanup operations with deleted count
- Metrics: `streamflow_cleanup_workflows_deleted_total`
- Metrics: `streamflow_cleanup_last_run_timestamp_seconds`

#### Success Criteria

**Functional**:
- ✅ Completed workflows deleted after TTL expires
- ✅ `workflow_events` table NEVER cleaned up (audit requirement)
- ✅ Batch deletion prevents long transactions
- ✅ Configurable via environment variables
- ✅ Can be disabled for testing/development

**Non-Functional**:
- ✅ Cleanup runs in background without blocking orchestrator
- ✅ Handles large batches efficiently (< 1 second per 1000 workflows)
- ✅ No race conditions with running workflows

#### Design Decisions

**Tables Cleanup Strategy**:
| Table             | Cleanup Strategy                          | Retention    |
|-------------------|-------------------------------------------|--------------|
| workflows         | ✅ Delete after TTL (default: 7 days)     | Configurable |
| activity_queue    | ✅ Delete after completion/TTL            | Configurable |
| workflow_events   | ❌ **NEVER delete** (audit trail)         | Forever      |

**Why workflow_events is Never Deleted**:
- Audit trail and compliance requirement
- Provides complete history of all workflow executions
- Post-MVP: Use table partitioning for query performance (see post-mvp.md Story 2.3)

**Source**: Absurd analysis Section 5 - Cleanup Strategy with TTL

---

## Post-MVP Examples (Optional Enhancements)

### Example 12: Notification Activities
**Duration**: 2-3 days
**Epic 5**: US-5.7 (Notification Activities)

#### Activities
- `slack_send` - Send Slack notification
- `email_send` - Send email via SMTP
- `discord_send` - Discord webhook
- `teams_send` - Microsoft Teams notification

### Example 13: Edge/IoT Activities (Unique Differentiator)
**Duration**: 4-5 days
**Epic 5**: US-5.8 (Edge/IoT Activities)

#### Activities
- `gpio_read` / `gpio_write` - Raspberry Pi GPIO
- `i2c_communicate` - I2C device communication
- `camera_capture` - Capture image from camera
- `gps_location` - Get GPS coordinates

---

## Implementation Schedule

### Phase Overview - UPDATED for Option 1 (Token Streaming Pre-Launch)
- **Examples 1-6**: ✅ **COMPLETE** (~22-28 days) - Core workflow features + LLM + loops
- **🎯 Token Streaming**: 📋 **NEXT PRIORITY** (5-6 days) - US-1A.9a + US-7.1 before launch
  - US-1A.9a: WebSocket Infrastructure (2 days)
  - US-7.1: LLM Token Streaming (3-4 days)
- **Examples 7-10**: Advanced features (8-11 days)
- **US-3.6**: CLI Tooling (4-5 days, can run in parallel)
- **Total MVP with Token Streaming**: 35-45 days (7-9 weeks)

### Detailed Schedule - REVISED

| Milestone              | Duration   | Features                                                        | Epic                        | Cumulative Days |
|-----------------------|------------|------------------------------------------------------------------|----------------------------|-----------------|
| 1                     | 3-4 days   | Sequential workflows, basic templates, HTTP GET/POST            | Epic 3, Epic 5             | 3-4             |
| 2                     | 3-4 days   | Conditional branching, secrets, PostgreSQL                      | Epic 3, Epic 5             | 6-8             |
| 3                     | 4-5 days   | Parallel execution, file management                             | Epic 3, Epic 5             | 10-13           |
| 4                     | 5-6 days   | Activity settings, LLM (Anthropic), cost tracking               | Epic 3, Epic 5             | 15-19           |
| 5                     | 4-5 days   | Model fallback, LLM (OpenAI, Gemini)                           | Epic 5                     | 19-24           |
| 6                     | 3-4 days   | Iterative workflows (loops), semantic caching                   | Epic 3 (US-3.4), Epic 5    | 22-28 ✅        |
| **🎯 US-1A.9a**       | **2 days** | **WebSocket Infrastructure for token streaming**                | **Epic 1A**                | **24-30**       |
| **🎯 US-7.1**         | **3-4 days** | **LLM Token Streaming (Anthropic, OpenAI, Google)**           | **Epic 7**                 | **27-34** 🎯    |
| 7                     | 5-6 days   | (Example 7 content if needed, or skip - US-3.4 done in Ex. 6)  | Epic 3                     | 32-40           |
| 8                     | 3-4 days   | Advanced file management, external storage                      | Epic 5                     | 35-44           |
| 10                    | 3-4 days   | HTTP/DB advanced features                                       | Epic 5                     | 38-48           |
| 11                    | 2-3 days   | Activity scheduling (delay, scheduled_for)                     | Epic 3                     | 40-51           |
| **US-3.6**            | 4-5 days   | **CLI Tooling** (validate, test, visualize)                    | **Epic 3**                 | **44-56**       |

**Notes**:
- Example 6 already implements US-3.4 (Iterative Workflows) with `examples/06-agentic-research.yaml`
- Example 7 in original plan may be redundant - US-3.4 complete
- Token streaming (US-1A.9a + US-7.1) inserted as priority after Example 6
- Total with token streaming: ~44-56 days vs. original 39-52 days (+5-7 days for streaming)

### Milestone Checkpoints

**Checkpoint 1** (After Example 3 - ~10-13 days): ✅ **COMPLETE**
- ✅ Sequential, conditional, and parallel workflows work
- ✅ HTTP and PostgreSQL activities functional
- ✅ File management (outputs, references) complete
- ✅ **Demo**: Multi-document processing pipeline with file handling (`examples/03-document-processing.yaml`)

**Checkpoint 2** (After Example 6 - ~22-28 days): ✅ **COMPLETE**
- ✅ LLM activities with multiple model providers (Anthropic, OpenAI, Gemini)
- ✅ Cost tracking and budget enforcement
- ✅ Caching for cost savings
- ✅ Retry and timeout mechanisms
- ✅ **Iterative workflows (US-3.4) with loops and iteration-scoped outputs**
- **Demo**: Agentic research workflow with loops (`examples/06-agentic-research.yaml`)

**Checkpoint 3** (After US-7.1 Token Streaming - ~27-34 days): 🎯 **NEXT MILESTONE**
- 📋 WebSocket infrastructure complete (US-1A.9a)
- 📋 Token-by-token streaming from LLM activities (US-7.1)
- 📋 ChatGPT-style real-time UX for AI workflows
- 📋 Streaming integration with Example 6 (agentic research)
- 📋 Core AI-native differentiator delivered
- **Demo**: Live token streaming from agentic research workflow

**Final MVP** (After Examples 7-10 + CLI - ~44-56 days):
- ✅ All Epic 3 requirements complete (including scheduling, validation, CLI)
- ✅ All critical Epic 5 requirements complete
- ✅ **Token streaming delivered pre-launch** (Epic 7 - US-7.1)
- ✅ Production-ready workflow capabilities with AI-native features
- **Demo**: Complete system with token streaming, scheduled workflows, transactions, and advanced features

---

## Epic 3 Coverage Matrix

| User Story                         | Examples   | Status                                                  |
|------------------------------------|-----------|--------------------------------------------------------|
| US-3.1: Sequential Workflows       | 1, 2      | ✅ Complete                                            |
| US-3.2: Conditional Branching      | 2         | ✅ Complete (MiniJinja evaluation, depends_on alias)   |
| US-3.3: Parallel Execution         | 3         | ✅ Complete (Fan-out/fan-in, batch scheduling)         |
| US-3.4: Iterative Workflows        | **6**     | ✅ Complete (`examples/06-agentic-research.yaml`)      |
| US-3.5: Activity Settings          | 4, 5, 6   | ✅ Complete (retry, timeout, budget, caching)          |
| US-3.6: YAML Validation            | US-3.6    | 📋 Planned (Post-Token Streaming)                     |
| US-3.7: Activity Scheduling/Delays | 10        | 📋 Planned (Post-Token Streaming)                     |

## Epic 5 Coverage Matrix

| User Story                    | Examples  | Status       |
|-------------------------------|---------|--------------|
| US-5.1: Multi-Model LLM       | 4, 5    | ✅ Complete |
| US-5.2: AI Cost Tracking      | 4       | ✅ Complete |
| US-5.3: Semantic Caching      | 6       | ✅ Complete |
| US-5.4: Object Storage        | 3, 8    | ✅ Complete (WorkflowStorage with PostgreSQL Large Objects, file upload/download, FILE references) |
| US-5.5: HTTP Operations       | 1, 3    | ✅ Complete (GET/POST with headers, query params, file upload/download) |
| US-5.6: Database Operations   | 2       | ✅ Complete (postgres_query with parameterized queries) |
| US-5.7: Notifications         | Post-MVP| 🔮 Post-MVP |
| US-5.8: Edge/IoT              | Post-MVP| 🔮 Post-MVP |

---

## Testing Strategy

### Per-Example Testing
Each example includes:
1. **Unit tests**: Individual activity executors
2. **Integration tests**: YAML parser + activity execution
3. **End-to-end tests**: Full workflow via API submission
   - Load workflow from `examples/NN-*.yaml`
   - Submit via REST API
   - Verify execution and results
4. **Example workflow**: Created in `examples/` directory as runnable demonstration

### Regression Testing
- Maintain test suite that loads all workflows from `examples/`
- Run full suite before each example completion
- Ensure new features don't break existing workflows
- Example workflows serve as integration test fixtures

### Performance Testing
- Benchmark each example's example workflow from `examples/`
- Track execution time, memory usage
- Ensure no performance regressions
- Store benchmark results for comparison across examples

---

## Risk Management

### Technical Risks

**R1: LLM Model Provider API Changes**
- Mitigation: Abstract model provider interface, version API calls
- Fallback: Document required API versions

**R2: Large File Handling Performance**
- Mitigation: Streaming for S3, artifact references
- Fallback: Document size limits, recommend external processing

**R3: Loop Infinite Execution**
- Mitigation: Max iteration limits, budget limits, timeout enforcement
- Fallback: Manual workflow termination via API

### Dependency Risks

**R4: Redis Optional Dependency**
- Mitigation: Graceful degradation without Redis
- Fallback: Disable caching, workflows still work

**R5: AWS SDK Changes**
- Mitigation: Pin SDK versions, test before upgrades
- Fallback: Support multiple SDK versions

---

## Success Criteria (Overall)

### Functional Requirements
- ✅ All Epic 3 user stories implemented and tested
- ✅ All critical Epic 5 user stories implemented (US-5.1 to US-5.6)
- ✅ All example workflows execute successfully
- ✅ YAML validation catches common errors
- ✅ CLI tooling works end-to-end

### Non-Functional Requirements
- ✅ Workflow execution latency: <10ms P99 (orchestrator overhead)
- ✅ LLM cost tracking accuracy: ±5% of actual cost
- ✅ Cache hit rate: >70% for repeated queries (when Redis available)
- ✅ Binary size: <15MB (including all activities)
- ✅ Memory footprint: <100MB for 10 concurrent workflows

### Documentation Requirements
- ✅ YAML syntax reference
- ✅ Built-in activity catalog (all activities documented)
- ✅ Example workflows for each example
- ✅ Migration guide (from JSON to YAML)
- ✅ CLI usage documentation

---

## Next Steps

1. **Review and approve** this implementation plan
2. **Set up project structure** for YAML parser and activity library
3. **Create examples directory** (`examples/`) with README.md structure
4. **Begin Example 1**: Simple sequential workflow with HTTP
   - Implement the workflow definition parser
   - Create `examples/01-weather-report.yaml`
5. **Establish testing framework** for end-to-end workflow tests
   - Tests should load and execute workflows from `examples/`
6. **Create activity executor trait** for consistent activity interface

---

## Appendix: Activity Type Summary

### HTTP Activities
All HTTP activities support:
- Custom headers (including `Authorization: Bearer <token>`, Basic auth, API keys)
- Template expressions in headers (e.g., `{{SECRET.api_key}}`)
- Query parameters
- Request/response body handling

Activities:
- ✅ `http_request` - Generic HTTP request (configurable method: GET, POST, PUT, DELETE, PATCH, etc.)
  - Supports all HTTP methods via `method` parameter
  - Full control over headers, query params, request body, and files

### Database Activities
- ✅ `postgres_query` - Execute SQL queries with parameter binding
  - SELECT: Returns result rows
  - INSERT/UPDATE/DELETE: Returns affected row count and RETURNING clause values
  - Supports parameterized queries for SQL injection prevention
- ✅ `postgres_transaction` - Multi-statement atomic transaction
  - Multiple SQL statements executed atomically
  - Automatic rollback on error
  - RETURNING clause support

### LLM Activities
- ✅ `llm_prompt` - LLM completion (OpenAI, Anthropic, Gemini, Ollama)
- ✅ `embeddings` - Generate LLM embeddings (OpenAI, Gemini, Ollama)

### External Storage Activities
**Note**: File management is a cross-cutting framework capability. These activities provide integration with external storage services (not workflow storage).

- `s3_get` - Fetch file from external S3 bucket into workflow storage
- `s3_put` - Upload file from workflow storage to external S3 bucket
- `s3_list` - List files in external S3 bucket
- `s3_delete` - Delete file from external S3 bucket
- `gcs_get` / `gcs_put` / `gcs_list` / `gcs_delete` - Google Cloud Storage
- `azure_blob_get` / `azure_blob_put` / `azure_blob_list` / `azure_blob_delete` - Azure Blob Storage

### Notification Activities
- [ ] `email_send` - MVP
- `slack_send` (Post-MVP)
- `discord_send` (Post-MVP)
- `teams_send` (Post-MVP)

### Scripting Activities (Post-MVP external worker)
- `python_script` - Execute Python script with file inputs/outputs

### Edge/IoT Activities (Post-MVP)
- `gpio_read` 
- `gpio_write`
- `i2c_communicate`
- `camera_capture`
- `gps_location`
