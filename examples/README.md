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
| `06a-faq-bot-caching.yaml`            | Semantic caching for LLM responses, cache hit tracking, cost savings                           | Anthropic API key, PostgreSQL, Redis (optional) |
| `06b-rag-index-builder.yaml`          | Embedding generation, pgvector indexing, batch processing, parallel storage                    | OpenAI API key, PostgreSQL with pgvector, Webhook |
| `06c-rag-query.yaml`                  | Complete RAG pattern: embed → search → augment → generate, MiniJinja loops                     | OpenAI + Anthropic API keys, PostgreSQL with pgvector, Webhook |
| `07a-agentic-research-simple.yaml`    | Iterative workflows (loops), iteration-scoped storage, budget-aware loops, conditional exit    | Anthropic API key  |
| `07b-agentic-research-complete.yaml` | Complete iterative research: HTTP search, file iteration storage, dual success/failure paths   | Anthropic API key, Search API, Webhook URL |
| `08a-rate-limited-api-calls.yaml`    | Activity scheduling with `delay`, rate limiting, sequential delays (5s between calls)          | API key, Webhook URL |
| `08b-scheduled-daily-report.yaml`    | Activity scheduling with `scheduled_for`, absolute timestamps, ISO 8601, template scheduling   | Anthropic API key, Webhook URL |
| `08c-delayed-reminders.yaml`         | Cascading delays, escalating reminders (1h → 4h → 24h), multi-webhook notifications           | Webhook URLs |

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

### Example 6a: FAQ Bot with Semantic Caching

This workflow demonstrates semantic caching for LLM responses with dramatic cost reduction for repeated questions:
1. Answers user questions using Claude Haiku with aggressive caching
2. Tracks cache hits/misses for cost analysis
3. Logs all interactions with cache metrics

**Prerequisites:**
- Anthropic API key (Claude Haiku 4)
- PostgreSQL database with faq_log table
- Optional: Redis for distributed caching (falls back to in-memory)

**Database Setup:**
```sql
CREATE TABLE faq_log (
    id SERIAL PRIMARY KEY,
    question TEXT NOT NULL,
    answer TEXT NOT NULL,
    cost DECIMAL(10, 6),
    cache_hit BOOLEAN NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

**Redis Setup (optional, for production):**
```bash
docker run -d -p 6379:6379 redis:7-alpine
export STREAMFLOW_CACHE_PROVIDER=redis
export STREAMFLOW_REDIS_URL=redis://localhost:6379
```

**Run with StreamFlow CLI:**
```bash
export ANTHROPIC_API_KEY=your-key
streamflow run examples/06a-faq-bot-caching.yaml \
  --input question="What are your business hours?" \
  --secret db_url=postgres://user:pass@localhost:5432/dbname
```

**Expected behavior:**

**First question**: "What are your business hours?"
1. **answer_question**: LLM call → cache miss → costs $0.001
2. **store_answer**: Logs question, answer, cost=$0.001, cache_hit=false

**Second identical question** (within 1 hour): "What are your business hours?"
1. **answer_question**: Cache hit → costs $0.000 (100% savings!)
2. **store_answer**: Logs question, answer, cost=$0.000, cache_hit=true

**Cost Analysis (100 questions, 70% cache hit rate):**
- Without caching: 100 × $0.001 = **$0.100**
- With caching: (30 × $0.001) + (70 × $0.000) = **$0.030**
- **Savings: $0.070 (70% reduction!)**

**Features demonstrated:**
- ✅ Semantic caching for LLM responses (US-3.5, US-5.3)
- ✅ Cache configuration: enabled, ttl_seconds, key
- ✅ Cache hit tracking in outputs ({{answer_question.result.cache_hit}})
- ✅ Cost tracking with cache awareness
- ✅ Budget enforcement
- ✅ Cache key generation from parameters

### Example 6b: RAG Index Builder

This workflow demonstrates embedding generation and vector index building for RAG:
1. Generates embeddings for document chunks using OpenAI
2. Stores chunks with embeddings in PostgreSQL with pgvector
3. Executes parallel storage for efficiency
4. Sends completion notification

**Prerequisites:**
- OpenAI API key for embedding generation
- PostgreSQL with pgvector extension
- Webhook URL for completion notification

**Database Setup:**
```sql
-- Enable pgvector extension
CREATE EXTENSION IF NOT EXISTS vector;

-- Document chunks table with vector column
CREATE TABLE document_chunks (
    id SERIAL PRIMARY KEY,
    content TEXT NOT NULL,
    embedding vector(1536) NOT NULL,  -- OpenAI text-embedding-3-small dimension
    metadata JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Create vector similarity index (IVFFlat for cosine distance)
CREATE INDEX document_chunks_embedding_idx ON document_chunks
USING ivfflat (embedding vector_cosine_ops)
WITH (lists = 100);
```

**Run with StreamFlow CLI:**
```bash
export OPENAI_API_KEY=your-key
streamflow run examples/06b-rag-index-builder.yaml \
  --input chunks='["Rust is a systems programming language...","Rust'\''s ownership model ensures memory safety...","Cargo is Rust'\''s package manager..."]' \
  --input source="rust_documentation" \
  --input notification_webhook_url=https://webhook.site/your-id \
  --secret db_url=postgres://user:pass@localhost:5432/dbname
```

**Expected behavior:**
1. **generate_embeddings**: Sends 3 chunks to OpenAI → receives 3 × 1536-dimensional vectors (~$0.00002)
2. **store_chunk_1, store_chunk_2, store_chunk_3**: Execute in parallel, storing chunks with embeddings
3. **confirm_indexing**: Waits for all storage to complete (fan-in), POSTs notification with metadata

**Total cost**: ~$0.00002 (embedding only, storage is free)
**Total time**: ~1-2 seconds

**Features demonstrated:**
- ✅ embedding_generate activity (US-5.1)
- ✅ Batch embedding processing
- ✅ Array template expressions: {{generate_embeddings.embeddings.embeddings[0]}}
- ✅ Vector type support: $2::vector casting in PostgreSQL
- ✅ JSONB metadata storage
- ✅ Parallel execution (fan-out: 1 → 3)
- ✅ Fan-in synchronization (3 → 1)
- ✅ pgvector similarity index creation

### Example 6c: RAG Query and Q&A

This workflow demonstrates the complete RAG (Retrieval-Augmented Generation) pattern:
1. Embeds user question for semantic search
2. Searches knowledge base using pgvector similarity
3. Passes retrieved context to LLM
4. Generates grounded answer
5. Logs Q&A with cost tracking

**Prerequisites:**
- OpenAI API key for embedding generation
- Anthropic API key for LLM (Claude Sonnet)
- PostgreSQL with pgvector and populated document_chunks table
- Run 06b-rag-index-builder.yaml first to populate the index
- Webhook URL for response delivery

**Database Setup (includes qa_log table):**
```sql
-- Same document_chunks table as 06b, plus:
CREATE TABLE qa_log (
    id SERIAL PRIMARY KEY,
    question TEXT NOT NULL,
    answer TEXT NOT NULL,
    chunks_used INTEGER NOT NULL,
    embedding_cost DECIMAL(10, 6),
    llm_cost DECIMAL(10, 6),
    total_cost DECIMAL(10, 6),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

**Run with StreamFlow CLI:**
```bash
export OPENAI_API_KEY=your-key
export ANTHROPIC_API_KEY=your-key
streamflow run examples/06c-rag-query.yaml \
  --input question="What is Rust's ownership model?" \
  --input response_webhook_url=https://webhook.site/your-id \
  --secret db_url=postgres://user:pass@localhost:5432/dbname
```

**Expected behavior:**
1. **embed_question**: Converts question to 1536-dimensional vector (~$0.000002)
2. **search_similar_chunks**: Uses pgvector cosine distance (<=>), finds top 3 similar chunks
3. **generate_answer**: LLM generates answer using retrieved context (~$0.015)
4. **store_qa_result**: Logs Q&A with cost breakdown
5. **send_response**: POSTs answer to webhook with metadata

**Total cost**: ~$0.015002 (embedding + LLM)
**Total time**: ~2-3 seconds

**RAG Pattern (4 Steps):**
1. **Embed**: Convert question to vector
2. **Search**: Find top-k most similar chunks (cosine distance)
3. **Augment**: Pass retrieved chunks as context to LLM
4. **Generate**: LLM produces grounded answer

**Features demonstrated:**
- ✅ Complete RAG workflow pattern
- ✅ embedding_generate for question embedding
- ✅ pgvector similarity search: embedding <=> $1::vector
- ✅ MiniJinja loops in prompts: {% for chunk in search_similar_chunks.result.rows %}
- ✅ Template expressions for complex data access (chunk.content, chunk.metadata.source)
- ✅ Cost tracking across multiple activities
- ✅ Sequential data flow with dependencies

**Combining 06a + 06c (RAG with Caching):**
Add cache settings to embed_question and generate_answer activities to reduce costs by 50-70% for repeated queries.

**Related Documentation:**
- Semantic Caching: `docs/features/semantic-caching.md`
- Multi-Provider LLM: `docs/implementation/US-5.1-multi-provider-llm.md`
- Embedding Generation: `docs/implementation/US-5.1-multi-provider-llm.md`

### Example 7a: Simple Agentic Research (Iterative Workflows with Loops)

This workflow demonstrates iterative/looping workflows (US-3.4) with a simplified, self-contained example:
1. Initializes research plan with strategy and success criteria
2. Performs iterative "research" using LLM (simulated search, no external APIs)
3. Evaluates after each iteration if information is sufficient
4. Loops back for more research if insufficient (back-edge dependency)
5. Compiles final report when sufficient information is gathered

**Prerequisites:**
- Anthropic API key (Claude Haiku 4 and Sonnet 4.5)

**Run with StreamFlow CLI:**
```bash
export ANTHROPIC_API_KEY=your-key
streamflow run examples/07a-agentic-research-simple.yaml \
  --input topic="Benefits of Rust for systems programming"
```

**Expected behavior:**
1. **initialize** creates research plan (~$0.001)
2. **perform_search** iteration 0: First research pass (~$0.005)
3. **evaluate** iteration 0: Returns "CONTINUE" (~$0.001)
4. **perform_search** iteration 1: Second research pass, builds on iteration 0 (~$0.005)
5. **evaluate** iteration 1: Returns "CONTINUE" (~$0.001)
6. **perform_search** iteration 2: Third research pass (~$0.005)
7. **evaluate** iteration 2: Returns "SUFFICIENT" (~$0.001)
8. **compile_report**: Synthesizes all findings from 3 iterations (~$0.020)
9. Total cost: ~$0.039, Total iterations: 3

**Loop exit conditions:**
- Normal: `evaluate` returns "SUFFICIENT" (condition-based)
- Safety: Reaches 5 iterations (iteration_limit)
- Budget: Exceeds $0.05 for perform_search (budget-based)
- Error: Any activity fails

**Features demonstrated:**
- ✅ Loop execution via back-edge (`evaluate → perform_search`)
- ✅ Iteration-scoped storage (`iteration_scoped: true`)
- ✅ Access all iteration results (`{{perform_search.findings}}` array)
- ✅ Access latest iteration (`{{evaluate.result.content | last}}`)
- ✅ Iteration counter (`{{ACTIVITY.iteration}}`)
- ✅ Budget accumulation across iterations
- ✅ Remaining budget tracking (`{{ACTIVITY.remaining_budget_usd}}`)
- ✅ Conditional loop back (checking for "CONTINUE")
- ✅ Conditional loop exit (checking for "SUFFICIENT")
- ✅ Maximum iterations limit (`iteration_limit: 5`)

**Why "Simple"?**
- Uses only LLM activities (no HTTP search APIs)
- Uses string matching for conditions (no structured field extraction)
- Single success path (no explicit failure handling)
- Inline JSON results (no file storage)
- Can run immediately with just Anthropic API key

### Example 7b: Complete Agentic Research (Production-Ready Pattern)

This workflow demonstrates the **full Example 7 specification** from the implementation plan:
1. Performs real HTTP-based research using external search APIs
2. Stores large search results as files with iteration scoping
3. Uses structured field extraction from LLM outputs
4. Implements dual success/failure paths
5. Demonstrates advanced array access patterns and complex boolean conditions

**Prerequisites:**
- Anthropic API key (Claude Haiku 4 and Sonnet 4.5)
- Search API key (SerpAPI, Brave Search, or custom search service)
- Webhook URL to receive final results

**Run with StreamFlow CLI:**
```bash
export ANTHROPIC_API_KEY=your-key
streamflow run examples/07b-agentic-research-complete.yaml \
  --input research_topic="Impact of quantum computing on cryptography" \
  --input publish_url=https://webhook.site/your-id \
  --secret search_api_key=your-search-api-key
```

**Expected behavior (success scenario):**
1. **search_information** iteration 0: HTTP search API call → results stored as file (~$0.10)
2. **evaluate_sufficiency** iteration 0: Returns `sufficient=false` (~$0.001)
3. **search_information** iteration 1: Refined search → results file 1 (~$0.10)
4. **evaluate_sufficiency** iteration 1: Returns `sufficient=false` (~$0.001)
5. **search_information** iteration 2: Comprehensive search → results file 2 (~$0.10)
6. **evaluate_sufficiency** iteration 2: Returns `sufficient=true` (~$0.001)
7. **compile_report**: Synthesizes all 3 iterations from file storage (~$0.020)
8. **publish_success**: POSTs report file + metadata to webhook (~$0.00)
9. Total cost: ~$0.323, Total iterations: 3

**Expected behavior (failure scenario - budget exhausted):**
1-4. [Same through iteration 1]
5. **evaluate_sufficiency** iteration 4: `sufficient=false`, `remaining_budget_usd=$0.007` (~$0.001)
6. Loop exits (budget < $0.10 threshold)
7. **publish_failure**: POSTs failure report with gaps analysis (~$0.00)
8. Total cost: ~$0.493, Total iterations: 4

**Features demonstrated (beyond 07a):**
- ✅ Real HTTP-based research (not LLM simulation)
- ✅ File storage with iteration scoping (`type: file`, `iteration_scoped: true`)
- ✅ Structured field extraction (`{{evaluate.sufficient}}` not string matching)
- ✅ Array access with bracket notation (`{{search_information[*].results}}`)
- ✅ Dual success/failure paths (`publish_success` and `publish_failure`)
- ✅ Complex boolean conditions (`AND`, `<`, `>`)
- ✅ Budget checks in conditions (`{{evaluate.remaining_budget_usd}} > 0.10`)
- ✅ File uploads in HTTP requests (`files:` parameter)
- ✅ Production-ready error handling

**Differences from 07a:**

| Feature                  | 07a (Simple)                | 07b (Complete)                       |
|--------------------------|-----------------------------|--------------------------------------|
| Research activity        | LLM simulates research      | Real HTTP search API calls           |
| Loop condition syntax    | String matching             | Structured field access              |
| Results storage          | Inline JSON                 | Files with iteration scoping         |
| Array access             | Implicit (Jinja filters)    | Explicit ([*] bracket notation)      |
| Failure handling         | Implicit (timeout/error)    | Explicit publish_failure activity    |
| Budget checks            | Activity-level only         | Condition-level remaining checks     |
| External dependencies    | None (LLM-only)             | Search API + webhook endpoint        |
| Production readiness     | Educational demo            | Production-ready pattern             |

**Note**: Adapt the search API configuration to your provider:
- **SerpAPI**: `url: "https://serpapi.com/search"`, add `api_key` to query params
- **Brave Search**: `url: "https://api.search.brave.com/res/v1/web/search"`
- **Custom**: Any search service that returns JSON results

---

### Example 8: Activity Scheduling and Delays

Example 8 demonstrates activity scheduling features using both relative delays (`delay`) and absolute timestamps (`scheduled_for`).

#### Example 8a: Rate-Limited API Calls (`08a-rate-limited-api-calls.yaml`)

This workflow demonstrates using `delay` to respect API rate limits by spacing out sequential HTTP requests.

**Use Case**: Make multiple API calls while respecting rate limits (e.g., 1 request per 5 seconds)

**Prerequisites:**
- API key for the API service
- Webhook URL to receive aggregated results

**Run with StreamFlow CLI:**
```bash
streamflow run examples/08a-rate-limited-api-calls.yaml \
  --input webhook_url=https://webhook.site/your-unique-id \
  --secret api_key=your-api-key
```

**Expected behavior:**
1. First API call executes immediately (page 1)
2. Second API call waits 5 seconds, then executes (page 2)
3. Third API call waits another 5 seconds, then executes (page 3)
4. Results are aggregated and sent to webhook after all calls complete
5. Total workflow time: ~10 seconds (5s + 5s delays)

**Features demonstrated:**
- ✅ `settings.delay` for relative time delays
- ✅ Duration units: seconds (`"5s"`)
- ✅ Sequential delays with `depends_on` chains
- ✅ Rate limiting pattern for API compliance
- ✅ Result aggregation from multiple delayed activities

**Supported delay units:**
- `ms` - milliseconds: `"500ms"`
- `s` - seconds: `"5s"`
- `m` or `mi` - minutes: `"30m"` or `"30mi"`
- `h` - hours: `"2h"`
- `d` - days: `"7d"`
- `w` - weeks: `"1w"`
- `mo` - months: `"2mo"` (calendar-aware)
- `y` - years: `"1y"` (handles leap years)

**Timing diagram:**
```
t=0s     call_api_1 ━━━┓
                        │
t=5s                    └─→ call_api_2 ━━━┓
                                           │
t=10s                                      └─→ call_api_3 ━━━┓
                                                               │
t=10s+                                                         └─→ aggregate_results
```

---

#### Example 8b: Scheduled Daily Report (`08b-scheduled-daily-report.yaml`)

This workflow demonstrates using `scheduled_for` to execute activities at specific absolute times.

**Use Case**: Generate and send daily reports at a scheduled time (e.g., 9 AM Pacific)

**Prerequisites:**
- Anthropic API key
- Webhook URL to receive the report

**Run with StreamFlow CLI:**
```bash
# Schedule for a future timestamp (ISO 8601 format with timezone)
streamflow run examples/08b-scheduled-daily-report.yaml \
  --input report_date="2025-12-01" \
  --input report_time="2025-12-01T09:00:00-08:00" \
  --input notification_webhook="https://webhook.site/your-unique-id" \
  --secret anthropic_api_key=your-api-key
```

**Expected behavior:**
1. Workflow is submitted immediately
2. `generate_report` activity waits until `report_time` (9 AM Pacific)
3. At scheduled time, LLM generates the daily report
4. Report is sent to webhook immediately after generation
5. Budget limit of $0.10 enforced

**Features demonstrated:**
- ✅ `settings.scheduled_for` for absolute timestamps
- ✅ ISO 8601 timestamp format with timezone
- ✅ Template resolution in `scheduled_for`: `"{{INPUT.report_time}}"`
- ✅ Scheduled LLM execution with budget limits
- ✅ Workers respect scheduling (won't claim early)

**Timestamp formats supported:**
- UTC: `"2025-12-01T09:00:00Z"`
- With timezone offset: `"2025-12-01T09:00:00-08:00"`
- Microsecond precision: `"2025-12-01T09:00:00.123456Z"`

**Notes:**
- If `scheduled_for` is in the past, activity executes immediately (with warning logged)
- Timezone is converted to UTC for storage
- Template variables can provide dynamic scheduling: `"{{INPUT.deadline}}"`

---

#### Example 8c: Delayed Reminder System (`08c-delayed-reminders.yaml`)

This workflow demonstrates cascading delays for escalating notifications.

**Use Case**: Send escalating reminders at increasing intervals (1 hour → 4 hours → 24 hours)

**Prerequisites:**
- Webhook URLs for user, manager, and on-call notifications

**Run with StreamFlow CLI:**
```bash
streamflow run examples/08c-delayed-reminders.yaml \
  --input task_name="Complete quarterly report" \
  --input assigned_user="alice@example.com" \
  --input user_webhook="https://webhook.site/user-id" \
  --input manager_webhook="https://webhook.site/manager-id" \
  --input oncall_webhook="https://webhook.site/oncall-id"
```

**Expected behavior:**
1. Initial notification sent immediately
2. First reminder sent after 1 hour
3. Escalated reminder sent after 4 hours total (3h from first reminder)
4. Final critical escalation sent after 24 hours total (20h from escalated)
5. Each notification goes to appropriate recipient (user → manager → on-call)

**Features demonstrated:**
- ✅ Cascading delays with different intervals
- ✅ Duration units: hours (`"1h"`, `"3h"`, `"20h"`)
- ✅ Multi-stage escalation pattern
- ✅ Different notification targets for each stage
- ✅ Priority escalation over time

**Timing diagram:**
```
t=0h     send_initial_notification ━━┓
                                      │
t=1h                                  └─→ send_first_reminder ━━┓
                                                                 │
t=4h                                                             └─→ send_escalated_reminder ━━┓
                                                                                                │
t=24h                                                                                           └─→ send_final_escalation
```

**Common patterns:**
- **Immediate retry**: `delay: "500ms"` (sub-second)
- **Short delays**: `delay: "5s"` or `"30s"`
- **Medium delays**: `delay: "5m"` or `"1h"`
- **Long delays**: `delay: "7d"` or `"1w"`
- **Template-based**: `delay: "{{INPUT.delay_minutes}}m"` (dynamic)

**Delay vs. Scheduled_for:**

| Feature          | `delay` (relative)                     | `scheduled_for` (absolute)           |
|------------------|----------------------------------------|--------------------------------------|
| Use case         | Wait for duration after ready          | Execute at specific time             |
| Format           | Duration string (`"5s"`, `"2h"`)       | ISO 8601 timestamp                   |
| Reference point  | When activity becomes ready            | Clock time                           |
| Template example | `"{{INPUT.wait_time}}m"`               | `"{{INPUT.deadline}}"`               |
| Typical use      | Rate limiting, retries, staged delays  | Scheduled reports, future execution  |

---

## Next Steps

- Example 10 will show advanced file management with external storage (S3-compatible)

See [docs/implementation/mvp-workflows-implementation-plan.md](../docs/implementation/mvp-workflows-implementation-plan.md) for the complete implementation roadmap.
