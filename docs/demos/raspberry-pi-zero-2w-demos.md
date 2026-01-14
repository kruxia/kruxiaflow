# Compelling Kruxiaflow Demos for Raspberry Pi Zero 2 W

## Executive Summary

This document presents compelling demonstration concepts for kruxiaflow running on a **Raspberry Pi Zero 2 W**, designed to showcase the system's capabilities to potential customers building agentic AI applications. All demos use **built-in activities only** (no custom code required) and include custom UIs to visualize kruxiaflow's key differentiators: cost tracking, streaming output, multi-provider fallback, and budget enforcement.

---

## Raspberry Pi Zero 2 W Specifications

- **CPU**: 1GHz quad-core 64-bit ARM Cortex-A53
- **RAM**: 512MB LPDDR2
- **Size**: 65mm × 30mm × 5mm
- **Power**: 5V, ~300mA typical
- **Storage**: microSD card
- **Connectivity**: 2.4GHz 802.11 b/g/n wireless, Bluetooth 4.2

**Deployment Constraints:**
- Lightweight PostgreSQL instance (< 200MB RAM)
- Single worker process with 2-3 concurrent activity slots
- API server with minimal resource footprint
- Custom UI served via static HTML/JS (no heavy frameworks)

**Why This Device?**
- **Edge Computing Narrative**: "Run AI orchestration at the edge, not just the cloud"
- **Cost Efficiency**: "$15 hardware running $0.50 workflows"
- **Portability**: Demo fits in pocket, runs on battery pack
- **Wow Factor**: "This tiny device orchestrates GPT-4, Claude, and Gemini simultaneously"

---

## Demo #1: Live AI Command Center (RECOMMENDED)

**Tagline:** *"Watch AI workflows execute in real-time with live cost tracking"*

### Overview

A visual dashboard showing multiple concurrent AI workflows executing simultaneously, each with real-time:
- **Streaming token output** (WebSocket updates)
- **Cost accumulation** (per-activity and total)
- **Model routing decisions** (which provider/model, why)
- **Savings comparison** (vs. "always use GPT-4" baseline)
- **Budget enforcement** (visual budget meter, alerts)
- **Fallback resilience** (automatic provider switching)

### Visual Design

```
┌─────────────────────────────────────────────────────────────────┐
│  🎯 Kruxiaflow Live Command Center                    [Pi Logo] │
│  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ │
│                                                                   │
│  💰 Total Cost Today: $2.47   💾 Workflows: 23   ⏱️ Uptime: 4h  │
│  📊 Savings vs. GPT-4-only: $14.23 (85%)                        │
│  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ │
│                                                                   │
│  ┌──────────────────────────────┬────────────────────────────┐  │
│  │ 🔍 Research: "Rust vs Go"    │ 📧 Email Summary           │  │
│  │ ──────────────────────────── │ ───────────────────────── │  │
│  │ Status: ⚡ Streaming          │ Status: ✅ Completed       │  │
│  │ Cost: $0.047 / $0.50 budget  │ Cost: $0.003              │  │
│  │ Model: Claude Sonnet 3.5     │ Model: Haiku 3.5          │  │
│  │ ───────────────────────────  │ ───────────────────────── │  │
│  │ [████████████████░░░] 85%    │ Saved: $0.127 (98%)       │  │
│  │                              │                            │  │
│  │ "Rust and Go both excel in   │ Summary: 5 emails, 2      │  │
│  │  concurrent programming, but  │ requiring action...       │  │
│  │  Rust's ownership model..."   │ [View Details]            │  │
│  │  [Streaming live...]          │                           │  │
│  └──────────────────────────────┴────────────────────────────┘  │
│                                                                   │
│  ┌──────────────────────────────┬────────────────────────────┐  │
│  │ 🌐 News Analysis             │ 💬 Content Moderation      │  │
│  │ ──────────────────────────── │ ───────────────────────── │  │
│  │ Status: 🔄 Running (3/5)     │ Status: ⚠️ Budget Alert    │  │
│  │ Cost: $0.21 / $1.00 budget   │ Cost: $0.42 / $0.50       │  │
│  │ Model: GPT-4o-mini           │ Model: Haiku → Sonnet     │  │
│  │ ───────────────────────────  │ ───────────────────────── │  │
│  │ Parallel: 3 articles         │ [██████████████████░] 84% │  │
│  │ • "Tech trends" ✅           │                           │  │
│  │ • "Market news" ⚡           │ Fallback: Claude rate     │  │
│  │ • "AI policy" 🕐             │ limited → GPT-4 used      │  │
│  └──────────────────────────────┴────────────────────────────┘  │
│                                                                   │
│  📈 Real-time Savings Chart                                      │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │  $$                                                          ││
│  │  15│            ━━━ Baseline (GPT-4 only)                   ││
│  │  12│          ╱                                              ││
│  │   9│         ╱                                               ││
│  │   6│        ╱                                                ││
│  │   3│  ━━━━━━━━━━━━━━━━━ Kruxiaflow (Smart Routing)         ││
│  │   0└──────────────────────────────────────────────────────▶││
│  │     9am    10am    11am    12pm    1pm    2pm              ││
│  └─────────────────────────────────────────────────────────────┘│
└───────────────────────────────────────────────────────────────────┘
```

### Workflows Running

1. **Research Assistant** (from `09b-streaming-research.yaml`)
   - User asks questions via simple form
   - Streams comprehensive answers with citations
   - Shows model routing: Haiku for prep, Sonnet for analysis
   - Cost comparison: "$0.047 vs. $0.18 with GPT-4 only"

2. **Email Summarization** (modified `05-research-assistant.yaml`)
   - Polls email API every 5 minutes
   - Extracts key information with Haiku (cheap)
   - Sends summary via email
   - Perfect for showing cost efficiency: "5 emails for $0.003"

3. **News Analysis** (based on `http_request` + `llm_prompt`)
   - Fetches RSS feeds from multiple sources
   - Parallel LLM analysis (3 articles simultaneously)
   - Demonstrates parallel execution and cost tracking

4. **Content Moderation** (from `04-moderate-content.yaml`)
   - Simulated user content submissions
   - LLM classification with budget limits
   - Shows budget enforcement and fallback chains

### Technical Implementation

**Backend:**
- PostgreSQL on Pi Zero 2 W (lightweight config: `shared_buffers=64MB`, `max_connections=10`)
- Kruxiaflow API server (single process)
- Kruxiaflow worker with `max_concurrent_activities=3`
- All services run via `docker-compose` or systemd

**Frontend:**
- Static HTML/CSS/JS dashboard (no build step)
- WebSocket connections for streaming:
  - `ws://pi.local:8080/api/v1/activities/{id}/ws` for token streaming
- Polling for cost/status updates:
  - `GET /api/v1/workflows/{id}` every 500ms for activity status
  - `GET /api/v1/workflows/{id}/cost` every 1s for cost updates
  - `GET /api/v1/cost/analytics` for aggregate stats
- Chart.js for real-time savings visualization
- Vanilla JS (no React/Vue - keep it lightweight)

**Demo Script:**

1. **Boot the Pi** (30 seconds to start all services)
2. **Open Dashboard** on tablet/laptop connected to Pi's WiFi
3. **Submit Research Question**: "Compare Rust vs Go for systems programming"
   - Watch streaming tokens appear live
   - Show cost meter incrementing: $0.001... $0.010... $0.047
   - Show model routing: "Using Claude Sonnet 3.5 ($3/M tokens)"
   - Compare to baseline: "GPT-4 would cost $0.18 (4x more)"
4. **Trigger Budget Alert**: Submit expensive workflow approaching limit
   - Watch budget meter turn yellow at 80%
   - Show graceful degradation: switches to cheaper model
5. **Demonstrate Fallback**: Simulate Claude rate limit (mock or real)
   - Watch automatic failover: "Claude 429 → GPT-4"
   - Workflow continues without interruption
6. **Show Historical Analytics**:
   - "23 workflows today, total cost $2.47"
   - "Saved $14.23 vs. naive GPT-4-only approach"

### Why This Demo Wins

✅ **Shows ALL key features**: streaming, cost tracking, budget enforcement, fallback, parallel execution
✅ **Visually compelling**: Live updates, charts, multiple workflows
✅ **Addresses #1 pain point**: "AI costs are unpredictable" → "Here's real-time cost control"
✅ **Demonstrates reliability**: Fallback chains prevent single-provider outages
✅ **Quantifiable ROI**: "85% cost reduction with maintained quality"
✅ **Edge computing wow factor**: "This $15 device orchestrates enterprise AI"
✅ **Perfect for trade shows**: Self-contained, portable, eye-catching

---

## Demo #2: Personal AI Research Station

**Tagline:** *"Ask any question, get a comprehensive report with full cost transparency"*

### Overview

A focused, single-purpose demo showing deep research workflows with iterative information gathering, streaming results, and budget-aware execution. Based on `07b-agentic-research-complete.yaml` pattern.

### Visual Design

```
┌─────────────────────────────────────────────────────────────────┐
│  🔬 AI Research Station                    Running on: Pi Zero 2W│
│  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ │
│                                                                   │
│  Research Question:                                               │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │ What are the latest developments in quantum computing?   │   │
│  └──────────────────────────────────────────────────────────┘   │
│                                [🔍 Start Research]               │
│                                                                   │
│  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ │
│                                                                   │
│  📊 Research Progress                                            │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │ Iteration 1/5                                            │   │
│  │ [████████████░░░░░░░░░░░░░] Searching sources...        │   │
│  │                                                          │   │
│  │ ✅ Generate questions ($0.001, Haiku, 0.2s)             │   │
│  │ ✅ Search sources     ($0.10, HTTP API, 1.4s)           │   │
│  │ ⚡ Evaluate results   ($0.001, Haiku, streaming...)      │   │
│  │ 🕐 Compile report     (pending)                          │   │
│  │ 🕐 Publish results    (pending)                          │   │
│  └──────────────────────────────────────────────────────────┘   │
│                                                                   │
│  💰 Cost Tracking                                                │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │ Current:   $0.102                                        │   │
│  │ Budget:    $0.50                                         │   │
│  │ Remaining: $0.398                                        │   │
│  │ [████████████████████░░░░░░░░░░░░░░░░░░░░░░░] 20%       │   │
│  │                                                          │   │
│  │ vs. GPT-4 baseline: $0.35 saved (77% cheaper)           │   │
│  └──────────────────────────────────────────────────────────┘   │
│                                                                   │
│  📄 Evaluation (Streaming Live)                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │ {"sufficient": false,                                    │   │
│  │  "reason": "Need more information on quantum error      │   │
│  │   correction and recent breakthroughs",                 │   │
│  │  "gaps": ["Error correction methods", "IBM vs Google    │   │
│  │   approaches", "Commercial timeline"]}                  │   │
│  │                                                          │   │
│  │ ➜ Scheduling iteration 2 with refined search...         │   │
│  └──────────────────────────────────────────────────────────┘   │
│                                                                   │
│  🎯 Model Routing Decisions                                      │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │ • Questions: Haiku 3.5 ($0.25/M tokens) ✅ Fast & cheap  │   │
│  │ • Search API: HTTP request ($0.10/call) ✅ Required      │   │
│  │ • Evaluation: Haiku 3.5 ($0.25/M tokens) ✅ Fast         │   │
│  │ • Final report: Sonnet 3.5 ($3/M tokens) 🎯 Quality     │   │
│  └──────────────────────────────────────────────────────────┘   │
└───────────────────────────────────────────────────────────────────┘
```

### Workflow Features

- **Iterative Research Loop** (up to 5 iterations, budget-limited)
- **Real HTTP Search** (using free/demo APIs like DuckDuckGo, Brave Search)
- **Streaming Evaluation** (watch LLM decide if research is sufficient)
- **Intelligent Model Routing**:
  - Haiku for planning and evaluation (cheap, fast)
  - Sonnet for final synthesis (quality matters here)
- **Budget Enforcement**: Stops gracefully if approaching limit
- **File Storage**: Large search results stored as files, not in memory

### Demo Script

1. **Enter Research Question**: "What are the latest quantum computing breakthroughs?"
2. **Watch Iteration 1**:
   - Generate questions: $0.001 (Haiku, 200ms)
   - Search sources: $0.10 (HTTP, 1.5s)
   - Evaluate: $0.001, streams JSON showing insufficient info
3. **Watch Iteration 2**: Refined search based on gaps
4. **Watch Iteration 3**: Evaluation decides sufficient=true
5. **Final Report**: Sonnet synthesizes comprehensive report (streaming)
6. **Show Total Cost**: "$0.323 vs. $1.20 with GPT-4 only"

### Why This Works

- Shows **agentic behavior** (LLM deciding when to stop iterating)
- Demonstrates **budget awareness** in loops (prevents runaway costs)
- **Streaming evaluation** is mesmerizing to watch
- Perfect for showing **"AI building AI workflows"** concept
- Runs entirely on Pi Zero 2 W (except external search API)

---

## Demo #3: Smart Home AI Assistant

**Tagline:** *"Voice-controlled AI that costs pennies per conversation"*

### Overview

A voice-activated assistant running on Raspberry Pi that demonstrates cost-optimized LLM routing for different types of commands. Shows local fallback with Ollama when internet is unavailable.

### Visual Design

```
┌─────────────────────────────────────────────────────────────────┐
│  🏠 Smart Home AI Assistant               [Pi Zero 2W + Mic]    │
│  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ │
│                                                                   │
│  [🎤 Listening...]                                               │
│                                                                   │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │  🗣️ You: "What's the weather in Chicago?"                │   │
│  │                                                          │   │
│  │  🤖 Assistant: "The weather in Chicago today is partly  │   │
│  │     cloudy with a high of 72°F and a low of 58°F.      │   │
│  │     Light winds from the east at 5-10 mph."            │   │
│  │                                                          │   │
│  │  💰 Cost: $0.002 | Model: Haiku 3.5 | Time: 1.2s       │   │
│  │  💾 Saved: $0.018 vs. GPT-4                             │   │
│  └──────────────────────────────────────────────────────────┘   │
│                                                                   │
│  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ │
│                                                                   │
│  📊 Today's Stats                                                │
│  ┌───────────────────────┬──────────────────────────────────┐   │
│  │ Conversations: 47     │ Total Cost: $0.13                │   │
│  │ Avg Response: 0.8s    │ Savings: $1.87 (93%)             │   │
│  │ Uptime: 12h 34m       │ Fallback Uses: 2 (offline mode)  │   │
│  └───────────────────────┴──────────────────────────────────┘   │
│                                                                   │
│  🎯 Intelligent Routing                                          │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │ Simple queries       → Haiku 3.5    ($0.002)  ✅ 89%     │   │
│  │ Complex analysis     → Sonnet 3.5   ($0.020)  🎯 9%      │   │
│  │ Offline/local        → Ollama       ($0.000)  🏠 2%      │   │
│  └──────────────────────────────────────────────────────────┘   │
│                                                                   │
│  Recent Conversations:                                           │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │ • "Set timer 5 min"        $0.001  Haiku     0.6s       │   │
│  │ • "Weather Chicago"        $0.002  Haiku     1.2s       │   │
│  │ • "Explain quantum comp"   $0.021  Sonnet    3.4s  🎯   │   │
│  │ • "Turn off lights"        $0.000  Ollama    0.4s  🏠   │   │
│  └──────────────────────────────────────────────────────────┘   │
└───────────────────────────────────────────────────────────────────┘
```

### Workflow Pattern

```yaml
activities:
  # Step 1: Classify query complexity
  - key: classify_query
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-3-5-haiku-20241022  # Cheap classifier
      prompt: |
        Classify this query as 'simple' or 'complex':
        "{{INPUT.user_query}}"

        Simple: weather, timers, basic facts, device control
        Complex: analysis, explanations, creative tasks

        Respond with JSON: {"complexity": "simple" or "complex"}

  # Step 2a: Handle simple queries (cheap model)
  - key: answer_simple
    activity_name: llm_prompt
    parameters:
      model:
        - anthropic/claude-3-5-haiku-20241022
        - openai/gpt-4o-mini
        - ollama/llama3.2:1b  # Local fallback
      prompt: "{{INPUT.user_query}}"
      max_tokens: 150
    depends_on:
      - activity_key: classify_query
        condition: "{{classify_query.result.complexity == 'simple'}}"

  # Step 2b: Handle complex queries (better model)
  - key: answer_complex
    activity_name: llm_prompt
    parameters:
      model:
        - anthropic/claude-3-5-sonnet-20241022
        - openai/gpt-4o
      prompt: "{{INPUT.user_query}}"
      max_tokens: 500
    depends_on:
      - activity_key: classify_query
        condition: "{{classify_query.result.complexity == 'complex'}}"

  # Step 3: Fetch weather data if needed (conditional)
  - key: fetch_weather
    activity_name: http_request
    parameters:
      method: GET
      url: "https://api.weather.gov/gridpoints/LOT/76,73/forecast"
    depends_on:
      - activity_key: classify_query
        condition: "{{INPUT.user_query | lower | contains('weather')}}"
```

### Hardware Setup

- **Raspberry Pi Zero 2 W** running kruxiaflow
- **USB Microphone** for voice input (e.g., PlayStation Eye)
- **Speaker** via 3.5mm jack or Bluetooth
- **Optional**: LED ring showing listening state

### Demo Features

1. **Cost Optimization**: Simple queries use Haiku ($0.002), complex use Sonnet ($0.020)
2. **Local Fallback**: When offline, uses Ollama running locally ($0.00)
3. **Real-time Stats**: Shows cost per conversation and daily totals
4. **Voice Interface**: Natural interaction, not just text
5. **Device Control**: Can trigger home automation via HTTP calls

### Why This Resonates

- **Relatable**: Everyone understands voice assistants (Alexa, Siri)
- **Cost Transparency**: "47 conversations cost $0.13" is powerful
- **Edge Computing**: "Runs on $15 hardware" differentiates from cloud-only
- **Offline Capability**: Ollama fallback shows resilience
- **Perfect for homes/small businesses**: Practical use case

---

## Demo #4: Real-time Content Moderation Station

**Tagline:** *"Budget-protected content moderation with multi-provider resilience"*

### Overview

A content moderation system demonstrating budget enforcement and automatic fallback when providers fail. Perfect for showing how kruxiaflow prevents runaway costs from abusive users.

### Visual Design

```
┌─────────────────────────────────────────────────────────────────┐
│  🛡️ Content Moderation Station          Budget Protection: ON  │
│  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ │
│                                                                   │
│  Submit Content for Review:                                      │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │ This is a sample comment that needs to be reviewed...   │   │
│  │                                                          │   │
│  └──────────────────────────────────────────────────────────┘   │
│                               [🔍 Moderate Content]              │
│                                                                   │
│  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ │
│                                                                   │
│  💰 Budget Protection                                            │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │ Daily Budget:  $5.00                                     │   │
│  │ Used Today:    $3.87                                     │   │
│  │ Remaining:     $1.13                                     │   │
│  │ [██████████████████████████████████████░░░░░] 77%       │   │
│  │                                                          │   │
│  │ ⚠️ Budget Alert: Approaching daily limit                │   │
│  │ ➜ Switching to cheaper models for remaining requests    │   │
│  └──────────────────────────────────────────────────────────┘   │
│                                                                   │
│  📊 Today's Moderation Stats                                     │
│  ┌─────────────────────┬────────────────────────────────────┐   │
│  │ Total Reviews: 1,247│ Avg Cost: $0.003/review           │   │
│  │ Flagged: 23 (1.8%)  │ Savings: $12.13 (75% vs baseline) │   │
│  │ Response: 0.4s avg  │ Fallbacks: 3 (provider timeouts)  │   │
│  └─────────────────────┴────────────────────────────────────┘   │
│                                                                   │
│  🎯 Intelligent Model Selection                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │ Current Budget Status:  ⚠️ 77% used                      │   │
│  │ Selected Model:         Haiku 3.5 (cheapest)            │   │
│  │ Fallback Chain:         Haiku → GPT-4o-mini → Gemini    │   │
│  │                                                          │   │
│  │ When budget > 50% remaining:  Use Sonnet (quality)      │   │
│  │ When budget < 50% remaining:  Use Haiku (efficient)     │   │
│  │ When budget < 10% remaining:  Rate limit + alerts       │   │
│  └──────────────────────────────────────────────────────────┘   │
│                                                                   │
│  Recent Moderation Results:                                      │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │ ✅ "Great post!"           Safe     $0.002  Haiku  0.3s  │   │
│  │ ✅ "Thanks for sharing"    Safe     $0.002  Haiku  0.4s  │   │
│  │ ⚠️ "You're wrong!"         Review   $0.005  Sonnet 0.6s  │   │
│  │ 🚫 "[Offensive content]"   Blocked  $0.003  Haiku  0.5s  │   │
│  │ ✅ "Interesting point"     Safe     $0.002  Haiku  0.3s  │   │
│  └──────────────────────────────────────────────────────────┘   │
│                                                                   │
│  🔄 Fallback Resilience Demo                                     │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │ [Simulate Provider Failure]                              │   │
│  │                                                          │   │
│  │ Recent Fallback Event:                                   │   │
│  │ ⚠️ 12:34 PM - Anthropic rate limit (429)                │   │
│  │ ✅ Auto-switched to OpenAI GPT-4o-mini                   │   │
│  │ ⏱️ Total delay: +0.2s (no workflow failure)             │   │
│  └──────────────────────────────────────────────────────────┘   │
└───────────────────────────────────────────────────────────────────┘
```

### Workflow Implementation

```yaml
activities:
  # Budget-aware moderation with intelligent routing
  - key: moderate_content
    activity_name: llm_prompt
    parameters:
      # Model selection based on remaining budget
      model:
        - anthropic/claude-3-5-haiku-20241022     # Primary: cheap
        - openai/gpt-4o-mini                      # Fallback 1
        - google/gemini-1.5-flash                 # Fallback 2
      prompt: |
        Moderate this content. Classify as: safe, review, or blocked.
        Provide JSON response:
        {
          "classification": "safe|review|blocked",
          "confidence": 0.0-1.0,
          "reason": "brief explanation"
        }

        Content: {{INPUT.user_content}}
      max_tokens: 100
    settings:
      budget:
        limit_usd: 5.00      # Daily budget limit
        action: abort        # Stop processing if exceeded
      retry:
        max_attempts: 3      # Retry on provider failures

  # Log result to database
  - key: log_moderation
    activity_name: postgres_query
    parameters:
      db_url: "{{SECRET.db_url}}"
      query: |
        INSERT INTO moderation_log
        (content_id, classification, confidence, cost_usd, model)
        VALUES ($1, $2, $3, $4, $5)
      params:
        - "{{INPUT.content_id}}"
        - "{{moderate_content.result.classification}}"
        - "{{moderate_content.result.confidence}}"
        - "{{moderate_content.cost_usd}}"
        - "{{moderate_content.model}}"
    depends_on:
      - moderate_content

  # Alert admin if budget threshold reached
  - key: send_budget_alert
    activity_name: email_send
    parameters:
      smtp_url: "{{SECRET.smtp_url}}"
      from: "alerts@example.com"
      to: ["admin@example.com"]
      subject: "⚠️ Moderation Budget Alert - {{WORKFLOW.remaining_budget_percent}}% Remaining"
      text_body: |
        Budget alert for content moderation system:

        Daily Budget: $5.00
        Used: ${{WORKFLOW.total_cost_usd}}
        Remaining: ${{WORKFLOW.remaining_budget_usd}}

        System has automatically switched to cheaper models.
    depends_on:
      - activity_key: moderate_content
        condition: "{{WORKFLOW.remaining_budget_percent}} < 25"
```

### Demo Script

1. **Normal Operation**: Submit 10 clean comments
   - Show each costs ~$0.002 (Haiku)
   - Total: $0.020 for 10 reviews
   - Compare: "GPT-4 would cost $0.15 (7.5x more)"

2. **Trigger Budget Alert**: Simulate 1,000+ reviews
   - Watch budget meter climb
   - At 75%, see alert: "Switching to cheaper models"
   - At 90%, see warning: "Budget limit approaching"

3. **Demonstrate Fallback**: Simulate Anthropic timeout
   - Send moderation request
   - Show: "⚠️ Anthropic timeout → GPT-4o-mini"
   - Workflow completes successfully

4. **Abuse Protection**: Try to exceed budget
   - Submit 2,000 moderation requests
   - System stops at $5.00: "Budget limit reached"
   - Show: "Protected from $50+ runaway costs"

### Why This Matters

- **Addresses Real Pain Point**: Content moderation can cost $100k+/month at scale
- **Budget Protection**: Shows how kruxiaflow prevents billing disasters
- **Resilience**: Automatic fallback prevents service disruption
- **ROI Clear**: "$0.003 per moderation vs. $0.012 with GPT-4 only"
- **Perfect for SaaS Companies**: They face this exact problem

---

## Demo #5: Edge AI News Aggregator

**Tagline:** *"Personalized news digests running on edge hardware"*

### Overview

A news aggregation system that fetches RSS feeds, analyzes articles with LLM, generates personalized summaries, and emails daily digests - all from a Raspberry Pi.

### Features

- **Parallel Processing**: Analyze 10+ news articles simultaneously
- **Cost Tracking**: Show per-article analysis costs
- **Scheduled Execution**: Runs automatically every morning
- **Email Delivery**: Sends HTML digest via SMTP

### Workflow Pattern

```yaml
activities:
  # Fetch RSS feeds in parallel
  - key: fetch_tech_news
    activity_name: http_request
    parameters:
      method: GET
      url: "https://feeds.arstechnica.com/arstechnica/technology-lab"

  - key: fetch_business_news
    activity_name: http_request
    parameters:
      method: GET
      url: "https://feeds.bloomberg.com/markets/news.rss"

  # Analyze each article with LLM (parallel execution)
  - key: analyze_articles
    activity_name: llm_prompt
    iteration_limit: 10  # Process up to 10 articles
    parameters:
      model: anthropic/claude-3-5-haiku-20241022
      prompt: |
        Summarize this article in 2-3 sentences:
        {{INPUT.article_text}}

  # Compile digest with better model
  - key: compile_digest
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-3-5-sonnet-20241022
      prompt: |
        Create a personalized news digest from these summaries:
        {{analyze_articles[*].result.content}}

  # Email the digest
  - key: send_digest
    activity_name: email_send
    parameters:
      to: ["{{INPUT.subscriber_email}}"]
      subject: "Your Daily News Digest - {{WORKFLOW.date}}"
      html_body: "{{compile_digest.result.content}}"
```

### Demo Value

- Shows **scheduled workflows** (cron-like execution)
- Demonstrates **parallel article analysis** (10 articles at once)
- Perfect for **"set it and forget it"** edge deployment
- Clear ROI: "Daily digest costs $0.15 vs. $2.50 for GPT-4"

---

## Demo #6: IoT Sensor Intelligence Hub

**Tagline:** *"Turn sensor data into insights with edge AI"*

### Overview

Demonstrates kruxiaflow as an IoT edge processing hub: collect sensor data → analyze with LLM → alert on anomalies → log to database.

### Use Case Example

**Smart Office Environmental Monitoring:**
- Temperature, humidity, CO2 sensors
- LLM analyzes patterns and detects anomalies
- Sends alerts when conditions are suboptimal
- Generates weekly reports with insights

### Workflow Pattern

```yaml
activities:
  # Collect sensor readings (simulated or real IoT devices)
  - key: read_sensors
    activity_name: http_request
    parameters:
      method: GET
      url: "http://localhost:9090/sensors/current"

  # Analyze with LLM
  - key: analyze_conditions
    activity_name: llm_prompt
    parameters:
      model:
        - anthropic/claude-3-5-haiku-20241022
        - ollama/llama3.2:1b  # Local fallback
      prompt: |
        Analyze these environmental readings:
        Temperature: {{read_sensors.response.temperature}}°F
        Humidity: {{read_sensors.response.humidity}}%
        CO2: {{read_sensors.response.co2}}ppm

        Provide assessment: optimal/suboptimal/alert
        Include brief recommendation if needed.

  # Log to database
  - key: log_reading
    activity_name: postgres_query
    parameters:
      query: |
        INSERT INTO sensor_log (temperature, humidity, co2, assessment, cost_usd)
        VALUES ($1, $2, $3, $4, $5)

  # Send alert if needed (conditional)
  - key: send_alert
    activity_name: email_send
    parameters:
      subject: "⚠️ Environmental Alert: {{analyze_conditions.result.status}}"
      text_body: "{{analyze_conditions.result.recommendation}}"
    depends_on:
      - activity_key: analyze_conditions
        condition: "{{analyze_conditions.result.status == 'alert'}}"
```

### Demo Value

- **Edge Computing**: Perfect fit for Raspberry Pi (sensors, local processing)
- **Cost Efficiency**: "1000 sensor readings/day = $2/month"
- **Local Fallback**: Ollama ensures operation even offline
- **Real-world IoT**: Resonates with industrial/commercial customers

---

## Comparison Matrix

| Demo                       | Visual Impact | Cost Tracking | Streaming | Edge Computing | Fallback | Target Audience           | Setup Complexity |
|----------------------------|---------------|---------------|-----------|----------------|----------|---------------------------|------------------|
| **#1: Command Center**     | ⭐⭐⭐⭐⭐        | ⭐⭐⭐⭐⭐        | ⭐⭐⭐⭐⭐    | ⭐⭐⭐⭐          | ⭐⭐⭐⭐⭐    | AI/SaaS Developers        | Medium           |
| **#2: Research Station**   | ⭐⭐⭐⭐         | ⭐⭐⭐⭐⭐        | ⭐⭐⭐⭐     | ⭐⭐⭐⭐          | ⭐⭐⭐⭐     | AI Researchers            | Low              |
| **#3: Smart Home**         | ⭐⭐⭐          | ⭐⭐⭐⭐         | ⭐⭐        | ⭐⭐⭐⭐⭐         | ⭐⭐⭐⭐⭐    | Consumers/Small Business  | High (hardware)  |
| **#4: Content Moderation** | ⭐⭐⭐⭐         | ⭐⭐⭐⭐⭐        | ⭐⭐        | ⭐⭐⭐           | ⭐⭐⭐⭐⭐    | SaaS/Social Platforms     | Low              |
| **#5: News Aggregator**    | ⭐⭐⭐          | ⭐⭐⭐⭐         | ⭐⭐        | ⭐⭐⭐⭐          | ⭐⭐⭐      | Consumers                 | Low              |
| **#6: IoT Sensor Hub**     | ⭐⭐           | ⭐⭐⭐          | ⭐         | ⭐⭐⭐⭐⭐         | ⭐⭐⭐⭐     | Industrial/Commercial     | High (hardware)  |

**Recommendation:** **Demo #1 (Live AI Command Center)** is the clear winner for maximum impact with investors and potential customers.

---

## Implementation Guide

### Quick Start: Demo #1 on Raspberry Pi Zero 2 W

#### Hardware Requirements

- Raspberry Pi Zero 2 W
- microSD card (32GB+ recommended)
- USB power supply (5V 2A)
- USB-to-Ethernet adapter (optional for faster setup)
- Laptop/tablet for viewing dashboard

#### Software Setup

**1. Install Raspberry Pi OS Lite (64-bit)**

```bash
# Use Raspberry Pi Imager
# Select: Raspberry Pi OS Lite (64-bit)
# Configure WiFi and SSH before flashing
```

**2. Install Docker & Docker Compose**

```bash
ssh pi@raspberrypi.local

# Update system
sudo apt update && sudo apt upgrade -y

# Install Docker
curl -fsSL https://get.docker.com | sh
sudo usermod -aG docker pi
newgrp docker

# Install Docker Compose
sudo apt install docker-compose -y
```

**3. Clone Kruxiaflow Repository**

```bash
git clone https://github.com/kruxiaflow/kruxiaflow.git
cd kruxiaflow
```

**4. Configure Environment**

```bash
# Create .env file
cat > .env <<EOF
# Database
POSTGRES_PASSWORD=kruxiaflow_demo

# API Keys (add your own)
ANTHROPIC_API_KEY=sk-ant-...
OPENAI_API_KEY=sk-...
GOOGLE_API_KEY=...

# SMTP (use Mailhog for testing)
SMTP_URL=smtp://mailhog:1025

# Resource Limits (Pi Zero 2 W optimized)
KRUXIAFLOW_POSTGRES_POOL_MAX_CONNECTIONS=5
KRUXIAFLOW_WORKER_MAX_CONCURRENT_ACTIVITIES=3
EOF
```

**5. Create Docker Compose Configuration**

```yaml
# docker-compose.pi-demo.yml
version: '3.8'

services:
  postgres:
    image: postgres:15-alpine
    environment:
      POSTGRES_DB: kruxiaflow
      POSTGRES_USER: kruxiaflow
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}
    volumes:
      - postgres_data:/var/lib/postgresql/data
    command: >
      postgres
      -c shared_buffers=64MB
      -c effective_cache_size=128MB
      -c max_connections=10
    deploy:
      resources:
        limits:
          memory: 200M

  api:
    image: kruxiaflow/api:latest
    environment:
      DATABASE_URL: postgres://kruxiaflow:${POSTGRES_PASSWORD}@postgres/kruxiaflow
      ANTHROPIC_API_KEY: ${ANTHROPIC_API_KEY}
      OPENAI_API_KEY: ${OPENAI_API_KEY}
    ports:
      - "8080:8080"
    depends_on:
      - postgres
    deploy:
      resources:
        limits:
          memory: 150M

  worker:
    image: kruxiaflow/worker:latest
    environment:
      DATABASE_URL: postgres://kruxiaflow:${POSTGRES_PASSWORD}@postgres/kruxiaflow
      ANTHROPIC_API_KEY: ${ANTHROPIC_API_KEY}
      OPENAI_API_KEY: ${OPENAI_API_KEY}
      KRUXIAFLOW_WORKER_MAX_CONCURRENT_ACTIVITIES: 3
    depends_on:
      - postgres
    deploy:
      resources:
        limits:
          memory: 150M

  mailhog:
    image: mailhog/mailhog:latest
    ports:
      - "1025:1025"
      - "8025:8025"

  dashboard:
    image: nginx:alpine
    volumes:
      - ./demos/dashboard:/usr/share/nginx/html:ro
    ports:
      - "80:80"

volumes:
  postgres_data:
```

**6. Start Services**

```bash
docker-compose -f docker-compose.pi-demo.yml up -d

# Wait for services to start
sleep 30

# Check status
docker-compose -f docker-compose.pi-demo.yml ps

# Seed LLM pricing data
docker-compose -f docker-compose.pi-demo.yml exec api \
  kruxiaflow seed-llm
```

**7. Deploy Dashboard**

```bash
# Copy dashboard files
mkdir -p demos/dashboard
# ... copy dashboard HTML/CSS/JS files
```

**8. Load Demo Workflows**

```bash
# Submit research workflow definition
curl -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Content-Type: application/yaml" \
  --data-binary @examples/09b-streaming-research.yaml

# Submit moderation workflow
curl -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Content-Type: application/yaml" \
  --data-binary @examples/04-moderate-content.yaml
```

**9. Access Dashboard**

```bash
# Open in browser
http://raspberrypi.local/
```

#### Performance Tuning

**PostgreSQL Optimization:**
```sql
-- Aggressive vacuuming for limited RAM
ALTER SYSTEM SET autovacuum_naptime = '10s';
ALTER SYSTEM SET autovacuum_vacuum_cost_limit = 500;
SELECT pg_reload_conf();
```

**Swap Configuration:**
```bash
# Increase swap (helps with memory spikes)
sudo dphys-swapfile swapoff
sudo sed -i 's/CONF_SWAPSIZE=100/CONF_SWAPSIZE=1024/' /etc/dphys-swapfile
sudo dphys-swapfile setup
sudo dphys-swapfile swapon
```

---

## Sales Pitch Template

Use this script when demoing to potential customers:

### The Problem (30 seconds)

> "AI API costs are the #1 pain point for companies building agentic applications. Last year, Anthropic had to discontinue their 'unlimited' Claude Code plan because users were consuming 10 billion tokens per month. 73% of funded AI startups are essentially API wrappers with no cost control, burning through runway on unpredictable LLM bills. And when OpenAI or Anthropic goes down, your entire product stops working."

### The Demo (2 minutes)

> "Watch this Raspberry Pi—a $15 device with 512MB of RAM—orchestrate GPT-4, Claude, and Gemini simultaneously with full cost transparency. [Submit research workflow] See this streaming output? We're using Claude Sonnet for quality where it matters, but the preparation steps use Haiku at 1/10th the cost. This workflow costs $0.047. If we used GPT-4 for everything? $0.18—four times more expensive.
>
> Now watch what happens when we approach our budget limit. [Trigger alert] See that? Automatic switch to cheaper models. And if Claude hits a rate limit? [Simulate failure] Instant failover to GPT-4. The workflow never fails—it just adapts.
>
> This dashboard shows real-time cost tracking across all workflows. Today we've run 23 workflows for $2.47. The naive GPT-4-only approach would have cost $16.70. That's 85% savings with the same quality."

### The Value (30 seconds)

> "Kruxiaflow gives you production-grade workflow orchestration with built-in cost control and multi-provider resilience. Define workflows in YAML using our six built-in activities—no custom code needed. Deploy anywhere: Raspberry Pi, AWS, your data center. And most importantly, you'll never get a surprise $50,000 API bill, because budget enforcement is built into the platform."

---

## Conclusion

**Recommended Demo:** #1 (Live AI Command Center)

**Why:**
- Shows all key features in one interface
- Visually compelling (live streaming, charts, multiple workflows)
- Addresses #1 customer pain point (cost control)
- Demonstrates competitive advantages (multi-provider, edge deployment)
- Easy to understand ("this saves 85%" is instantly compelling)
- Perfect scale for Raspberry Pi Zero 2 W
- Works great in trade show / demo environments

**Expected Impact:**
- **Wow Factor**: "This tiny device orchestrates enterprise AI"
- **ROI Clarity**: "85% cost reduction vs. baseline"
- **Reliability**: "Automatic failover prevents downtime"
- **Accessibility**: "No code required, just YAML"

**Next Steps:**
1. Build dashboard UI (HTML/CSS/JS)
2. Optimize docker-compose for Pi Zero 2 W
3. Create demo script and video walkthrough
4. Test end-to-end on actual hardware
5. Package as downloadable Pi image for customers to try
