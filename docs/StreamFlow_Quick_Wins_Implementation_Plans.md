# StreamFlow Quick Wins: Implementation Plans

**Version**: 1.0  
**Date**: November 27, 2025  
**Status**: Ready for Implementation  
**Total Estimated Effort**: 6-9 days

These five items are the highest-impact, lowest-effort work to prepare StreamFlow for a successful public launch. Each plan includes acceptance criteria, file changes, and testing requirements.

---

## Quick Win #1: Polish README with Quick Start

**Effort**: 1 day  
**Impact**: High — README is the first thing visitors see  
**Files**: `README.md`, potentially `docs/QUICK_START.md`

### Current State
The README likely focuses on technical architecture. For launch, it needs to immediately communicate value and get users running in 60 seconds.

### Target README Structure

```markdown
# StreamFlow

**Workflow orchestration built for the AI era** — Temporal's durability in a 4.5MB binary, with native LLM cost tracking.

[![GitHub Stars](badge)][stars]
[![License](badge)][license]
[![Test Coverage](badge)][coverage]

## Why StreamFlow?

- **🚀 5-Minute Setup**: Single binary, just PostgreSQL. No Kubernetes, no Kafka, no complexity.
- **💰 Built-in LLM Cost Tracking**: Track every token, set budgets, cache repeated queries.
- **🔄 Durable Execution**: When step 7 of 12 fails, pick up where you left off.
- **🤖 Multi-Provider LLM**: OpenAI, Anthropic, Google, Ollama — with automatic fallback.
- **⚡ 4.5MB Binary**: Deploy anywhere, including edge devices.

## Quick Start (60 seconds)

### Option 1: Docker (recommended)
```bash
docker run -d -p 8080:8080 streamflow/streamflow:latest
```

### Option 2: Binary Download
```bash
curl -sSL https://streamflow.dev/install.sh | sh
streamflow serve
```

### Run Your First Workflow

```bash
# Create a simple workflow
cat > hello.yaml << 'EOF'
name: hello_world
activities:
  greet:
    activity: llm_prompt
    parameters:
      model: anthropic/claude-3-haiku-20240307
      messages:
        - role: user
          content: "Say hello in a creative way!"
EOF

# Submit and watch it run
curl -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Content-Type: application/yaml" \
  --data-binary @hello.yaml

curl -X POST http://localhost:8080/api/v1/workflows \
  -H "Content-Type: application/json" \
  -d '{"definition_name": "hello_world", "input": {}}'
```

## Key Features

### LLM Cost Tracking
Every LLM call tracks tokens and costs automatically:
```yaml
activities:
  analyze:
    activity: llm_prompt
    parameters:
      model: anthropic/claude-3-haiku-20240307
    settings:
      budget:
        limit_usd: 0.10  # Abort if cost exceeds $0.10
```

### Semantic Caching
Cache LLM responses to slash costs on repeated queries:
```yaml
settings:
  cache:
    enabled: true
    ttl_seconds: 3600  # Cache for 1 hour
```

### Multi-Provider Fallback
Automatically fall back to cheaper models when budget is tight:
```yaml
parameters:
  model:
    - openai/gpt-4o           # Try first
    - anthropic/claude-3-haiku # Fallback
    - ollama/llama3           # Local fallback
```

## Example Workflows

| Example | Description | Key Features |
|---------|-------------|--------------|
| [01-weather-report](examples/01-weather-report.yaml) | Fetch weather, send notification | HTTP requests, sequential execution |
| [04-moderate-content](examples/04-moderate-content.yaml) | AI content moderation | LLM, cost tracking, retry |
| [05-research-assistant](examples/05-research-assistant.yaml) | Multi-model AI assistant | Provider fallback, budget limits |
| [06a-faq-bot-caching](examples/06a-faq-bot-caching.yaml) | FAQ bot with caching | Semantic caching, cost savings |
| [07a-agentic-research](examples/07a-agentic-research-simple.yaml) | Agentic research loop | Iterative workflows |

[View all 10 examples →](examples/README.md)

## Documentation

- [Getting Started Guide](docs/getting-started.md)
- [YAML Workflow Reference](docs/yaml-reference.md)
- [Built-in Activities](docs/activities.md)
- [Configuration](docs/configuration.md)
- [API Reference](docs/api-reference.md)

## Comparison

| Feature | StreamFlow | Temporal | Airflow | LangChain |
|---------|:----------:|:--------:|:-------:|:---------:|
| Single binary | ✅ | ❌ | ❌ | N/A |
| Native LLM support | ✅ | ❌ | ❌ | ✅ |
| Cost tracking | ✅ | ❌ | ❌ | ❌ |
| Durable execution | ✅ | ✅ | ⚠️ | ❌ |
| Semantic caching | ✅ | ❌ | ❌ | ⚠️ |
| Setup time | 5 min | 1+ day | 1+ day | 30 min |

## Community

- [Discord](https://discord.gg/streamflow) — Get help, share workflows
- [GitHub Discussions](https://github.com/streamflow/streamflow/discussions) — Ideas and Q&A
- [Twitter](https://twitter.com/streamflowdev) — Updates and tips

## License

Apache 2.0 — See [LICENSE](LICENSE)
```

### Implementation Tasks

1. **Restructure README.md** (~3 hours)
   - [ ] Add hero section with tagline and badges
   - [ ] Write "Why StreamFlow?" bullet points (benefit-focused)
   - [ ] Create 60-second quick start section
   - [ ] Add feature highlights with YAML snippets
   - [ ] Create example workflow table linking to files
   - [ ] Add comparison table vs competitors
   - [ ] Add community/documentation links

2. **Create supporting assets** (~2 hours)
   - [ ] Design badges (stars, license, coverage)
   - [ ] Ensure all linked examples exist and work
   - [ ] Verify all documentation links are valid

3. **Test the quick start flow** (~1 hour)
   - [ ] Follow instructions on fresh machine
   - [ ] Time each step, target <60 seconds total
   - [ ] Fix any friction points discovered

### Acceptance Criteria

- [ ] New user can understand value prop in <10 seconds (above fold)
- [ ] New user can run first workflow in <60 seconds
- [ ] All code examples in README are copy-pasteable and work
- [ ] No broken links
- [ ] Comparison table is accurate and fair

---

## Quick Win #2: Dockerfile with Health Check

**Effort**: 1 day  
**Impact**: High — removes biggest adoption friction for data engineers  
**Files**: `Dockerfile`, `docker-compose.yml`, `.dockerignore`

### Architecture Decision

**Option A**: StreamFlow + External PostgreSQL (recommended for MVP)
- User runs `docker-compose up` with StreamFlow + PostgreSQL containers
- Simpler, follows 12-factor app principles
- Matches production deployment patterns

**Option B**: All-in-One with Embedded PostgreSQL
- Single container with PostgreSQL inside
- More convenient for demos but not production-realistic
- Higher image size, more complexity

**Recommendation**: Ship Option A first (simpler, more useful), Option B as future enhancement.

### Implementation

#### Dockerfile

```dockerfile
# StreamFlow Dockerfile
# Multi-stage build for minimal image size

# Stage 1: Build (if building from source)
FROM rust:1.75-slim as builder

WORKDIR /app
COPY . .

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

RUN cargo build --release --bin streamflow

# Stage 2: Runtime
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -s /bin/false streamflow

# Copy binary
COPY --from=builder /app/target/release/streamflow /usr/local/bin/streamflow

# Copy example workflows and config
COPY examples/ /opt/streamflow/examples/
COPY config/ /opt/streamflow/config/

# Set ownership
RUN chown -R streamflow:streamflow /opt/streamflow

USER streamflow
WORKDIR /opt/streamflow

# Environment defaults
ENV STREAMFLOW_API_PORT=8080 \
    STREAMFLOW_LOG_LEVEL=info \
    STREAMFLOW_LOG_FORMAT=json

# Expose API port
EXPOSE 8080

# Health check
HEALTHCHECK --interval=10s --timeout=5s --start-period=30s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

# Default command
ENTRYPOINT ["streamflow"]
CMD ["serve"]
```

#### docker-compose.yml

```yaml
# StreamFlow Docker Compose
# Quick start: docker-compose up -d

version: '3.8'

services:
  streamflow:
    image: streamflow/streamflow:latest
    # Or build locally:
    # build: .
    ports:
      - "8080:8080"
    environment:
      DATABASE_URL: postgres://streamflow:streamflow@postgres:5432/streamflow
      STREAMFLOW_LOG_LEVEL: info
      STREAMFLOW_LOG_FORMAT: json
      # LLM API keys (optional - only needed for LLM activities)
      ANTHROPIC_API_KEY: ${ANTHROPIC_API_KEY:-}
      OPENAI_API_KEY: ${OPENAI_API_KEY:-}
      GOOGLE_API_KEY: ${GOOGLE_API_KEY:-}
    depends_on:
      postgres:
        condition: service_healthy
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/health"]
      interval: 10s
      timeout: 5s
      retries: 5
      start_period: 30s
    restart: unless-stopped

  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_USER: streamflow
      POSTGRES_PASSWORD: streamflow
      POSTGRES_DB: streamflow
    volumes:
      - streamflow_data:/var/lib/postgresql/data
      # Initialize with pgvector extension
      - ./docker/init-db.sql:/docker-entrypoint-initdb.d/init.sql:ro
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U streamflow -d streamflow"]
      interval: 5s
      timeout: 5s
      retries: 5
    restart: unless-stopped

volumes:
  streamflow_data:
```

#### docker/init-db.sql

```sql
-- Initialize StreamFlow database with required extensions
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS "vector";  -- For semantic caching/RAG

-- Note: StreamFlow will run migrations on startup
-- This just ensures extensions are available
```

#### .dockerignore

```
# Build artifacts
target/
*.rs.bk

# Git
.git/
.gitignore

# IDE
.idea/
.vscode/
*.swp

# Documentation (not needed in image)
docs/
*.md
!README.md

# Tests
**/tests/
**/test_*.rs

# Development files
.env
.env.*
docker-compose.override.yml
```

### Implementation Tasks

1. **Create Dockerfile** (~2 hours)
   - [ ] Write multi-stage Dockerfile
   - [ ] Optimize for minimal image size (<50MB target)
   - [ ] Add health check
   - [ ] Set proper user permissions (non-root)
   - [ ] Include example workflows in image

2. **Create docker-compose.yml** (~1 hour)
   - [ ] Configure StreamFlow + PostgreSQL services
   - [ ] Add health check dependencies
   - [ ] Configure environment variable passthrough for API keys
   - [ ] Add volume for PostgreSQL persistence

3. **Create init-db.sql** (~30 min)
   - [ ] Enable required PostgreSQL extensions
   - [ ] Add any seed data for demos

4. **Test the complete flow** (~2 hours)
   - [ ] `docker-compose up -d` starts cleanly
   - [ ] Health checks pass
   - [ ] Can submit workflow via API
   - [ ] Logs are visible via `docker-compose logs`
   - [ ] Graceful shutdown works

5. **Document Docker deployment** (~1 hour)
   - [ ] Add Docker section to README
   - [ ] Create DOCKER.md with advanced options

### Acceptance Criteria

- [ ] `docker-compose up -d` brings up working StreamFlow in <60 seconds
- [ ] Health check endpoint works and Docker reports healthy
- [ ] Can submit and run example workflow
- [ ] Image size <50MB (excluding PostgreSQL)
- [ ] Works on Linux, macOS, Windows (Docker Desktop)
- [ ] Environment variables properly configure API keys

---

## Quick Win #3: `streamflow costs` CLI Command

**Effort**: 2 days  
**Impact**: Very High — makes cost tracking visible, proves key differentiator  
**Files**: `streamflow/src/commands/costs.rs`, `streamflow/src/main.rs`

### Feature Overview

Add CLI commands to query and display LLM cost data:

```bash
# Show costs for a specific workflow
streamflow costs workflow <workflow_id>

# Show costs summary across all workflows
streamflow costs summary [--since 24h] [--workflow-type <type>]

# Show most expensive workflows
streamflow costs top [--limit 10] [--since 7d]

# Export costs to CSV
streamflow costs export [--since 30d] [--output costs.csv]
```

### Implementation

#### New File: `streamflow/src/commands/costs.rs`

```rust
use clap::{Args, Subcommand};
use sqlx::PgPool;
use rust_decimal::Decimal;
use chrono::{DateTime, Utc, Duration};
use comfy_table::{Table, Row, Cell, Color};
use serde::Serialize;

#[derive(Args)]
pub struct CostsCommand {
    #[command(subcommand)]
    pub command: CostsSubcommand,
    
    /// Output format (table, json, csv)
    #[arg(long, default_value = "table")]
    pub format: OutputFormat,
}

#[derive(Subcommand)]
pub enum CostsSubcommand {
    /// Show costs for a specific workflow
    Workflow(WorkflowCostsArgs),
    /// Show cost summary across workflows
    Summary(SummaryCostsArgs),
    /// Show most expensive workflows
    Top(TopCostsArgs),
    /// Export costs to file
    Export(ExportCostsArgs),
}

#[derive(Args)]
pub struct WorkflowCostsArgs {
    /// Workflow ID
    pub workflow_id: String,
    /// Include individual activity costs
    #[arg(long)]
    pub detailed: bool,
}

#[derive(Args)]
pub struct SummaryCostsArgs {
    /// Time range (e.g., "24h", "7d", "30d")
    #[arg(long, default_value = "24h")]
    pub since: String,
    /// Filter by workflow type
    #[arg(long)]
    pub workflow_type: Option<String>,
}

#[derive(Args)]
pub struct TopCostsArgs {
    /// Number of workflows to show
    #[arg(long, default_value = "10")]
    pub limit: usize,
    /// Time range
    #[arg(long, default_value = "7d")]
    pub since: String,
}

#[derive(Args)]
pub struct ExportCostsArgs {
    /// Time range
    #[arg(long, default_value = "30d")]
    pub since: String,
    /// Output file path
    #[arg(long, default_value = "costs.csv")]
    pub output: String,
}

// ===== Data Structures =====

#[derive(Serialize)]
pub struct WorkflowCost {
    pub workflow_id: String,
    pub workflow_type: String,
    pub total_cost_usd: Decimal,
    pub total_tokens: i64,
    pub activity_count: i32,
    pub cache_hits: i32,
    pub cache_misses: i32,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Serialize)]
pub struct ActivityCost {
    pub activity_key: String,
    pub activity_type: String,
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: Decimal,
    pub cache_hit: bool,
    pub duration_ms: i64,
}

#[derive(Serialize)]
pub struct CostSummary {
    pub period: String,
    pub total_cost_usd: Decimal,
    pub total_workflows: i64,
    pub total_activities: i64,
    pub total_tokens: i64,
    pub cache_hit_rate: f64,
    pub avg_cost_per_workflow: Decimal,
    pub by_model: Vec<ModelCost>,
    pub by_workflow_type: Vec<WorkflowTypeCost>,
}

#[derive(Serialize)]
pub struct ModelCost {
    pub model: String,
    pub total_cost_usd: Decimal,
    pub total_tokens: i64,
    pub request_count: i64,
}

#[derive(Serialize)]
pub struct WorkflowTypeCost {
    pub workflow_type: String,
    pub total_cost_usd: Decimal,
    pub workflow_count: i64,
}

// ===== Implementation =====

pub async fn execute(args: CostsCommand, pool: &PgPool) -> anyhow::Result<()> {
    match args.command {
        CostsSubcommand::Workflow(workflow_args) => {
            show_workflow_costs(pool, &workflow_args, &args.format).await
        }
        CostsSubcommand::Summary(summary_args) => {
            show_cost_summary(pool, &summary_args, &args.format).await
        }
        CostsSubcommand::Top(top_args) => {
            show_top_costs(pool, &top_args, &args.format).await
        }
        CostsSubcommand::Export(export_args) => {
            export_costs(pool, &export_args).await
        }
    }
}

async fn show_workflow_costs(
    pool: &PgPool,
    args: &WorkflowCostsArgs,
    format: &OutputFormat,
) -> anyhow::Result<()> {
    // Query workflow cost data from workflow_events table
    // The cost data is stored in ActivityCompleted events
    
    let workflow_cost = sqlx::query_as!(
        WorkflowCostRow,
        r#"
        SELECT 
            w.id as workflow_id,
            w.workflow_type,
            w.created_at,
            w.completed_at,
            COALESCE(SUM((e.payload->>'cost_usd')::DECIMAL), 0) as total_cost_usd,
            COALESCE(SUM((e.payload->>'input_tokens')::BIGINT), 0) as total_input_tokens,
            COALESCE(SUM((e.payload->>'output_tokens')::BIGINT), 0) as total_output_tokens,
            COUNT(CASE WHEN e.event_type = 'ActivityCompleted' THEN 1 END) as activity_count,
            COUNT(CASE WHEN e.payload->>'cache_hit' = 'true' THEN 1 END) as cache_hits
        FROM workflows w
        LEFT JOIN workflow_events e ON w.id = e.workflow_id 
            AND e.event_type = 'ActivityCompleted'
        WHERE w.id = $1
        GROUP BY w.id, w.workflow_type, w.created_at, w.completed_at
        "#,
        uuid::Uuid::parse_str(&args.workflow_id)?
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| anyhow::anyhow!("Workflow not found: {}", args.workflow_id))?;

    // Display based on format
    match format {
        OutputFormat::Table => {
            println!("\n📊 Workflow Cost Report");
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!("Workflow ID:    {}", workflow_cost.workflow_id);
            println!("Type:           {}", workflow_cost.workflow_type);
            println!("Status:         {}", if workflow_cost.completed_at.is_some() { "✅ Completed" } else { "🔄 Running" });
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!("💰 Total Cost:      ${:.6}", workflow_cost.total_cost_usd);
            println!("📝 Total Tokens:    {} (in: {}, out: {})", 
                workflow_cost.total_input_tokens + workflow_cost.total_output_tokens,
                workflow_cost.total_input_tokens,
                workflow_cost.total_output_tokens);
            println!("⚡ Activities:      {}", workflow_cost.activity_count);
            println!("💾 Cache Hits:      {} ({:.1}%)", 
                workflow_cost.cache_hits,
                if workflow_cost.activity_count > 0 {
                    (workflow_cost.cache_hits as f64 / workflow_cost.activity_count as f64) * 100.0
                } else { 0.0 });
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
            
            if args.detailed {
                println!("📋 Activity Breakdown:");
                // Query and display individual activities
                show_activity_breakdown(pool, &args.workflow_id).await?;
            }
        }
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&workflow_cost)?);
        }
        OutputFormat::Csv => {
            println!("workflow_id,workflow_type,total_cost_usd,total_tokens,activity_count,cache_hits");
            println!("{},{},{},{},{},{}",
                workflow_cost.workflow_id,
                workflow_cost.workflow_type,
                workflow_cost.total_cost_usd,
                workflow_cost.total_input_tokens + workflow_cost.total_output_tokens,
                workflow_cost.activity_count,
                workflow_cost.cache_hits);
        }
    }
    
    Ok(())
}

async fn show_activity_breakdown(pool: &PgPool, workflow_id: &str) -> anyhow::Result<()> {
    let activities = sqlx::query!(
        r#"
        SELECT 
            e.payload->>'activity_key' as activity_key,
            e.payload->>'activity_type' as activity_type,
            e.payload->>'model' as model,
            (e.payload->>'input_tokens')::BIGINT as input_tokens,
            (e.payload->>'output_tokens')::BIGINT as output_tokens,
            (e.payload->>'cost_usd')::DECIMAL as cost_usd,
            (e.payload->>'cache_hit')::BOOLEAN as cache_hit,
            e.timestamp
        FROM workflow_events e
        WHERE e.workflow_id = $1
          AND e.event_type = 'ActivityCompleted'
          AND e.payload->>'cost_usd' IS NOT NULL
        ORDER BY e.timestamp
        "#,
        uuid::Uuid::parse_str(workflow_id)?
    )
    .fetch_all(pool)
    .await?;

    let mut table = Table::new();
    table.set_header(vec!["Activity", "Model", "Tokens", "Cost", "Cache"]);
    
    for activity in activities {
        let tokens = activity.input_tokens.unwrap_or(0) + activity.output_tokens.unwrap_or(0);
        let cache_icon = if activity.cache_hit.unwrap_or(false) { "✅" } else { "❌" };
        
        table.add_row(vec![
            activity.activity_key.unwrap_or_default(),
            activity.model.unwrap_or_else(|| "N/A".to_string()),
            tokens.to_string(),
            format!("${:.6}", activity.cost_usd.unwrap_or_default()),
            cache_icon.to_string(),
        ]);
    }
    
    println!("{table}");
    Ok(())
}

// ... Additional implementation for summary, top, export commands
```

#### Update: `streamflow/src/main.rs`

Add the costs command to the CLI:

```rust
#[derive(Subcommand)]
enum Commands {
    /// Start all StreamFlow services
    Serve(ServeArgs),
    /// Query and display cost information
    Costs(CostsCommand),
    /// Validate workflow definition files
    Validate(ValidateArgs),
    // ... existing commands
}
```

### SQL Queries Needed

The cost data is already being stored in `workflow_events` table with `ActivityCompleted` events containing:
- `cost_usd` (Decimal)
- `input_tokens` (i64)
- `output_tokens` (i64)
- `model` (String)
- `cache_hit` (bool)

Key queries:

```sql
-- Workflow cost summary
SELECT 
    w.id,
    w.workflow_type,
    SUM((e.payload->>'cost_usd')::DECIMAL) as total_cost,
    SUM((e.payload->>'input_tokens')::BIGINT + (e.payload->>'output_tokens')::BIGINT) as total_tokens,
    COUNT(CASE WHEN e.payload->>'cache_hit' = 'true' THEN 1 END) as cache_hits
FROM workflows w
JOIN workflow_events e ON w.id = e.workflow_id
WHERE e.event_type = 'ActivityCompleted'
  AND e.payload->>'cost_usd' IS NOT NULL
GROUP BY w.id, w.workflow_type;

-- Cost summary by time period
SELECT 
    DATE_TRUNC('day', e.timestamp) as day,
    SUM((e.payload->>'cost_usd')::DECIMAL) as daily_cost,
    COUNT(DISTINCT e.workflow_id) as workflow_count
FROM workflow_events e
WHERE e.event_type = 'ActivityCompleted'
  AND e.timestamp > NOW() - INTERVAL '30 days'
GROUP BY DATE_TRUNC('day', e.timestamp)
ORDER BY day;

-- Top expensive workflows
SELECT 
    w.id,
    w.workflow_type,
    SUM((e.payload->>'cost_usd')::DECIMAL) as total_cost
FROM workflows w
JOIN workflow_events e ON w.id = e.workflow_id
WHERE e.event_type = 'ActivityCompleted'
  AND w.created_at > NOW() - INTERVAL '7 days'
GROUP BY w.id, w.workflow_type
ORDER BY total_cost DESC
LIMIT 10;
```

### Implementation Tasks

1. **Create costs command module** (~4 hours)
   - [ ] Define CLI arguments structure with clap
   - [ ] Implement `workflow` subcommand
   - [ ] Implement `summary` subcommand
   - [ ] Implement `top` subcommand
   - [ ] Implement `export` subcommand

2. **Implement output formatters** (~2 hours)
   - [ ] Table format with comfy-table (colored, aligned)
   - [ ] JSON format with serde
   - [ ] CSV format for export

3. **Write SQL queries** (~2 hours)
   - [ ] Query workflow costs from events
   - [ ] Aggregate costs by model/type
   - [ ] Time-based filtering

4. **Add to main CLI** (~1 hour)
   - [ ] Register costs command
   - [ ] Add to help text
   - [ ] Handle database connection

5. **Testing** (~3 hours)
   - [ ] Test with sample workflow data
   - [ ] Test all output formats
   - [ ] Test edge cases (no costs, no workflows)

6. **Documentation** (~1 hour)
   - [ ] Add usage examples to README
   - [ ] Document in CLI help text

### Acceptance Criteria

- [ ] `streamflow costs workflow <id>` shows cost breakdown
- [ ] `streamflow costs summary` shows aggregated costs
- [ ] `streamflow costs top` shows most expensive workflows
- [ ] `streamflow costs export` creates valid CSV
- [ ] All commands support `--format json` for scripting
- [ ] Output is visually appealing with colors/formatting
- [ ] Works when no cost data exists (graceful empty state)

### Example Output

```
$ streamflow costs workflow abc123-def456

📊 Workflow Cost Report
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Workflow ID:    abc123-def456
Type:           research_assistant
Status:         ✅ Completed
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
💰 Total Cost:      $0.023400
📝 Total Tokens:    1,847 (in: 324, out: 1,523)
⚡ Activities:      3
💾 Cache Hits:      1 (33.3%)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

$ streamflow costs workflow abc123-def456 --detailed

📋 Activity Breakdown:
┌──────────────────┬─────────────────────────┬────────┬───────────┬───────┐
│ Activity         │ Model                   │ Tokens │ Cost      │ Cache │
├──────────────────┼─────────────────────────┼────────┼───────────┼───────┤
│ ask_question     │ anthropic/claude-3-haiku│ 1,203  │ $0.018000 │ ❌    │
│ summarize        │ anthropic/claude-3-haiku│ 412    │ $0.004200 │ ❌    │
│ format_response  │ anthropic/claude-3-haiku│ 232    │ $0.001200 │ ✅    │
└──────────────────┴─────────────────────────┴────────┴───────────┴───────┘
```

---

## Quick Win #4: `streamflow validate` CLI Command

**Effort**: 1-2 days  
**Impact**: Medium — prevents frustrating errors, improves DX  
**Files**: `streamflow/src/commands/validate.rs`, `core/src/workflow/validator.rs`

### Feature Overview

```bash
# Validate a single workflow file
streamflow validate workflow.yaml

# Validate all workflows in a directory
streamflow validate examples/

# Validate with verbose output
streamflow validate workflow.yaml --verbose

# Output as JSON (for CI integration)
streamflow validate workflow.yaml --format json
```

### Validation Checks

1. **Syntax Validation**
   - Valid YAML syntax
   - Required fields present (`name`, `activities`)
   - Correct field types

2. **Semantic Validation**
   - Activity types exist (`http_request`, `llm_prompt`, `postgres_query`, etc.)
   - Template expressions are valid (`{{INPUT.name}}`, `{{activity.output}}`)
   - No circular dependencies in `depends_on`
   - Referenced activities exist
   - Parameter types match activity requirements

3. **Best Practice Warnings**
   - Budget not set for LLM activities
   - No retry policy for network activities
   - Large timeout values

### Implementation

#### New File: `streamflow/src/commands/validate.rs`

```rust
use clap::Args;
use std::path::{Path, PathBuf};
use colored::Colorize;
use walkdir::WalkDir;

#[derive(Args)]
pub struct ValidateArgs {
    /// Path to workflow file or directory
    pub path: PathBuf,
    
    /// Show detailed validation information
    #[arg(long)]
    pub verbose: bool,
    
    /// Output format (text, json)
    #[arg(long, default_value = "text")]
    pub format: OutputFormat,
    
    /// Treat warnings as errors
    #[arg(long)]
    pub strict: bool,
}

#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub file: PathBuf,
    pub valid: bool,
    pub errors: Vec<ValidationError>,
    pub warnings: Vec<ValidationWarning>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ValidationError {
    pub code: String,
    pub message: String,
    pub location: Option<Location>,
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ValidationWarning {
    pub code: String,
    pub message: String,
    pub location: Option<Location>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Location {
    pub line: usize,
    pub column: usize,
    pub context: String,
}

pub async fn execute(args: ValidateArgs) -> anyhow::Result<i32> {
    let files = collect_yaml_files(&args.path)?;
    
    if files.is_empty() {
        eprintln!("{}", "No YAML files found".yellow());
        return Ok(1);
    }
    
    let mut results = Vec::new();
    let mut has_errors = false;
    let mut has_warnings = false;
    
    for file in &files {
        let result = validate_workflow_file(file)?;
        has_errors |= !result.valid;
        has_warnings |= !result.warnings.is_empty();
        results.push(result);
    }
    
    // Output results
    match args.format {
        OutputFormat::Text => print_text_results(&results, args.verbose),
        OutputFormat::Json => print_json_results(&results)?,
    }
    
    // Exit code
    if has_errors {
        Ok(1)
    } else if args.strict && has_warnings {
        Ok(1)
    } else {
        Ok(0)
    }
}

fn collect_yaml_files(path: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    
    if path.is_file() {
        if path.extension().map_or(false, |ext| ext == "yaml" || ext == "yml") {
            files.push(path.to_path_buf());
        }
    } else if path.is_dir() {
        for entry in WalkDir::new(path)
            .max_depth(3)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "yaml" || ext == "yml") {
                files.push(path.to_path_buf());
            }
        }
    }
    
    Ok(files)
}

fn validate_workflow_file(path: &Path) -> anyhow::Result<ValidationResult> {
    let content = std::fs::read_to_string(path)?;
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    
    // 1. YAML Syntax Validation
    let yaml_result: Result<serde_yaml::Value, _> = serde_yaml::from_str(&content);
    if let Err(e) = yaml_result {
        errors.push(ValidationError {
            code: "E001".to_string(),
            message: format!("Invalid YAML syntax: {}", e),
            location: extract_yaml_error_location(&e, &content),
            suggestion: Some("Check YAML indentation and syntax".to_string()),
        });
        return Ok(ValidationResult {
            file: path.to_path_buf(),
            valid: false,
            errors,
            warnings,
        });
    }
    
    // 2. Schema Validation - Parse as WorkflowDefinition
    let definition_result: Result<WorkflowDefinition, _> = serde_yaml::from_str(&content);
    match definition_result {
        Ok(definition) => {
            // 3. Semantic Validation
            validate_workflow_semantics(&definition, &mut errors, &mut warnings);
            
            // 4. Best Practice Checks
            check_best_practices(&definition, &mut warnings);
        }
        Err(e) => {
            errors.push(ValidationError {
                code: "E002".to_string(),
                message: format!("Invalid workflow structure: {}", e),
                location: None,
                suggestion: Some("Ensure required fields (name, activities) are present".to_string()),
            });
        }
    }
    
    Ok(ValidationResult {
        file: path.to_path_buf(),
        valid: errors.is_empty(),
        errors,
        warnings,
    })
}

fn validate_workflow_semantics(
    definition: &WorkflowDefinition,
    errors: &mut Vec<ValidationError>,
    warnings: &mut Vec<ValidationWarning>,
) {
    let valid_activity_types = [
        "http_request",
        "llm_prompt",
        "postgres_query",
        "postgres_transaction",
        "embedding_generate",
        "email_send",
    ];
    
    let activity_keys: HashSet<_> = definition.activities.keys().collect();
    
    for (key, activity) in &definition.activities {
        // Check activity type exists
        if !valid_activity_types.contains(&activity.activity.as_str()) {
            errors.push(ValidationError {
                code: "E010".to_string(),
                message: format!("Unknown activity type '{}' in activity '{}'", activity.activity, key),
                location: None,
                suggestion: Some(format!("Valid types: {}", valid_activity_types.join(", "))),
            });
        }
        
        // Check depends_on references exist
        if let Some(deps) = &activity.depends_on {
            for dep in deps {
                if !activity_keys.contains(&dep.as_str()) {
                    errors.push(ValidationError {
                        code: "E011".to_string(),
                        message: format!("Activity '{}' depends on unknown activity '{}'", key, dep),
                        location: None,
                        suggestion: Some(format!("Available activities: {}", activity_keys.iter().join(", "))),
                    });
                }
            }
        }
        
        // Validate template expressions
        validate_template_expressions(key, activity, &activity_keys, errors);
    }
    
    // Check for circular dependencies
    if let Err(cycle) = detect_circular_dependencies(definition) {
        errors.push(ValidationError {
            code: "E020".to_string(),
            message: format!("Circular dependency detected: {}", cycle.join(" -> ")),
            location: None,
            suggestion: Some("Remove or restructure dependencies to break the cycle".to_string()),
        });
    }
}

fn check_best_practices(
    definition: &WorkflowDefinition,
    warnings: &mut Vec<ValidationWarning>,
) {
    for (key, activity) in &definition.activities {
        // Warn if LLM activity has no budget
        if activity.activity == "llm_prompt" {
            if activity.settings.as_ref().map_or(true, |s| s.budget.is_none()) {
                warnings.push(ValidationWarning {
                    code: "W001".to_string(),
                    message: format!("LLM activity '{}' has no budget limit", key),
                    location: None,
                });
            }
        }
        
        // Warn if HTTP activity has no retry policy
        if activity.activity == "http_request" {
            if activity.settings.as_ref().map_or(true, |s| s.retry.is_none()) {
                warnings.push(ValidationWarning {
                    code: "W002".to_string(),
                    message: format!("HTTP activity '{}' has no retry policy", key),
                    location: None,
                });
            }
        }
    }
}

fn print_text_results(results: &[ValidationResult], verbose: bool) {
    let total = results.len();
    let valid = results.iter().filter(|r| r.valid).count();
    let with_warnings = results.iter().filter(|r| !r.warnings.is_empty()).count();
    
    for result in results {
        if result.valid && result.warnings.is_empty() && !verbose {
            println!("{} {}", "✓".green(), result.file.display());
        } else if result.valid && !result.warnings.is_empty() {
            println!("{} {} ({} warnings)", "⚠".yellow(), result.file.display(), result.warnings.len());
            for warning in &result.warnings {
                println!("  {} {}: {}", "warning".yellow(), warning.code, warning.message);
            }
        } else {
            println!("{} {} ({} errors)", "✗".red(), result.file.display(), result.errors.len());
            for error in &result.errors {
                println!("  {} {}: {}", "error".red(), error.code, error.message);
                if let Some(suggestion) = &error.suggestion {
                    println!("    {} {}", "hint:".blue(), suggestion);
                }
            }
        }
    }
    
    println!();
    println!("Validated {} file(s): {} valid, {} with errors, {} with warnings",
        total,
        valid.to_string().green(),
        (total - valid).to_string().red(),
        with_warnings.to_string().yellow(),
    );
}
```

### Implementation Tasks

1. **Create validate command module** (~3 hours)
   - [ ] Define CLI arguments
   - [ ] Implement file/directory collection
   - [ ] Wire up to main CLI

2. **Implement validation logic** (~4 hours)
   - [ ] YAML syntax validation
   - [ ] Schema validation (WorkflowDefinition)
   - [ ] Semantic validation (references, types)
   - [ ] Template expression validation
   - [ ] Circular dependency detection

3. **Implement output formatters** (~2 hours)
   - [ ] Text output with colors and icons
   - [ ] JSON output for CI integration
   - [ ] Verbose mode with suggestions

4. **Add best practice warnings** (~1 hour)
   - [ ] Missing budget on LLM activities
   - [ ] Missing retry on HTTP activities
   - [ ] Other common issues

5. **Testing** (~2 hours)
   - [ ] Test with valid workflows
   - [ ] Test with various error types
   - [ ] Test directory scanning

### Acceptance Criteria

- [ ] `streamflow validate workflow.yaml` validates single file
- [ ] `streamflow validate examples/` validates directory
- [ ] Clear error messages with line numbers where possible
- [ ] Suggestions for how to fix errors
- [ ] Exit code 0 for valid, 1 for errors
- [ ] `--format json` outputs machine-readable results
- [ ] Works with all 10 example workflows

### Example Output

```
$ streamflow validate examples/

✓ examples/01-weather-report.yaml
✓ examples/02-user-validation.yaml
⚠ examples/04-moderate-content.yaml (1 warning)
  warning W001: LLM activity 'analyze_content' has no budget limit
✗ examples/bad-workflow.yaml (2 errors)
  error E010: Unknown activity type 'invalid_activity' in activity 'step1'
    hint: Valid types: http_request, llm_prompt, postgres_query, ...
  error E011: Activity 'step2' depends on unknown activity 'nonexistent'
    hint: Available activities: step1, step3

Validated 12 file(s): 10 valid, 1 with errors, 1 with warnings
```

---

## Quick Win #5: RAG Pipeline Example with Cost Optimization

**Effort**: 1-2 days  
**Impact**: High for AI segment — shows real-world value  
**Files**: `examples/11-rag-cost-optimized.yaml`, `examples/README.md`

### Feature Overview

Create a comprehensive RAG (Retrieval-Augmented Generation) example that showcases:
- Document embedding and storage
- Semantic search
- LLM generation with retrieved context
- **Cost optimization through caching and smart model selection**

This example should be the "poster child" for StreamFlow's AI capabilities.

### Example Workflow

#### File: `examples/11-rag-cost-optimized.yaml`

```yaml
# RAG Pipeline with Cost Optimization
# 
# This workflow demonstrates a production-ready RAG pattern with:
# - Semantic caching to avoid redundant embeddings and LLM calls
# - Budget-aware model selection (falls back to cheaper models)
# - Cost tracking at every step
#
# Expected cost savings vs naive approach:
# - 70%+ on repeated queries (semantic cache)
# - 50%+ on generation (smart model fallback)
#
# Prerequisites:
# - PostgreSQL with pgvector extension
# - OpenAI API key (for embeddings)
# - Anthropic or OpenAI API key (for generation)
#
# Usage:
#   streamflow run examples/11-rag-cost-optimized.yaml \
#     --input '{"query": "What is StreamFlow?", "collection": "docs"}'

name: rag_cost_optimized
description: Production RAG pipeline with aggressive cost optimization

# Workflow-level budget limit
settings:
  budget:
    limit_usd: 0.50
    action: abort

activities:
  # Step 1: Check semantic cache for this exact query
  # If we've answered this before, skip embedding and retrieval entirely
  check_query_cache:
    activity: postgres_query
    parameters:
      query: |
        SELECT 
          response,
          created_at,
          cost_usd
        FROM rag_cache 
        WHERE collection = $1 
          AND query_hash = md5($2)
          AND created_at > NOW() - INTERVAL '1 hour'
        LIMIT 1
      params:
        - "{{INPUT.collection}}"
        - "{{INPUT.query}}"
    outputs:
      - cached_response
      - cache_hit

  # Step 2: Generate query embedding (only if cache miss)
  # Uses semantic caching - similar queries return cached embeddings
  embed_query:
    activity: embedding_generate
    condition: "{{ not check_query_cache.cache_hit }}"
    parameters:
      provider: openai
      model: text-embedding-3-small  # $0.02/1M tokens - very cheap
      input: "{{INPUT.query}}"
    outputs:
      - embedding
      - cost_usd
    settings:
      cache:
        enabled: true
        ttl_seconds: 86400  # Cache embeddings for 24 hours
        similarity_threshold: 0.95  # Reuse for very similar queries

  # Step 3: Retrieve relevant documents via vector search
  retrieve_context:
    activity: postgres_query
    condition: "{{ not check_query_cache.cache_hit }}"
    parameters:
      query: |
        SELECT 
          content,
          metadata,
          1 - (embedding <=> $1::vector) as similarity
        FROM document_chunks
        WHERE collection = $2
        ORDER BY embedding <=> $1::vector
        LIMIT 5
      params:
        - "{{embed_query.embedding}}"
        - "{{INPUT.collection}}"
    outputs:
      - documents
    depends_on:
      - embed_query

  # Step 4: Format context for LLM
  format_context:
    activity: http_request  # Using a simple template formatter
    condition: "{{ not check_query_cache.cache_hit }}"
    parameters:
      method: POST
      url: "{{SECRET.FORMATTER_URL}}"  # Or inline with llm_prompt
      body:
        documents: "{{retrieve_context.documents}}"
        query: "{{INPUT.query}}"
    outputs:
      - formatted_context
    depends_on:
      - retrieve_context

  # Step 5: Generate answer with cost-aware model selection
  # Budget-aware fallback: tries expensive model first, falls back to cheaper
  generate_answer:
    activity: llm_prompt
    condition: "{{ not check_query_cache.cache_hit }}"
    parameters:
      model:
        # Fallback chain - automatically selects based on remaining budget
        - anthropic/claude-sonnet-4-5-20250929   # Best quality ($3/$15 per M)
        - anthropic/claude-3-haiku-20240307       # Good balance ($0.25/$1.25 per M)
        - openai/gpt-4o-mini                      # Cheapest capable ($0.15/$0.60 per M)
      messages:
        - role: system
          content: |
            You are a helpful assistant. Answer the user's question based ONLY on 
            the provided context. If the context doesn't contain the answer, say so.
            
            Context:
            {{format_context.formatted_context}}
        - role: user
          content: "{{INPUT.query}}"
      max_tokens: 1000
    outputs:
      - response
      - model_used
      - cost_usd
    settings:
      budget:
        limit_usd: 0.10  # Per-activity budget
        action: fallback  # Try next model in chain
      cache:
        enabled: true
        ttl_seconds: 3600
    depends_on:
      - format_context

  # Step 6: Cache the response for future queries
  cache_response:
    activity: postgres_query
    condition: "{{ not check_query_cache.cache_hit }}"
    parameters:
      query: |
        INSERT INTO rag_cache (collection, query_hash, query_text, response, model_used, cost_usd)
        VALUES ($1, md5($2), $2, $3, $4, $5)
        ON CONFLICT (collection, query_hash) 
        DO UPDATE SET response = $3, model_used = $4, cost_usd = $5, created_at = NOW()
      params:
        - "{{INPUT.collection}}"
        - "{{INPUT.query}}"
        - "{{generate_answer.response}}"
        - "{{generate_answer.model_used}}"
        - "{{generate_answer.cost_usd}}"
    depends_on:
      - generate_answer

  # Step 7: Return final response (from cache or fresh generation)
  prepare_response:
    activity: http_request
    parameters:
      method: POST
      url: "{{SECRET.RESPONSE_FORMATTER_URL}}"
      body:
        query: "{{INPUT.query}}"
        response: "{{ check_query_cache.cached_response or generate_answer.response }}"
        from_cache: "{{ check_query_cache.cache_hit }}"
        model_used: "{{ generate_answer.model_used or 'cached' }}"
        total_cost: "{{ (embed_query.cost_usd or 0) + (generate_answer.cost_usd or 0) }}"
    outputs:
      - final_response
    depends_on:
      - check_query_cache
      - cache_response  # Optional dependency - runs if cache_response ran
```

#### Supporting SQL Setup

```sql
-- examples/11-rag-setup.sql
-- Run this to set up the required tables

-- Enable pgvector
CREATE EXTENSION IF NOT EXISTS vector;

-- Document chunks table (populated by indexing workflow)
CREATE TABLE IF NOT EXISTS document_chunks (
    id SERIAL PRIMARY KEY,
    collection VARCHAR(100) NOT NULL,
    content TEXT NOT NULL,
    embedding vector(1536),  -- OpenAI text-embedding-3-small dimension
    metadata JSONB,
    created_at TIMESTAMP DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_chunks_collection ON document_chunks(collection);
CREATE INDEX IF NOT EXISTS idx_chunks_embedding ON document_chunks 
    USING ivfflat (embedding vector_cosine_ops) WITH (lists = 100);

-- RAG response cache table
CREATE TABLE IF NOT EXISTS rag_cache (
    id SERIAL PRIMARY KEY,
    collection VARCHAR(100) NOT NULL,
    query_hash VARCHAR(32) NOT NULL,
    query_text TEXT NOT NULL,
    response TEXT NOT NULL,
    model_used VARCHAR(100),
    cost_usd DECIMAL(10, 6),
    created_at TIMESTAMP DEFAULT NOW(),
    UNIQUE(collection, query_hash)
);

CREATE INDEX IF NOT EXISTS idx_rag_cache_lookup 
    ON rag_cache(collection, query_hash, created_at);
```

### Documentation Update

Add to `examples/README.md`:

```markdown
### Example 11: RAG Pipeline with Cost Optimization

**File**: `11-rag-cost-optimized.yaml`

**Features Demonstrated**:
- Complete RAG (Retrieval-Augmented Generation) pipeline
- Semantic caching at query and response levels
- Budget-aware model fallback
- PostgreSQL with pgvector for similarity search
- Cost tracking and optimization

**Cost Optimization Strategies**:
1. **Query-level caching**: Skip entire pipeline for repeated queries
2. **Embedding caching**: Reuse embeddings for similar queries (95% threshold)
3. **Response caching**: Cache LLM responses for 1 hour
4. **Model fallback**: Start with best model, fall back to cheaper on budget constraints

**Expected Savings**:
| Scenario | Naive Cost | StreamFlow Cost | Savings |
|----------|------------|-----------------|---------|
| Repeated query | $0.05 | $0.00 | 100% |
| Similar query | $0.05 | $0.02 | 60% |
| New query (budget ok) | $0.05 | $0.05 | 0% |
| New query (budget tight) | $0.05 | $0.02 | 60% |

**Prerequisites**:
1. PostgreSQL with pgvector extension
2. OpenAI API key (for embeddings)
3. Anthropic or OpenAI API key (for generation)
4. Run setup SQL: `psql -f examples/11-rag-setup.sql`

**Usage**:
```bash
# Index some documents first (using example 06b)
streamflow run examples/06b-rag-index-builder.yaml \
  --input '{"chunks": ["StreamFlow is...", "Features include..."]}'

# Query with RAG
streamflow run examples/11-rag-cost-optimized.yaml \
  --input '{"query": "What is StreamFlow?", "collection": "docs"}'

# Check costs
streamflow costs workflow <workflow_id> --detailed
```
```

### Implementation Tasks

1. **Create example workflow** (~3 hours)
   - [ ] Write YAML workflow definition
   - [ ] Test with real LLM providers
   - [ ] Verify caching behavior
   - [ ] Verify fallback behavior

2. **Create setup SQL** (~1 hour)
   - [ ] Document chunks table with vector index
   - [ ] RAG cache table
   - [ ] Necessary indexes

3. **Update examples README** (~1 hour)
   - [ ] Add example 11 documentation
   - [ ] Include cost comparison table
   - [ ] Add prerequisites and usage

4. **End-to-end testing** (~2 hours)
   - [ ] Test cache hit path
   - [ ] Test cache miss path
   - [ ] Test budget fallback
   - [ ] Verify cost tracking accuracy

5. **Create demo script** (~1 hour)
   - [ ] Script to index sample docs
   - [ ] Script to run queries
   - [ ] Script to show cost savings

### Acceptance Criteria

- [ ] Workflow runs successfully end-to-end
- [ ] Cache hit skips embedding and generation
- [ ] Budget fallback works correctly
- [ ] Cost tracking shows savings
- [ ] Documentation explains all features
- [ ] Demo script works out of the box

---

## Summary: Implementation Order

| # | Item | Effort | Dependencies | Priority |
|---|------|--------|--------------|----------|
| 1 | README Polish | 1 day | None | Do first |
| 2 | Dockerfile | 1 day | None | Do first |
| 3 | `streamflow costs` | 2 days | None | High impact |
| 4 | `streamflow validate` | 1-2 days | None | Good DX |
| 5 | RAG Example | 1-2 days | Items 3,4 helpful | Demo value |

**Recommended sequence**:
- **Day 1-2**: README + Dockerfile (parallel tracks)
- **Day 3-4**: `streamflow costs` command
- **Day 5**: `streamflow validate` command  
- **Day 6**: RAG example + final testing

**Total**: 6-9 days to complete all 5 quick wins

After these are complete, you'll have:
- ✅ Professional README that converts visitors
- ✅ One-command Docker deployment
- ✅ Visible cost tracking (your key differentiator)
- ✅ Developer-friendly validation
- ✅ Showcase example for AI use case

This sets you up for a strong Show HN launch.
