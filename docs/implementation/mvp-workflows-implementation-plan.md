# MVP Workflows Implementation Plan

**Version**: 1.0
**Date**: 2025-11-11
**Status**: Planning

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
├── 01-weather-report.yaml          # 1: Sequential workflow (✅ IMPLEMENTED)
├── 01b-weather-report-dynamic.yaml # 1b: Dynamic templates variant (✅ IMPLEMENTED)
├── 02-user-validation.yaml         # 2: Conditional branching (🚧 IN PROGRESS)
└── README.md                       # Index of examples with descriptions
```

**Future Examples** (in `docs/implementation/future-examples/`):
```
docs/implementation/future-examples/
├── 03-document-processing.yaml     # 3: Parallel execution
├── 04-content-moderation.yaml      # 4: LLM with retry/budget
├── 05-research-assistant.yaml      # 5: Multi-model LLM
├── 06-faq-bot.yaml                 # 6: Semantic caching
├── 07-research-agent.yaml          # 7: Iterative workflows
├── 08-data-pipeline.yaml           # 8: Advanced file storage
├── 09-order-processing.yaml        # 9: HTTP/DB advanced
└── 10-reminder-system.yaml         # 10: Scheduled/delayed activities
```

**Note**: Examples are moved from `future-examples/` to `examples/` only after full implementation and testing.

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
   - Supports `preceding` and `following` relationships
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
- ✅ `depends_on` as alias for `preceding` (user-friendly YAML syntax)
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
**Duration**: 4-5 days
**Epic 3**: US-3.3 (Parallel Execution - Fan-Out/Fan-In)
**Epic 5**: US-5.4 (Object Storage and File Management)

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
1. Parallel activity scheduling (multiple ready activities scheduled simultaneously)
2. Fan-in synchronization (wait for ALL preceding activities)
3. **File management infrastructure**:
   - Object storage backend (S3, GCS, Azure Blob, MinIO, or local filesystem)
   - File upload when activity completes with `type: file` output
   - File download when activity references `{{FILE.activity_key.name}}`
   - Path format: `{workflow_id}/{activity_key}/{filename}`
   - Streaming upload/download (no full memory load)
4. **HTTP activity file support**:
   - GET: Save response body as file
   - POST: Send files via multipart/form-data
   - File parameter handling in activity executor
5. **Activity interface for files**:
   - Activities receive file paths or URLs (not content)
   - Activities write outputs to provided paths
   - Framework handles upload after completion
6. End-to-end test: Verify parallel execution with file passing, check fan-in waits for all

#### Success Criteria
- ✅ Multiple activities execute in parallel
- ✅ Fan-in waits for ALL preceding activities before executing
- ✅ S3 downloads/uploads complete successfully
- ✅ Large files handled efficiently (streaming, not full memory load)

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
1. Activity settings parser (retry, timeout, budget)
2. **Orchestrator retry logic** (NOT in queue or workers):
   - Add `handle_activity_failed_event()` in orchestrator
   - Check retry settings from workflow definition
   - Calculate backoff delay: `base_seconds * factor^(attempt-1)` capped at `max_seconds`
   - Track attempt count in `workflows.state_data` JSONB
   - Publish `ActivityScheduled` event with `scheduled_for` = NOW() + backoff
   - Record attempt history in `workflow_events` (immutable event log)
3. Timeout enforcement (tokio timeout)
4. Budget tracking service
   - Pre-execution budget check
   - Post-execution cost recording
   - Budget exceeded handling
5. Anthropic activity executor
   - API integration (openai crate or reqwest)
   - Token counting
   - Cost calculation (tokens × price per model)
6. Cost storage in PostgreSQL
7. End-to-end test: Verify retry on failure, budget enforcement
   - Test: Activity fails, orchestrator schedules retry with backoff
   - Test: Max attempts reached, activity fails permanently
   - Test: Exponential backoff delay increases correctly
   - Test: Fixed backoff delay remains constant

**Design Note**: Retry logic implemented in orchestrator event handlers (NOT database stored procedures or workers) for clean separation of concerns and horizontal scalability. See absurd analysis Section 3.

#### Success Criteria
- ✅ LLM activity completes successfully with Anthropic
- ✅ Retries occur on transient failures (rate limits, network errors)
- ✅ Activity aborts when budget exceeded
- ✅ Cost tracked accurately in USD
- ✅ Timeout enforced correctly

---

### Example 5: Multi-Model LLM with Automatic Fallback
**Duration**: 4-5 days
**Epic 3**: (No new YAML features - builds on Example 4)
**Epic 5**: US-5.1 (Multi-Model LLM - Complete)

#### Example Workflow: AI Research Assistant with Model Fallback
```yaml
name: research_assistant
description: Ask LLM question with automatic model fallback for reliability

activities:
  ask_question:
    activity: llm_prompt
    parameters:
      model_chain: # Automatic fallback chain
        - anthropic/haiku-4-5
        - openai/gpt-4-turbo
        - gemini/2-5-flash
      messages:
        - role: user
          content: "{{INPUT.question}}"
      max_tokens: 2000
    outputs:
      - response
      - provider
      - cost_usd
    settings:
      retry:
        max_attempts: 3
        backoff: exponential
      budget:
        limit_usd: 1.00
        on_exceeded: abort

  store_response:
    activity: postgres_query
    parameters:
      query: |
        INSERT INTO research_log
        (question, answer, provider, cost, created_at)
        VALUES ($1, $2, $3, $4, NOW())
      params:
        - "{{INPUT.question}}"
        - "{{ask_question.response}}"
        - "{{ask_question.provider}}"
        - "{{ask_question.cost_usd}}"
    depends_on:
      - ask_question
```

#### YAML Features Implemented
- ✅ Model fallback configuration
- ✅ `model_chain:` with fallback chain

#### Built-in Activities Implemented
- ✅ `llm_prompt` - OpenAI model
- ✅ `llm_prompt` - Gemini model
- ✅ Model fallback logic (try each in order until success)

#### Implementation Tasks
1. Model abstraction layer (trait for LLM models)
2. OpenAI API integration
3. Gemini API integration
4. Model fallback engine
   - Try models in order
   - Record which model succeeded
5. Model-specific cost calculation
6. End-to-end test: Verify fallback to next model on failure

#### Success Criteria
- ✅ Multiple LLM providers supported (OpenAI, Anthropic, Gemini)
- ✅ Automatic fallback on model failure or rate limits
- ✅ Track which model was used
- ✅ Cost calculated correctly per model

---

### Example 6: Semantic Caching for Cost Savings
**Duration**: 3-4 days
**Epic 3**: US-3.5 (Activity Settings - Caching)
**Epic 5**: US-5.3 (Semantic Caching)

#### Example Workflow: FAQ Bot with Caching
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

#### YAML Features Implemented
- ✅ Cache settings: `enabled`, `ttl_seconds`, `key`
- ✅ Cache hit indicator in output

#### Built-in Activities Implemented
- ✅ Caching layer for LLM activities
- ✅ Redis-backed cache (optional dependency)
- ✅ Cache key generation from parameters

#### Implementation Tasks
1. Caching service trait (abstract cache backend)
2. Redis cache implementation (redis crate)
3. Cache key generation (hash of relevant parameters)
4. Cache lookup before activity execution
5. Cache storage after activity completion
6. TTL expiration (handled by Redis)
7. Graceful degradation when Redis unavailable
8. End-to-end test: Verify cache hit, check cost_usd = 0.0 on hit

#### Success Criteria
- ✅ Cache hit returns cached result (cost_usd = 0.0)
- ✅ Cache miss executes activity and stores result
- ✅ TTL expiration works correctly
- ✅ Works with Redis when available
- ✅ Gracefully degrades without Redis (no caching, workflow continues)

---

### Example 7: Iterative Workflows with Budget-Aware Loops
**Duration**: 5-6 days
**Epic 3**: US-3.4 (Iterative Workflows / Loops)
**Epic 5**: (No new activities - combines existing)

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

### Example 8: Advanced File Management Features
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

### Example 9: Additional HTTP and Database Features
**Duration**: 3-4 days
**Epic 3**: (No new YAML features)
**Epic 5**: US-5.5 (HTTP - Complete), US-5.6 (Database - Complete)

#### Example Workflow: API Integration with Transaction
```yaml
name: order_processing
description: Process order with API calls and database transaction

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
      statements:
        - query: |
            INSERT INTO orders
            (customer_id, product_id, quantity, amount, payment_txn_id, created_at)
            VALUES ($1, $2, $3, $4, $5, NOW())
            RETURNING order_id
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
    activity: http_request
    parameters:
      method: POST
      url: "{{INPUT.notification_webhook}}"
      body:
        order_id: "{{record_order.order_id}}"
        customer_id: "{{INPUT.customer_id}}"
        status: "confirmed"
    depends_on:
      - record_order
```

#### Built-in Activities Implemented
- ✅ `http_request` - Generic HTTP request (any method)
- ✅ `http_graphql` - GraphQL query execution
- ✅ HTTP authentication patterns:
  - Bearer token: `Authorization: Bearer {{SECRET.token}}`
  - Basic auth: `Authorization: Basic <base64(user:pass)>`
  - API key header: `X-API-Key: {{SECRET.api_key}}`
  - Custom auth headers
- ✅ `postgres_transaction` - Multi-statement transaction
- ✅ `postgres_query` - Execute SQL queries (SELECT, INSERT, UPDATE, DELETE)
- ✅ `sqlite_query` - SQLite support
- ✅ `redis_get` / `redis_set` - Redis operations

#### Implementation Tasks
1. HTTP activity enhancements
   - Generic request method (GET, POST, PUT, DELETE, PATCH)
   - GraphQL query execution
   - Full header customization (all HTTP activities)
   - Basic auth helper (optional: base64 encoding of user:pass)
   - OAuth 2.0 authentication flow (token exchange)
   - Request/response logging
2. PostgreSQL query activity
   - Support for SELECT, INSERT, UPDATE, DELETE
   - RETURNING clause support for INSERT/UPDATE/DELETE
   - Returns result rows for SELECT, metadata for INSERT/UPDATE/DELETE
3. PostgreSQL transaction support
   - Multi-statement transactions (BEGIN, COMMIT, ROLLBACK)
   - RETURNING clause support
   - Transaction rollback on error
4. SQLite activity executor
5. Redis activity executor (get, set, delete, expire)
6. Connection pooling for databases
7. End-to-end test: Transaction rollback on error, HTTP auth methods

#### Success Criteria
- ✅ HTTP supports all major methods and auth types
- ✅ GraphQL queries execute correctly
- ✅ PostgreSQL transactions commit/rollback atomically
- ✅ SQLite and Redis activities work
- ✅ Connection pooling reduces overhead

---

### Example 10: Scheduled and Delayed Activities
**Duration**: 2-3 days
**Epic 3**: US-3.7 (Activity Scheduling and Delays)
**Epic 5**: (No new activities - exposes existing infrastructure)
**Status**: Not Started

#### Example Workflow: Scheduled Reminder System
```yaml
name: reminder_system
description: Scheduled reminder system demonstrating activity delays and absolute scheduling

activities:
  # Step 1: Validate the reminder request immediately
  validate_reminder:
    activity_name: http_request
    parameters:
      method: POST
      url: "{{INPUT.validation_api}}"
      headers:
        Content-Type: "application/json"
      body:
        recipient: "{{INPUT.recipient}}"
        message: "{{INPUT.message}}"
        reminder_count: 3
    outputs:
      - validation_result

  # Step 2: Wait 5 minutes before sending first reminder
  # Uses delay_seconds to wait after validation completes
  send_first_reminder:
    activity_name: http_request
    parameters:
      method: POST
      url: "{{INPUT.notification_webhook}}"
      headers:
        Content-Type: "application/json"
      body:
        recipient: "{{INPUT.recipient}}"
        message: "Reminder (1/3): {{INPUT.message}}"
        timestamp: "{{WORKFLOW.current_time}}"
    settings:
      delay_seconds: 300  # Wait 5 minutes after validation completes
    depends_on:
      - validate_reminder:
          condition: "{{validate_reminder.validation_result.valid}} == true"

  # Step 3: Wait 1 hour before second reminder
  # Demonstrates longer delay between sequential activities
  send_second_reminder:
    activity_name: http_request
    parameters:
      method: POST
      url: "{{INPUT.notification_webhook}}"
      headers:
        Content-Type: "application/json"
      body:
        recipient: "{{INPUT.recipient}}"
        message: "Reminder (2/3): {{INPUT.message}}"
        timestamp: "{{WORKFLOW.current_time}}"
    settings:
      delay_seconds: 3600  # Wait 1 hour after first reminder
    depends_on:
      - send_first_reminder

  # Step 4: Schedule final reminder for specific deadline
  # Uses absolute timestamp scheduling via scheduled_at
  send_final_reminder:
    activity_name: http_request
    parameters:
      method: POST
      url: "{{INPUT.notification_webhook}}"
      headers:
        Content-Type: "application/json"
      body:
        recipient: "{{INPUT.recipient}}"
        message: "Final Reminder (3/3): {{INPUT.message}}"
        deadline: "{{INPUT.deadline}}"
        timestamp: "{{WORKFLOW.current_time}}"
    settings:
      scheduled_at: "{{INPUT.deadline}}"  # Absolute ISO 8601 timestamp
    depends_on:
      - send_second_reminder
```

#### YAML Features Implemented

**New Features (US-3.7)**:
- ✅ `settings.delay_seconds` - Delay activity execution by specified seconds
- ✅ `settings.scheduled_at` - Schedule activity for absolute timestamp (ISO 8601)
- ✅ Template support in `scheduled_at` - Dynamic scheduling from workflow inputs

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
- Add `delay_seconds: Option<u64>` field
- Add `scheduled_at: Option<String>` field (ISO 8601 timestamp)
- Validation: `scheduled_at` must be valid ISO 8601 or template expression
- Validation: Cannot specify both `delay_seconds` and `scheduled_at`

**2. Update Orchestrator Activity Scheduling**
- When scheduling activity, check `settings.delay_seconds`
- If present, calculate `scheduled_for = NOW() + delay_seconds`
- If `scheduled_at` present, parse timestamp and use as `scheduled_for`
- Template evaluation: Resolve `{{INPUT.*}}` expressions before parsing timestamp

**3. Database Changes**
- ✅ **No changes needed** - `activity_queue.scheduled_for` already exists
- ✅ Worker polling already filters by `scheduled_for <= NOW()`

**4. Testing**
- Unit tests for ActivitySettings validation
- Integration test: Verify delayed activity doesn't execute early
- Integration test: Verify scheduled activity executes at correct time
- Integration test: Verify template resolution in `scheduled_at`
- End-to-end test with example workflow

**5. Documentation**
- Update workflow definition language docs
- Add scheduling examples to documentation
- Document use cases: rate limiting, delayed retries, scheduled reports

#### Success Criteria

**Functional**:
- ✅ Activities can be delayed by seconds (`delay_seconds`)
- ✅ Activities can be scheduled for absolute time (`scheduled_at`)
- ✅ Templates work in `scheduled_at` parameter
- ✅ Workers don't claim activities before `scheduled_for` time
- ✅ Validation prevents both `delay_seconds` and `scheduled_at` together
- ✅ Example workflow `docs/implementation/future-examples/10-reminder-system.yaml` runs successfully

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
   - `delay_seconds`: Relative to when activity becomes ready (simple, common case)
   - `scheduled_at`: Absolute timestamp (for scheduled reports, deadlines)
2. **Mutually Exclusive**: Cannot specify both (validation error)
3. **Template Support**: `scheduled_at` can use templates for dynamic scheduling
4. **No Event Suspension (Yet)**: This example only covers time-based delays, not external events

**Post-MVP Enhancement**:
- Event-driven suspension (`wait_for_event` activity) - see absurd analysis Section 2, Phase 2

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

### Example 11: Notification Activities
**Duration**: 2-3 days
**Epic 5**: US-5.7 (Notification Activities)

#### Activities
- `slack_send_message` - Send Slack notification
- `email_send` - Send email via SMTP
- `discord_send` - Discord webhook
- `teams_send` - Microsoft Teams notification

### Example 12: Edge/IoT Activities (Unique Differentiator)
**Duration**: 4-5 days
**Epic 5**: US-5.8 (Edge/IoT Activities)

#### Activities
- `gpio_read` / `gpio_write` - Raspberry Pi GPIO
- `i2c_communicate` - I2C device communication
- `camera_capture` - Capture image from camera
- `gps_location` - Get GPS coordinates

---

## Implementation Schedule

### Phase Overview
- **Total Duration**: 37-50 days (7.5-10 weeks)
- **Examples 1-7**: Core MVP workflow features (27-34 days)
- **Examples 8-10**: Advanced features (8-11 days)
- **US-3.6**: CLI Tooling (4-5 days, can run in parallel with Examples 8-10)
- **Total MVP**: 39-52 days

### Detailed Schedule

| Example      | Duration | Epic 3 Features                                       | Epic 5 Features                            | Cumulative Days |
|------------|----------|-------------------------------------------------------|--------------------------------------------|-----------------|
| 1          | 3-4 days | Sequential workflows, basic templates                 | HTTP GET/POST                              | 3-4             |
| 2          | 3-4 days | Conditional branching, secrets                        | PostgreSQL execute/query                   | 6-8             |
| 3          | 4-5 days | Parallel execution (fan-out/fan-in), file management  | File outputs & references                  | 10-13           |
| 4          | 5-6 days | Activity settings (retry, timeout, budget)            | LLM (Anthropic), cost tracking             | 15-19           |
| 5          | 4-5 days | Model fallback                                        | LLM (OpenAI, Gemini)                       | 19-24           |
| 6          | 3-4 days | Caching settings                                      | Semantic caching (Redis)                   | 22-28           |
| 7          | 5-6 days | Iterative workflows, loops, iteration-scoped outputs  | (Combined existing)                        | 27-34           |
| 8          | 3-4 days | (Enhancements)                                        | Advanced file management, external storage | 30-38           |
| 9          | 3-4 days | (Enhancements)                                        | HTTP/DB advanced features                  | 33-42           |
| 10         | 2-3 days | **Activity scheduling** (delay_seconds, scheduled_at) | (Exposes existing infrastructure)          | 35-45           |
| **US-3.6** | 4-5 days | **CLI Tooling** (validate, test, visualize)          | (Cross-cutting tooling)                    | **39-52**       |

### Milestone Checkpoints

**Checkpoint 1** (After Example 3 - ~10-13 days):
- ✅ Sequential, conditional, and parallel workflows work
- ✅ HTTP and PostgreSQL activities functional
- ✅ File management (outputs, references) complete
- **Demo**: Multi-document processing pipeline with file handling

**Checkpoint 2** (After Example 6 - ~22-28 days):
- ✅ LLM activities with multiple model providers (Anthropic, OpenAI, Gemini)
- ✅ Cost tracking and budget enforcement
- ✅ Caching for cost savings
- ✅ Retry and timeout mechanisms
- **Demo**: AI research assistant with cost control

**Checkpoint 3** (After Example 7 - ~27-34 days):
- ✅ Iterative workflows with loops and iteration-scoped outputs
- ✅ YAML validation and CLI tooling
- ✅ Complete Epic 3 and core Epic 5
- **Demo**: Agentic research workflow + CLI tools

**Final MVP** (After Example 10 - ~39-52 days):
- ✅ All Epic 3 requirements complete (including scheduling)
- ✅ All critical Epic 5 requirements complete
- ✅ Production-ready workflow capabilities
- ✅ Activity scheduling for delayed and scheduled execution
- **Demo**: Complete system with scheduled workflows, transactions, and advanced features

---

## Epic 3 Coverage Matrix

| User Story                         | Examples  | Status        |
|------------------------------------|---------|---------------|
| US-3.1: Sequential Workflows       | 1, 2, 3 | ✅ Complete   |
| US-3.2: Conditional Branching      | 2, 7    | ✅ Complete (MiniJinja evaluation, depends_on alias)   |
| US-3.3: Parallel Execution         | 3       | ✅ Complete   |
| US-3.4: Iterative Workflows        | 7       | ✅ Complete   |
| US-3.5: Activity Settings          | 4, 6    | ✅ Complete   |
| US-3.6: YAML Validation            | US-3.6  | ✅ Complete   |
| US-3.7: Activity Scheduling/Delays | 10      | Not Started   |

## Epic 5 Coverage Matrix

| User Story                    | Examples  | Status       |
|-------------------------------|---------|--------------|
| US-5.1: Multi-Model LLM       | 4, 5    | ✅ Complete  |
| US-5.2: AI Cost Tracking      | 4       | ✅ Complete  |
| US-5.3: Semantic Caching      | 6       | ✅ Complete  |
| US-5.4: Object Storage        | 3, 8    | ✅ Complete  |
| US-5.5: HTTP Operations       | 1, 9    | ✅ Complete  |
| US-5.6: Database Operations   | 2, 9    | ✅ Complete (postgres_query implemented)  |
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
- `http_request` - Generic HTTP request (configurable method: GET, POST, PUT, DELETE, PATCH, etc.)
  - Supports all HTTP methods via `method` parameter
  - Full control over headers, query params, request body, and files
- `http_graphql` - GraphQL query with authentication

### Database Activities
- `postgres_query` - Execute SQL queries with parameter binding
  - SELECT: Returns result rows
  - INSERT/UPDATE/DELETE: Returns affected row count and RETURNING clause values
  - Supports parameterized queries for SQL injection prevention
- `postgres_transaction` - Multi-statement atomic transaction
  - Multiple SQL statements executed atomically
  - Automatic rollback on error
  - RETURNING clause support
- `sqlite_query` - SQLite query execution (same interface as postgres_query)
- `redis_get` - Redis GET operation
- `redis_set` - Redis SET operation

### LLM Activities
- `llm_prompt` - LLM completion (OpenAI, Anthropic, Gemini)
- `llm_embed` - Generate embeddings (future)

### External Storage Activities
**Note**: File management is a cross-cutting framework capability. These activities provide integration with external storage services (not workflow storage).

- `s3_get` - Fetch file from external S3 bucket into workflow storage
- `s3_put` - Upload file from workflow storage to external S3 bucket
- `s3_list` - List files in external S3 bucket
- `s3_delete` - Delete file from external S3 bucket
- `gcs_get` / `gcs_put` / `gcs_list` / `gcs_delete` - Google Cloud Storage
- `azure_blob_get` / `azure_blob_put` / `azure_blob_list` / `azure_blob_delete` - Azure Blob Storage
- `minio_get` / `minio_put` / `minio_list` / `minio_delete` - MinIO (self-hosted S3-compatible)

### Scripting Activities
- `python_script` - Execute Python script with file inputs/outputs

### Notification Activities (Post-MVP)
- `slack_send_message`
- `email_send`
- `discord_send`
- `teams_send`

### Edge/IoT Activities (Post-MVP)
- `gpio_read` / `gpio_write`
- `i2c_communicate`
- `camera_capture`
- `gps_location`
