# Client CLI: The 4th Component of the Kruxia Flow Binary

**Date**: 2026-02-07
**Status**: Proposal
**Priority**: P1 (High — developer experience and operational productivity)

## Overview

The `kruxiaflow` binary currently serves three roles:

| Component      | Commands                             | Purpose                         |
|----------------|--------------------------------------|---------------------------------|
| API Server     | `api`                                | HTTP/REST endpoints             |
| Orchestrator   | `orchestrator`                       | Workflow evaluation & scheduling|
| Worker         | `worker`                             | Built-in activity execution     |
| *(combined)*   | `serve`                              | All three in one process        |

This proposal adds a fourth role: a **client CLI** that gives developers and operators
a terminal-native interface for the full workflow lifecycle — from authoring to
deployment to cost analysis. Some commands wrap the REST API; others use internal
libraries directly for offline use.

### Current CLI Commands (already implemented)

Server/infrastructure:
- `kruxiaflow serve` — Launch all services together
- `kruxiaflow api` / `orchestrator` / `worker` — Individual service launchers

Operations (via API):
- `kruxiaflow health` — Check service health
- `kruxiaflow status` — Show detailed service status

Info (local):
- `kruxiaflow version` — Version and build info

Database (direct DB — to be grouped under `kruxiaflow db`):
- `kruxiaflow migrate` — Database migrations
- `kruxiaflow seed-llm` — Load LLM model catalog
- `kruxiaflow seed-client` — Seed OAuth client credentials
- `kruxiaflow profile` — PostgreSQL performance profiling

### Design Principles

1. **Offline where possible**: `validate`, `visualize`, and `compile` work without a
   running server or database — they use the same parsing and validation libraries as
   the server.
2. **API where necessary**: `workflow`, `activity`, and `cost` commands authenticate
   via OAuth2 and call the REST API — the same API that the Python SDK and HTTP
   clients use.
3. **Direct DB for admin**: `admin` commands operate directly on the database for
   bootstrap scenarios where the API server may not be running.
4. **Consistent output**: All commands support `--format table|json|csv` (where
   applicable) for both human use and scripting/CI.
5. **Shell completion**: All commands support bash, zsh, and fish completions.

---

## Reorganization: `kruxiaflow db`

The four existing commands that connect directly to PostgreSQL via `DATABASE_URL`
are grouped under a `db` subcommand. They share the same access pattern (direct DB,
no API server required) and the same audience (operators doing setup/maintenance).

```bash
# Before (current)                    # After
kruxiaflow migrate                    kruxiaflow db migrate
kruxiaflow migrate --status           kruxiaflow db migrate --status
kruxiaflow seed-llm config/...        kruxiaflow db seed-llm config/...
kruxiaflow seed-client                kruxiaflow db seed-client
kruxiaflow profile                    kruxiaflow db profile
kruxiaflow profile --explain          kruxiaflow db profile --explain
```

All other existing commands (`serve`, `api`, `orchestrator`, `worker`, `health`,
`status`, `version`) remain at the top level unchanged. `health` and `status` stay
top-level because they are the most commonly used operational commands and `health`
is referenced in Docker HEALTHCHECK directives.

---

## Proposed Commands

### Tier 1 — Authoring & Validation (offline, no server required)

These commands use the internal workflow parsing and validation libraries. They need
no database connection and no running server, making them ideal for local development
and CI pipelines.

#### `kruxiaflow validate <path>`

Validate workflow definitions for syntax and semantic correctness.

```bash
# Validate a single file
kruxiaflow validate workflow.yaml

# Validate all workflows in a directory
kruxiaflow validate examples/

# JSON output for CI
kruxiaflow validate examples/ --format json

# Treat warnings as errors (for CI --strict mode)
kruxiaflow validate examples/ --strict
```

**Validation checks:**
- YAML syntax (valid YAML, correct types)
- Schema validation (required fields: `name`, `activities`)
- Activity types exist (`http_request`, `llm_prompt`, `postgres_query`, etc.)
- Dependency references valid (all `depends_on` / `dependency_of` targets exist)
- Cycle detection (disallow invalid cycles, allow valid loops)
- Output references valid (referenced outputs exist in upstream activities)
- Template expression syntax (`{{INPUT.*}}`, `{{OUTPUT.*}}`, `{{SECRET.*}}`, etc.)
- Best practice warnings (missing budget on LLM activities, missing retry on HTTP)

**Output:**
```
$ kruxiaflow validate examples/

  examples/01-weather-report.yaml            ok
  examples/04-moderate-content.yaml          ok (1 warning)
    W001: LLM activity 'analyze_content' has no budget limit
  examples/bad-workflow.yaml                 FAIL (2 errors)
    E010: Unknown activity type 'invalid_activity' in 'step1'
      hint: Valid types: http_request, llm_prompt, postgres_query, ...
    E011: Activity 'step2' depends on unknown activity 'nonexistent'
      hint: Available activities: step1, step3

Validated 12 files: 10 ok, 1 failed, 1 with warnings
```

**Exit codes:** 0 = valid, 1 = errors (or warnings in `--strict` mode)

**Source:** mvp-requirements.md US-3.6, US-9.1

---

#### `kruxiaflow visualize <path>`

Generate DAG diagrams from workflow definitions.

```bash
# Output Mermaid syntax to stdout
kruxiaflow visualize workflow.yaml --format mermaid

# Render to file
kruxiaflow visualize workflow.yaml --format png --output workflow.png
kruxiaflow visualize workflow.yaml --format svg --output workflow.svg

# Interactive HTML with pan/zoom
kruxiaflow visualize workflow.yaml --format html --output workflow.html
```

**Features:**
- Parse workflow into graph structure
- Show activity dependencies with directed edges
- Highlight conditional edges and loop constructs
- Color-code activity types (LLM = one color, HTTP = another, etc.)
- Supported formats: `mermaid` (text), `png`, `svg`, `html`

**Source:** mvp-requirements.md US-3.6, US-9.1

---

#### ~~`kruxiaflow compile <path>`~~ — moved to Python SDK

Compiling Python workflow definitions to YAML requires a Python interpreter. Rather
than embedding Python in the Rust binary, this functionality belongs in the Python
SDK (e.g. `kruxiaflow compile workflow.py` as a Python CLI entry point, or
`workflow.to_yaml()` as a library method). See post-mvp.md Story 4.6.

---

### Tier 2 — Workflow Lifecycle (wraps REST API)

These commands authenticate via OAuth2 and call the Kruxia Flow API. They require a
running server. Connection is configured via:

```bash
# Environment variables (recommended)
export KRUXIAFLOW_API_URL=http://localhost:8080
export KRUXIAFLOW_CLIENT_ID=kruxiaflow-docker-client
export KRUXIAFLOW_CLIENT_SECRET=<secret>

# Or CLI flags
kruxiaflow workflow list --api-url http://localhost:8080 --client-id ... --client-secret ...
```

#### `kruxiaflow workflow` subcommands

```bash
# Deploy a workflow definition (validate + upload)
kruxiaflow workflow deploy workflow.yaml

# List deployed workflow definitions
kruxiaflow workflow list
kruxiaflow workflow list --format json

# Start a workflow instance
kruxiaflow workflow run <definition_name> --input '{"key": "value"}'
kruxiaflow workflow run <definition_name> --input-file input.json

# Check workflow instance status
kruxiaflow workflow status <workflow_id>
kruxiaflow workflow status <workflow_id> --format json

# View workflow event log
kruxiaflow workflow logs <workflow_id>
kruxiaflow workflow logs <workflow_id> --follow      # stream events
```

**API mappings:**

| CLI Command          | HTTP Method | Endpoint                                    |
|----------------------|-------------|---------------------------------------------|
| `workflow deploy`    | POST        | `/api/v1/workflow_definitions`              |
| `workflow list`      | GET         | `/api/v1/workflow_definitions`              |
| `workflow run`       | POST        | `/api/v1/workflows`                         |
| `workflow status`    | GET         | `/api/v1/workflows/{id}`                    |
| `workflow logs`      | GET         | `/api/v1/workflows/{id}/events`             |

**Source:** post-mvp.md Story 4.6, mvp-requirements.md US-9.1

---

#### `kruxiaflow activity` subcommands

```bash
# List queued activities
kruxiaflow activity list
kruxiaflow activity list --status pending
kruxiaflow activity list --worker std
kruxiaflow activity list --format json
```

**API mapping:**

| CLI Command      | HTTP Method | Endpoint                   |
|------------------|-------------|----------------------------|
| `activity list`  | GET         | `/api/v1/activities`       |

**Source:** post-mvp.md Story 4.6

---

#### `kruxiaflow cost` subcommands

Expose the existing cost tracking API through a formatted CLI interface.

```bash
# Cost summary for a specific workflow
kruxiaflow cost workflow <workflow_id>
kruxiaflow cost workflow <workflow_id> --detailed    # per-activity breakdown

# Aggregated cost analytics
kruxiaflow cost analytics
kruxiaflow cost analytics --since 7d
kruxiaflow cost analytics --start-date 2026-01-01 --end-date 2026-01-31

# Top most expensive workflows
kruxiaflow cost top --limit 10 --since 30d

# Export to CSV
kruxiaflow cost export --since 30d --output costs.csv
```

**API mappings:**

| CLI Command        | HTTP Method | Endpoint                                        |
|--------------------|-------------|-------------------------------------------------|
| `cost workflow`    | GET         | `/api/v1/workflows/{id}/cost`                   |
| `cost workflow -d` | GET         | `/api/v1/workflows/{id}/cost/history`           |
| `cost analytics`   | GET         | `/api/v1/cost/analytics`                        |
| `cost top`         | GET         | `/api/v1/cost/analytics` (sorted, needs extension or client-side) |
| `cost export`      | GET         | `/api/v1/cost/analytics` (CSV formatting)       |

**Example output:**
```
$ kruxiaflow cost workflow abc123-def456

  Workflow Cost Report
  ────────────────────────────────────────
  Workflow ID:    abc123-def456
  Definition:     moderate_content
  Status:         Completed
  ────────────────────────────────────────
  Total Cost:     $0.000490
  Total Tokens:   186 (in: 110, out: 76)
  Activities:     1
  Budget:         $0.10 (0.5% used)
  ────────────────────────────────────────

$ kruxiaflow cost workflow abc123-def456 --detailed

  Activity Breakdown:
  ┌───────────────────┬────────────────────────────────┬────────┬───────────┐
  │ Activity          │ Model                          │ Tokens │ Cost      │
  ├───────────────────┼────────────────────────────────┼────────┼───────────┤
  │ analyze_content   │ anthropic/claude-haiku-4-5     │ 186    │ $0.000490 │
  └───────────────────┴────────────────────────────────┴────────┴───────────┘

$ kruxiaflow cost analytics --since 7d

  Cost Analytics (last 7 days)
  ────────────────────────────────────────
  Total Workflows:     142
  Total Cost:          $3.47
  Avg Cost/Activity:   $0.0082
  ────────────────────────────────────────
```

**Source:** Quick_Wins_Implementation_Plans.md Quick Win #3

---

### Tier 3 — Testing & Development (offline + optional server)

#### `kruxiaflow test <path>`

Execute a workflow locally in single-process mode for rapid iteration.

```bash
# Test with inline input
kruxiaflow test workflow.yaml --input '{"query": "test"}'

# Test with input file
kruxiaflow test workflow.yaml --input-file test.json

# Debug mode: step-by-step with state inspection
kruxiaflow test workflow.yaml --input-file test.json --debug

# Dry run: validate + show execution plan without running
kruxiaflow test workflow.yaml --input-file test.json --dry-run
```

**Features:**
- Load workflow definition from file (YAML)
- Execute workflow locally (single-process, embedded orchestrator + worker)
- Display execution trace (activity order, outputs, timing)
- Show final workflow state and outputs
- Report total cost and execution time
- Debug mode: pause between activities, inspect state
- Requires DATABASE_URL (uses real database for event storage)

**Source:** mvp-requirements.md US-3.6, US-9.1

---

#### `kruxiaflow dev`

Development watch mode with hot reload.

```bash
# Watch a directory for changes
kruxiaflow dev --watch workflows/

# Watch with auto-test on change
kruxiaflow dev --watch workflows/ --test --input test.json
```

**Features:**
- File watcher on YAML workflow files
- Hot reload: re-validate and optionally re-test on changes
- <30 second edit-test-result cycle
- Mock activities for fast testing (skip real HTTP/LLM calls)

**Source:** mvp-requirements.md US-9.6

---

### Tier 4 — Administration (direct database access)

These commands connect directly to the database. They are for bootstrap and
administrative scenarios where the API server may not be running.

#### `kruxiaflow admin` subcommands

```bash
# OAuth client management
kruxiaflow admin create-client "My Worker"
kruxiaflow admin list-clients
kruxiaflow admin revoke-client <client_id>

# User management
kruxiaflow admin create-user --username admin --email admin@example.com
kruxiaflow admin reset-password --username admin
```

Note: `db seed-client` already exists for the bootstrap case. The `admin` subcommands
extend this to full CRUD for OAuth clients and users.

**Source:** architecture.md (Auth section), post-mvp.md Story 4.6

---

### Tier 5 — Migration Tools (offline)

#### `kruxiaflow import` subcommands

```bash
# Import from Temporal
kruxiaflow import temporal --workflows ./temporal --output ./kruxiaflow

# Import from Airflow
kruxiaflow import airflow --dags ./airflow/dags --output ./kruxiaflow
```

**Features:**
- Analyze source workflow structure
- Generate YAML for straightforward workflows
- Generate Python builder for complex workflows
- Migration report: coverage analysis, manual steps required
- Operator/activity type mapping

**Source:** mvp-requirements.md US-9.3, US-9.4

---

## Implementation Approach

### Phasing

| Phase | Commands                               | Type      | Dependencies               |
|-------|----------------------------------------|-----------|----------------------------|
| 0     | `db` (regroup migrate/seed/profile)    | Reorg     | None — mechanical refactor |
| 1     | `validate`, `visualize`                | Offline   | Existing parse/validate libs |
| 2     | `workflow deploy/list/run/status/logs` | API wrap  | OAuth2 client in CLI       |
| 3     | `cost workflow/analytics/top/export`   | API wrap  | Phase 2 auth               |
| 4     | `test`                                 | Hybrid    | Embedded orchestrator      |
| 5     | `admin`                                | Direct DB | None beyond Phase 0        |
| 6     | `dev`                                  | Offline   | File watcher               |
| 7     | `import temporal/airflow`              | Offline   | Parser for each format     |

Phase 0 is a mechanical refactor with no new functionality. Phases 1-3 deliver the
highest value with the lowest effort. Phase 1 is purely offline. Phases 2-3 share
the OAuth2 client authentication layer.

### Architecture

```
kruxiaflow/src/
  commands/
    mod.rs                # existing: api, serve, health, status, version, ...
    db.rs                 # Phase 0 — regroups migrate, seed_llm, seed_client, profile
    validate.rs           # Phase 1 — uses core::workflow::parser
    visualize.rs          # Phase 1 — uses core::workflow::parser
    workflow_cli.rs       # Phase 2 — HTTP client wrapping REST API
    activity_cli.rs       # Phase 2
    cost_cli.rs           # Phase 3
    test_cli.rs           # Phase 4 — embedded execution
    admin.rs              # Phase 5
    dev.rs                # Phase 6
    import_temporal.rs    # Phase 7
    import_airflow.rs     # Phase 7
  client.rs               # Shared: OAuth2 token management, HTTP client
```

`db.rs` is a thin wrapper that delegates to the existing `migrate`, `seed_llm`,
`seed_client`, and `profile` modules — the implementation stays in those files.

The `client.rs` module provides a reusable authenticated HTTP client used by all
API-wrapping commands (Phases 2-3). It handles:
- Token acquisition via client credentials grant
- Token caching and refresh
- Configurable base URL
- JSON/YAML content negotiation

### Global Flags

All commands inherit the existing global flags plus new ones for API access:

```
Global flags (existing):
  --database-url <URL>       PostgreSQL connection (env: DATABASE_URL)
  --log-level <LEVEL>        Log verbosity (env: KRUXIAFLOW_LOG_LEVEL)
  --log-format <FORMAT>      Log format: text, json (env: KRUXIAFLOW_LOG_FORMAT)

Global flags (new):
  --api-url <URL>            API server URL (env: KRUXIAFLOW_API_URL)
  --client-id <ID>           OAuth2 client ID (env: KRUXIAFLOW_CLIENT_ID)
  --client-secret <SECRET>   OAuth2 client secret (env: KRUXIAFLOW_CLIENT_SECRET)
  --format <FORMAT>          Output format: table, json, csv (default: table)
```

### Shell Completions

```bash
# Generate completions
kruxiaflow completions bash > /etc/bash_completion.d/kruxiaflow
kruxiaflow completions zsh > ~/.zfunc/_kruxiaflow
kruxiaflow completions fish > ~/.config/fish/completions/kruxiaflow.fish
```

This is built into clap via `clap_complete`.

---

## Sources

This proposal consolidates CLI plans from:

| Source Document                          | Relevant Sections                    |
|------------------------------------------|--------------------------------------|
| docs/mvp-requirements.md                 | US-3.6, US-9.1, US-9.3, US-9.4, US-9.6 |
| docs/post-mvp.md                         | Story 4.6                           |
| docs/architecture.md                     | Auth CLI Commands section            |
| docs/notes/2025-10-25-v02-sonnet.md      | CLI tooling sections                 |
| Quick_Wins_Implementation_Plans.md       | Quick Win #3 (costs), #4 (validate) |
