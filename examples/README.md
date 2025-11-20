# StreamFlow Workflow Examples

This directory contains example workflows that demonstrate StreamFlow features progressively from simple to complex.

## Available Examples

| Example                           | Features Demonstrated                                                           | Prerequisites     |
|-----------------------------------|---------------------------------------------------------------------------------|-------------------|
| `01-weather-report.yaml`          | Sequential workflow, HTTP request (GET/POST), headers, template expressions     | Webhook URL       |
| `01b-weather-report-dynamic.yaml` | Dynamic templates, workflow input                                               | Webhook URL       |
| `02-user-validation.yaml`         | Conditional branching, PostgreSQL query, depends_on conditions                  | Database, Webhook |
| `03-document-processing.yaml`     | Parallel execution, fan-out/fan-in, multiple dependencies                       | HTTP endpoints    |
| `04-moderate-content.yaml`        | LLM activity, cost tracking, budget limits, retry with exponential backoff      | Anthropic API key, Database |

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

## Next Steps

- Example 5 will show multi-model LLM fallback patterns
- Example 6 will demonstrate semantic caching with embeddings
- Example 7 will introduce iterative workflows with loops

See [docs/implementation/mvp-workflows-implementation-plan.md](../docs/implementation/mvp-workflows-implementation-plan.md) for the complete implementation roadmap.
