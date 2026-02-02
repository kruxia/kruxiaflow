# Quick Start

Get Kruxia Flow running in 60 seconds.

## Prerequisites

- Docker and Docker Compose

## Start Kruxia Flow

```bash
# Clone and start
git clone https://github.com/kruxia/kruxiaflow.git
cd kruxiaflow
./docker up -d
./docker logs -f

# Wait for services to be healthy (~30 seconds)
docker compose ps

# API is ready at http://localhost:8080
curl http://localhost:8080/health
```

## Verify Installation

```bash
# Check health
curl http://localhost:8080/health
# {"status":"ok"}

# View API documentation
open http://localhost:8080/api/v1/docs
```

## Your First Workflow

Create a simple workflow that fetches weather data:

```bash
# Get an auth token
TOKEN=$(curl -s -X POST http://localhost:8080/api/v1/oauth/token \
  -d "grant_type=client_credentials" \
  -d "client_id=kruxiaflow-docker-client" \
  -d "client_secret=kruxiaflow-dev-secret" | jq -r '.access_token')

# Submit a workflow
curl -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "definition_name": "weather_report",
    "input": {"city": "San Francisco"}
  }'
```

## Example Workflows

Kruxia Flow includes 10 production-ready examples in the `examples/` directory:

| Example                    | Concepts                                    |
|----------------------------|---------------------------------------------|
| 01-weather-report.yaml     | Sequential workflow, HTTP requests          |
| 02-user-validation.yaml    | Conditional branching, PostgreSQL           |
| 03-document-processing.yaml| Parallel execution, fan-out/fan-in          |
| 04-moderate-content.yaml   | LLM with cost tracking, retry               |
| 05-research-assistant.yaml | Multi-model fallback, budget-aware          |
| 06a-faq-bot-caching.yaml   | Semantic caching, vector search             |

## Stop Kruxia Flow

```bash
./docker down
```

## Next Steps

- [Architecture](architecture.md) - Understand the system design
- [Budget Configuration](budget-configuration.md) - Set up cost controls
- [Loops Guide](loops-guide.md) - Build iterative workflows
