# Kruxia Flow

[![CI](https://github.com/kruxia/kruxiaflow/actions/workflows/main-ci.yml/badge.svg)](https://github.com/kruxia/kruxiaflow/actions/workflows/main-ci.yml)
[![Docker Image](https://img.shields.io/docker/image-size/kruxia/kruxiaflow/latest?label=docker%20image)](https://hub.docker.com/r/kruxia/kruxiaflow)
[![License: AGPL v3](https://img.shields.io/badge/License-AGPL_v3-blue.svg)](https://www.gnu.org/licenses/agpl-3.0)
[![Discord](https://img.shields.io/discord/1457098705214640333?logo=discord&label=Discord)](https://discord.gg/ZJAzygCq)

**AI-native durable workflows with built-in cost controls**

The only durable workflow engine built for AI. Workflows survive crashes, budgets control AI spend, and costs are tracked per token. Ships as a 7.5 MB binary that runs anywhere.

```
Single Binary Deployment | AI Cost Tracking Built-in | Runs Anywhere
```

## Why Kruxia Flow?

Kruxia Flow is a **durable execution engine** — in the same category as Temporal and Inngest, not batch schedulers like Airflow. We're built for AI from the ground up.

### Built For

- **AI startups** — Ship AI agents to production with built-in cost tracking and budget control. Survive crashes, stop runaway spend.
- **Small businesses** — Define workflows with no code, deploy one binary and one database. No cluster, no DevOps team. Production reliability for tens of dollars a month.
- **Data teams** — Combine batch pipelines and AI agents in one platform. [Python SDK](https://github.com/kruxia/kruxiaflow-python) with pandas and DuckDB, without a 4GB footprint or a $1K/month vendor lock-in.

### The Problem

LLM costs spiral out of control, and existing tools can't help:

- **Invisible AI spend**: No workflow engine tracks LLM costs natively. Teams with cost observability report 30-50% savings, but you're left stitching together external tools with no budget control to stop a runaway agent.
- **Temporal's operational tax**: 7+ components to self-host, and teams report 8 engineering-months per year on maintenance. Zero LLM awareness.
- **LangGraph isn't a workflow engine**: Python-only, no native scheduling, and requires the proprietary LangSmith platform for production at ~$1,000 per million executions.

### The Solution

Kruxia Flow combines durable execution with AI-native features:

| Feature              | Kruxia Flow        | Temporal      | Inngest       | LangGraph        |
|----------------------|:------------------:|:-------------:|:-------------:|:----------------:|
| Durable execution    | **Yes**            | **Yes**       | **Yes**       | Partial          |
| LLM cost tracking    | **Yes**            | —             | Partial       | via LangSmith    |
| Budget control       | **Yes**            | —             | —             | —                |
| Model fallback       | **Yes**            | —             | **Yes**       | Partial          |
| Token streaming      | **Yes**            | —             | —             | **Yes**          |
| Self-host complexity | **1 binary + PG**  | 7+ components | 1 binary + PG | Proprietary      |
| Throughput           | **93 wf/s**        | 66 wf/s       | Not tested    | N/A              |
| Binary size          | **7.5 MB**         | ~200 MB       | Not tested    | N/A              |
| Docker image         | **63 MB**          | ~500 MB       | Not tested    | N/A              |
| Peak memory          | **328 MB**         | ~425 MB       | Not tested    | N/A              |
| Open source          | **AGPL-3.0 + MIT** | MIT           | SSPL          | MIT              |

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

### 3. Run a Workflow

Deploy the weather report example and run it:

```bash
# Deploy the workflow definition
curl -s -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: text/yaml" \
  --data-binary @examples/01-weather-report.yaml | jq .

# Submit a workflow instance
WORKFLOW_ID=$(curl -s -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "definition_name": "weather_report",
    "input": {"webhook_url": "https://httpbin.org/post"}
  }' | jq .workflow_id | tr -d '"'); echo $WORKFLOW_ID
```

This fetches a weather forecast from the National Weather Service API and POSTs
the result to a webhook. Copy the `workflow_id` from the response to check status:

```bash
curl -s http://localhost:8080/api/v1/workflows/$WORKFLOW_ID \
  -H "Authorization: Bearer $TOKEN" | jq .
```

If it succeeded, you've got mail! Check the weather report at http://localhost:8025/ 

### 4. Run an LLM Workflow

For AI workflows, set your provider API key in your shell environment and restart:

```bash
# Set your Anthropic API key (add to ~/.bashrc or ~/.zshrc to persist)
export ANTHROPIC_API_KEY=your-key-here

# Restart the server to pick up the new key
./docker down && ./docker up -d
```

Then deploy and run the content moderation example:

```bash
# Deploy the moderation workflow
curl -s -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: text/yaml" \
  --data-binary @examples/04-moderate-content.yaml | jq .

# Submit a moderation request
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

Check workflow result and cost tracking with the `workflow_id` from the response:

```bash
curl -s http://localhost:8080/api/v1/workflows/$WORKFLOW_ID \
  -H "Authorization: Bearer $TOKEN" | jq .

# View cost summary for the workflow
curl -s http://localhost:8080/api/v1/workflows/$WORKFLOW_ID/cost \
  -H "Authorization: Bearer $TOKEN" | jq .

# View cost breakdown for the workflow activities
curl -s http://localhost:8080/api/v1/workflows/$WORKFLOW_ID/cost/history \
  -H "Authorization: Bearer $TOKEN" | jq .
```

See `examples/` for 15+ example workflows covering parallel execution, model fallback,
caching, loops, scheduling, and RAG patterns. API docs at
http://localhost:8080/api/v1/docs.

## Key Features

### Built-in LLM Cost Tracking

The std (built-in) `llm_prompt` and `embedding` activities help you control costs: They estimate costs in advance using the published cost data for LLM models (stored in [config/llm_models.yaml](config/llm_models.yaml)) and will only run activities that won't exceed the budget, if provided. Then when the LLM activity is run, the costs and token counts are recorded so that cost metrics can be analyzed and workflows optimized.

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

Workflows survive crashes and restart from where they left off.

### Multi-Provider LLM Support

Native support for all major providers:

- **Anthropic**: Claude 4.5 Sonnet, Claude 4.5 Haiku
- **OpenAI**: GPT-5.1, GPT-4o, GPT-4o-mini, GPT-3.5 Turbo
- **Google**: Gemini Pro, Gemini Flash
- **Ollama**: Self-hosted open models

## Examples

Kruxia Flow includes 15+ production-ready example workflows in YAML and Python:

| #  | Example                         | Concepts Demonstrated                              |
|----|---------------------------------|----------------------------------------------------|
| 1  | [Weather Report][ex1]           | Sequential workflow, HTTP requests, templates      |
| 2  | [User Validation][ex2]          | Conditional branching, PostgreSQL queries          |
| 3  | [Document Processing][ex3]      | Parallel execution, fan-out/fan-in, file storage   |
| 4  | [Content Moderation][ex4]       | LLM with cost tracking, retry with backoff         |
| 5  | [Research Assistant][ex5]       | Multi-model fallback, budget-aware selection       |
| 6  | [FAQ Bot / RAG][ex6]            | Semantic caching, vector search, embeddings        |
| 7  | [Agentic Research][ex7]         | Iterative loops, agent patterns                    |
| 8  | [Scheduled Tasks][ex8]          | Delays, rate limiting, scheduled execution         |
| 9  | [Token Streaming][ex9]          | Real-time LLM streaming via WebSocket              |
| 10 | [Order Processing][ex10]        | HTTP, database transactions, email notifications   |
| 11 | [GitHub Health Check][ex11]     | Python SDK, HTTP API integration                   |
| 12 | [Sales ETL Pipeline][ex12]      | Python SDK, pandas, DuckDB SQL on DataFrames       |
| 13 | [Customer Churn Prediction][ex13] | Python SDK, parallel ML training, LLM explanations |
| 14 | [Document Intelligence][ex14]   | Python SDK, AI-powered document analysis           |
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

Kruxia Flow is a single Rust binary with PostgreSQL as the only required dependency. Runs anywhere: on cloud VMs, on-premise computers, or edge devices like Raspberry Pi Zero.

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
- 15+ example workflows
- LLM cost tracking and budgets
- Multi-provider LLM support
- Token streaming
- Human-in-the-loop workflows
- [Python SDK](https://github.com/kruxia/kruxiaflow-python) (install from GitHub)

### Next
- Semantic caching
- Web dashboard for cost visualization
- Airflow migration guide
- Kubernetes Helm chart

### Later
- TypeScript SDK
- RBAC and multi-tenancy
- Kafka protocol event backend
- S3-compatible workflow storage backend

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

AGPL-3.0 License - See [LICENSE](LICENSE) for details.

---

**Kruxia Flow** - AI-native durable workflows that run everywhere, with built-in LLM cost controls and streaming.
