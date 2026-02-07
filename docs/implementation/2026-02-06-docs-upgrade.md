# Documentation Upgrade — 2026-02-06

Align core project documents with refined landing page messaging from kruxiaflow.com.

## Source Materials

- **Landing page**: `kruxiaflow.com/kruxiaflow-landing.html` (refined public messaging)
- **Content review**: `kruxiaflow.com/docs/2026-02-06-content-review.md` (reviewer feedback)

## Changes

### 1. README.md

| #   | Change                                            | Status    |
|-----|---------------------------------------------------|-----------|
| 1a  | Update description paragraph                      | Complete  |
| 1b  | Add "Built For" audience section                  | Complete  |
| 1c  | Refine problem framing (3 pillars)                | Complete  |
| 1d  | Uncomment & update comparison table (add Inngest) | Complete  |
| 1e  | "guarantee" → "deliver" for exactly-once          | Complete  |
| 1f  | Update examples count to 15+                      | Complete  |
| 1g  | Add Python SDK examples to examples table         | Complete  |
| 1h  | Update "deploy anywhere" language                 | Complete  |

#### 1a. Update description paragraph

Current:
> A lightweight, high-performance workflow engine designed for AI applications. Track every token, cache intelligently, and never exceed your LLM budget. Run on anything from the edge to the cloud.

Proposed:
> The only durable workflow engine built for AI. Workflows survive crashes, budgets control AI spend, and costs are tracked per token. Ships as a 7.5 MB binary that runs anywhere.

#### 1b. Add "Built For" audience section

Insert after the "Why Kruxia Flow?" heading, before "The Problem":

```markdown
### Built For

- **AI startups** — Ship AI agents to production with built-in cost tracking and budget control. Survive crashes, stop runaway spend.
- **Small businesses** — Define workflows with no code, deploy one binary and one database. No cluster, no DevOps team. Production reliability for tens of dollars a month.
- **Data teams** — Combine batch pipelines and AI agents in one platform. Python SDK with pandas and DuckDB, without a 4GB footprint or a $1K/month vendor lock-in.
```

#### 1c. Refine problem framing

Replace the current "The Problem" bullet list with the landing page's three-pillar approach:

```markdown
### The Problem

LLM costs spiral out of control, and existing tools can't help:

- **Invisible AI spend**: No workflow engine tracks LLM costs natively. Teams with cost observability report 30-50% savings, but you're left stitching together external tools with no budget control to stop a runaway agent.
- **Temporal's operational tax**: 7+ components to self-host, and teams report 8 engineering-months per year on maintenance. Zero LLM awareness.
- **LangGraph isn't a workflow engine**: Python-only, no native scheduling, and requires the proprietary LangSmith platform for production at ~$1,000 per million executions.
```

#### 1d. Uncomment & update comparison table

Removed the old commented-out comparison section. Replaced the existing feature table with the full comparison matching the landing page, including Inngest.

#### 1e. "guarantee" → "deliver" for exactly-once

Changed "exactly-once guarantees" → "exactly-once semantics" in Architecture section.

#### 1f. Update examples count

Changed "10+" to "15+" in all references.

#### 1g. Add Python SDK examples to examples table

Added rows for Python SDK examples 11-15 (GitHub Health Check, Sales ETL, Customer Churn Prediction, Document Intelligence, Content Moderation System).

#### 1h. Update "deploy anywhere" language

Added to Architecture section: "Runs anywhere: on cloud VMs, on-premise computers, or edge devices like Raspberry Pi Zero."

### 2. docs/quickstart.md

| #   | Change                                       | Status    |
|-----|----------------------------------------------|-----------|
| 2a  | Fix hardcoded secret → read from .env        | Complete  |
| 2b  | `./docker up -d` → `./docker up --examples`  | Complete  |
| 2c  | Add cost tracking step                       | Complete  |
| 2d  | Add workflow definition deploy step          | Complete  |
| 2e  | Update examples count to 15+                 | Complete  |

Rewrote quickstart.md to follow the landing page's 4-step flow:
1. Clone and start (with `--examples` flag)
2. Get an access token (reading secret from `.env`)
3. Deploy and run a workflow (two-step: deploy definition, then submit instance)
4. Track costs (status, cost summary, cost history)

### 3. docs/architecture.md

| #   | Change                              | Status    |
|-----|-------------------------------------|-----------|
| 3a  | Binary size 10-15MB → 7.5 MB       | Complete  |
| 3b  | RAM footprint 50MB → 328 MB peak   | Complete  |
| 3c  | Mermaid diagram label update        | Complete  |
| 3d  | Performance target caveat           | Complete  |
