# Quick Start

Get Kruxia Flow running in 2 minutes.

## Prerequisites

- Docker and Docker Compose

## 1. Clone and Start

```bash
git clone https://github.com/kruxia/kruxiaflow.git
cd kruxiaflow
./docker up --examples

# Wait for "listening on 0.0.0.0:8080" then verify in another terminal:
./docker exec kruxiaflow /kruxiaflow health
```

## 2. Get an Access Token

Kruxia Flow always runs with OAuth2 security, so you need client authentication to
run workflows. Use the generated client credentials to get an access token:

```bash
# Read the generated client secret from .env
CLIENT_SECRET=$(grep KRUXIAFLOW_CLIENT_SECRET .env | cut -d= -f2)

TOKEN=$(curl -s -X POST http://localhost:8080/api/v1/oauth/token \
  -d "grant_type=client_credentials" \
  -d "client_id=kruxiaflow-docker-client" \
  -d "client_secret=$CLIENT_SECRET" | jq -r '.access_token')
```

## 3. Deploy and Run a Workflow

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
      "user_content": "Check out this amazing product!",
      "content_id": "test-001"
    }
  }' | jq -r '.workflow_id'); echo $WORKFLOW_ID
```

Note: The content moderation example requires an LLM API key. Set `ANTHROPIC_API_KEY`
in your environment and restart with `./docker down && ./docker up --examples`.
For a non-LLM example, use `examples/01-weather-report.yaml` with
`definition_name: weather_report`.

## 4. Track Costs

```bash
# Check workflow status
curl -s http://localhost:8080/api/v1/workflows/$WORKFLOW_ID \
  -H "Authorization: Bearer $TOKEN" | jq .

# View cost summary
curl -s http://localhost:8080/api/v1/workflows/$WORKFLOW_ID/cost \
  -H "Authorization: Bearer $TOKEN" | jq .

# View cost breakdown per activity
curl -s http://localhost:8080/api/v1/workflows/$WORKFLOW_ID/cost/history \
  -H "Authorization: Bearer $TOKEN" | jq .
```

## Example Workflows

Kruxia Flow includes 15+ production-ready examples in `examples/` (YAML) and the
[Python SDK repo](https://github.com/kruxia/kruxiaflow-python/tree/main/examples):

| Example                     | Concepts                                    |
|-----------------------------|---------------------------------------------|
| 01-weather-report.yaml      | Sequential workflow, HTTP requests          |
| 02-user-validation.yaml     | Conditional branching, PostgreSQL           |
| 03-document-processing.yaml | Parallel execution, fan-out/fan-in          |
| 04-moderate-content.yaml    | LLM with cost tracking, retry              |
| 05-research-assistant.yaml  | Multi-model fallback, budget-aware         |
| 06a-faq-bot-caching.yaml    | Semantic caching, vector search            |
| 12_sales_etl_pipeline.py    | Python SDK, pandas, DuckDB SQL             |
| 13_customer_churn_prediction.py | Python SDK, parallel ML training        |

See the full list in [examples/README.md](../examples/README.md) and the
[Python SDK examples](https://github.com/kruxia/kruxiaflow-python/tree/main/examples).
API docs at http://localhost:8080/api/v1/docs.

## Stop Kruxia Flow

```bash
./docker down
```

## Next Steps

- [Architecture](architecture.md) - Understand the system design
- [Budget Configuration](budget-configuration.md) - Set up cost controls
- [Loops Guide](loops-guide.md) - Build iterative workflows
