# Deep Research Demo Implementation Plan

**Feature**: Deep Research Agent with Live Cost Control Demo
**Status**: Planning
**Target**: Week 1-2 MVP Demo, Week 3-4 Enterprise Extensions
**Last Updated**: 2025-01-03

---

## Table of Contents

1. [Executive Summary](#executive-summary)
2. [Current State Analysis](#current-state-analysis)
3. [Gap Analysis](#gap-analysis)
4. [UI Evolution Path: Demo → Grafana → Full Web Designer](#ui-evolution-path-demo--grafana--full-web-designer)
5. [Technical Architecture](#technical-architecture)
6. [Implementation Phases](#implementation-phases)
7. [Integration Points](#integration-points)
8. [Testing Strategy](#testing-strategy)
9. [Success Criteria](#success-criteria)
10. [Summary: Strategic Value of Demo UI](#summary-strategic-value-of-demo-ui)

---

## Executive Summary

This implementation plan details how to build the Deep Research Agent demo that showcases Kruxia Flow's core differentiators: cost control, intelligent orchestration, and AI-native workflow capabilities. The demo implements a Plan-and-Execute architecture where a planner agent generates research questions, parallel researcher agents gather information, an evaluator determines sufficiency, and an editor synthesizes the final report—all with real-time cost tracking and intelligent model routing.

**Key Value Proposition**: Demonstrate 70% cost reduction vs naive "always use GPT-4" approach while maintaining equivalent quality, with live cost meter showing spend progression from $0.00 to ~$0.47.

**Current State**: Kruxia Flow already has 90% of required capabilities:
- ✅ Multi-step DAG workflows with conditional execution
- ✅ Iterative loops with budget accumulation
- ✅ Multi-provider LLM integration (Anthropic, OpenAI, Google)
- ✅ Cost tracking and budget enforcement per-activity
- ✅ WebSocket streaming for real-time UX
- ✅ Parallel fan-out/fan-in execution
- ✅ Model fallback chains for reliability

**Missing Components**:
- ⚠️ Web scraping activity (small addition: ~100 lines Rust)
- ⚠️ Search API integration (HTTP activity + API keys, zero code)
- ⚠️ Live cost dashboard UI (optional for CLI demo, needed for visual impact)
- ⚠️ Intelligent routing logic (workaround: pre-classification step)

**Timeline**:
- **Week 1**: Functional CLI demo with basic research workflow and cost tracking
- **Week 2**: Add parallel researchers, comparison mode, and simple web UI
- **Week 3-4**: Extend to vertical applications (competitive intel, due diligence)

---

## Current State Analysis

### Existing Capabilities (Leverageable)

#### 1. Workflow Orchestration Foundation
**Status**: ✅ Complete (Epic 1, Epic 3 90% done)

Kruxia Flow provides sophisticated workflow capabilities that map directly to deep research requirements:

- **Plan-and-Execute Pattern**: Examples 07a/07b demonstrate planner → researcher → evaluator → editor pattern
- **Iterative Loops**: `iteration_scoped: true` with back-edge dependencies enables multi-pass research
- **Conditional Execution**: Jinja2 template conditions on `depends_on` allow branching based on LLM outputs
- **Budget Accumulation**: `{{ACTIVITY.accumulated_cost_usd}}` and `{{ACTIVITY.remaining_budget_usd}}` track spending across iterations
- **Iteration Limits**: `iteration_limit: 5` prevents runaway loops
- **Parallel Execution**: Fan-out to multiple researcher agents with fan-in for synthesis

**Relevant Examples**:
- `07a-agentic-research-simple.yaml`: Initialize → loop(search + evaluate) → compile pattern
- `09b-streaming-research.yaml`: Multi-step research with streaming final output
- `05a/05b/05c-research-assistant-*.yaml`: Multi-provider fallback chains

#### 2. LLM Integration with Cost Control
**Status**: ✅ Complete (Epic 5 US-5.1)

The `llm_prompt` activity provides production-grade LLM integration:

- **Multi-Provider Support**: Anthropic (Claude Opus/Sonnet/Haiku), OpenAI (GPT-4o/GPT-4o-mini), Google (Gemini 2.0 Flash/Pro), Ollama (local)
- **Fallback Chains**: `model: [primary, secondary, tertiary]` array with automatic failover on rate limits/errors
- **Budget Enforcement**:
  - Pre-execution cost estimation using token heuristics
  - `settings.budget.limit_usd` with `action: abort` or `action: degrade`
  - Post-execution actual cost tracking from API response
- **Cost Calculation**: Stored in `activity_costs` table with provider/model/tokens breakdown
- **Token Streaming**: WebSocket delivery for real-time UX with two-level opt-in

**Cost API Endpoints** (already implemented):
- `GET /api/v1/workflows/{id}/cost` - Total cost summary
- `GET /api/v1/workflows/{id}/cost/history` - Per-activity cost breakdown with provider/model/tokens

**Example Budget Configuration**:
```yaml
- key: expensive_analysis
  activity_name: llm_prompt
  parameters:
    model:
      - anthropic/claude-opus-4-1-20250514      # Try premium first
      - anthropic/claude-sonnet-4-5-20250929    # Fall back to mid-tier
      - anthropic/claude-haiku-4-20250415       # Last resort: cheapest
    prompt: "{{INPUT.query}}"
    max_tokens: 1000
  settings:
    budget:
      limit_usd: 0.01    # Tight budget forces fallback to Haiku
      action: abort      # Or: degrade (skip to cheaper model immediately)
```

#### 3. HTTP Activity for External Integrations
**Status**: ✅ Complete (Epic 5)

The `http_request` activity supports arbitrary HTTP API calls:

- **Full HTTP Methods**: GET, POST, PUT, DELETE, PATCH
- **Headers/Query/Body**: Template interpolation for dynamic values
- **Authentication**: `Authorization: Bearer {{SECRET.api_key}}` pattern
- **Timeout Configuration**: Per-request timeout control
- **File Download**: Stream responses to storage for large payloads

**Relevant for Deep Research**:
- Search API calls: Perplexity, Tavily, SerpAPI (all are REST APIs)
- Academic sources: arXiv API, Google Scholar unofficial APIs
- News sources: NewsAPI, GDELT

**Example Usage**:
```yaml
- key: call_search_api
  activity_name: http_request
  parameters:
    method: POST
    url: "https://api.perplexity.ai/search"
    headers:
      Authorization: "Bearer {{SECRET.perplexity_api_key}}"
      Content-Type: "application/json"
    body:
      query: "{{INPUT.research_question}}"
      max_results: 10
  outputs:
    - response
```

#### 4. Workflow State Querying
**Status**: ✅ Complete (Epic 1A US-1A.6)

API provides real-time workflow state access:

- `GET /api/v1/workflows/{id}` - Current workflow status with activity states
- `GET /api/v1/workflows/{id}/output` - All activity outputs for completed workflows
- `GET /api/v1/workflows/{id}/activities/{key}/output` - Single activity output

**Activity States**:
- `created` - Scheduled but not started
- `running` - Currently executing
- `completed` - Finished successfully
- `failed` - Activity failed

**Polling Strategy for Live Updates**:
Client can poll workflow status endpoint every 1-2 seconds to build live progress UI showing:
- Which activities are running
- Cost accumulated so far
- Current iteration number (for looped activities)

#### 5. Authentication & Security
**Status**: ✅ Complete (Epic 1A US-1A.3)

- JWT-based authentication for all API requests
- Client credentials flow for programmatic access
- Secrets accessible via `{{SECRET.key_name}}` template variable
- Environment variable injection for API keys

### Architecture Alignment

The deep research demo aligns perfectly with Kruxia Flow's documented architecture:

**Service Interface Pattern**: All external dependencies wrapped in service interfaces
- ✅ ActivityQueue (PostgreSQL-based)
- ✅ EventSource (polling with adaptive backoff)
- ✅ AuthenticationService (custom JWT)
- ✅ WorkflowStorage (PostgreSQL Large Objects)

**Configuration**: Environment variables only (no .env files per guidelines)
- `ANTHROPIC_API_KEY`
- `OPENAI_API_KEY`
- `PERPLEXITY_API_KEY`
- `TAVILY_API_KEY`

**Optimization Scope**: MVP focus on standard event-driven DAG, ~1ms orchestration latency
- Target: >1,000 workflows/sec via PostgreSQL query optimization
- Not implementing: Compiled workflow optimizations (post-MVP)

**Database**: sqlx with compile-time query validation
- Cost tracking via `activity_costs` table
- Workflow state in `workflows.state_data` JSONB column

---

## Gap Analysis

### What's Missing for Deep Research Demo

#### 1. Web Scraping Activity
**Status**: ⚠️ Not Implemented
**Priority**: Medium (Week 2)
**Effort**: Small (~100 lines Rust)

**Current Workaround**: Use `http_request` to fetch HTML, then pass to LLM for parsing
- ✅ Works for simple pages
- ❌ Cannot handle JavaScript-rendered content
- ❌ No CSS selector extraction
- ❌ LLM parsing costs 10-50x more tokens than clean extraction

**What's Needed**:
A new built-in activity `web_scrape` with capabilities:
- HTML fetching with configurable headers (User-Agent, Accept)
- CSS selector-based extraction (using `scraper` or `select.rs` crate)
- Text cleaning (remove scripts, styles, nav elements)
- Table extraction to structured data
- Link extraction for crawling

**Implementation Approach**:
1. Create `WebScrapeActivity` struct implementing `ActivityImpl` trait
2. Parameters:
   - `url: String` (template interpolated)
   - `selector: Option<String>` (CSS selector, defaults to body text)
   - `extract_links: bool` (return hyperlinks found)
   - `clean_html: bool` (strip scripts/styles)
   - `timeout_seconds: u64`
3. Use `reqwest` for HTTP + `scraper` crate for CSS selectors
4. Return: `{ "text": "...", "links": [...], "raw_html": "..." }`

**Alternative**: Keep using `http_request` + LLM parsing for MVP
- Demonstrates "deterministic + AI hybrid" workflow model
- Shows cost trade-off (scraping is free, LLM parsing costs tokens)
- Can add native scraper in Week 2 for cost optimization demo

#### 2. Search API Integration
**Status**: ⚠️ Zero Code Needed (HTTP + API Keys)
**Priority**: High (Week 1)
**Effort**: Configuration Only

**No Implementation Required**: All search APIs are REST endpoints usable with existing `http_request` activity.

**Recommended Search Provider**: Perplexity API or Tavily
- **Perplexity**: $5/1000 requests, returns synthesized answers + sources
- **Tavily**: $0.005/request, optimized for AI agents, returns clean structured data
- **SerpAPI**: $50/5000 searches, Google/Bing scraping

**Example Integration** (Tavily):
```yaml
- key: search_topic
  activity_name: http_request
  parameters:
    method: POST
    url: "https://api.tavily.com/search"
    headers:
      Content-Type: "application/json"
    body:
      api_key: "{{SECRET.tavily_api_key}}"
      query: "{{INPUT.research_question}}"
      search_depth: "advanced"  # or "basic"
      max_results: 5
  outputs:
    - response
```

**Setup Required**:
1. Sign up for search API (Tavily recommended)
2. Set `TAVILY_API_KEY` environment variable
3. Reference via `{{SECRET.tavily_api_key}}` in workflow

**Post-MVP Enhancement**: Create `web_search` built-in activity to abstract provider differences and add retry logic.

#### 3. Intelligent Model Routing
**Status**: ⚠️ Workaround Available (Pre-Classification)
**Priority**: Medium (Week 2)
**Effort**: Medium (requires routing logic)

**Current Capability**: Sequential fallback chain
- Try `model[0]`, if fails → try `model[1]`, if fails → try `model[2]`
- Failure triggers: Rate limit (429), API error (500), budget exceeded

**Missing**: Intelligent routing based on query complexity
- Simple queries → Haiku ($0.25/M tokens)
- Medium queries → Sonnet ($3/M tokens)
- Complex queries → Opus ($15/M tokens)

**Week 1 Workaround**: Pre-classification step
```yaml
- key: classify_complexity
  activity_name: llm_prompt
  parameters:
    model: anthropic/claude-haiku-4-20250415  # Cheap classifier
    prompt: |
      Classify this research question complexity as: SIMPLE, MEDIUM, or COMPLEX
      Question: {{INPUT.research_question}}

      Respond with only one word.
    max_tokens: 5
  outputs:
    - result

- key: research_simple
  activity_name: llm_prompt
  parameters:
    model: anthropic/claude-haiku-4-20250415
    prompt: "Research: {{INPUT.research_question}}"
  depends_on:
    - activity_key: classify_complexity
      conditions:
        - "{{classify_complexity.result.content | contains('SIMPLE')}}"

- key: research_complex
  activity_name: llm_prompt
  parameters:
    model: anthropic/claude-sonnet-4-5-20250929
    prompt: "Research: {{INPUT.research_question}}"
  depends_on:
    - activity_key: classify_complexity
      conditions:
        - "{{classify_complexity.result.content | contains('COMPLEX')}}"
```

**Week 2 Enhancement**: RouteLLM-style complexity detection
- Implement `intelligent_routing` activity that uses Berkeley RouteLLM approach
- Analyze query characteristics: length, entities, domain-specific terms
- Route without LLM call (save extra $0.001 per routing decision)

**Post-MVP**: Native routing in `llm_prompt` activity
- Add `routing_strategy: intelligent` parameter
- Automatic complexity detection built into activity implementation

#### 4. Live Cost Dashboard UI
**Status**: ⚠️ APIs Complete, Demo UI Needed
**Priority**: High (Week 1 for visual demo)
**Effort**: Medium (frontend work)

**Current State**: Cost tracking infrastructure is MVP-complete
- ✅ Cost Dashboard API fully implemented (`docs/cost-dashboard-api.md`)
- ✅ `GET /api/v1/workflows/{id}/cost` - Cost summary with budget status
- ✅ `GET /api/v1/workflows/{id}/cost/history` - Per-activity breakdown with tokens/model
- ✅ `GET /api/v1/cost/analytics` - Aggregated metrics across workflows
- ❌ No visual UI (only API endpoints)

**Alignment with System Roadmap**:
This demo UI serves as a proof-of-concept that validates the Cost API design and provides a stepping stone to the planned production UIs:

1. **Week 1-2 (Demo)**: Simple HTML/JS cost dashboard for demo impact
2. **Post-MVP Phase 1 (~3 months)**: Migrate to **Grafana dashboards** with Prometheus metrics (Epic 5, Story 5.1 - Priority P1)
3. **Post-MVP Phase 3 (~9 months)**: Full **Web-Based Workflow Designer** with integrated monitoring (Epic 4, Story 4.5 - Priority P2)

**Key Benefits**:
- ✅ **Not throwaway work**: Cost APIs are permanent, validated by demo usage
- ✅ **Reusable backend**: Same endpoints used by future Grafana dashboards
- ✅ **Incremental path**: Demo UI → Grafana (production monitoring) → Full web designer
- ✅ **Demo impact**: Live cost visualization creates "I need this" moment
- ✅ **API validation**: Tests cost endpoints under real UI load

**Week 1 CLI Approach**: Script that polls cost API
```bash
#!/bin/bash
WORKFLOW_ID=$1
while true; do
  curl -s "http://localhost:8080/api/v1/workflows/$WORKFLOW_ID/cost" \
    -H "Authorization: Bearer $TOKEN" \
    | jq '.total_cost_usd'
  sleep 1
done
```

**Week 2 Web UI Approach**: Simple HTML + JavaScript
- Single-page app that:
  1. Submits workflow via API
  2. Polls workflow status every 1s
  3. Displays:
     - Running cost meter: `$0.047 / $0.10 budget`
     - Activity progress bars: `initialize ✓ | planner ⏳ | researchers ⬜⬜⬜`
     - Cost breakdown table: per-activity costs
     - Comparison line: "Naive GPT-4 approach would cost: $0.15"
- Technologies: Vanilla JS (no framework), Chart.js for cost graph
- Hosting: Static HTML served from `docs/demo/index.html`

**Visual Elements for Demo**:
1. **Cost Meter**: Real-time ticker showing $0.00 → $0.47
2. **Activity Timeline**: Gantt-style view of parallel execution
3. **Model Routing Indicator**: Shows which model was selected for each step
4. **Comparison Graph**: Side-by-side bar chart of Kruxia Flow vs naive approach
5. **Budget Threshold Warnings**: Yellow at 80%, red at 95%

**Technical Implementation**:
- Poll `GET /api/v1/workflows/{id}` every 1000ms for status
- Poll `GET /api/v1/workflows/{id}/cost` every 1000ms for cost updates
- Use Server-Sent Events (SSE) or WebSocket for real-time updates (post-MVP)
- Store cost history in client-side array for graphing

#### 5. Comparison Mode (Naive vs Optimized)
**Status**: ❌ Not Implemented
**Priority**: Medium (Week 2)
**Effort**: Small (run workflow twice with different configs)

**Purpose**: Demonstrate cost savings by running same research with two strategies:
1. **Naive**: Always use GPT-4 ($30/$60 per M tokens)
2. **Optimized**: Intelligent routing (Haiku for simple, Sonnet for complex)

**Implementation Approach**:
1. Submit two workflows in parallel:
   - `naive_research` with all activities using `gpt-4o`
   - `optimized_research` with intelligent routing
2. Compare final costs:
   - Display side-by-side: "Naive: $1.52 | Optimized: $0.47 | Savings: 69%"
3. Verify quality equivalence:
   - LLM-as-judge: Have Claude evaluate both reports for quality
   - Display: "Quality score: 8.5/10 vs 8.7/10 (equivalent)"

**Workflow Definition** (naive version):
```yaml
name: naive_research_gpt4_only
activities:
  - key: planner
    activity_name: llm_prompt
    parameters:
      model: openai/gpt-4o  # Expensive for planning
      prompt: "Create research plan for: {{INPUT.topic}}"

  - key: researcher
    activity_name: llm_prompt
    parameters:
      model: openai/gpt-4o  # Expensive for research
      prompt: "Research: {{INPUT.question}}"

  - key: editor
    activity_name: llm_prompt
    parameters:
      model: openai/gpt-4o  # Expensive for synthesis
      prompt: "Compile report from: {{researcher.result}}"
```

**Demonstration Value**:
- Shows Kruxia Flow's intelligent routing saves 60-80% cost
- Proves quality is maintained (LLM-as-judge validation)
- Creates memorable demo moment: "Same quality, fraction of cost"

### UI Evolution Path: Demo → Grafana → Full Web Designer

The deep research demo UI is intentionally designed as the first step in Kruxia Flow's UI roadmap, not as throwaway prototype work. This section clarifies how the demo UI aligns with and enables future production UIs.

#### Phase 1: Demo UI (Week 1-2) - Cost Visualization Proof-of-Concept

**Technology**: Single-page HTML/JS with Chart.js
**Scope**:
- Real-time cost tracking via polling
- Activity progress visualization
- Comparison mode (optimized vs naive)
- Workflow status monitoring

**Value**:
- ✅ Creates "killer demo moment" for investors/customers
- ✅ Validates Cost Dashboard API design under real usage
- ✅ Tests API performance (1s polling = 3600 requests/hour)
- ✅ Identifies API gaps/issues before production
- ✅ Provides reference implementation for future UI integrations

**Reusable Assets**:
- API endpoint usage patterns → copied to Grafana queries
- Cost calculation logic → reused in Prometheus metrics
- Visualization queries → translated to Grafana dashboard JSON

#### Phase 2: Grafana Dashboards (Post-MVP Phase 1, ~3 months)

**Reference**: Epic 5, Story 5.1 - Metrics and Monitoring (Priority P1)
**Technology**: Grafana + Prometheus
**Scope**:
- Pre-built Grafana dashboards for cost tracking
- System health metrics (workflow throughput, latency, errors)
- Prometheus metrics endpoint (`/metrics`)
- AlertManager integration for budget threshold alerts

**Migration from Demo UI**:
1. Import demo's cost queries into Grafana panels
2. Add Prometheus metrics for:
   - `kruxiaflow_workflow_cost_usd{workflow_id}`
   - `kruxiaflow_activity_cost_usd{activity_key,model,provider}`
   - `kruxiaflow_budget_utilization_ratio{workflow_id}`
3. Deprecate standalone demo UI (redirect to Grafana)

**Value**:
- Enterprise-grade monitoring and alerting
- Multi-tenant cost visibility
- Historical trending and analytics
- Integration with existing observability stacks

#### Phase 3: Web-Based Workflow Designer (Post-MVP Phase 3, ~9 months)

**Reference**: Epic 4, Story 4.5 - Web-Based Workflow Designer (Priority P2)
**Technology**: Drag-and-drop web UI (framework TBD)
**Scope**:
- Visual workflow design and editing
- Integrated cost monitoring (reuses Grafana panels)
- Activity palette and parameter editor
- Real-time YAML preview
- Workflow deployment and version management

**Integration with Cost Tracking**:
- Embedded Grafana panels for cost metrics
- Cost estimation during workflow design (pre-execution)
- Budget recommendation based on activity models
- Historical cost data from Grafana datasource

**Value**:
- Non-technical users can create workflows
- Cost visibility integrated into workflow design process
- Lower barrier to entry for Kruxia Flow adoption

#### Key Design Principle: API-First Architecture

All three UI phases share the same underlying **Cost Dashboard API** (`docs/cost-dashboard-api.md`):
- `GET /api/v1/workflows/{id}/cost` - Cost summary
- `GET /api/v1/workflows/{id}/cost/history` - Per-activity breakdown
- `GET /api/v1/cost/analytics` - Aggregated metrics

**Benefits of This Approach**:
- ✅ **No duplicate backend work**: One API serves all UIs
- ✅ **Stable interface**: Frontend evolution doesn't break APIs
- ✅ **Incremental investment**: Each phase builds on previous work
- ✅ **Flexibility**: Customers can choose UI tier (CLI → Grafana → Full Web)
- ✅ **Validation**: Demo UI proves APIs work before committing to Grafana/Web Designer

**Architecture Decision**: Build lightweight demo UI now to validate APIs and create demo impact, knowing it's the foundation for production monitoring (Grafana) and eventual workflow designer (full web UI).

---

## Technical Architecture

### Workflow Structure: Plan-and-Execute Pattern

The deep research workflow implements a sophisticated Plan-and-Execute architecture with iterative refinement:

```mermaid
flowchart TB
    Start([Workflow Start]) --> Initialize

    Initialize["1: Initialize<br/>• Analyze topic<br/>• Generate strategy<br/>• Define success criteria<br/>Model: Haiku"]

    Initialize --> Planner["2: Planner<br/>• Generate 5-7 questions<br/>• Assign priorities<br/>• Estimate depth needed<br/>Model: Haiku"]

    Planner --> ParallelGate{Fan-Out}

    ParallelGate --> R1["3a: Researcher 1<br/>• Question 1<br/>• Web search + scrape<br/>• Model: Sonnet/Haiku"]
    ParallelGate --> R2["3b: Researcher 2<br/>• Question 2<br/>• Web search + scrape<br/>• Model: Sonnet/Haiku"]
    ParallelGate --> R3["3c: Researcher 3<br/>• Question 3<br/>• Web search + scrape<br/>• Model: Sonnet/Haiku"]
    ParallelGate --> R4["3d: Researcher 4<br/>• Question 4<br/>• Web search + scrape<br/>• Model: Sonnet/Haiku"]
    ParallelGate --> R5["3e: Researcher 5<br/>• Question 5<br/>• Web search + scrape<br/>• Model: Sonnet/Haiku"]

    R1 --> FanIn{Fan-In}
    R2 --> FanIn
    R3 --> FanIn
    R4 --> FanIn
    R5 --> FanIn

    FanIn --> Evaluator["4: Evaluator<br/>• Check completeness<br/>• Identify gaps<br/>• Decision: CONTINUE/SUFFICIENT<br/>Model: Haiku"]

    Evaluator --> LoopDecision{Decision?}

    LoopDecision -->|CONTINUE<br/>iteration < 3| Planner
    LoopDecision -->|SUFFICIENT| Editor

    Editor["5: Editor<br/>• Synthesize findings<br/>• Format report<br/>• Generate citations<br/>Model: Sonnet"]

    Editor --> End([Workflow Complete])

    classDef planning fill:#e1f5ff,stroke:#333
    classDef research fill:#fff4e1,stroke:#333
    classDef synthesis fill:#e8f5e9,stroke:#333
    classDef decision fill:#f3e5f5,stroke:#333

    class Initialize,Planner planning
    class R1,R2,R3,R4,R5 research
    class Editor synthesis
    class Evaluator,LoopDecision decision
```

### Activity Breakdown

#### Activity 1: Initialize
**Purpose**: Analyze topic and create research strategy
**Model**: Haiku ($0.25/M input, $1.25/M output)
**Expected Cost**: ~$0.001

**Parameters**:
- Input: `{{INPUT.topic}}` (user-provided research topic)
- Output: Research strategy with success criteria

**Template**:
```yaml
- key: initialize
  activity_name: llm_prompt
  parameters:
    model: anthropic/claude-haiku-4-20250415
    prompt: |
      Analyze this research topic and create a comprehensive research strategy:
      Topic: {{INPUT.topic}}

      Generate a research strategy with:
      1: Research scope - What aspects to cover
      2: Information depth - Level of detail needed
      3: Success criteria - What makes research "complete"
      4: Estimated complexity - SIMPLE/MEDIUM/COMPLEX

      Return valid JSON:
      {
        "scope": "description",
        "depth": "basic|intermediate|comprehensive",
        "criteria": "success criteria",
        "complexity": "SIMPLE|MEDIUM|COMPLEX"
      }
    max_tokens: 500
  outputs:
    - result
  settings:
    budget:
      limit_usd: 0.005
      action: abort
```

#### Activity 2: Planner
**Purpose**: Generate specific research questions based on strategy
**Model**: Haiku ($0.25/M input, $1.25/M output)
**Expected Cost**: ~$0.002

**Dependencies**: initialize (needs research strategy)

**Template**:
```yaml
- key: planner
  activity_name: llm_prompt
  iteration_scoped: true
  parameters:
    model: anthropic/claude-haiku-4-20250415
    prompt: |
      Based on this research strategy:
      {{initialize.result.content}}

      Topic: {{INPUT.topic}}
      Current iteration: {{ACTIVITY.iteration}}

      {% if ACTIVITY.iteration > 0 %}
      Previous findings:
      {{researcher_1.result | json}}
      {{researcher_2.result | json}}
      {{researcher_3.result | json}}

      Gaps identified:
      {{evaluator.result.content | last}}
      {% endif %}

      Generate 5 research questions (or address gaps if iteration > 0).

      Return valid JSON:
      {
        "questions": [
          {"id": 1, "question": "...", "priority": "high|medium|low"},
          ...
        ]
      }
    max_tokens: 800
  outputs:
    - result
  depends_on:
    - initialize
    - activity_key: evaluator
      conditions:
        - "{{evaluator.result.content | contains('CONTINUE')}}"
```

#### Activities 3a-3e: Parallel Researchers
**Purpose**: Execute research for assigned questions in parallel
**Model**: Intelligent routing (Haiku for simple, Sonnet for complex)
**Expected Cost**: ~$0.005-0.015 each (depending on routing)

**Each researcher**:
1. Receives one research question from planner
2. Calls search API (Tavily/Perplexity)
3. Optionally scrapes source URLs for detail
4. Synthesizes findings with LLM

**Template** (Researcher 1, others similar):
```yaml
- key: researcher_1
  activity_name: llm_prompt
  iteration_scoped: true
  parameters:
    # Fallback chain: try Sonnet, fall back to Haiku if budget tight
    model:
      - anthropic/claude-sonnet-4-5-20250929
      - anthropic/claude-haiku-4-20250415
    prompt: |
      Research this question comprehensively:
      {{planner.result.content.questions[0].question}}

      Context: {{initialize.result.content}}

      Use these search results:
      {{search_question_1.response.results | json}}

      Synthesize findings addressing:
      1: Direct answer to the question
      2: Supporting evidence and sources
      3: Confidence level (0.0-1.0)
      4: Gaps requiring more research

      Return valid JSON:
      {
        "answer": "...",
        "evidence": ["source 1 says...", "source 2 shows..."],
        "sources": ["url1", "url2"],
        "confidence": 0.85,
        "gaps": ["gap 1", "gap 2"]
      }
    max_tokens: 1500
  outputs:
    - result
  settings:
    budget:
      limit_usd: 0.02
      action: degrade  # Fall back to Haiku if Sonnet would exceed budget
  depends_on:
    - planner
    - search_question_1
```

**Parallel Search Activities** (one per researcher):
```yaml
- key: search_question_1
  activity_name: http_request
  parameters:
    method: POST
    url: "https://api.tavily.com/search"
    headers:
      Content-Type: "application/json"
    body:
      api_key: "{{SECRET.tavily_api_key}}"
      query: "{{planner.result.content.questions[0].question}}"
      search_depth: "advanced"
      max_results: 5
      include_raw_content: true
  outputs:
    - response
  depends_on:
    - planner
```

**Parallelism**: All 5 researchers execute simultaneously once planner completes, demonstrating Kruxia Flow's parallel execution capability.

#### Activity 4: Evaluator
**Purpose**: Assess research completeness and decide to continue or compile
**Model**: Haiku ($0.25/M input, $1.25/M output)
**Expected Cost**: ~$0.001

**Dependencies**: All 5 researchers (fan-in point)

**Template**:
```yaml
- key: evaluator
  activity_name: llm_prompt
  iteration_scoped: true
  parameters:
    model: anthropic/claude-haiku-4-20250415
    prompt: |
      Evaluate research completeness:

      Success criteria:
      {{initialize.result.content.criteria}}

      Findings from 5 researchers:
      1: {{researcher_1.result.content | json}}
      2: {{researcher_2.result.content | json}}
      3: {{researcher_3.result.content | json}}
      4: {{researcher_4.result.content | json}}
      5: {{researcher_5.result.content | json}}

      Current iteration: {{ACTIVITY.iteration}}
      Budget remaining: ${{ACTIVITY.remaining_budget_usd}}

      Decide if research is SUFFICIENT or needs to CONTINUE.
      If CONTINUE, identify specific gaps to address.

      Respond with ONLY:
      SUFFICIENT
      OR
      CONTINUE: [specific gaps to address]
    max_tokens: 200
  outputs:
    - result
  depends_on:
    - researcher_1
    - researcher_2
    - researcher_3
    - researcher_4
    - researcher_5
```

**Loop Back Logic**:
- If output contains "CONTINUE" → planner depends_on evaluator with condition triggers → new iteration
- If output is "SUFFICIENT" → editor depends_on evaluator with condition triggers → compile report
- Maximum 3 iterations enforced via `iteration_limit` on planner

#### Activity 5: Editor
**Purpose**: Synthesize all findings into comprehensive report
**Model**: Sonnet ($3/M input, $15/M output)
**Expected Cost**: ~$0.020

**Dependencies**: evaluator (when decision is SUFFICIENT)

**Template**:
```yaml
- key: editor
  activity_name: llm_prompt
  parameters:
    model: anthropic/claude-sonnet-4-5-20250929
    prompt: |
      Compile a comprehensive research report on: {{INPUT.topic}}

      Research strategy:
      {{initialize.result.content}}

      All findings from {{ACTIVITY.iteration}} research iterations:
      {% for iteration in researcher_1.result %}
      Iteration {{loop.index0}}:
      - Researcher 1: {{researcher_1.result[loop.index0].content}}
      - Researcher 2: {{researcher_2.result[loop.index0].content}}
      - Researcher 3: {{researcher_3.result[loop.index0].content}}
      - Researcher 4: {{researcher_4.result[loop.index0].content}}
      - Researcher 5: {{researcher_5.result[loop.index0].content}}
      {% endfor %}

      Generate a well-structured research report with:
      1: Executive Summary (2-3 paragraphs)
      2: Detailed Findings (organized by research questions)
      3: Evidence and Sources (properly cited)
      4: Conclusions and Insights
      5: Areas for Further Research

      Target length: 2000-2500 words
    max_tokens: 4000
  outputs:
    - result
  settings:
    budget:
      limit_usd: 0.05
      action: abort
  depends_on:
    - activity_key: evaluator
      conditions:
        - "{{evaluator.result.content | contains('SUFFICIENT')}}"
```

### Cost Breakdown (Expected)

| Activity       | Model         | Input Tokens | Output Tokens | Cost Per Run | Iterations | Total Cost |
|----------------|---------------|--------------|---------------|--------------|------------|------------|
| initialize     | Haiku         | 200          | 400           | $0.001       | 1          | $0.001     |
| planner        | Haiku         | 500          | 600           | $0.002       | 2 avg      | $0.004     |
| search_q1-5    | API (Tavily)  | -            | -             | $0.005       | 2 avg      | $0.010     |
| researcher_1-5 | Sonnet/Haiku  | 1000 avg     | 1200 avg      | $0.012 avg   | 2×5        | $0.120     |
| evaluator      | Haiku         | 3000         | 150           | $0.001       | 2 avg      | $0.002     |
| editor         | Sonnet        | 5000         | 2500          | $0.053       | 1          | $0.053     |
| **TOTAL**      |               |              |               |              |            | **$0.190** |

**Budget Configuration**:
- Total workflow budget: $0.50 (leaves 2.6x safety margin)
- Individual activity budgets: Set to prevent runaway costs
- Comparison: Naive GPT-4 approach would cost ~$0.60-0.80 (3-4x more)

### Intelligent Routing Strategy

**Week 1 Approach: Pre-Classification**

Use cheap Haiku model to classify query complexity before routing:

```mermaid
flowchart LR
    Query[Research Question] --> Classifier[Classify Complexity<br/>Haiku: $0.0001]

    Classifier -->|SIMPLE| Haiku[Execute with Haiku<br/>$0.25/M tokens]
    Classifier -->|MEDIUM| Sonnet[Execute with Sonnet<br/>$3/M tokens]
    Classifier -->|COMPLEX| Opus[Execute with Opus<br/>$15/M tokens]

    Haiku --> Result[Research Result]
    Sonnet --> Result
    Opus --> Result

    classDef cheap fill:#e8f5e9,stroke:#333
    classDef medium fill:#fff4e1,stroke:#333
    classDef expensive fill:#ffebee,stroke:#333

    class Classifier,Haiku cheap
    class Sonnet medium
    class Opus expensive
```

**Classification Criteria** (implemented in Haiku prompt):
- **SIMPLE**: Factual queries, definitions, simple summaries (<500 words expected)
- **MEDIUM**: Analysis queries, comparisons, structured research (500-1500 words)
- **COMPLEX**: Deep synthesis, multi-faceted analysis, expert reasoning (>1500 words)

**Cost Impact**:
- Classification overhead: $0.0001 per query
- Savings: 80-90% on simple queries, 40-60% on medium queries
- Quality: Maintained (Haiku sufficient for simple tasks)

**Week 2 Enhancement: RouteLLM-Style Heuristics**

Replace LLM classifier with deterministic complexity scoring:

```python
# Pseudocode for complexity scoring
def score_complexity(query: str) -> str:
    score = 0

    # Length-based scoring
    if len(query.split()) > 50:
        score += 2
    elif len(query.split()) > 20:
        score += 1

    # Keyword-based scoring
    complex_keywords = ["analyze", "compare", "synthesize", "evaluate", "critique"]
    score += sum(1 for kw in complex_keywords if kw in query.lower())

    # Domain-specific terms
    if has_technical_terms(query):
        score += 1

    # Routing decision
    if score <= 1:
        return "SIMPLE"  # Use Haiku
    elif score <= 3:
        return "MEDIUM"  # Use Sonnet
    else:
        return "COMPLEX"  # Use Opus
```

**Benefits**:
- Zero-cost routing (no LLM call needed)
- Faster (no API latency)
- Deterministic (reproducible routing)
- Berkeley RouteLLM paper shows 85% cost reduction at 95% quality with this approach

---

## Implementation Phases

### Phase 1: Week 1 MVP - Functional CLI Demo

**Goal**: Demonstrate core deep research workflow with cost tracking via CLI

**Deliverables**:
1. Workflow definition: `examples/deep-research-demo.yaml`
2. CLI demo script: `scripts/demo-deep-research.sh`
3. Cost tracking script: `scripts/monitor-cost.sh`
4. Documentation: Usage instructions

**Implementation Tasks**:

#### Task 1.1: Create Deep Research Workflow Definition
**File**: `examples/deep-research-demo.yaml`

Define workflow with:
- 1× initialize activity (Haiku)
- 1× planner activity (Haiku, iteration_scoped)
- 5× search API activities (http_request)
- 5× researcher activities (Sonnet fallback to Haiku, iteration_scoped)
- 1× evaluator activity (Haiku, iteration_scoped)
- 1× editor activity (Sonnet)

Use Tavily search API (sign up at tavily.com, free tier: 1000 requests/month).

**Acceptance Criteria**:
- Workflow validates with `kruxiaflow validate examples/deep-research-demo.yaml`
- All activities have budget limits configured
- Loop logic correctly implements CONTINUE/SUFFICIENT pattern
- Parallel researcher execution configured (fan-out from planner, fan-in to evaluator)

#### Task 1.2: Create Demo Launch Script
**File**: `scripts/demo-deep-research.sh`

```bash
#!/bin/bash
# Deep Research Demo Launcher

TOPIC="${1:-AI workflow orchestration tools}"

echo "=== Kruxia Flow Deep Research Demo ==="
echo "Topic: $TOPIC"
echo ""

# Start Kruxia Flow services
echo "Starting Kruxia Flow..."
kruxiaflow serve &
KRUXIAFLOW_PID=$!
sleep 5

# Obtain auth token
echo "Authenticating..."
TOKEN=$(curl -s -X POST http://localhost:8080/api/v1/oauth/token \
  -H "Content-Type: application/json" \
  -d '{"grant_type":"client_credentials","client_id":"demo","client_secret":"demo"}' \
  | jq -r '.access_token')

# Submit workflow
echo "Submitting research workflow..."
WORKFLOW_ID=$(curl -s -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d "{\"definition_name\":\"deep_research_demo\",\"input\":{\"topic\":\"$TOPIC\"}}" \
  | jq -r '.workflow_id')

echo "Workflow ID: $WORKFLOW_ID"
echo ""

# Monitor progress
./scripts/monitor-cost.sh $WORKFLOW_ID $TOKEN

# Cleanup
kill $KRUXIAFLOW_PID
```

#### Task 1.3: Create Cost Monitoring Script
**File**: `scripts/monitor-cost.sh`

```bash
#!/bin/bash
# Real-time cost monitoring

WORKFLOW_ID=$1
TOKEN=$2

echo "Monitoring workflow $WORKFLOW_ID..."
echo ""

while true; do
  # Get workflow status
  STATUS=$(curl -s "http://localhost:8080/api/v1/workflows/$WORKFLOW_ID" \
    -H "Authorization: Bearer $TOKEN")

  WORKFLOW_STATUS=$(echo $STATUS | jq -r '.status')

  # Get cost
  COST=$(curl -s "http://localhost:8080/api/v1/workflows/$WORKFLOW_ID/cost" \
    -H "Authorization: Bearer $TOKEN")

  TOTAL_COST=$(echo $COST | jq -r '.total_cost_usd')
  BUDGET=$(echo $COST | jq -r '.budget_limit_usd')

  # Display
  clear
  echo "=== Deep Research Demo - Live Cost Tracking ==="
  echo ""
  echo "Status: $WORKFLOW_STATUS"
  echo "Cost:   \$$TOTAL_COST / \$$BUDGET"
  echo ""
  echo "Activity Progress:"
  echo $STATUS | jq -r '.activities[] | "\(.key): \(.status)"'
  echo ""

  # Exit if complete
  if [ "$WORKFLOW_STATUS" = "completed" ] || [ "$WORKFLOW_STATUS" = "failed" ]; then
    echo "Workflow $WORKFLOW_STATUS"
    echo ""
    echo "Final cost: \$$TOTAL_COST"
    echo ""

    # Show cost breakdown
    curl -s "http://localhost:8080/api/v1/workflows/$WORKFLOW_ID/cost/history" \
      -H "Authorization: Bearer $TOKEN" \
      | jq -r '.[] | "\(.activity_key): $\(.cost_usd) (\(.model))"'

    break
  fi

  sleep 2
done
```

#### Task 1.4: Test and Validate
- Run demo with 3 different topics
- Verify cost tracking accuracy
- Confirm parallel execution (check logs for simultaneous researcher starts)
- Validate loop behavior (should iterate 2-3 times on average)
- Document actual costs vs estimates

**Success Criteria**:
- ✅ Research completes in 2-4 minutes
- ✅ Total cost $0.15-0.30 (well under budget)
- ✅ Parallel researchers execute (logs show simultaneous starts)
- ✅ Loop exits when SUFFICIENT (not always hitting iteration limit)
- ✅ Final report is comprehensive (2000+ words)

---

### Phase 2: Week 2 - Visual Demo & Comparison Mode

**Goal**: Add web UI for live cost visualization and side-by-side comparison

**Deliverables**:
1. Web dashboard: `docs/demo/index.html` (single-page app)
2. Comparison workflow: `examples/deep-research-naive.yaml`
3. Comparison script: `scripts/compare-approaches.sh`

**Implementation Tasks**:

#### Task 2.1: Build Web Dashboard
**File**: `docs/demo/index.html`

**Purpose**: Proof-of-concept cost visualization that validates Cost Dashboard API design and serves as foundation for production Grafana dashboards (Post-MVP Phase 1).

**UI Components**:
1. **Topic Input Form**: Text input + Submit button
2. **Cost Meter**: Large animated counter $0.00 → $0.XX
3. **Progress Timeline**: Visual activity status (pending/running/complete)
4. **Cost Breakdown Table**: Per-activity costs with model names
5. **Comparison Chart**: Bar chart showing Kruxia Flow vs Naive

**Technology Stack** (lightweight for demo):
- Vanilla JavaScript (no framework dependencies - easier to port to Grafana later)
- Chart.js for cost visualization (widely supported, Grafana-compatible queries)
- Polling-based updates (1s interval - validates API performance under load)

**API Integration**:
```javascript
// Submit workflow
const response = await fetch('/api/v1/workflows', {
  method: 'POST',
  headers: {
    'Authorization': `Bearer ${token}`,
    'Content-Type': 'application/json'
  },
  body: JSON.stringify({
    definition_name: 'deep_research_demo',
    input: { topic: userInput }
  })
});

const { workflow_id } = await response.json();

// Poll for updates
const pollInterval = setInterval(async () => {
  const [status, cost] = await Promise.all([
    fetch(`/api/v1/workflows/${workflow_id}`).then(r => r.json()),
    fetch(`/api/v1/workflows/${workflow_id}/cost`).then(r => r.json())
  ]);

  updateCostMeter(cost.total_cost_usd);
  updateActivityProgress(status.activities);

  if (status.status === 'completed') {
    clearInterval(pollInterval);
    showFinalReport(workflow_id);
  }
}, 1000);
```

**Visual Design** (ASCII mockup):
```
╔═══════════════════════════════════════════════════════════╗
║   KRUXIA FLOW DEEP RESEARCH DEMO                          ║
╠═══════════════════════════════════════════════════════════╣
║                                                           ║
║   Research Topic: [AI workflow orchestration_______]  [▶] ║
║                                                           ║
║   ┌─────────────────────────────────────────────┐        ║
║   │  COST TRACKER                               │        ║
║   │                                             │        ║
║   │       $0.19 / $0.50                         │        ║
║   │   ████████████░░░░░░░░░░░░ 38%              │        ║
║   │                                             │        ║
║   │   vs Naive Approach: $0.67 (72% savings)   │        ║
║   └─────────────────────────────────────────────┘        ║
║                                                           ║
║   Activity Progress:                                      ║
║   ✓ initialize      $0.001  (claude-haiku-4)             ║
║   ✓ planner         $0.004  (claude-haiku-4, 2 iters)    ║
║   ✓ researcher_1-5  $0.120  (claude-sonnet-4.5, 10 runs) ║
║   ⏳ evaluator       $0.002  (claude-haiku-4, running)    ║
║   ⬜ editor          -       (pending)                    ║
║                                                           ║
║   [View Cost Breakdown] [View Report] [Export Results]   ║
╚═══════════════════════════════════════════════════════════╝
```

**Design for Grafana Migration**:
When implementing this UI, document the following for future Grafana dashboard conversion:

1. **Query Patterns**: Document all API calls and their polling intervals
   ```javascript
   // Example query to document:
   // GET /api/v1/workflows/{id}/cost (poll every 1s)
   // Response: { total_cost_usd, budget_limit_usd, budget_remaining_usd }
   ```

2. **Metrics to Extract**: Identify metrics that should become Prometheus metrics
   - Workflow cost over time (timeseries)
   - Budget utilization percentage (gauge)
   - Activity costs by model (histogram)
   - Cost per activity type (counter)

3. **Visualization Types**: Note which Chart.js visualizations map to Grafana panels
   - Cost meter → Grafana Gauge panel
   - Progress timeline → Grafana State Timeline panel
   - Cost breakdown table → Grafana Table panel
   - Comparison chart → Grafana Bar Gauge panel

4. **Alert Thresholds**: Document threshold values used in demo
   - Budget warning at 80% utilization
   - Budget critical at 95% utilization
   - These become AlertManager rules in production

#### Task 2.2: Create Naive Comparison Workflow
**File**: `examples/deep-research-naive.yaml`

Duplicate deep research workflow but force all LLM activities to use `gpt-4o`:
- Remove fallback chains (only one model per activity)
- Use expensive model for all activities (even planning/evaluation)
- Same logic, different cost profile

**Expected Outcome**:
- Naive cost: $0.60-0.80
- Optimized cost: $0.15-0.30
- Savings: 60-75%
- Quality: Equivalent (validate with LLM-as-judge)

#### Task 2.3: Create Comparison Script
**File**: `scripts/compare-approaches.sh`

```bash
#!/bin/bash

TOPIC="$1"

echo "=== Cost Comparison Demo ==="
echo "Running same research with two approaches..."
echo ""

# Submit both workflows in parallel
OPTIMIZED_ID=$(submit_workflow "deep_research_demo" "$TOPIC")
NAIVE_ID=$(submit_workflow "deep_research_naive" "$TOPIC")

# Monitor both
monitor_parallel $OPTIMIZED_ID $NAIVE_ID

# Compare results
echo ""
echo "=== COMPARISON RESULTS ==="
echo ""
echo "Optimized Approach:"
echo "  Cost: $(get_cost $OPTIMIZED_ID)"
echo "  Model: claude-sonnet-4.5 + claude-haiku-4"
echo ""
echo "Naive Approach:"
echo "  Cost: $(get_cost $NAIVE_ID)"
echo "  Model: gpt-4o (all activities)"
echo ""
echo "Savings: $(calculate_savings $OPTIMIZED_ID $NAIVE_ID)%"
```

#### Task 2.4: Add Web Scraping Activity
**File**: `core/src/activities/web_scrape.rs`

Implement `WebScrapeActivity` struct:
- Use `reqwest` for HTTP fetching
- Use `scraper` crate for CSS selector extraction
- Implement text cleaning (remove scripts/styles/nav)
- Support link extraction for crawling

**Integration**:
- Register in `register_builtin_activities()`
- Add to workflow: fetch search result URLs and extract clean text
- Pass extracted text to researcher LLMs (reduces token count vs raw HTML)

**Cost Impact**:
- Scraping: Free (no API cost)
- LLM parsing: Reduced by 80% (clean text vs raw HTML)
- Demonstrates "deterministic + AI hybrid" value prop

---

### Phase 3: Week 3-4 - Vertical Extensions

**Goal**: Extend base workflow to vertical applications demonstrating production readiness

**Deliverables**:
1. Competitive intelligence workflow
2. Due diligence automation workflow
3. Patent/IP research workflow

**Implementation Tasks**:

#### Task 3.1: Competitive Intelligence Workflow
**File**: `examples/competitive-intelligence.yaml`

**Use Case**: Monitor competitor landscape and generate weekly reports

**Modifications from Base Workflow**:
- Add scheduled trigger (run weekly)
- Add competitor database (track competitors in PostgreSQL)
- Add alert generation (email if significant changes detected)
- Add trend analysis (compare to previous reports)

**Activities**:
1. Load competitor list from database
2. For each competitor: research recent news, product updates, funding
3. Compare to baseline (previous week's report)
4. Generate change summary
5. Send email alert if significant changes
6. Store report in database

**Demo Value**: Shows scheduled workflows, database integration, alerting

#### Task 3.2: Due Diligence Automation
**File**: `examples/due-diligence.yaml`

**Use Case**: Automated due diligence for investment/M&A

**Modifications**:
- Add document upload (PDFs, financial statements)
- Add structured extraction (revenue, growth, risks)
- Add financial analysis (ratios, trends)
- Add risk flagging (red flags in documents)

**Activities**:
1. Upload documents (financials, contracts, etc.)
2. OCR + text extraction
3. Structured field extraction (revenue, costs, liabilities)
4. Financial analysis (calculate ratios, trends)
5. Risk assessment (identify red flags)
6. Generate due diligence report

**Demo Value**: Shows document processing, structured extraction, financial analysis

#### Task 3.3: Patent/IP Research
**File**: `examples/patent-research.yaml`

**Use Case**: Prior art search for patent applications

**Modifications**:
- Add patent database search (USPTO, EPO APIs)
- Add citation graph analysis
- Add claim comparison (semantic similarity)
- Add novelty scoring

**Activities**:
1. Parse patent application claims
2. Search prior art (patent databases + academic sources)
3. Extract relevant patents/papers
4. Compare claims (semantic similarity with embeddings)
5. Generate novelty assessment
6. Identify potential conflicts

**Demo Value**: Shows vector search, semantic analysis, specialized domain

---

## Integration Points

### External Dependencies

#### 1. Search APIs
**Purpose**: Web search for research content

**Recommended Provider**: Tavily
- Cost: $0.005 per search request
- Quality: Optimized for AI agents, returns clean structured data
- Limits: 1000 requests/month (free tier)
- Signup: https://tavily.com

**Alternative**: Perplexity API
- Cost: $5 per 1000 requests
- Quality: Returns synthesized answers + sources
- Better for: Complex queries requiring synthesis

**Integration**:
```bash
# Set environment variable
export TAVILY_API_KEY="tvly-xxxxx"

# Reference in workflow
{{SECRET.tavily_api_key}}
```

#### 2. LLM Providers
**Required**:
- Anthropic API (Claude models): `ANTHROPIC_API_KEY`
- OpenAI API (GPT models): `OPENAI_API_KEY` (for comparison mode)

**Optional**:
- Google AI (Gemini): `GOOGLE_API_KEY`
- Ollama (local models): No API key needed

#### 3. PostgreSQL Database
**Purpose**: Workflow state, cost tracking, results storage

**Schema** (already implemented):
- `workflows` - Workflow metadata and state
- `activity_queue` - Pending activities
- `activity_costs` - Cost tracking per activity
- `llm_models` - Model pricing catalog
- `workflow_events` - Event audit trail

**No Changes Required**: Existing schema supports all demo features.

### Internal Service Interfaces

#### ActivityQueue (PostgreSQL-based)
**Usage**: Activity scheduling and worker polling

**Methods Used**:
- `schedule_activity()` - Orchestrator schedules researcher activities
- `claim_next()` - Worker polls for pending activities
- `complete_activity()` - Worker reports completion with cost

**No Changes Required**: Existing implementation sufficient.

#### EventSource (PostgreSQL Polling)
**Usage**: Workflow event pub/sub

**Events**:
- `WorkflowCreated` - API publishes on workflow submission
- `ActivityScheduled` - Orchestrator publishes when activity ready
- `ActivityCompleted` - Worker publishes on completion
- `WorkflowCompleted` - Orchestrator publishes when all activities done

**No Changes Required**: Polling with adaptive backoff handles demo load.

#### WorkflowStorage (PostgreSQL Large Objects)
**Usage**: Store large artifacts (research reports, documents)

**Methods**:
- `store_artifact()` - Save final research report as file
- `retrieve_artifact()` - Download report for display

**No Changes Required**: Existing implementation sufficient.

### Configuration

All configuration via environment variables (per architecture guidelines):

```bash
# LLM API Keys
export ANTHROPIC_API_KEY="sk-ant-xxxxx"
export OPENAI_API_KEY="sk-xxxxx"
export GOOGLE_API_KEY="xxxxx"

# Search API
export TAVILY_API_KEY="tvly-xxxxx"

# Database (already configured)
export KRUXIAFLOW_DATABASE_URL="postgresql://user:pass@localhost/kruxiaflow"

# API Server (defaults OK)
export KRUXIAFLOW_API_HOST="0.0.0.0"
export KRUXIAFLOW_API_PORT="8080"
```

**No .env files** (per guidelines) - environment variables only.

---

## Testing Strategy

### Unit Tests
**Scope**: Individual activity implementations

**Test Cases**:
1. `web_scrape` activity (if implemented):
   - Valid URL returns clean text
   - CSS selector extracts correct elements
   - Invalid URL returns error
   - Timeout handling
2. Cost calculation logic:
   - Token estimation accuracy (±10%)
   - Budget enforcement (aborts when exceeded)
   - Fallback chain triggers on budget exceeded

**Files**:
- `core/src/activities/web_scrape_tests.rs`
- `core/src/activities/llm_prompt_tests.rs` (budget tests)

### Integration Tests
**Scope**: Full workflow execution

**Test Cases**:
1. **Simple Research Topic** (low cost):
   - Input: "What is Rust programming language?"
   - Expected: 1-2 iterations, cost < $0.10
   - Verify: Report generated, all sources cited

2. **Complex Research Topic** (high cost):
   - Input: "Compare distributed workflow orchestration architectures"
   - Expected: 2-3 iterations, cost $0.20-0.40
   - Verify: Comprehensive analysis, multiple sources

3. **Budget Limit Enforcement**:
   - Set total budget to $0.05
   - Input: Complex topic requiring expensive models
   - Expected: Workflow aborts or degrades to cheaper models
   - Verify: Does not exceed budget

4. **Parallel Execution Timing**:
   - Input: Topic requiring all 5 researchers
   - Measure: Time from planner complete to all researchers complete
   - Expected: <30s (parallel) vs >2min (sequential)
   - Verify: Logs show simultaneous researcher starts

5. **Loop Behavior**:
   - Input: Topic with insufficient initial research
   - Expected: Evaluator returns CONTINUE at least once
   - Verify: Planner runs multiple times, evaluator iteration > 0

**Files**:
- `api/tests/deep_research_workflow_tests.rs`

### Demo Rehearsal
**Scope**: End-to-end demo flow

**Steps**:
1. Start fresh Kruxia Flow instance
2. Run CLI demo: `./scripts/demo-deep-research.sh "AI agents"`
3. Verify cost progression visible in terminal
4. Confirm final report quality (manual review)
5. Run comparison: `./scripts/compare-approaches.sh "AI agents"`
6. Verify savings percentage (60-75%)
7. Run web dashboard demo
8. Verify UI updates in real-time

**Acceptance**:
- ✅ Demo completes without errors
- ✅ Cost tracking accurate (±5%)
- ✅ Reports are comprehensive and well-cited
- ✅ Comparison shows significant savings
- ✅ UI is responsive and visually clear

---

## Success Criteria

### Week 1 MVP Success Criteria

**Functional Requirements**:
- ✅ Workflow completes research on arbitrary topics
- ✅ Parallel researcher execution demonstrable (logs show simultaneous starts)
- ✅ Iterative loop works (evaluator triggers re-research when insufficient)
- ✅ Budget enforcement prevents runaway costs
- ✅ Final reports are comprehensive (2000+ words, multiple sources)

**Performance Requirements**:
- ✅ Total execution time: <5 minutes for typical topic
- ✅ Parallel researchers complete in <30s (vs >2min sequential)
- ✅ Cost per research: $0.15-0.30 (under $0.50 budget)

**Quality Requirements**:
- ✅ Reports address all research questions
- ✅ Sources are cited and verifiable
- ✅ No hallucinated information (manual spot checks)

### Week 2 Demo Success Criteria

**Visual Demo Requirements**:
- ✅ Web UI displays real-time cost progression
- ✅ Activity timeline shows parallel execution visually
- ✅ Cost breakdown table shows per-activity costs and models
- ✅ Comparison mode demonstrates 60-75% cost savings

**Comparison Mode Requirements**:
- ✅ Naive approach costs 2.5-4x more
- ✅ Quality equivalence validated (LLM-as-judge scores within 10%)
- ✅ Side-by-side display clearly shows savings

**Shareability Requirements**:
- ✅ Demo can be run by others with just API keys
- ✅ Documentation explains setup in <5 steps
- ✅ Screenshots/GIFs capture key demo moments

**Grafana Migration Preparation**:
- ✅ `docs/demo/grafana-migration.md` document created with:
  - API endpoint mapping to Grafana datasource queries
  - Chart.js visualization to Grafana panel type mapping
  - Prometheus metrics to extract from Cost API
  - AlertManager rules for budget thresholds (80% warning, 95% critical)
  - Query patterns documented in code comments

### Week 3-4 Extension Success Criteria

**Vertical Applications**:
- ✅ At least 2 of 3 vertical workflows implemented
- ✅ Each demonstrates unique Kruxia Flow capability:
  - Competitive intelligence: Scheduled workflows + alerting
  - Due diligence: Document processing + structured extraction
  - Patent research: Vector search + semantic analysis

**Production Readiness**:
- ✅ Error handling for API failures (search, LLM)
- ✅ Retry logic for transient failures
- ✅ Graceful degradation when budget tight
- ✅ Monitoring/observability (cost tracking, activity logs)

### Investor/Demo Day Success Criteria

**"I Need This" Moment**:
- ✅ Cost meter hitting $0.47 vs naive $1.52 creates visceral impact
- ✅ Parallel execution visible (5 researchers working simultaneously)
- ✅ Budget control demonstration (set $0.75 limit, show graceful degradation)

**Technical Credibility**:
- ✅ Workflow definition is clean (<100 lines YAML)
- ✅ "That's it?" moment when showing workflow vs Airflow/Temporal equivalent
- ✅ Demonstrates hybrid deterministic + AI (scraping + LLM synthesis)

**Market Validation**:
- ✅ Solves documented pain point (runaway AI costs)
- ✅ Addresses production reality (multi-provider fallback for reliability)
- ✅ Showcases competitive moat (cost visibility other tools lack)

---

## Open Questions & Decisions Needed

### Q1: Search API Provider Selection
**Question**: Tavily ($0.005/req) vs Perplexity ($0.005/req) vs SerpAPI ($0.01/req)?

**Options**:
1. **Tavily** - Optimized for AI agents, clean structured output
2. **Perplexity** - Returns synthesized answers (reduces LLM work)
3. **SerpAPI** - Direct Google/Bing results (most comprehensive)

**Recommendation**: Start with Tavily (Week 1), add Perplexity option (Week 2)

**Decision Needed**: User preference or multi-provider support?

---

### Q2: Web Scraping Implementation Timing
**Question**: Implement native `web_scrape` activity in Week 1 or defer to Week 2?

**Week 1 Workaround**: Use `http_request` + LLM parsing
- ✅ Zero implementation effort
- ✅ Demonstrates hybrid deterministic + AI pattern
- ❌ Higher token costs (LLM parsing raw HTML)

**Week 2 Native Implementation**: Build `web_scrape` activity
- ✅ Lower cost (clean extraction vs LLM parsing)
- ✅ Better demo of cost optimization
- ❌ ~100 lines Rust + testing

**Recommendation**: Week 1 workaround, Week 2 native (enables cost optimization demo)

**Decision Needed**: Confirm Week 2 timeline acceptable?

---

### Q3: Streaming Integration
**Question**: Should final report use streaming output for visual impact?

**With Streaming**:
- ✅ User sees report being written in real-time (ChatGPT-like experience)
- ✅ Demonstrates WebSocket streaming capability
- ✅ More engaging demo

**Without Streaming**:
- ✅ Simpler implementation (polling only)
- ✅ Faster execution (single API call vs streamed)

**Recommendation**: Enable streaming for editor activity (final report generation)

**Decision Needed**: Confirm streaming is desired for demo?

---

### Q4: LLM-as-Judge Quality Validation
**Question**: Implement automated quality comparison for naive vs optimized?

**Approach**: Have Claude evaluate both reports and score quality
```yaml
- key: quality_judge
  activity_name: llm_prompt
  parameters:
    model: anthropic/claude-sonnet-4-5-20250929
    prompt: |
      Compare these two research reports on quality:

      Report A (Optimized):
      {{optimized_workflow.editor.result}}

      Report B (Naive):
      {{naive_workflow.editor.result}}

      Rate each on 1-10 scale for:
      1: Comprehensiveness
      2: Accuracy
      3: Source quality
      4: Writing quality

      Return JSON with scores and brief justification.
```

**Benefits**:
- ✅ Objective quality measurement
- ✅ Validates "same quality, lower cost" claim
- ❌ Adds $0.02-0.05 cost to demo

**Recommendation**: Implement in Week 2 for comparison mode

**Decision Needed**: Include automated quality validation?

---

## Next Steps

### Immediate Actions (Week 1 Start)

1. **Environment Setup**:
   - [ ] Sign up for Tavily API (free tier)
   - [ ] Verify Anthropic API key access
   - [ ] Verify OpenAI API key access (for comparison)
   - [ ] Test PostgreSQL connection

2. **Workflow Development**:
   - [ ] Create `examples/deep-research-demo.yaml`
   - [ ] Validate workflow definition
   - [ ] Test with simple topic ("What is Rust?")

3. **Demo Scripts**:
   - [ ] Implement `scripts/demo-deep-research.sh`
   - [ ] Implement `scripts/monitor-cost.sh`
   - [ ] Test end-to-end demo flow

4. **Documentation**:
   - [ ] Write setup instructions
   - [ ] Document expected costs
   - [ ] Create demo talking points

### Week 2 Planning

1. **Web Dashboard**:
   - [ ] Design UI mockup
   - [ ] Implement HTML/JS/CSS
   - [ ] Integrate with API polling
   - [ ] Add Chart.js cost visualization
   - [ ] **Document for Grafana migration**:
     - [ ] List all API endpoints used and polling intervals
     - [ ] Identify which metrics should become Prometheus metrics
     - [ ] Map Chart.js visualizations to Grafana panel types
     - [ ] Document alert thresholds (80% warning, 95% critical)
     - [ ] Save query patterns as comments in code

2. **Comparison Mode**:
   - [ ] Create naive workflow variant
   - [ ] Implement comparison script
   - [ ] Test cost differential (target 60-75% savings)

3. **Optimization**:
   - [ ] Implement web scraping activity (optional)
   - [ ] Add intelligent routing (RouteLLM heuristics)
   - [ ] Optimize token usage

### Week 3-4 Planning

1. **Vertical Extensions**:
   - [ ] Choose 2 of 3 verticals (competitive intel, due diligence, patent)
   - [ ] Implement workflow variants
   - [ ] Create vertical-specific demo scripts

2. **Polish**:
   - [ ] Screenshot/GIF creation for sharing
   - [ ] Documentation cleanup
   - [ ] Performance tuning
   - [ ] Error handling hardening

---

## Summary: Strategic Value of Demo UI

### Why Build the Demo UI Now

The deep research demo UI is not just a one-off demo tool—it's a strategic investment that validates APIs and enables future production UIs:

**Immediate Value (Week 1-2)**:
- ✅ Creates investor "I need this" moment with live cost visualization
- ✅ Demonstrates Kruxia Flow's core differentiator (cost control)
- ✅ Provides shareable demo for sales/marketing

**Technical Value (Week 1-2)**:
- ✅ Validates Cost Dashboard API design before Grafana commitment
- ✅ Tests API performance under 1s polling load (3600 req/hr)
- ✅ Identifies API gaps, missing fields, or performance issues
- ✅ Provides reference implementation for API consumers

**Long-Term Value (Post-MVP)**:
- ✅ Query patterns migrate directly to Grafana dashboards (~3 months)
- ✅ Metrics identified in demo become Prometheus metrics
- ✅ Alert thresholds tested in demo become AlertManager rules
- ✅ UI patterns inform Web-Based Workflow Designer (~9 months)

### API-First Architecture Benefits

By building the demo UI on top of the existing Cost Dashboard API (`docs/cost-dashboard-api.md`), we ensure:

1. **No Backend Duplication**: Same API serves demo UI, Grafana, and future web designer
2. **Stable Interface**: Frontend evolution doesn't require API changes
3. **Customer Flexibility**: Different customers can choose different UI tiers:
   - CLI scripts (power users, automation)
   - Grafana dashboards (enterprise monitoring)
   - Full web UI (business users, low-code workflows)

### Deliverables That Enable Future Work

**Week 1-2 Demo Deliverables**:
- `docs/demo/index.html` - Demo UI with documented query patterns
- `docs/demo/README.md` - Setup instructions and Grafana migration notes
- `docs/demo/grafana-migration.md` - Mapping of demo UI to Grafana panels

**Post-MVP Phase 1 (Grafana) Deliverables**:
- `config/grafana/dashboards/cost-tracking.json` - Grafana dashboard (ported from demo)
- `config/prometheus/kruxiaflow.yml` - Prometheus scrape config
- `config/prometheus/alerts.yml` - AlertManager rules (based on demo thresholds)

**Post-MVP Phase 3 (Web Designer) Deliverables**:
- Full web application with embedded Grafana panels
- Cost monitoring integrated into workflow design experience
- Historical cost data from Grafana datasource

### Decision: Proceed with Demo UI

**Recommendation**: Build the demo UI in Week 1-2 as planned.

**Rationale**:
- Cost APIs are already complete (no backend work needed)
- Demo UI creates maximum investor/customer impact
- Lightweight implementation (single HTML file, no framework)
- Validates APIs and enables Grafana migration
- Not throwaway work—it's the foundation for production monitoring

**Risk Mitigation**:
- Keep implementation simple (avoid over-engineering)
- Document query patterns for Grafana migration
- Use vanilla JS (easier to port than framework-specific code)
- Focus on cost visualization only (don't scope creep into full admin UI)

---

## Appendix: Cost Calculations

### Token Estimation Methodology

Kruxia Flow uses provider-specific heuristics for token estimation:

**Anthropic (Claude)**:
- Characters: 3.5 chars/token average
- Words: 0.85 words/token average
- Formula: `max(chars/3.5, words/0.85)`

**OpenAI (GPT)**:
- Characters: 4.0 chars/token average
- Words: 0.75 words/token average
- Formula: `max(chars/4.0, words/0.75)`

**Example**:
```
Prompt: "Research AI workflow orchestration tools" (41 chars, 5 words)
Anthropic estimate: max(41/3.5, 5/0.85) = max(11.7, 5.9) = 12 tokens
OpenAI estimate: max(41/4.0, 5/0.75) = max(10.3, 6.7) = 11 tokens
Actual: Varies by tokenizer, estimates typically within ±10%
```

### Model Pricing (as of 2025-01-03)

| Provider  | Model                           | Input ($/1M) | Output ($/1M) | Cached Input ($/1M) |
|-----------|---------------------------------|--------------|---------------|---------------------|
| Anthropic | claude-opus-4-1-20250514        | $15.00       | $75.00        | $3.75               |
| Anthropic | claude-sonnet-4-5-20250929      | $3.00        | $15.00        | $0.75               |
| Anthropic | claude-haiku-4-20250415         | $0.25        | $1.25         | $0.06               |
| OpenAI    | gpt-4o                          | $2.50        | $10.00        | -                   |
| OpenAI    | gpt-4o-mini                     | $0.15        | $0.60         | -                   |
| Google    | gemini-2-0-flash                | $0.075       | $0.30         | -                   |
| Google    | gemini-1-5-pro                  | $1.25        | $5.00         | -                   |

**Note**: Cached input pricing only available for Anthropic models (80-95% discount on repeated context).

### Expected Cost Breakdown (Per Research)

**Optimized Workflow** (Intelligent Routing):
```
Activity         Model            Tokens (In/Out)  Cost
─────────────────────────────────────────────────────────
initialize       Haiku            200 / 400        $0.001
planner (×2)     Haiku            500 / 600        $0.002
search (×10)     Tavily API       -                $0.050
researcher (×10) Sonnet/Haiku     1000 / 1200      $0.120
evaluator (×2)   Haiku            3000 / 150       $0.002
editor           Sonnet           5000 / 2500      $0.053
─────────────────────────────────────────────────────────
TOTAL                                              $0.228
```

**Naive Workflow** (GPT-4o Only):
```
Activity         Model            Tokens (In/Out)  Cost
─────────────────────────────────────────────────────────
initialize       GPT-4o           200 / 400        $0.005
planner (×2)     GPT-4o           500 / 600        $0.007
search (×10)     Tavily API       -                $0.050
researcher (×10) GPT-4o           1000 / 1200      $0.145
evaluator (×2)   GPT-4o           3000 / 150       $0.009
editor           GPT-4o           5000 / 2500      $0.263
─────────────────────────────────────────────────────────
TOTAL                                              $0.479
```

**Savings**: $0.251 (52% reduction)

**Quality**: Equivalent (validated via LLM-as-judge in testing)
