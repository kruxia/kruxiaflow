# Quick Start

Run your first **budgeted workflow** in under 5 minutes. You need Docker and
(optionally) an LLM API key — or [Ollama](https://ollama.com/) for a fully
local run.

## 1. Start Kruxia Flow

No clone required — fetch the compose file (it is the same `docker-compose.yml`
at the top of the repository) and start it in insecure dev mode:

```bash
curl -fsSL https://raw.githubusercontent.com/kruxia/kruxiaflow/main/docker-compose.yml -o docker-compose.yml
KRUXIAFLOW_INSECURE_DEV=true ANTHROPIC_API_KEY=your-key-here docker compose up -d
```

`OPENAI_API_KEY`, `GOOGLE_API_KEY`, and `OLLAMA_BASE_URL` work the same way —
set any of them, or none.

This runs Kruxia Flow (API + orchestrator + worker) and PostgreSQL. Two
one-shot init containers generate a local RSA keypair and fetch the LLM
pricing catalog used for budget enforcement.

> **Local evaluation only.** `KRUXIAFLOW_INSECURE_DEV=true` accepts
> unauthenticated requests. The API port is published on `127.0.0.1`, so
> nothing is reachable from your network. Before deploying anywhere real,
> leave the flag unset and configure real credentials — see the comments in
> the compose file.

```bash
# Verify it's up
curl -s http://localhost:8080/health
```

## 2. Run a Budgeted Workflow

Deploy a workflow with a **hard budget** — the engine estimates each LLM call
against published pricing and refuses to exceed the limit. No auth headers
needed in dev mode:

```bash
curl -s -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Content-Type: text/yaml" --data-binary @- <<'YAML'
name: quickstart_research
activities:
  - key: research
    worker: std
    activity_name: llm_prompt
    parameters:
      model:
        - anthropic/claude-sonnet-5      # preferred
        - openai/gpt-5.4-mini            # if budget is tight
      prompt: "In three concise bullet points: {{INPUT.topic}}"
      max_tokens: 500
    settings:
      budget:
        limit: 0.25
        action: abort
YAML
```

Submit it:

```bash
WORKFLOW_ID=$(curl -s -X POST http://localhost:8080/api/v1/workflows \
  -H "Content-Type: application/json" \
  -d '{"definition_name": "quickstart_research",
       "input": {"topic": "why do LLM agent costs spiral?"}}' \
  | jq -r .workflow_id); echo $WORKFLOW_ID
```

## 3. See What It Cost

```bash
# Status and the answer
curl -s http://localhost:8080/api/v1/workflows/$WORKFLOW_ID | \
  jq -r '.status, .activities[].outputs[]?.value.content'

# Cost summary — the payoff of that budget line
curl -s http://localhost:8080/api/v1/workflows/$WORKFLOW_ID/cost | jq .

# Token-level breakdown per activity (provider/model actually used, cached tokens)
curl -s http://localhost:8080/api/v1/workflows/$WORKFLOW_ID/cost/history | jq .
```

Using Ollama instead? Point `OLLAMA_BASE_URL` at your Ollama server (from
Docker: `http://host.docker.internal:11434`) and use an `ollama/...` model id
from `config/llm_models.yaml` — the whole pipeline runs locally.

> **Image tags**: `kruxia/kruxiaflow:latest` always equals the newest release —
> CI moves it only when a release is tagged, never for in-between `main`
> builds. For production deployments, pin an explicit version
> (e.g. `kruxia/kruxiaflow:0.7.0`) so upgrades happen on your schedule.

## Working from a checkout

The quickstart file is the repo's root `docker-compose.yml`. In a checkout:

```bash
git clone https://github.com/kruxia/kruxiaflow.git && cd kruxiaflow

./docker up              # full stack, pulled images, generated secrets
./docker up --examples   # + example workers, Mailpit, examples database
docker compose up        # development: builds from source via docker-compose.override.yml
```

The `./docker` script generates real random secrets into `.env` on first run
and enables the redis cache profile; see the compose file comments for the
knobs (`KRUXIAFLOW_PORT`, `COMPOSE_PROFILES=cache`, and friends).

## Stop Kruxia Flow

```bash
docker compose down        # keep data
docker compose down -v     # delete data volumes too
```

## Next Steps

- [Architecture](architecture.md) - Understand the system design
- [Budget Configuration](budget-configuration.md) - Set up cost controls
- [Loops Guide](loops-guide.md) - Build iterative workflows
