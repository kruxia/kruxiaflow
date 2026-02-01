# Kruxia Flow Workflow Examples

This directory contains example workflows that demonstrate Kruxia Flow features progressively from simple to complex.

## Examples Environment

All examples that send notifications or use a database are pre-configured to work with
the **examples environment**. Start it with:

```bash
# run the examples environment
./docker up --examples
```

This adds:
- **Mailpit** — captures all notification emails. View them at http://localhost:8025
- **kruxiaflow_examples database** — pre-created with all tables needed by the examples
- **Secrets** — `SECRET.db_url` is automatically available to example workflows when running the examples environment.

The init scripts in `examples/initdb/` run automatically on first postgres startup.
If you already have a running cluster, run `./docker clean && ./docker up --examples`
to recreate the volumes.

## Available Examples

| Example                                | Features Demonstrated                                                           | Prerequisites                                           |
|----------------------------------------|---------------------------------------------------------------------------------|---------------------------------------------------------|
| `01-weather-report.yaml`               | Sequential workflow, HTTP request (GET/POST), headers, template expressions     | Examples environment                                    |
| `01b-weather-report-dynamic.yaml`      | Chained API calls, dynamic URL from outputs, query parameters, workflow input   | Examples environment                                    |
| `02-user-validation.yaml`              | Conditional branching, PostgreSQL query, depends_on conditions                  | Examples environment                                    |
| `03-document-processing.yaml`          | Parallel execution, fan-out/fan-in, echo activity, multiple dependencies        | Examples environment                                    |
| `04-moderate-content.yaml`             | LLM activity, cost tracking, budget limits, retry with exponential backoff      | Examples environment, Anthropic API key                 |
| `05-research-assistant.yaml`           | Multi-model LLM fallback, budget-aware provider selection, cost optimization    | Examples environment, any LLM provider API key          |
| `05a-research-assistant-anthropic.yaml`| Budget-aware fallback with Anthropic models only                                | Examples environment, Anthropic API key                 |
| `05b-research-assistant-openai.yaml`   | Budget-aware fallback with OpenAI models only                                   | Examples environment, OpenAI API key                    |
| `05c-research-assistant-google.yaml`   | Budget-aware fallback with Google models only                                   | Examples environment, Google API key                    |
| `06a-faq-bot-caching.yaml`             | Semantic caching for LLM responses, cache hit tracking, cost savings            | Examples environment, Anthropic API key                 |
| `06b-rag-index-builder.yaml`           | Batch embedding generation, bulk pgvector storage, unnest insert                | Examples environment, Google API key                    |
| `06c-rag-query.yaml`                   | Complete RAG pattern: embed → search → augment → generate, MiniJinja loops      | Examples environment, Google + Anthropic API keys       |
| `07a-agentic-research-simple.yaml`     | Iterative workflows (loops), iteration-scoped storage, budget-aware loops       | Anthropic API key                                       |
| `07b-agentic-research-complete.yaml`   | Complete iterative research: model fallback, file storage, dual paths           | Examples environment, Anthropic API key                 |
| `08a-rate-limited-api-calls.yaml`      | Iterative loop with `delay`, rate limiting, httpbin.org, configurable page count | Examples environment                                    |
| `08b-scheduled-daily-report.yaml`      | Activity scheduling with `scheduled_for`, absolute timestamps, ISO 8601         | Examples environment, Anthropic API key                 |
| `08c-delayed-reminders.yaml`           | Cascading delays, escalating reminders (1m → 3m → 8m), email escalation         | Examples environment                                    |
| `09a-streaming-llm.yaml`               | LLM token streaming over WebSocket, provider fallback, two-level opt-in         |  Any LLM provider API key                               |
| `09b-streaming-research.yaml`          | Selective streaming, multi-step with streaming final output, cost tracking      | Anthropic API key                                       |
| `10-order-processing.yaml`             | E-commerce order flow, postgres_query, postgres_transaction, Mailpit email      | Examples environment                                    |

## Running Examples

Every example follows the same two-step pattern: **deploy** the workflow definition,
then **submit** an instance with inputs.

### Authentication

All API calls require a Bearer token. Obtain one first:

```bash
TOKEN=$(curl -s -X POST http://localhost:8080/api/v1/oauth/token \
  -d "grant_type=client_credentials" \
  -d "client_id=kruxiaflow-docker-client" \
  -d "client_secret=kruxiaflow-dev-secret" | jq -r '.access_token')
```

### Deploy and Submit

```bash
# Deploy a workflow definition (accepts YAML directly)
curl -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Authorization: Bearer $TOKEN" \
  --data-binary @examples/01-weather-report.yaml

# Submit a workflow instance with inputs
curl -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"definition_name": "weather_report", "input": {}}'
```

### Check Status

```bash
# Get workflow status (use the workflow_id from the submit response)
curl http://localhost:8080/api/v1/workflows/$WORKFLOW_ID \
  -H "Authorization: Bearer $TOKEN"
```

## Viewing Notifications

All example notifications are sent to **Mailpit** which captures them locally.
View all captured messages at **http://localhost:8025**.

## Template Expression Syntax

Kruxia Flow supports the following template expression formats:

### Input Variables
Access workflow input parameters:
```yaml
url: "{{INPUT.address}}"
```

### Activity Outputs
Access outputs from previous activities:
```yaml
temperature: "{{fetch_weather.response.body.properties.periods[0].temperature}}"
```

### Secrets
Access secret values (for API keys, database URLs):
```yaml
db_url: "{{SECRET.db_url}}"
```

### Workflow Variables
Access workflow-level metadata:
```yaml
workflow_id: "{{WORKFLOW.id}}"
```

---

### Example 1: Weather Report Pipeline

Sequential workflow that fetches weather data from the National Weather Service API
and sends a notification email via Mailpit.

```bash
curl -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Authorization: Bearer $TOKEN" \
  --data-binary @examples/01-weather-report.yaml

curl -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"definition_name": "weather_report", "input": {}}'
```

**Expected behavior:**
1. Fetches forecast data from weather.gov API (HTTP GET)
2. Sends an email via Mailpit REST API (HTTP POST) with temperature and forecast data
3. View the notification at http://localhost:8025

**Features demonstrated:**
- Sequential activity execution via `depends_on`
- HTTP GET and POST requests
- Template expressions for activity output access (`{{fetch_weather.response.body.properties...}}`)
- Workflow context variables (`{{WORKFLOW.id}}`)

---

### Example 1b: Dynamic Weather Report

Chains multiple API calls where each URL is built dynamically from the previous
activity's output. Geocodes an address, resolves the NWS grid point, fetches the
forecast, then sends a notification.

```bash
curl -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Authorization: Bearer $TOKEN" \
  --data-binary @examples/01b-weather-report-dynamic.yaml

curl -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "definition_name": "weather_report_dynamic",
    "input": {
      "address": "1600 Pennsylvania Avenue NW, Washington, DC 20500",
      "city": "Washington",
      "state": "DC"
    }
  }'
```

**Features demonstrated:**
- Chained API calls (geocode → grid lookup → forecast → notify)
- Dynamic URL construction from activity outputs
- Query parameters on HTTP requests
- Workflow input via `{{INPUT.*}}` templates

---

### Example 2: User Validation with Conditional Branching

Uses the `echo` activity to pass through workflow input, then conditionally stores
the user in either the `valid_users` or `invalid_users` table based on the `valid`
input field. Sends a notification after either branch completes.

```bash
curl -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Authorization: Bearer $TOKEN" \
  --data-binary @examples/02-user-validation.yaml

# Valid user — takes the store_valid_user branch
curl -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "definition_name": "validate_user",
    "input": {"email": "user@example.com", "valid": true}
  }'

# Invalid user — takes the store_invalid_user branch
curl -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "definition_name": "validate_user",
    "input": {"email": "bad@example.com", "valid": false}
  }'
```

**Features demonstrated:**
- `echo` activity to pass input through as activity output
- Conditional `depends_on` with expressions (`{{check_email.echo.valid == true}}`)
- PostgreSQL query activity (`postgres_query`)
- Multiple conditional branches from same activity
- Fan-in pattern (notification waits for either branch)

---

### Example 3: Multi-Document Processing Pipeline

Fetches 3 documents in parallel via HTTP, processes each in parallel using `echo`
activities, aggregates all results (fan-in), then sends a summary notification.
No external services required beyond httpbin.org.

```
fetch_doc1 ──→ process_doc1 ──┐
fetch_doc2 ──→ process_doc2 ──┼──→ aggregate_results ──→ store_summary
fetch_doc3 ──→ process_doc3 ──┘
```

```bash
curl -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Authorization: Bearer $TOKEN" \
  --data-binary @examples/03-document-processing.yaml

curl -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"definition_name": "process_documents", "input": {}}'
```

**Features demonstrated:**
- Parallel activity execution (fan-out: 3 fetches, 3 processes)
- Fan-in synchronization (aggregate waits for all 3 process activities)
- Multiple `depends_on` relationships
- `echo` activity for data pass-through between pipeline stages
- Template expressions accessing nested output fields (`{{fetch_doc1.response.body.args.title}}`)

---

### Example 4: LLM Content Moderation with Cost Tracking and Retry

Uses Claude Haiku to analyze user content for policy violations, with automatic retry
on failure and budget enforcement. Stores the moderation result in PostgreSQL.

```bash
curl -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Authorization: Bearer $TOKEN" \
  --data-binary @examples/04-moderate-content.yaml

curl -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "definition_name": "moderate_content",
    "input": {
      "user_content": "Check out this amazing product!",
      "content_id": "content_12345"
    }
  }'
```

**Expected behavior:**
1. `analyze_content`: Claude Haiku analyzes the content and returns a moderation decision (violates, reason, severity, confidence)
2. If the API call fails, retries up to 3 times with exponential backoff (2s, 4s, 8s)
3. If the budget limit ($0.50) is reached, the workflow aborts
4. `store_moderation_result`: Stores the decision and token usage in PostgreSQL

**Features demonstrated:**
- LLM activity (`llm_prompt`) with Anthropic Claude
- Exponential backoff retry (`settings.retry`)
- Budget enforcement with abort action (`settings.budget`)
- Sequential dependency with LLM output passed to database query

---

### Example 5: Multi-Model LLM with Budget-Aware Fallback

Research assistant that tries multiple LLM providers in order, automatically skipping
expensive models that exceed the budget. Stores the result with provider metadata in
PostgreSQL.

```bash
curl -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Authorization: Bearer $TOKEN" \
  --data-binary @examples/05-research-assistant.yaml

curl -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "definition_name": "research_assistant",
    "input": {
      "question": "What are the key differences between Rust and Go for systems programming?"
    }
  }'
```

**Expected behavior (with $0.01 budget):**
1. **OpenAI o1-pro** ($150/$600 per M tokens): SKIPPED — exceeds budget
2. **Anthropic Claude Sonnet 4.5** ($3/$15 per M tokens): SKIPPED — exceeds budget
3. **Google Gemini Flash Lite** ($0.075/$0.30 per M tokens): USED — fits budget
4. Result stored in `research_log` with provider, model, cost, and token usage

**Features demonstrated:**
- Multi-model LLM fallback chain with budget-aware provider selection
- Cost estimation before execution (prevents expensive API calls)
- Actual cost tracking via `{{ask_question.result.cost_usd}}`
- Provider/model metadata in outputs

### Example 5a/b/c: Single-Provider Budget-Aware Fallback

Single-provider variants for users with only one API key:

#### Example 5a: Anthropic Only

Fallback chain: Claude Opus 4.5 → Claude Sonnet 4.5 → Claude Haiku 4.5

```bash
curl -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Authorization: Bearer $TOKEN" \
  --data-binary @examples/05a-research-assistant-anthropic.yaml

curl -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "definition_name": "research_assistant_anthropic",
    "input": {"question": "What are the key differences between Rust and Go?"}
  }'
```

#### Example 5b: OpenAI Only

Fallback chain: o1 → GPT-5 Mini → GPT-5 Nano

```bash
curl -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Authorization: Bearer $TOKEN" \
  --data-binary @examples/05b-research-assistant-openai.yaml

curl -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "definition_name": "research_assistant_openai",
    "input": {"question": "What are the key differences between Rust and Go?"}
  }'
```

#### Example 5c: Google Only

Fallback chain: Gemini 3 Pro Preview → Gemini 2.5 Flash → Gemini 2.0 Flash Lite

```bash
curl -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Authorization: Bearer $TOKEN" \
  --data-binary @examples/05c-research-assistant-google.yaml

curl -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "definition_name": "research_assistant_google",
    "input": {"question": "What are the key differences between Rust and Go?"}
  }'
```

---

### Example 6a: FAQ Bot with Semantic Caching

Answers user questions using Claude Haiku with semantic caching. Repeated questions
are served from cache at zero cost. Logs all interactions with cache metrics.

```bash
curl -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Authorization: Bearer $TOKEN" \
  --data-binary @examples/06a-faq-bot-caching.yaml

curl -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "definition_name": "faq_bot",
    "input": {"question": "What are your business hours?"}
  }'
```

**Expected behavior:**
- First question: LLM call → cache miss → costs ~$0.001
- Second identical question (within TTL): cache hit → costs $0.000

**Features demonstrated:**
- Semantic caching for LLM responses (enabled, ttl_seconds, key)
- Cache hit tracking in outputs (`{{answer_question.result.cache_hit}}`)
- Cost tracking with cache awareness

---

### Example 6b: RAG Index Builder

Generates embeddings for all document chunks in a single batch using Google Gemini
Embedding, then bulk-inserts all chunks with their embeddings into PostgreSQL
with pgvector in one query.

```bash
curl -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Authorization: Bearer $TOKEN" \
  --data-binary @examples/06b-rag-index-builder.yaml

curl -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "definition_name": "rag_index_builder",
    "input": {
      "chunks": ["Rust is a systems programming language...", "Ownership model ensures memory safety...", "Cargo is the package manager..."],
      "source": "rust_documentation"
    }
  }'
```

**Expected behavior:**
1. `generate_embeddings`: Sends all chunks to Google Gemini → receives 3072-dimensional vectors
2. `store_chunks`: Bulk-inserts all chunks with embeddings in one query using `unnest`
3. `confirm_indexing`: Sends notification with chunk count and cost

**Features demonstrated:**
- Batch embedding generation (`google/gemini-embedding-001`)
- Bulk vector insert using `jsonb_array_elements` with `ORDINALITY` join
- Vector type casting (`::text::vector`) in PostgreSQL
- JSONB metadata construction with `jsonb_build_object`

---

### Example 6c: RAG Query and Q&A

Complete RAG pattern: embed user question → search pgvector for similar chunks →
pass retrieved context to LLM → generate grounded answer → log Q&A with cost tracking.

**Prerequisite**: Run 06b first to populate the index.

```bash
curl -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Authorization: Bearer $TOKEN" \
  --data-binary @examples/06c-rag-query.yaml

curl -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "definition_name": "rag_query",
    "input": {"question": "What is Rust'\''s ownership model?"}
  }'
```

**Expected behavior:**
1. `embed_question`: Converts question to vector
2. `search_similar_chunks`: pgvector cosine distance search, finds top 3 chunks
3. `generate_answer`: LLM generates answer using retrieved context
4. `store_qa_result`: Logs Q&A with cost breakdown
5. `send_response`: Sends answer via Mailpit

**Features demonstrated:**
- Complete RAG workflow pattern (embed → search → augment → generate)
- pgvector similarity search (`embedding <=> $1::vector`)
- MiniJinja loops in prompts (`{% for chunk in search_similar_chunks.result.rows %}`)
- Cost tracking across multiple activities

---

### Example 7a: Simple Agentic Research (Iterative Workflows)

Iterative research workflow using only LLM activities. Initializes a research plan,
performs iterative research passes, evaluates sufficiency after each iteration, and
loops back for more if insufficient. Compiles a final report when done.

```bash
curl -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Authorization: Bearer $TOKEN" \
  --data-binary @examples/07a-agentic-research-simple.yaml

curl -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "definition_name": "simple_agentic_research",
    "input": {"topic": "Benefits of Rust for systems programming"}
  }'
```

**Loop exit conditions:**
- Normal: `evaluate` returns "SUFFICIENT"
- Safety: Reaches 5 iterations (`iteration_limit`)
- Budget: Exceeds $0.05 for `perform_search`
- Error: Any activity fails

**Features demonstrated:**
- Loop execution via back-edge (`evaluate → perform_search`)
- Iteration-scoped storage (`iteration_scoped: true`)
- Iteration counter (`{{ACTIVITY.iteration}}`)
- Budget accumulation and remaining budget tracking across iterations
- Conditional loop back/exit based on string matching

---

### Example 7b: Complete Agentic Research (Production-Ready)

Full agentic research workflow using Claude Haiku 4.5 for iterative research (with Gemini
fallback), file storage with iteration scoping, structured field extraction, and dual
success/failure paths.

```bash
curl -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Authorization: Bearer $TOKEN" \
  --data-binary @examples/07b-agentic-research-complete.yaml

curl -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "definition_name": "agentic_research_complete",
    "input": {"research_topic": "Impact of quantum computing on cryptography"}
  }'
```

**Features demonstrated (beyond 07a):**
- Model fallback: Haiku 4.5 primary, Gemini 2.0 Flash Lite fallback for research
- File storage with iteration scoping (`type: file`, `iteration_scoped: true`)
- Structured field extraction (`{{evaluate.sufficient}}`)
- Array access with bracket notation (`{{search_information[*].results}}`)
- Dual success/failure paths (`publish_success` and `publish_failure`)
- Complex boolean conditions (`AND`, `<`, `>`)
- Budget checks in conditions

**Differences from 07a:**

| Feature                | 07a (Simple)               | 07b (Complete)                             |
|------------------------|----------------------------|--------------------------------------------|
| Research activity      | LLM simulates research     | Haiku 4.5 with Gemini fallback              |
| Loop condition syntax  | String matching            | Structured field access                     |
| Results storage        | Inline JSON                | Files with iteration scoping                |
| Array access           | Implicit (Jinja filters)   | Explicit ([*] bracket notation)             |
| Failure handling       | Implicit (timeout/error)   | Explicit publish_failure activity           |
| Budget checks          | Activity-level only        | Condition-level remaining checks            |
| External dependencies  | None (LLM-only)            | Anthropic API key (+ Google for fallback)   |
| Production readiness   | Educational demo           | Production-ready pattern                    |

---

### Example 8: Activity Scheduling and Delays

Example 8 demonstrates activity scheduling using both relative delays (`delay`) and
absolute timestamps (`scheduled_for`).

#### Example 8a: Rate-Limited API Calls (Iterative with Delay)

Uses an iterative loop with `delay` to fetch multiple pages from httpbin.org with
rate limiting. The page count is configurable via workflow input (default: 3, max: 5).

```bash
curl -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Authorization: Bearer $TOKEN" \
  --data-binary @examples/08a-rate-limited-api-calls.yaml

curl -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"definition_name": "rate_limited_api_calls", "input": {"num_pages": 3}}'
```

**Expected behavior:**
1. `fetch_page` iteration 0 executes immediately (page 0)
2. `fetch_page` iteration 1 waits 5 seconds, then executes (page 1)
3. `fetch_page` iteration 2 waits 5 seconds, then executes (page 2)
4. Loop exits (iteration >= num_pages)
5. `aggregate_results` sends all page results to Mailpit

```
t=0s     fetch_page[0] ━━━┓
                            │ (loop back, 5s delay)
t=5s     fetch_page[1] ━━━┓
                            │ (loop back, 5s delay)
t=10s    fetch_page[2] ━━━┓
                            │ (loop exits)
t=10s+                     └─→ aggregate_results
```

**Features demonstrated:**
- Iterative loop with fixed count (no evaluation activity needed)
- `settings.delay` for rate limiting between iterations
- `iteration_scoped: true` for per-iteration output storage
- Loop-back `depends_on` (activity depends on itself with condition)
- Array access pattern (`{{fetch_page[*].result}}`)
- Configurable iteration count via workflow input with default

**Supported delay units:** `ms`, `s`, `m`/`mi`, `h`, `d`, `w`, `mo`, `y`

---

#### Example 8b: Scheduled Daily Report

Uses `scheduled_for` to execute an LLM report generation at a specific absolute time.

```bash
curl -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Authorization: Bearer $TOKEN" \
  --data-binary @examples/08b-scheduled-daily-report.yaml

curl -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "definition_name": "scheduled_daily_report",
    "input": {
      "report_time": "2025-12-01T09:00:00-08:00"
    }
  }'
```

**Expected behavior:**
1. Workflow is submitted immediately
2. `generate_report` waits until `report_time` (9 AM Pacific)
3. At scheduled time, Claude Sonnet generates the daily report
4. `send_report` sends the report via Mailpit

**Features demonstrated:**
- `settings.scheduled_for` with ISO 8601 timestamps
- Template resolution in `scheduled_for`: `"{{INPUT.report_time}}"`
- Scheduled LLM execution with budget limits ($0.10)

**Delay vs. Scheduled_for:**

| Feature          | `delay` (relative)                    | `scheduled_for` (absolute)          |
|------------------|---------------------------------------|-------------------------------------|
| Use case         | Wait for duration after ready         | Execute at specific time            |
| Format           | Duration string (`"5s"`, `"2h"`)      | ISO 8601 timestamp                  |
| Reference point  | When activity becomes ready           | Clock time                          |
| Template example | `"{{INPUT.wait_time}}m"`              | `"{{INPUT.deadline}}"`              |
| Typical use      | Rate limiting, retries, staged delays | Scheduled reports, future execution |

---

#### Example 8c: Delayed Reminder System

Cascading delays for escalating notifications (1m → 3m → 8m), with each stage
sent to a different recipient. Uses short delays suitable for demo purposes;
in production, these would typically be hours or days.

```bash
curl -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Authorization: Bearer $TOKEN" \
  --data-binary @examples/08c-delayed-reminders.yaml

curl -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "definition_name": "delayed_reminder_system",
    "input": {
      "task_name": "Complete quarterly report",
      "assigned_user": "alice@example.com",
      "manager_email": "manager@example.com",
      "oncall_email": "oncall@example.com"
    }
  }'
```

```
t=0m     send_initial_notification ━━┓
                                      │
t=1m                                  └─→ send_first_reminder ━━┓
                                                                 │
t=3m                                                             └─→ send_escalated_reminder ━━┓
                                                                                                │
t=8m                                                                                            └─→ send_final_escalation
```

**Features demonstrated:**
- Cascading delays with different intervals (`"1m"`, `"2m"`, `"5m"`)
- Multi-stage escalation pattern (user → manager → on-call)
- Email notifications via Mailpit REST API

---

### Example 9a: LLM Token Streaming

Single LLM activity with token streaming over WebSocket. Tokens are delivered to
connected WebSocket clients in real-time as they are generated. Falls back to
non-streaming if no subscribers are connected.

```bash
curl -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Authorization: Bearer $TOKEN" \
  --data-binary @examples/09a-streaming-llm.yaml

curl -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "definition_name": "streaming_llm_example",
    "input": {"topic": "a robot learning to paint"}
  }'
```

**Streaming client connection:**
1. Submit workflow, get `workflow_id`
2. Get activity ID from `GET /api/v1/workflows/{workflow_id}`
3. Connect WebSocket: `ws://localhost:8080/api/v1/activities/{activity_id}/ws?token=$TOKEN`
4. Receive `token`, `complete`, or `error` messages

**Features demonstrated:**
- `streaming: true` on LLM activities
- Two-level opt-in: activity flag + WebSocket subscribers
- Provider fallback chain (Anthropic → OpenAI → Google), all supporting streaming
- Retry with exponential backoff

---

### Example 9b: Streaming Research Workflow

Multi-step research workflow where only the final analysis activity streams tokens.
Data collection steps run in non-streaming mode for efficiency.

```bash
curl -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Authorization: Bearer $TOKEN" \
  --data-binary @examples/09b-streaming-research.yaml

curl -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "definition_name": "streaming_research_workflow",
    "input": {"topic": "Impact of quantum computing on cryptography"}
  }'
```

**Workflow pattern:**
```
summarize_topic ──┬──► analyze_findings (STREAMING)
gather_sources  ──┘
```

**Expected behavior:**
1. `summarize_topic`: Brief overview of the topic (non-streaming, Claude Haiku)
2. `gather_sources`: Generates 3 key research questions (non-streaming, Claude Haiku)
3. `analyze_findings`: Comprehensive analysis using both outputs (streaming, Claude Sonnet with Haiku fallback)

**Features demonstrated:**
- Selective streaming: only the final long-form output streams
- Non-streaming preparation steps for efficiency
- Multi-step workflow with streaming final output
- Budget enforcement with warn action ($0.10)

---

### Example 10: Order Processing

Complete e-commerce order processing flow using real database resources: validate
inventory, reserve stock, process payment (via DB insert), record the order in an
atomic database transaction, and send an HTML confirmation email via Mailpit. 

(NOTE: We know that a proper e-commerce application might be better with a different
architecture than just direct database queries. We're just demonstrating a complex
workflow with the resources we have here, which include a database and don't include
order system APIs or event streams.)

```bash
curl -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Authorization: Bearer $TOKEN" \
  --data-binary @examples/10-order-processing.yaml

curl -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "definition_name": "order_processing",
    "input": {
      "customer_id": "cust_123",
      "customer_email": "customer@example.com",
      "product_id": "prod_001",
      "quantity": 2,
      "amount": 99.99
    }
  }'
```

**Workflow pattern:**
```
validate_inventory → reserve_inventory → process_payment → record_order → send_confirmation
  (postgres_query)    (postgres_query)    (postgres_query)  (postgres_txn)  (http_request)
```

**Expected behavior:**
1. `validate_inventory`: SELECT to check stock availability (unreserved count)
2. `reserve_inventory`: UPDATE to reserve stock (conditional on sufficient unreserved inventory)
3. `process_payment`: INSERT into payments table (returns transaction_id)
4. `record_order`: Atomic `postgres_transaction` with INSERT order + UPDATE inventory (RETURNING order_id)
5. `send_confirmation`: HTML email with order details via Mailpit REST API

**Features demonstrated:**
- `postgres_query` with SELECT, UPDATE RETURNING, and INSERT RETURNING
- Conditional `depends_on` (`{{validate_inventory.result.rows[0].unreserved >= INPUT.quantity}}`)
- `postgres_transaction` with multiple statements and RETURNING clause
- `http_request` to Mailpit REST API for HTML email
- Sequential dependency chain modeling a real-world order flow
- All resources are local (database + Mailpit) — no external APIs needed
