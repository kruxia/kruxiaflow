# Kruxia Flow

[![CI](https://github.com/kruxia/kruxiaflow/actions/workflows/main-ci.yml/badge.svg)](https://github.com/kruxia/kruxiaflow/actions/workflows/main-ci.yml)
[![Docker Image](https://img.shields.io/docker/image-size/kruxia/kruxiaflow/latest?label=docker%20image)](https://hub.docker.com/r/kruxia/kruxiaflow)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://www.apache.org/licenses/LICENSE-2.0)
[![Discord](https://img.shields.io/discord/1457098705214640333?logo=discord&label=Discord)](https://discord.gg/ZJAzygCq)

**Budgeted workflows: durable execution with hard cost limits built in.**

Put a spending limit on your agents. Kruxia Flow runs durable workflows that **stop, downgrade, or ask a human** when the budget runs out — with every token's cost tracked in your own PostgreSQL. One 7.5 MB binary, one database, no other infrastructure.

```yaml
activities:
  - key: research
    activity_name: llm_prompt
    parameters:
      model:
        - anthropic/claude-sonnet-4-5-20250929  # preferred
        - openai/gpt-4o-mini                    # if budget is tight
      prompt: "Research and summarize: {{INPUT.topic}}"
      max_tokens: 1000
    settings:
      budget:
        limit_usd: 0.25   # hard ceiling, enforced before the call
        action: abort
```

That budget is not a metric or an alert — the engine **enforces it before spending**, falls back to cheaper models under pressure, and records what every activity actually cost.

## Why Kruxia Flow?

Agents make 3–10× more LLM calls than chatbots, and a retry loop at 2am can spend your month's budget before breakfast. The standard fix today is a gateway that caps API keys. But a key-level cap can't tell a runaway loop from a busy day, can't pause *one workflow* to ask a human, and can't tell you what a particular job cost.

Kruxia Flow is **cost-governed orchestration**: a durable execution engine — the same category as Temporal and Inngest, not a batch scheduler like Airflow — where budgets, per-token cost tracking, model fallback, and human approval gates are engine primitives, enforced per workflow:

- **Hard budgets, enforced in the engine.** Set `limit_usd` per activity or per workflow. Activities that would exceed it don't run.
- **Budget-aware model fallback.** Declare an ordered model list; the engine downgrades to cheaper models as budget tightens instead of failing.
- **Ask a human.** Workflows suspend on `wait_for_signal` — for budget approvals, review gates, or any human-in-the-loop step — and resume days later, surviving restarts while they wait.
- **Costs in your database.** Token-level splits (input / output / cache) per activity, per attempt, queryable via the cost API. Your spend data lives in your Postgres, not a vendor dashboard.
- **Durable by construction.** Event-sourced execution over PostgreSQL: crashes resume where they left off, with exactly-once semantics.

Built for teams that run LLM pipelines on their own infrastructure — including fully local with [Ollama](https://ollama.com/) — and want the spend governed as carefully as the data.

### How it compares

| Capability                        | Kruxia Flow        | Temporal      | Inngest       | LangGraph        | LLM gateways    |
|-----------------------------------|:------------------:|:-------------:|:-------------:|:----------------:|:----------------:|
| Hard budget enforcement           | **Per workflow**   | —             | —             | —                | Per API key      |
| LLM cost tracking                 | **Per token**      | —             | Partial       | via LangSmith    | Per request      |
| Budget-aware model fallback       | **Yes**            | —             | **Yes**       | Partial          | Some             |
| Suspend for human approval        | **Yes**            | **Yes**       | **Yes**       | **Yes**          | —                |
| Durable execution                 | **Yes**            | **Yes**       | **Yes**       | Partial          | —                |
| Token streaming                   | **Yes**            | —             | —             | **Yes**          | **Yes**          |
| Self-host footprint               | **1 binary + PG**  | 7+ components | 1 binary + PG | Proprietary SaaS | varies           |

Gateways cap keys; engines should cap workflows. Kruxia Flow is the only durable execution engine where the budget is a first-class primitive of the workflow itself.

## Getting Started

### 1. Start Kruxia Flow

```bash
git clone https://github.com/kruxia/kruxiaflow.git
cd kruxiaflow
./docker up --examples

# Wait for "listening on 0.0.0.0:8080" then verify in another terminal:
./docker exec kruxiaflow /kruxiaflow health
```

That's it. Kruxia Flow is running with PostgreSQL and Redis, ready to execute workflows.

### 2. Get an Access Token

Kruxia Flow always runs with OAuth2 security, so you'll need client authentication to
run workflows. The simplest approach for local running is to use the generated client
credentials to get an access token:

```bash
# Read the generated client secret from .env
CLIENT_SECRET=$(grep KRUXIAFLOW_CLIENT_SECRET .env | cut -d= -f2)

TOKEN=$(curl -s -X POST http://localhost:8080/api/v1/oauth/token \
  -d "grant_type=client_credentials" \
  -d "client_id=kruxiaflow-docker-client" \
  -d "client_secret=$CLIENT_SECRET" | jq -r '.access_token')
```

### 3. Run a Budgeted Workflow

Set your provider API key and start the server (skip the key to use only non-LLM examples):

```bash
# Set your Anthropic API key (add to ~/.bashrc or ~/.zshrc to persist)
export ANTHROPIC_API_KEY=your-key-here

# Restart the server to pick up the new key
./docker down && ./docker up -d
```

Deploy and run the content moderation example — an LLM workflow with cost tracking:

```bash
# Deploy the workflow definition
curl -s -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: text/yaml" \
  --data-binary @examples/04-moderate-content.yaml | jq .

# Submit a workflow instance
WORKFLOW_ID=$(curl -s -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "definition_name": "moderate_content",
    "input": {
      "user_content": "Check out this amazing project!",
      "content_id": "test-001"
    }
  }' | jq .workflow_id | tr -d '"'); echo $WORKFLOW_ID
```

### 4. See What It Cost

```bash
# Workflow status and result
curl -s http://localhost:8080/api/v1/workflows/$WORKFLOW_ID \
  -H "Authorization: Bearer $TOKEN" | jq .

# Cost summary for the workflow
curl -s http://localhost:8080/api/v1/workflows/$WORKFLOW_ID/cost \
  -H "Authorization: Bearer $TOKEN" | jq .

# Token-level cost breakdown per activity
curl -s http://localhost:8080/api/v1/workflows/$WORKFLOW_ID/cost/history \
  -H "Authorization: Bearer $TOKEN" | jq .
```

Prefer a non-LLM first run? Deploy `examples/01-weather-report.yaml` the same way — it
fetches a forecast and POSTs it to a webhook, no API key needed.

See `examples/` for 15+ example workflows covering parallel execution, model fallback,
caching, loops, scheduling, and RAG patterns. API docs at
http://localhost:8080/api/v1/docs.

## Key Features

### Hard Budgets, Enforced Before Spending

The built-in `llm_prompt` and `embedding` activities estimate cost in advance from
published model pricing ([config/llm_models.yaml](config/llm_models.yaml)) and refuse to
run activities that would exceed the budget. When an activity runs, actual costs and
token counts are recorded per attempt.

```yaml
activities:
  - key: analyze
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-sonnet-4-5-20250929
      prompt: "Analyze this document..."
      max_tokens: 500
    settings:
      budget:
        limit_usd: 0.50
        action: abort
```

Real-time costs are visible per workflow and per activity.

### Budget-Aware Model Fallback

Automatically fall back to cheaper models when budget is constrained:

```yaml
activities:
  - key: generate
    activity_name: llm_prompt
    parameters:
      model:
        - anthropic/claude-sonnet-4-5-20250929  # Try first
        - openai/gpt-4o-mini                    # If budget constrained
        - anthropic/claude-haiku-4-20250415     # Last resort
      prompt: "Generate a summary..."
      max_tokens: 500
    settings:
      budget:
        limit_usd: 0.10
        action: abort
```

### Human-in-the-Loop Approval Gates

Workflows can suspend and wait — for a budget sign-off, a content review, or any human
decision — then resume when signaled, even days later, surviving restarts in between:

```yaml
activities:
  - key: await_approval
    settings:
      wait_for_signal: approval
```

```bash
# Resume the waiting workflow
curl -X POST http://localhost:8080/api/v1/workflows/$WORKFLOW_ID/signal \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"signal_name": "approval", "data": {"approved": true}}'
```

### Result Caching

Save on LLM costs by caching repeated queries:

```yaml
activities:
  - key: answer
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-haiku-4-20250415
      prompt: "{{INPUT.question}}"
      max_tokens: 200
    settings:
      cache:
        enabled: true
        ttl_seconds: 3600
        key:
          - llm_prompt
          - "{{parameters.model}}"
          - "{{parameters.prompt}}"
```

Identical queries hit cache instead of the LLM. (NOTE: Semantic caching is planned.)

### Durable Execution

Workflows survive crashes and restart from where they left off. Execution is
event-sourced over PostgreSQL with exactly-once semantics — the full history of every
run, including costs and approvals, is queryable after the fact.

### Multi-Provider LLM Support

Native support for all major providers:

- **Anthropic**: Claude 4.5 Sonnet, Claude 4.5 Haiku
- **OpenAI**: GPT-5.1, GPT-4o, GPT-4o-mini, GPT-3.5 Turbo
- **Google**: Gemini Pro, Gemini Flash
- **Ollama**: Self-hosted open models — run the whole pipeline, models included, on your own hardware

## Examples

Kruxia Flow includes 15+ production-ready example workflows in YAML and Python:

| #  | Example                           | Concepts Demonstrated                              |
|----|-----------------------------------|----------------------------------------------------|
| 1  | [Weather Report][ex1]             | Sequential workflow, HTTP requests, templates      |
| 2  | [User Validation][ex2]            | Conditional branching, PostgreSQL queries          |
| 3  | [Document Processing][ex3]        | Parallel execution, fan-out/fan-in, file storage   |
| 4  | [Content Moderation][ex4]         | LLM with cost tracking, retry with backoff         |
| 5  | [Research Assistant][ex5]         | Multi-model fallback, budget-aware selection       |
| 6  | [FAQ Bot / RAG][ex6]              | Semantic caching, vector search, embeddings        |
| 7  | [Agentic Research][ex7]           | Iterative loops, agent patterns                    |
| 8  | [Scheduled Tasks][ex8]            | Delays, rate limiting, scheduled execution         |
| 9  | [Token Streaming][ex9]            | Real-time LLM streaming via WebSocket              |
| 10 | [Order Processing][ex10]          | HTTP, database transactions, email notifications   |
| 11 | [GitHub Health Check][ex11]       | Python SDK, HTTP API integration                   |
| 12 | [Sales ETL Pipeline][ex12]        | Python SDK, pandas, DuckDB SQL on DataFrames       |
| 13 | [Customer Churn Prediction][ex13] | Python SDK, parallel ML training, LLM explanations |
| 14 | [Document Intelligence][ex14]     | Python SDK, AI-powered document analysis           |
| 15 | [Content Moderation System][ex15] | Python SDK, multi-stage moderation pipeline        |

[ex1]: examples/README.md#example-1-weather-report-pipeline
[ex2]: examples/README.md#example-2-user-validation-with-conditional-branching
[ex3]: examples/README.md#example-3-multi-document-processing-pipeline
[ex4]: examples/README.md#example-4-llm-content-moderation-with-cost-tracking-and-retry
[ex5]: examples/README.md#example-5-multi-model-llm-with-budget-aware-fallback
[ex6]: examples/README.md#example-6a-faq-bot-with-semantic-caching
[ex7]: examples/README.md#example-7a-simple-agentic-research-iterative-workflows
[ex8]: examples/README.md#example-8-activity-scheduling-and-delays
[ex9]: examples/README.md#example-9a-llm-token-streaming
[ex10]: examples/README.md#example-10-order-processing
[ex11]: https://github.com/kruxia/kruxiaflow-python/blob/main/examples/11_github_health_check.py
[ex12]: https://github.com/kruxia/kruxiaflow-python/blob/main/examples/12_sales_etl_pipeline.py
[ex13]: https://github.com/kruxia/kruxiaflow-python/blob/main/examples/13_customer_churn_prediction.py
[ex14]: https://github.com/kruxia/kruxiaflow-python/blob/main/examples/14_document_intelligence.py
[ex15]: https://github.com/kruxia/kruxiaflow-python/blob/main/examples/15_content_moderation_system.py

## Architecture

Kruxia Flow is a single Rust binary with PostgreSQL as the only required dependency.
Run it on a cloud VM, on-premise, or entirely air-gapped alongside local models.

```
┌─────────────────────────────────────────────────────────────────┐
│                     Kruxia Flow (7.5MB binary)                  │
├─────────────────────────────────────────────────────────────────┤
│  API Server  │  Orchestrator  │  Worker Pool  │  Cost Tracker   │
└──────────────┴────────────────┴───────────────┴─────────────────┘
                              │
                              ▼
                    ┌───────────────────┐
                    │    PostgreSQL     │
                    │  (events, state,  │
                    │   costs, files)   │
                    └───────────────────┘
```

- **Event-driven**: Publish-subscribe architecture with exactly-once semantics
- **PostgreSQL-only**: No Kafka, Cassandra, or Elasticsearch required
- **Pluggable**: Include Redis for activity results caching
- **Planned**: Swap in Kafka for events, S3 for storage when you need scale [POST-MVP]

## Performance

Kruxia Flow is benchmarked favorably against industry-standard workflow engines (January 2026):

| Metric              | Kruxia Flow  | Temporal | Airflow |
|---------------------|--------------|----------|---------|
| Throughput (wf/sec) | **93**       | 66       | 8       |
| P99 Latency         | **0.9–1.5s** | 0.5–2.7s | 6–22s   |
| Peak Memory         | **328MB**    | 425MB    | 7.2GB   |
| Binary Size         | **7.5MB**    | ~200MB   | ~500MB+ |
| Docker Image        | **63MB**     | ~500MB   | ~1GB+   |

Benchmark methodology: Identical echo workflows (sequential, parallel, high-concurrency), Docker Compose environment, same hardware. See [`benchmarks/`](benchmarks/) for reproducible tests.

## Documentation

- **[Architecture](docs/architecture.md)** - System design and component overview
- **[MVP Requirements](docs/mvp-requirements.md)** - Product requirements and roadmap
- **[Implementation Plans](docs/implementation/)** - Detailed technical implementation specifications
- **[Post-MVP Roadmap](docs/post-mvp.md), [Features](docs/features/)** - Future features and integrations

## Development

### Prerequisites

- Docker and Docker Compose
- (Optional) Rust 1.90+ for local development

### Local Development

```bash
# Start development environment (hot reload)
./docker up --develop

# View logs
./docker logs -f

# Stop services
./docker down
```

### Running Tests

```bash
# Set up test database and run tests
./scripts/test.sh

# With coverage
./scripts/test.sh --coverage
```

### Manual Setup (without Docker)

```bash
# Install Rust and sqlx-cli
cargo install sqlx-cli --no-default-features --features postgres

# Start PostgreSQL manually
docker run -d --name pg -e POSTGRES_PASSWORD=dev -p 5432:5432 postgres:17

# Run migrations
export DATABASE_URL='postgres://postgres:dev@localhost:5432/kruxiaflow'
sqlx database create
sqlx migrate run

# Build and run
cargo build --release
./target/release/kruxiaflow serve
```

## Roadmap

### Now (Complete)
- Durable workflow execution
- Hard budgets, per-token cost tracking, budget-aware model fallback
- Human-in-the-loop workflows (`wait_for_signal` + signals API)
- Multi-provider LLM support (Anthropic, OpenAI, Google, Ollama)
- Token streaming
- 15+ example workflows
- [Python SDK](https://github.com/kruxia/kruxiaflow-python) — `pip install kruxiaflow`

### Next
- Frictionless local dev mode (no token setup)
- Rust worker SDK on crates.io
- CLI cost reports and a cost dashboard
- MCP server: agents author, run, and monitor budgeted workflows
- Semantic caching

### Later
- TypeScript SDK
- RBAC and multi-tenancy
- Kafka protocol event backend
- S3-compatible workflow storage backend
- Kubernetes Helm chart

See [Post-MVP Roadmap](docs/post-mvp.md) for details.

## Community

- **Discord**: [Join the Kruxia community](https://discord.gg/ZJAzygCq)
- **Bluesky**: [@kruxia.com](https://bsky.app/profile/kruxia.com)
- **GitHub Issues**: [Report bugs and request features](https://github.com/kruxia/kruxiaflow/issues)
- **Code of Conduct**: [Community guidelines](CODE_OF_CONDUCT.md)
- **Security**: [Report vulnerabilities](SECURITY.md)

## Contributing

Contributions are welcome! Please read our [Contributing Guidelines](CONTRIBUTING.md) before submitting PRs.

```bash
# Fork and clone
git clone https://github.com/YOUR_USERNAME/kruxiaflow.git

# Create a branch
git checkout -b feature/your-feature

# Make changes and test
./scripts/test.sh --coverage

# Submit a PR
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for detailed development setup and guidelines.

## License

Apache-2.0 — the engine and client SDKs alike. See [LICENSE](LICENSE) and
[docs/licensing-faq.md](docs/licensing-faq.md) for details.

---

**Kruxia Flow** — budgeted workflows: cost-governed orchestration for teams running LLM pipelines on their own infrastructure.
