# StreamFlow Workflow Examples

This directory contains example workflows that demonstrate StreamFlow features progressively from simple to complex.

## Available Examples

| Example                           | Features Demonstrated                                                        | Prerequisites     |
|-----------------------------------|------------------------------------------------------------------------------|-------------------|
| `01-weather-report.yaml`          | Sequential workflow, HTTP request (GET/POST), headers, template expressions  | Webhook URL       |
| `01b-weather-report-dynamic.yaml` | Dynamic templates, workflow input                                            | Webhook URL       |
| `02-user-validation.yaml`         | Conditional branching, PostgreSQL query, depends_on conditions               | Database, Webhook |
| `03-document-processing.yaml`     | Parallel execution, fan-out/fan-in, multiple dependencies                    | HTTP endpoints    |
| `04-moderate-content.yaml`        | LLM activity, cost tracking, budget limits, retry with exponential backoff   | Anthropic API key, Database |
| `05-research-assistant.yaml`      | Multi-model LLM fallback, budget-aware provider selection, cost optimization | Any LLM provider API key, Database |
| `05a-research-assistant-anthropic.yaml` | Budget-aware fallback with Anthropic models only                       | Anthropic API key, Database |
| `05b-research-assistant-openai.yaml` | Budget-aware fallback with OpenAI models only                             | OpenAI API key, Database |
| `05c-research-assistant-google.yaml` | Budget-aware fallback with Google models only                             | Google API key, Database |

## Running Examples

### Example 1: Weather Report Pipeline

This workflow demonstrates a simple sequential workflow that:
1. Fetches weather data from the National Weather Service API (HTTP GET)
2. Sends a notification to a webhook with extracted weather data (HTTP POST)

**Prerequisites:**
- A webhook URL to receive the notification (e.g., webhook.site, requestbin.com)

**Run with StreamFlow CLI:**
```bash
streamflow run examples/01-weather-report.yaml \
  --input webhook_url=https://webhook.site/your-unique-id
```

**Expected behavior:**
1. Workflow fetches forecast data from weather.gov API
2. Extracts temperature and conditions from the response
3. Posts formatted data to your webhook URL
4. Webhook receives JSON with temperature, conditions, and workflow_id

**Features demonstrated:**
- ✅ YAML workflow definition parsing
- ✅ Sequential activity execution via `following` relationships
- ✅ HTTP GET request with custom headers
- ✅ HTTP POST request with JSON body
- ✅ Template expressions for input substitution (`{{INPUT.webhook_url}}`)
- ✅ Template expressions for activity output access (`{{fetch_weather.body.properties...}}`)
- ✅ Workflow context variables (`{{WORKFLOW.id}}`)

## Template Expression Syntax

StreamFlow supports the following template expression formats:

### Input Variables
Access workflow input parameters:
```yaml
url: "{{INPUT.webhook_url}}"  # Where to POST the results
```

### Activity Outputs
Access outputs from previous activities:
```yaml
temperature: "{{fetch_weather.body.properties.periods[0].temperature}}"
```

### Secrets
Access secret values (for API keys, tokens):
```yaml
headers:
  Authorization: "Bearer {{SECRET.api_key}}"
```

### Workflow Variables
Access workflow-level metadata:
```yaml
workflow_id: "{{WORKFLOW.id}}"
```

### Example 2: User Validation with Conditional Branching

This workflow demonstrates conditional branching based on activity outputs:
1. Validates a user email using an HTTP service
2. Stores the user in either `valid_users` or `invalid_users` table based on validation result
3. Sends a notification after storage completes

**Prerequisites:**
- PostgreSQL database with `valid_users` and `invalid_users` tables
- Webhook URL to receive notifications

**Run with StreamFlow CLI:**
```bash
streamflow run examples/02-user-validation.yaml \
  --input email=user@example.com \
  --input notification_webhook_url=https://webhook.site/your-unique-id \
  --secret db_url=postgres://user:pass@localhost:5432/dbname
```

**Features demonstrated:**
- ✅ Conditional activity execution via `depends_on` conditions
- ✅ PostgreSQL query activity
- ✅ Multiple conditional branches from same activity
- ✅ Fan-in pattern (notification waits for either branch)

### Example 3: Multi-Document Processing Pipeline

This workflow demonstrates parallel execution with fan-out/fan-in patterns:
1. Fetches 3 documents in parallel (HTTP GET)
2. Processes 3 documents in parallel (HTTP POST, each depends on its fetch)
3. Aggregates results from all 3 (fan-in: waits for all)
4. Stores final summary (HTTP POST)

**Prerequisites:**
- HTTP endpoints for document fetching, processing, aggregation, and storage
- For testing: Use httpbin.org or local mock services

**Run with StreamFlow CLI:**
```bash
streamflow run examples/03-document-processing.yaml \
  --input doc1_url=https://httpbin.org/base64/Q29udGVudCBmb3IgZG9jdW1lbnQgMQ== \
  --input doc2_url=https://httpbin.org/base64/Q29udGVudCBmb3IgZG9jdW1lbnQgMg== \
  --input doc3_url=https://httpbin.org/base64/Q29udGVudCBmb3IgZG9jdW1lbnQgMw== \
  --input processing_service_url=https://httpbin.org/post \
  --input aggregator_url=https://httpbin.org/post \
  --input storage_webhook_url=https://httpbin.org/post
```

**Expected behavior:**
1. Three fetch activities execute in parallel (no dependencies)
2. Three process activities execute in parallel (each depends on its corresponding fetch)
3. Aggregate activity waits for all three process activities to complete (fan-in)
4. Store activity executes after aggregation completes
5. Data passed between activities via template expressions

**Features demonstrated:**
- ✅ Parallel activity execution (fan-out: 1 → many)
- ✅ Fan-in synchronization (many → 1: wait for all dependencies)
- ✅ Multiple `depends_on` relationships
- ✅ HTTP GET and POST requests
- ✅ Output references via template expressions (`{{activity.response.body}}`)

## Testing Examples

You can test examples using a webhook service:

1. **webhook.site** - Get a unique URL at https://webhook.site
2. **requestbin.com** - Create a bin at https://requestbin.com
3. **httpbin.org** - Free HTTP testing service for file uploads/downloads
4. **Local webhook** - Run a local server: `python -m http.server 8080`

### Example 4: LLM Activity with Cost Tracking and Retry

This workflow demonstrates AI-powered content moderation using an LLM with cost control and retry logic:
1. Uses Anthropic Claude to analyze user content for policy violations
2. Automatically retries on failures with exponential backoff
3. Enforces budget limits to control costs
4. Stores the moderation result in PostgreSQL

**Prerequisites:**
- Anthropic API key (set as `ANTHROPIC_API_KEY` environment variable)
- PostgreSQL database with `moderation_log` table

**Database Setup:**
```sql
CREATE TABLE moderation_log (
    id SERIAL PRIMARY KEY,
    content_id TEXT NOT NULL,
    decision TEXT NOT NULL,
    cost DECIMAL(10, 6),
    tokens INTEGER,
    moderated_at TIMESTAMPTZ NOT NULL
);
```

**Run with StreamFlow CLI:**
```bash
export ANTHROPIC_API_KEY=your-api-key-here
streamflow run examples/04-moderate-content.yaml \
  --input user_content="Check out this amazing product!" \
  --input content_id=content_12345 \
  --secret db_url=postgres://user:pass@localhost:5432/dbname
```

**Expected behavior:**
1. LLM activity analyzes the content and returns a moderation decision
2. If the API call fails, it automatically retries up to 3 times with exponential backoff (2s, 4s, 8s)
3. If the budget limit ($0.50) is reached, the workflow aborts
4. The moderation result is stored in PostgreSQL for audit trail
5. Token usage and cost information are captured in the database

**Features demonstrated:**
- ✅ LLM activity with Anthropic Claude
- ✅ Activity settings configuration (`timeout_seconds`, `retry`, `budget`)
- ✅ Exponential backoff retry strategy
- ✅ Budget enforcement with abort action
- ✅ Cost tracking (tokens and USD)
- ✅ Sequential dependencies with LLM output passing to database

### Example 5: Multi-Model LLM with Automatic Fallback and Budget-Aware Selection

This workflow demonstrates a research assistant with automatic model fallback and budget-aware provider selection:
1. Tries multiple LLM providers in order with different price points
2. **Budget-aware**: Automatically skips expensive models that exceed the budget
3. Falls back to cheaper models when budget is constrained
4. Tracks which provider/model was actually used in the response
5. Stores the result with provider metadata in PostgreSQL

**Prerequisites:**
- At least one LLM provider API key configured:
  - `ANTHROPIC_API_KEY` for Anthropic Claude models
  - `OPENAI_API_KEY` for OpenAI GPT models
  - `GOOGLE_API_KEY` for Google Gemini models
- PostgreSQL database with `research_log` table

**Database Setup:**
```sql
CREATE TABLE research_log (
    id SERIAL PRIMARY KEY,
    question TEXT NOT NULL,
    answer TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    cost DECIMAL(10, 6),
    prompt_tokens INTEGER,
    output_tokens INTEGER,
    total_tokens INTEGER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

**Run with StreamFlow CLI:**
```bash
# Configure at least one provider (or all for full fallback chain)
export OPENAI_API_KEY=your-openai-key       # Optional: o1-pro (will be skipped due to budget)
export ANTHROPIC_API_KEY=your-anthropic-key # Optional: Claude Sonnet (may be skipped)
export GOOGLE_API_KEY=your-google-key       # Recommended: Gemini Flash Lite (will fit budget)

streamflow run examples/05-research-assistant.yaml \
  --input question="What are the key differences between Rust and Go for systems programming?" \
  --secret db_url=postgres://user:pass@localhost:5432/dbname
```

**Expected behavior (with $0.01 budget):**
1. **OpenAI o1-pro** ($150/$600 per M tokens): SKIPPED - Estimated cost ~$0.615 exceeds budget
2. **Anthropic Claude Sonnet 4.5** ($3/$15 per M tokens): SKIPPED - Estimated cost ~$0.015 exceeds budget
3. **Google Gemini Flash Lite** ($0.075/$0.30 per M tokens): USED - Estimated cost ~$0.0003 fits budget
4. The response includes which provider/model was actually used (`google` / `gemini-2.0-flash-lite`)
5. **Actual cost is calculated** from token usage and stored in `cost_usd` field
6. The result is stored in PostgreSQL with full provider metadata and actual cost
7. Logs show warnings for skipped expensive models with budget reasons

**Features demonstrated:**
- ✅ Multi-model LLM fallback chain with different price points
- ✅ **Budget-aware provider selection** (skips expensive models automatically)
- ✅ Cost estimation before execution (prevents expensive API calls)
- ✅ **Actual cost tracking** - calculated from token usage and available via `{{activity.result.cost_usd}}`
- ✅ Automatic provider switching on failure or budget constraints
- ✅ Provider/model tracking in outputs
- ✅ High availability through redundancy
- ✅ Budget enforcement across providers
- ✅ Cost tracking per provider
- ✅ Support for Anthropic, OpenAI, Google, and Ollama
- ✅ Demonstrates real-world pricing from config/llm_models.yaml

### Example 5a/b/c: Single-Provider Budget-Aware Fallback

For users who only have **one API key**, we provide single-provider variants that demonstrate the same budget-aware fallback pattern:

#### Example 5a: Anthropic Only (`05a-research-assistant-anthropic.yaml`)

**Fallback chain** (with $0.01 budget):
1. **Claude Opus 4.1** ($15/$75 per M) → SKIPPED (~$0.076)
2. **Claude Sonnet 4.5** ($3/$15 per M) → SKIPPED (~$0.0153)
3. **Claude Haiku 3.5** ($0.80/$4 per M) → USED (~$0.0041)

```bash
export ANTHROPIC_API_KEY=your-key
streamflow run examples/05a-research-assistant-anthropic.yaml \
  --input question="What are the key differences between Rust and Go?" \
  --secret db_url=postgres://user:pass@localhost:5432/dbname
```

#### Example 5b: OpenAI Only (`05b-research-assistant-openai.yaml`)

**Fallback chain** (with $0.01 budget):
1. **o1** ($15/$60 per M) → SKIPPED (~$0.0615)
2. **GPT-5 Mini** ($0.25/$2 per M) → USED (~$0.00205)
3. **GPT-5 Nano** ($0.05/$0.40 per M) → Backup (~$0.00041)

```bash
export OPENAI_API_KEY=your-key
streamflow run examples/05b-research-assistant-openai.yaml \
  --input question="What are the key differences between Rust and Go?" \
  --secret db_url=postgres://user:pass@localhost:5432/dbname
```

#### Example 5c: Google Only (`05c-research-assistant-google.yaml`)

**Fallback chain** (with $0.01 budget):
1. **Gemini 3 Pro Preview** ($2/$12 per M) → SKIPPED (~$0.0122)
2. **Gemini 2.5 Flash** ($0.30/$2.50 per M) → USED (~$0.00253)
3. **Gemini 2.0 Flash Lite** ($0.075/$0.30 per M) → Backup (~$0.0003075)

```bash
export GOOGLE_API_KEY=your-key
streamflow run examples/05c-research-assistant-google.yaml \
  --input question="What are the key differences between Rust and Go?" \
  --secret db_url=postgres://user:pass@localhost:5432/dbname
```

**Note**: All three providers have models spanning different price points, so each variant demonstrates budget-aware fallback effectively!

## Next Steps

- Example 6 will demonstrate semantic caching with embeddings and RAG patterns
- Example 7 will introduce iterative workflows with loops
- Example 8 will show advanced file management with external storage

See [docs/implementation/mvp-workflows-implementation-plan.md](../docs/implementation/mvp-workflows-implementation-plan.md) for the complete implementation roadmap.
