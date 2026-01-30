# Test Plan: Kruxia Flow MCP Server

**Epic**: MCP Server for AI Agent Integration
**Status**: 🔮 Planned
**Priority**: P1 (High)
**Author**: QA Engineering Team
**Last Updated**: 2026-01-30

---

## 1. Test Plan Overview

This document defines the comprehensive test strategy for the Kruxia Flow MCP Server, covering protocol compliance, functional correctness, integration testing, and real-world AI agent scenarios.

### 1.1 Testing Tiers

```mermaid
flowchart TB
    subgraph T1["Tier 1: Unit Tests"]
        direction LR
        U1[Protocol Parser]
        U2[Schema Validation]
        U3[Mermaid Generator]
        U4[Cost Calculator]
    end

    subgraph T2["Tier 2: Integration Tests"]
        direction LR
        I1[MCP ↔ API]
        I2[Auth Flow]
        I3[WebSocket]
        I4[Error Handling]
    end

    subgraph T3["Tier 3: E2E Agent Tests"]
        direction LR
        E1[Claude Code]
        E2[MCP Inspector]
        E3[Custom Agents]
    end

    subgraph T4["Tier 4: Scenario Tests"]
        direction LR
        S1[Workflow Creation]
        S2[Monitoring]
        S3[Cost Tracking]
        S4[Human-in-Loop]
    end

    T1 --> T2 --> T3 --> T4
```

### 1.2 Test Coverage Goals

| Category                    | Coverage Target | Automation Level |
|-----------------------------|-----------------|------------------|
| MCP Protocol Compliance     | 100%            | Fully Automated  |
| Tool Schema Validation      | 100%            | Fully Automated  |
| API Integration             | 95%             | Fully Automated  |
| Error Handling              | 90%             | Fully Automated  |
| Agent Integration           | 80%             | Semi-Automated   |
| Natural Language Scenarios  | 60%             | Manual + Scripts |

---

## 2. Example AI Prompts for Workflow Creation

This section contains example prompts an AI agent would process, along with the expected Kruxia Flow workflow outputs.

### 2.1 Esoteric/Creative Prompts

#### Prompt 1: Moon Phase and Location Tracker

> "Create a workflow that fetches the current moon phase and illumination data for a given location, retrieves weather conditions, generates a summary using an LLM with visibility recommendations, and emails the results."

**Expected Workflow Structure**:

```yaml
name: moon_phase_tracker
description: Track moon phase and visibility conditions for a location

activities:
  - key: fetch_moon_data
    activity_name: http_request
    parameters:
      method: GET
      url: "https://api.astronomyapi.com/v2/bodies/positions/moon"
      headers:
        Authorization: "Bearer {{SECRET.astronomy_api_key}}"
      query_params:
        latitude: "{{INPUT.latitude}}"
        longitude: "{{INPUT.longitude}}"
        elevation: "{{INPUT.elevation | default(0)}}"
        from_date: "{{INPUT.date}}"
        to_date: "{{INPUT.date}}"
        time: "{{INPUT.time | default('20:00:00')}}"
    outputs:
      - response

  - key: fetch_weather
    activity_name: http_request
    parameters:
      method: GET
      url: "https://api.openweathermap.org/data/2.5/weather"
      query_params:
        lat: "{{INPUT.latitude}}"
        lon: "{{INPUT.longitude}}"
        appid: "{{SECRET.openweather_api_key}}"
        units: "metric"
    outputs:
      - response
    depends_on:
      - fetch_moon_data

  - key: generate_summary
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-sonnet-4-5-20250929
      system_prompt: |
        You are an astronomy assistant. Provide helpful information about
        moon phases and viewing conditions for stargazers and photographers.
      prompt: |
        Location: {{INPUT.location_name}}
        Coordinates: {{INPUT.latitude}}, {{INPUT.longitude}}
        Date/Time: {{INPUT.date}} at {{INPUT.time | default('20:00:00')}}

        Moon Data:
        {{fetch_moon_data.response.body | tojson}}

        Weather Conditions:
        {{fetch_weather.response.body | tojson}}

        Generate a summary covering:
        1) Current moon phase and illumination percentage
        2) Moonrise and moonset times
        3) Weather conditions affecting visibility
        4) Recommendations for moon observation or photography
      max_tokens: 1500
    outputs:
      - result
    settings:
      budget:
        limit_usd: 0.05
    depends_on:
      - fetch_weather

  - key: send_report
    activity_name: send_email
    parameters:
      to: "{{INPUT.email}}"
      subject: "Moon Phase Report for {{INPUT.location_name}}"
      html_body: |
        <h1>Moon Phase Report</h1>
        <p>Location: {{INPUT.location_name}}</p>
        <p>Date: {{INPUT.date}}</p>
        <hr/>
        {{generate_summary.result.content | markdown_to_html}}
    depends_on:
      - generate_summary
```

**Expected Mermaid Diagram**:

```mermaid
flowchart TB
    fetch_moon_data[fetch_moon_data<br/>HTTP: GET astronomyapi.com]
    fetch_weather[fetch_weather<br/>HTTP: GET openweathermap]
    generate_summary[generate_summary<br/>LLM: claude-sonnet-4-5]
    send_report[send_report<br/>Email]

    fetch_moon_data --> fetch_weather
    fetch_weather --> generate_summary
    generate_summary --> send_report
```

---

#### Prompt 2: Sourdough Bread Monitoring System

> "Build a workflow that monitors temperature and humidity from IoT sensors every 30 minutes, uses AI to analyze if conditions are optimal for sourdough fermentation, and sends alerts if adjustments are needed."

**Expected Workflow Structure**:

```yaml
name: sourdough_fermentation_monitor
description: AI-powered sourdough fermentation environment monitoring

activities:
  - key: fetch_sensor_data
    activity_name: http_request
    parameters:
      method: GET
      url: "{{INPUT.sensor_hub_url}}/api/readings"
      headers:
        X-API-Key: "{{SECRET.sensor_api_key}}"
    outputs:
      - response

  - key: fetch_fermentation_history
    activity_name: postgres_query
    parameters:
      query: |
        SELECT timestamp, temperature, humidity, co2_level, analysis
        FROM fermentation_log
        WHERE batch_id = $1
        ORDER BY timestamp DESC
        LIMIT 24
      params:
        - "{{INPUT.batch_id}}"
    outputs:
      - rows
    depends_on:
      - fetch_sensor_data

  - key: analyze_conditions
    activity_name: llm_prompt
    parameters:
      model:
        - anthropic/claude-haiku-4-20250415
        - google/gemini-2.0-flash-lite
      system_prompt: |
        You are an expert sourdough baker and fermentation scientist.
        Analyze environmental data and determine if conditions are optimal.

        Optimal ranges:
        - Temperature: 75-82°F (24-28°C) for active fermentation
        - Humidity: 65-75% relative humidity
        - CO2: Rising indicates healthy yeast activity

        Respond ONLY with valid JSON.
      prompt: |
        Current Readings:
        {{fetch_sensor_data.response.body | tojson}}

        Recent History:
        {{fetch_fermentation_history.rows | tojson}}

        Fermentation Stage: {{INPUT.stage}}
        Hours Since Start: {{INPUT.hours_elapsed}}

        Analyze and respond with:
        {
          "status": "optimal|warning|critical",
          "temperature_assessment": "...",
          "humidity_assessment": "...",
          "recommendations": ["..."],
          "estimated_ready_hours": N
        }
      max_tokens: 500
    outputs:
      - result
    settings:
      budget:
        limit_usd: 0.01
    depends_on:
      - fetch_fermentation_history

  - key: log_analysis
    activity_name: postgres_query
    parameters:
      query: |
        INSERT INTO fermentation_log
        (batch_id, timestamp, temperature, humidity, co2_level, analysis)
        VALUES ($1, NOW(), $2, $3, $4, $5)
      params:
        - "{{INPUT.batch_id}}"
        - "{{fetch_sensor_data.response.body.temperature}}"
        - "{{fetch_sensor_data.response.body.humidity}}"
        - "{{fetch_sensor_data.response.body.co2}}"
        - "{{analyze_conditions.result.content}}"
    depends_on:
      - analyze_conditions

  - key: send_alert
    activity_name: send_email
    parameters:
      to: "{{INPUT.alert_email}}"
      subject: "🍞 Sourdough Alert: {{analyze_conditions.result.content | fromjson | get('status')}}"
      text_body: |
        Batch: {{INPUT.batch_id}}
        Status: {{analyze_conditions.result.content | fromjson | get('status')}}

        Recommendations:
        {{analyze_conditions.result.content | fromjson | get('recommendations') | join('\n- ')}}
    depends_on:
      - activity_key: analyze_conditions
        conditions:
          - "{{analyze_conditions.result.content | contains(substring='warning') or analyze_conditions.result.content | contains(substring='critical')}}"
```

---

#### Prompt 3: Vintage Wine Label Authenticator

> "Create a workflow that takes a photo of a wine label, uses vision AI to extract text and assess authenticity markers, cross-references with a wine database, and generates a provenance report."

**Expected Workflow Structure**:

```yaml
name: wine_label_authenticator
description: AI-powered vintage wine label authentication and provenance verification

activities:
  - key: analyze_label_image
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-sonnet-4-5-20250929
      prompt: |
        Analyze this wine label image and extract:
        1) Winery/Producer name
        2) Wine name/Cuvée
        3) Vintage year
        4) Region/Appellation
        5) Alcohol percentage
        6) Any authenticity markers (holograms, embossing, serial numbers)
        7) Condition assessment (fading, damage, suspicious alterations)

        Image: {{INPUT.label_image_base64}}

        Return ONLY valid JSON:
        {
          "producer": "...",
          "wine_name": "...",
          "vintage": YYYY,
          "region": "...",
          "appellation": "...",
          "alcohol_pct": N.N,
          "authenticity_markers": [...],
          "condition_notes": [...],
          "suspicious_elements": [...]
        }
      max_tokens: 1000
    outputs:
      - result
    settings:
      budget:
        limit_usd: 0.05

  - key: lookup_wine_database
    activity_name: http_request
    parameters:
      method: POST
      url: "https://wine-database.example.com/api/v1/search"
      headers:
        Authorization: "Bearer {{SECRET.wine_db_key}}"
      body:
        producer: "{{analyze_label_image.result.content | fromjson | get('producer')}}"
        wine_name: "{{analyze_label_image.result.content | fromjson | get('wine_name')}}"
        vintage: "{{analyze_label_image.result.content | fromjson | get('vintage')}}"
    outputs:
      - response
    depends_on:
      - analyze_label_image

  - key: check_auction_records
    activity_name: http_request
    parameters:
      method: GET
      url: "https://wine-auction-api.example.com/provenance"
      query_params:
        producer: "{{analyze_label_image.result.content | fromjson | get('producer')}}"
        vintage: "{{analyze_label_image.result.content | fromjson | get('vintage')}}"
    outputs:
      - response
    depends_on:
      - analyze_label_image

  - key: generate_provenance_report
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-sonnet-4-5-20250929
      system_prompt: |
        You are a wine authentication expert. Generate a professional
        provenance and authenticity assessment report.
      prompt: |
        ## Label Analysis
        {{analyze_label_image.result.content}}

        ## Database Record
        {{lookup_wine_database.response.body | tojson}}

        ## Auction/Provenance History
        {{check_auction_records.response.body | tojson}}

        Generate a comprehensive authenticity report including:
        1) Authenticity confidence score (0-100)
        2) Identified risk factors
        3) Comparison with known authentic examples
        4) Estimated market value (if authentic)
        5) Recommendation (authentic/suspicious/likely counterfeit)
      max_tokens: 2000
    outputs:
      - result
    settings:
      budget:
        limit_usd: 0.10
    depends_on:
      - lookup_wine_database
      - check_auction_records

  - key: store_report
    activity_name: postgres_query
    parameters:
      query: |
        INSERT INTO authentication_reports
        (wine_id, label_hash, analysis_json, report, confidence_score, created_at)
        VALUES (
          $1,
          $2,
          $3::jsonb,
          $4,
          $5,
          NOW()
        )
        RETURNING id
      params:
        - "{{lookup_wine_database.response.body.wine_id}}"
        - "{{INPUT.label_image_hash}}"
        - "{{analyze_label_image.result.content}}"
        - "{{generate_provenance_report.result.content}}"
        - "{{generate_provenance_report.result.content | extract_confidence_score}}"
    outputs:
      - result
    depends_on:
      - generate_provenance_report
```

---

### 2.2 Prompts Inspired by Researcher Project

#### Prompt 4: Academic Paper Citation Network Analysis

> "Build a workflow like the researcher project that ingests a PDF paper, extracts citations, resolves them against CrossRef/OpenAlex, and builds a citation network graph."

**Expected Workflow Structure**:

```yaml
name: citation_network_builder
description: Ingest academic paper, extract and resolve citations, build network

activities:
  - key: download_paper
    activity_name: http_request
    parameters:
      method: GET
      url: "{{INPUT.paper_url}}"
      response_type: binary
      save_to_file: paper.pdf
    outputs:
      - file_path

  - key: extract_text
    activity_name: http_request
    parameters:
      method: POST
      url: "{{INPUT.parser_service_url}}/parse"
      headers:
        Content-Type: application/pdf
      body_file: "{{download_paper.file_path}}"
    outputs:
      - response
    depends_on:
      - download_paper

  - key: extract_citations
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-sonnet-4-5-20250929
      system_prompt: |
        You are an expert at identifying and extracting bibliographic citations.
        Extract ALL citations from the provided academic paper text.
        Return structured JSON for each citation.
      prompt: |
        Paper text:
        {{extract_text.response.body.text}}

        Extract all citations and return as JSON array:
        {
          "citations": [
            {
              "raw_text": "full citation string",
              "authors": ["Last, First", ...],
              "title": "...",
              "year": YYYY,
              "journal": "...",
              "doi": "10.xxxx/..." or null,
              "citation_context": "sentence where cited"
            }
          ]
        }
      max_tokens: 8000
    outputs:
      - result
    settings:
      budget:
        limit_usd: 0.20
    depends_on:
      - extract_text

  - key: resolve_crossref
    activity_name: http_request
    iteration_scoped: true
    iteration_limit: 50
    parameters:
      method: GET
      url: "https://api.crossref.org/works"
      query_params:
        query.bibliographic: "{{CURRENT_ITEM.title}} {{CURRENT_ITEM.authors | first}}"
        rows: "1"
      headers:
        User-Agent: "KruxiaFlow/1.0 (mailto:{{INPUT.contact_email}})"
    outputs:
      - response
    settings:
      rate_limit:
        requests_per_second: 5
    depends_on:
      - extract_citations
    iterate_over: "{{extract_citations.result.content | fromjson | get('citations')}}"

  - key: build_network_graph
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-haiku-4-20250415
      prompt: |
        Given these resolved citations:
        {{resolve_crossref.responses | tojson}}

        Generate a Mermaid flowchart showing the citation network.
        The source paper ({{INPUT.paper_title}}) should be the central node.
        Show connections to cited works and their relationships.

        Return ONLY the Mermaid diagram code:
        ```mermaid
        flowchart TB
        ...
        ```
      max_tokens: 2000
    outputs:
      - result
    settings:
      budget:
        limit_usd: 0.02
    depends_on:
      - resolve_crossref

  - key: store_network
    activity_name: postgres_query
    parameters:
      query: |
        INSERT INTO citation_networks
        (source_paper_doi, citations_json, mermaid_diagram, created_at)
        VALUES ($1, $2::jsonb, $3, NOW())
        RETURNING id
      params:
        - "{{INPUT.paper_doi}}"
        - "{{extract_citations.result.content}}"
        - "{{build_network_graph.result.content}}"
    outputs:
      - result
    depends_on:
      - build_network_graph
```

---

#### Prompt 5: Research Question Deep Dive (Researcher Pattern)

> "Answer a complex research question by retrieving relevant passages from my document library, extracting arguments and positions, and generating a structured analysis with citations."

**Expected Workflow** (mirrors `research_query_enhanced.yaml`):

```yaml
name: research_deep_dive
description: Answer research question with argument-aware RAG analysis

activities:
  - key: embed_query
    activity_name: embedding
    parameters:
      model: google/gemini-embedding-001
      input:
        - "{{INPUT.question}}"
    outputs:
      - embeddings
    settings:
      budget:
        limit: 0.01

  - key: search_passages
    activity_name: postgres_query
    parameters:
      db_url: "{{INPUT.db_url}}"
      query: |
        WITH query_embedding AS (
          SELECT $1::vector(3072) as embedding
        )
        SELECT
          p.id,
          p.content,
          p.page_start,
          s.title,
          s.authors,
          1 - (p.embedding <=> (SELECT embedding FROM query_embedding)) as similarity
        FROM passages p
        JOIN sources s ON p.source_id = s.id
        WHERE s.user_id = $2::uuid
        ORDER BY p.embedding <=> (SELECT embedding FROM query_embedding)
        LIMIT $3
      params:
        - "{{embed_query.embeddings[0]}}"
        - "{{INPUT.user_id}}"
        - "{{INPUT.limit | default(15)}}"
    outputs:
      - rows
    depends_on:
      - embed_query

  - key: extract_arguments
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-opus-4-5
      system_prompt: |
        Extract philosophical arguments, positions, and interpretations
        from the provided passages. Be conservative - only extract
        elements with >0.7 confidence.
      prompt: |
        Analyze these passages for arguments and positions:
        {% for passage in search_passages.rows %}
        ---
        PASSAGE_ID: {{passage.id}}
        SOURCE: {{passage.title}}
        PAGE: {{passage.page_start}}

        {{passage.content}}
        ---
        {% endfor %}

        Return JSON with: arguments, positions, interpretations arrays.
      max_tokens: 4000
    outputs:
      - result
    settings:
      budget:
        limit_usd: 0.15
    depends_on:
      - search_passages

  - key: generate_analysis
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-opus-4-5
      system_prompt: |
        You are a research assistant. Generate argument-aware analysis
        with proper citations (Author, Year, p. X).
      prompt: |
        QUESTION: {{INPUT.question}}

        PASSAGES:
        {% for p in search_passages.rows %}
        [{{p.title}}] (p. {{p.page_start}}): {{p.content}}
        {% endfor %}

        EXTRACTED STRUCTURE:
        {{extract_arguments.result.content}}

        Provide structured analysis with:
        1) Overview of main positions
        2) Key arguments with premise-conclusion structure
        3) Position mapping (who holds what views)
        4) Critical disagreements
        5) Synthesis answering the question
        6) Sources Cited section (mandatory)
      max_tokens: 4000
    outputs:
      - result
    settings:
      budget:
        limit_usd: 0.20
    depends_on:
      - extract_arguments
```

---

#### Prompt 6: Multi-Source Comparative Analysis (Researcher Pattern)

> "Compare how three different philosophers (Kant, Hume, and Aristotle) approach the concept of virtue. Use my uploaded texts and generate a structured comparison."

**Expected Workflow**:

```yaml
name: philosophical_comparison
description: Multi-source comparative analysis of philosophical concepts

activities:
  - key: embed_query
    activity_name: embedding
    parameters:
      model: google/gemini-embedding-001
      input:
        - "virtue ethics Kant categorical imperative duty"
        - "virtue ethics Hume moral sentiment sympathy"
        - "virtue ethics Aristotle eudaimonia character excellence"
    outputs:
      - embeddings

  - key: search_kant_passages
    activity_name: postgres_query
    parameters:
      db_url: "{{INPUT.db_url}}"
      query: |
        SELECT p.id, p.content, p.page_start, s.title
        FROM passages p
        JOIN sources s ON p.source_id = s.id
        WHERE s.user_id = $1::uuid
          AND s.authors::text ILIKE '%kant%'
        ORDER BY p.embedding <=> $2::vector(3072)
        LIMIT 10
      params:
        - "{{INPUT.user_id}}"
        - "{{embed_query.embeddings[0]}}"
    outputs:
      - rows
    depends_on:
      - embed_query

  - key: search_hume_passages
    activity_name: postgres_query
    parameters:
      db_url: "{{INPUT.db_url}}"
      query: |
        SELECT p.id, p.content, p.page_start, s.title
        FROM passages p
        JOIN sources s ON p.source_id = s.id
        WHERE s.user_id = $1::uuid
          AND s.authors::text ILIKE '%hume%'
        ORDER BY p.embedding <=> $2::vector(3072)
        LIMIT 10
      params:
        - "{{INPUT.user_id}}"
        - "{{embed_query.embeddings[1]}}"
    outputs:
      - rows
    depends_on:
      - embed_query

  - key: search_aristotle_passages
    activity_name: postgres_query
    parameters:
      db_url: "{{INPUT.db_url}}"
      query: |
        SELECT p.id, p.content, p.page_start, s.title
        FROM passages p
        JOIN sources s ON p.source_id = s.id
        WHERE s.user_id = $1::uuid
          AND s.authors::text ILIKE '%aristotle%'
        ORDER BY p.embedding <=> $2::vector(3072)
        LIMIT 10
      params:
        - "{{INPUT.user_id}}"
        - "{{embed_query.embeddings[2]}}"
    outputs:
      - rows
    depends_on:
      - embed_query

  - key: comparative_analysis
    activity_name: llm_prompt
    parameters:
      model: anthropic/claude-opus-4-5
      system_prompt: |
        You are an expert in comparative philosophy. Generate a rigorous
        comparison of philosophical positions with proper citations.
      prompt: |
        Compare how Kant, Hume, and Aristotle approach VIRTUE.

        ## Kant's Texts:
        {% for p in search_kant_passages.rows %}
        [{{p.title}}, p.{{p.page_start}}]: {{p.content}}
        {% endfor %}

        ## Hume's Texts:
        {% for p in search_hume_passages.rows %}
        [{{p.title}}, p.{{p.page_start}}]: {{p.content}}
        {% endfor %}

        ## Aristotle's Texts:
        {% for p in search_aristotle_passages.rows %}
        [{{p.title}}, p.{{p.page_start}}]: {{p.content}}
        {% endfor %}

        Generate a structured comparison:
        1) Each philosopher's definition of virtue
        2) Role of reason vs. emotion
        3) Relationship to happiness/good life
        4) Key agreements and disagreements
        5) Comparative table
        6) Sources Cited
      max_tokens: 5000
    outputs:
      - result
    settings:
      budget:
        limit_usd: 0.25
    depends_on:
      - search_kant_passages
      - search_hume_passages
      - search_aristotle_passages
```

---

## 3. Observability Test Cases

### 3.1 Workflow State Queries

#### Test Case OBS-01: List Running Workflows

**MCP Tool Call**:
```json
{
  "tool": "list_workflows",
  "arguments": {
    "status": "running",
    "limit": 10
  }
}
```

**Expected Response**:
```json
{
  "workflows": [
    {
      "workflow_id": "019353a1-b0c1-7000-8000-000000000001",
      "definition_name": "research_deep_dive",
      "status": "running",
      "started_at": "2026-01-30T10:30:00Z",
      "current_activity": "extract_arguments",
      "progress": {
        "completed": 2,
        "total": 4,
        "percentage": 50
      },
      "cost_so_far_usd": 0.032
    }
  ],
  "total_count": 1,
  "has_more": false
}
```

---

#### Test Case OBS-02: Get Workflow Status with Activities

**MCP Tool Call**:
```json
{
  "tool": "get_workflow_status",
  "arguments": {
    "workflow_id": "019353a1-b0c1-7000-8000-000000000001",
    "include_activities": true
  }
}
```

**Expected Response**:
```json
{
  "workflow_id": "019353a1-b0c1-7000-8000-000000000001",
  "definition_name": "research_deep_dive",
  "status": "running",
  "input": {
    "question": "What is the relationship between virtue and happiness?",
    "user_id": "550e8400-e29b-41d4-a716-446655440000"
  },
  "activities": [
    {
      "key": "embed_query",
      "status": "completed",
      "started_at": "2026-01-30T10:30:01Z",
      "completed_at": "2026-01-30T10:30:02Z",
      "duration_ms": 1200,
      "cost_usd": 0.001
    },
    {
      "key": "search_passages",
      "status": "completed",
      "started_at": "2026-01-30T10:30:02Z",
      "completed_at": "2026-01-30T10:30:03Z",
      "duration_ms": 450,
      "cost_usd": 0.000,
      "output_summary": "Found 15 passages"
    },
    {
      "key": "extract_arguments",
      "status": "running",
      "started_at": "2026-01-30T10:30:03Z",
      "progress": "Processing with claude-opus-4-5"
    },
    {
      "key": "generate_analysis",
      "status": "pending",
      "depends_on": ["extract_arguments"]
    }
  ],
  "cost_breakdown": {
    "embedding": 0.001,
    "database": 0.000,
    "llm": 0.031,
    "total_usd": 0.032
  }
}
```

---

#### Test Case OBS-03: Get Completed Activity Report

**MCP Tool Call**:
```json
{
  "tool": "get_activity_output",
  "arguments": {
    "workflow_id": "019353a1-b0c1-7000-8000-000000000001",
    "activity_key": "search_passages"
  }
}
```

**Expected Response**:
```json
{
  "activity_key": "search_passages",
  "status": "completed",
  "output": {
    "rows": [
      {
        "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
        "content": "Aristotle argues that eudaimonia is the highest good...",
        "page_start": 42,
        "title": "Nicomachean Ethics",
        "similarity": 0.89
      }
    ]
  },
  "metrics": {
    "rows_returned": 15,
    "query_time_ms": 45
  }
}
```

---

### 3.2 Cost Tracking Queries

#### Test Case COST-01: Get Workflow Cost Breakdown

**MCP Tool Call**:
```json
{
  "tool": "get_workflow_cost",
  "arguments": {
    "workflow_id": "019353a1-b0c1-7000-8000-000000000001"
  }
}
```

**Expected Response**:
```json
{
  "workflow_id": "019353a1-b0c1-7000-8000-000000000001",
  "status": "completed",
  "total_cost_usd": 0.187,
  "budget_limit_usd": 0.50,
  "budget_remaining_usd": 0.313,
  "cost_by_activity": [
    {
      "activity_key": "embed_query",
      "activity_type": "embedding",
      "model": "google/gemini-embedding-001",
      "cost_usd": 0.001,
      "tokens": {"input": 45, "output": 0}
    },
    {
      "activity_key": "search_passages",
      "activity_type": "postgres_query",
      "cost_usd": 0.000,
      "metrics": {"rows": 15, "query_ms": 45}
    },
    {
      "activity_key": "extract_arguments",
      "activity_type": "llm_prompt",
      "model": "anthropic/claude-opus-4-5",
      "cost_usd": 0.086,
      "tokens": {"input": 12500, "output": 2100}
    },
    {
      "activity_key": "generate_analysis",
      "activity_type": "llm_prompt",
      "model": "anthropic/claude-opus-4-5",
      "cost_usd": 0.100,
      "tokens": {"input": 15200, "output": 3800}
    }
  ],
  "cost_by_model": {
    "google/gemini-embedding-001": 0.001,
    "anthropic/claude-opus-4-5": 0.186
  }
}
```

---

#### Test Case COST-02: Estimate Workflow Cost Before Execution

**MCP Tool Call**:
```json
{
  "tool": "estimate_workflow_cost",
  "arguments": {
    "workflow_yaml": "name: research_deep_dive\n...",
    "input": {
      "question": "What is consciousness?",
      "user_id": "550e8400-e29b-41d4-a716-446655440000"
    }
  }
}
```

**Expected Response**:
```json
{
  "estimated_cost": {
    "min_usd": 0.12,
    "max_usd": 0.45,
    "expected_usd": 0.25
  },
  "assumptions": [
    "embed_query: 1 embedding call @ $0.001",
    "search_passages: Database query (negligible)",
    "extract_arguments: ~15K input tokens, ~2K output @ claude-opus-4-5 rates",
    "generate_analysis: ~18K input tokens, ~4K output @ claude-opus-4-5 rates"
  ],
  "warnings": [
    "Actual cost depends on passage content length",
    "LLM output length varies with question complexity"
  ],
  "budget_recommendation": 0.50
}
```

---

### 3.3 Human-in-the-Loop Monitoring

#### Test Case HITL-01: Get Workflows Waiting for Signal

**MCP Tool Call**:
```json
{
  "tool": "list_workflows",
  "arguments": {
    "status": "waiting_for_signal",
    "limit": 10
  }
}
```

**Expected Response**:
```json
{
  "workflows": [
    {
      "workflow_id": "019353a1-b0c1-7000-8000-000000000002",
      "definition_name": "content_approval_pipeline",
      "status": "waiting_for_signal",
      "waiting_activity": "human_review",
      "signal_name": "approval_decision",
      "waiting_since": "2026-01-30T09:15:00Z",
      "timeout_at": "2026-01-30T17:15:00Z",
      "context": {
        "content_type": "blog_post",
        "title": "AI in Healthcare",
        "preview_url": "https://preview.example.com/abc123"
      }
    }
  ],
  "total_count": 1
}
```

---

## 4. Visual Diagram Test Cases

### 4.1 Workflow Diagram Generation

#### Test Case VIZ-01: Render Sequential Workflow Diagram

**MCP Tool Call**:
```json
{
  "tool": "render_workflow_diagram",
  "arguments": {
    "workflow_id": "019353a1-b0c1-7000-8000-000000000001",
    "format": "mermaid",
    "include_status": true
  }
}
```

**Expected Response**:
```mermaid
flowchart TB
    subgraph Status["Workflow: research_deep_dive"]
        direction TB
        embed_query["✅ embed_query<br/>embedding<br/>0.001 USD"]
        search_passages["✅ search_passages<br/>postgres_query<br/>15 rows"]
        extract_arguments["🔄 extract_arguments<br/>llm_prompt<br/>Processing..."]
        generate_analysis["⏳ generate_analysis<br/>llm_prompt<br/>Pending"]
    end

    embed_query --> search_passages
    search_passages --> extract_arguments
    extract_arguments --> generate_analysis

    style embed_query fill:#90EE90
    style search_passages fill:#90EE90
    style extract_arguments fill:#FFD700
    style generate_analysis fill:#D3D3D3
```

---

#### Test Case VIZ-02: Render Parallel Workflow Diagram

**MCP Tool Call**:
```json
{
  "tool": "render_workflow_diagram",
  "arguments": {
    "workflow_definition": "philosophical_comparison",
    "format": "mermaid"
  }
}
```

**Expected Response**:
```mermaid
flowchart TB
    embed_query[embed_query<br/>embedding]

    subgraph Parallel["Parallel Searches"]
        search_kant[search_kant_passages<br/>postgres_query]
        search_hume[search_hume_passages<br/>postgres_query]
        search_aristotle[search_aristotle_passages<br/>postgres_query]
    end

    comparative_analysis[comparative_analysis<br/>llm_prompt]

    embed_query --> search_kant
    embed_query --> search_hume
    embed_query --> search_aristotle

    search_kant --> comparative_analysis
    search_hume --> comparative_analysis
    search_aristotle --> comparative_analysis
```

---

#### Test Case VIZ-03: Render Loop Workflow Diagram

**MCP Tool Call**:
```json
{
  "tool": "render_workflow_diagram",
  "arguments": {
    "workflow_definition": "simple_agentic_research",
    "format": "mermaid"
  }
}
```

**Expected Response**:
```mermaid
flowchart TB
    initialize[initialize<br/>llm_prompt<br/>Create research plan]
    perform_search[perform_search<br/>llm_prompt<br/>iteration_scoped: true]
    evaluate[evaluate<br/>llm_prompt<br/>Check if sufficient]
    compile_report[compile_report<br/>llm_prompt<br/>Final synthesis]

    initialize --> perform_search
    perform_search --> evaluate
    evaluate -->|"CONTINUE"| perform_search
    evaluate -->|"SUFFICIENT"| compile_report

    style perform_search stroke:#FF6B6B,stroke-width:3px
    style evaluate stroke:#FF6B6B,stroke-width:3px
```

---

## 5. Integration Test Scenarios

### 5.1 Full Workflow Lifecycle Test

```mermaid
sequenceDiagram
    participant Agent as AI Agent
    participant MCP as MCP Server
    participant API as Kruxia Flow API
    participant Orch as Orchestrator
    participant Worker as Worker

    Note over Agent,Worker: Test Scenario: Create and Monitor Research Workflow

    Agent->>MCP: validate_workflow(yaml)
    MCP->>API: POST /api/v1/workflow_definitions (dry-run)
    API-->>MCP: Validation result
    MCP-->>Agent: {valid: true, warnings: [...]}

    Agent->>MCP: submit_workflow(yaml, input)
    MCP->>API: POST /api/v1/workflow_definitions
    API-->>MCP: {definition_id}
    MCP->>API: POST /api/v1/workflows
    API-->>MCP: {workflow_id}
    MCP-->>Agent: {workflow_id, status: "pending"}

    loop Monitor Progress
        Agent->>MCP: get_workflow_status(workflow_id)
        MCP->>API: GET /api/v1/workflows/:id
        API-->>MCP: {status, activities, cost}
        MCP-->>Agent: {status, progress, cost_so_far}
    end

    Orch->>Worker: Execute activities
    Worker-->>Orch: Activity results

    Agent->>MCP: get_workflow_status(workflow_id)
    MCP-->>Agent: {status: "completed", output}

    Agent->>MCP: render_workflow_diagram(workflow_id)
    MCP-->>Agent: Mermaid diagram with completion status
```

---

### 5.2 Error Handling Test Cases

| Test ID | Scenario                              | Expected Behavior                              |
|---------|---------------------------------------|------------------------------------------------|
| ERR-01  | Invalid YAML syntax                   | Return parse error with line number            |
| ERR-02  | Unknown activity type                 | Return validation error listing valid types    |
| ERR-03  | Circular dependency                   | Return cycle detection error                   |
| ERR-04  | Budget exceeded during execution      | Return budget_exceeded error with cost details |
| ERR-05  | Referenced workflow not found         | Return 404 with helpful message                |
| ERR-06  | Authentication token expired          | Return 401 with refresh instructions           |
| ERR-07  | Rate limit exceeded                   | Return 429 with retry-after header             |
| ERR-08  | Activity timeout                      | Return timeout error with activity context     |

---

## 6. Test Environment Configuration

### 6.1 Local Development Testing

```bash
# Start Kruxia Flow with test configuration
KRUXIAFLOW_DATABASE_URL="postgres://test:test@localhost:5432/kruxiaflow_test" \
KRUXIAFLOW_LOG_LEVEL="debug" \
kruxiaflow --api-only

# Start MCP server in development mode
cd mcp-server
python -m kruxiaflow_mcp --debug --port 8081

# Run MCP Inspector
npx @anthropic/mcp-inspector http://localhost:8081
```

### 6.2 CI/CD Test Configuration

```yaml
# .github/workflows/mcp-server-tests.yml
name: MCP Server Tests

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    services:
      postgres:
        image: postgres:17
        env:
          POSTGRES_PASSWORD: test
        options: >-
          --health-cmd pg_isready
          --health-interval 10s
          --health-timeout 5s
          --health-retries 5

    steps:
      - uses: actions/checkout@v4

      - name: Start Kruxia Flow
        run: |
          cargo build --release
          ./target/release/kruxiaflow &

      - name: Run Protocol Compliance Tests
        run: |
          cd mcp-server
          pytest tests/protocol/ -v

      - name: Run Integration Tests
        run: |
          cd mcp-server
          pytest tests/integration/ -v

      - name: Run MCP Inspector Validation
        run: |
          npx @anthropic/mcp-inspector --validate http://localhost:8081
```

---

## 7. Manual Testing Checklist

### 7.1 Claude Code Integration Testing

- [ ] MCP server appears in Claude Code's available tools
- [ ] `list_workflow_definitions` returns results in Claude Code
- [ ] Claude can create a workflow from natural language description
- [ ] Claude can submit and monitor workflow execution
- [ ] Claude correctly interprets workflow completion status
- [ ] Claude can generate and display Mermaid diagrams
- [ ] Error messages from MCP server are helpful to Claude
- [ ] Cost information is accurately reported to Claude

### 7.2 IDE Extension Testing

- [ ] VS Code extension connects to MCP server
- [ ] Workflow templates appear in command palette
- [ ] Workflow status appears in status bar
- [ ] Activity output viewable in output panel
- [ ] Mermaid diagrams render in preview pane

### 7.3 Verbose Logging Review

Log files to examine for each test run:

```
logs/
├── mcp-server.log          # MCP protocol messages
├── mcp-server-debug.log    # Detailed tool execution
├── kruxiaflow-api.log      # API requests from MCP
└── kruxiaflow-orch.log     # Workflow execution details
```

**Key log patterns to verify**:
- `MCP_TOOL_CALL`: Every tool invocation with arguments
- `API_REQUEST`: Every API call made by MCP server
- `VALIDATION_ERROR`: Schema validation failures
- `COST_TRACKING`: Cost calculations and budget checks
- `WORKFLOW_STATE`: State transitions during execution

---

## 8. Test Data Fixtures

### 8.1 Sample Workflow Definitions

Located in `tests/fixtures/workflows/`:
- `simple-http.yaml`: Basic HTTP request workflow
- `llm-with-fallback.yaml`: LLM with model fallback
- `parallel-fan-out.yaml`: Parallel execution pattern
- `loop-until-done.yaml`: Iterative workflow with loop
- `human-approval.yaml`: Human-in-the-loop workflow
- `full-research.yaml`: Complete researcher-style workflow

### 8.2 Mock API Responses

Located in `tests/fixtures/mocks/`:
- `llm-responses.json`: Recorded LLM outputs
- `embedding-responses.json`: Recorded embeddings
- `postgres-results.json`: Sample database results

---

## 9. Post-Test Data Analysis

### 9.1 Test Data Collection Pipeline

After each test run, collect and aggregate data for analysis:

```mermaid
flowchart LR
    subgraph Sources["Data Sources"]
        Logs[MCP Server Logs]
        Audit[mcp_audit_log Table]
        Metrics[Prometheus Metrics]
        Artifacts[Generated Workflows]
    end

    subgraph ETL["Collection & Transform"]
        Extract[Extract Test Run Data]
        Enrich[Enrich with Metadata]
        Normalize[Normalize Formats]
    end

    subgraph Storage["Analytics Storage"]
        TestDB[(test_analytics DB)]
        Files[Artifact Storage]
    end

    subgraph Analysis["Analysis"]
        Stats[Statistical Analysis]
        Trends[Trend Detection]
        Regression[Regression Detection]
        Reports[Test Reports]
    end

    Sources --> ETL --> Storage --> Analysis
```

**Data to Collect Per Test Run**:

| Data Type                   | Source                | Purpose                                |
|-----------------------------|----------------------|----------------------------------------|
| Tool invocation logs        | mcp_audit_log        | Success rates, latency analysis        |
| Generated workflow YAML     | Artifact storage     | Consistency analysis                   |
| Natural language prompts    | mcp_audit_log        | Prompt effectiveness measurement       |
| Validation errors           | MCP server logs      | Error pattern analysis                 |
| Cost data                   | mcp_audit_log        | Cost estimation accuracy               |
| Prometheus metrics snapshot | Metrics endpoint     | Performance regression detection       |

### 9.2 Prompt and Workflow Logging

**Critical Requirement**: All AI-generated content must be logged for analysis.

```sql
-- Query to extract all prompt-to-workflow mappings
SELECT
    timestamp,
    agent_id,
    agent_type,
    session_id,
    natural_language_prompt,
    generated_workflow_yaml,
    validation_passed,
    validation_warnings,
    workflow_id,
    actual_cost_usd
FROM mcp_audit_log
WHERE tool_name = 'submit_workflow'
  AND timestamp >= NOW() - INTERVAL '24 hours'
ORDER BY timestamp DESC;
```

**Logging Requirements**:

| Field                       | Logged | Purpose                                             |
|-----------------------------|--------|-----------------------------------------------------|
| Original user prompt        | Yes    | Understand what user asked for                      |
| Agent's interpretation      | Yes    | Track how agent understood the request              |
| Generated YAML (pre-valid.) | Yes    | Analyze raw output before validation                |
| Validation warnings         | Yes    | Identify common generation mistakes                 |
| Final submitted YAML        | Yes    | Track what actually executed                        |
| Execution results           | Yes    | Correlate prompts to outcomes                       |

### 9.3 Analysis Queries

#### Query 1: Workflow Generation Consistency

Measure how consistently agents generate workflows for similar prompts:

```sql
-- Find prompts that were run multiple times and compare outputs
WITH prompt_groups AS (
    SELECT
        -- Normalize prompt (lowercase, trim whitespace)
        LOWER(TRIM(natural_language_prompt)) as normalized_prompt,
        COUNT(*) as run_count,
        COUNT(DISTINCT generated_workflow_yaml) as unique_outputs,
        COUNT(DISTINCT generated_workflow_yaml)::float / COUNT(*)::float as variance_ratio
    FROM mcp_audit_log
    WHERE tool_name = 'submit_workflow'
      AND natural_language_prompt IS NOT NULL
    GROUP BY LOWER(TRIM(natural_language_prompt))
    HAVING COUNT(*) > 1
)
SELECT
    normalized_prompt,
    run_count,
    unique_outputs,
    ROUND(variance_ratio * 100, 1) as variance_pct,
    CASE
        WHEN variance_ratio < 0.2 THEN 'Highly Consistent'
        WHEN variance_ratio < 0.5 THEN 'Moderately Consistent'
        ELSE 'Low Consistency'
    END as consistency_rating
FROM prompt_groups
ORDER BY run_count DESC;
```

#### Query 2: Cost Estimation Accuracy

Compare estimated vs actual costs:

```sql
SELECT
    workflow_definition_name,
    COUNT(*) as executions,
    AVG(estimated_cost_usd) as avg_estimated,
    AVG(actual_cost_usd) as avg_actual,
    AVG(ABS(estimated_cost_usd - actual_cost_usd)) as avg_absolute_error,
    AVG(ABS(estimated_cost_usd - actual_cost_usd) / NULLIF(actual_cost_usd, 0) * 100) as avg_pct_error,
    PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY ABS(estimated_cost_usd - actual_cost_usd)) as p95_error
FROM mcp_audit_log
WHERE actual_cost_usd IS NOT NULL
  AND estimated_cost_usd IS NOT NULL
GROUP BY workflow_definition_name
ORDER BY avg_pct_error DESC;
```

#### Query 3: Validation Error Frequency

Identify common workflow generation mistakes:

```sql
SELECT
    jsonb_array_elements_text(validation_warnings) as warning_type,
    COUNT(*) as occurrence_count,
    COUNT(DISTINCT agent_id) as affected_agents,
    ROUND(COUNT(*)::numeric / (SELECT COUNT(*) FROM mcp_audit_log WHERE validation_warnings IS NOT NULL) * 100, 1) as pct_of_warnings
FROM mcp_audit_log
WHERE validation_warnings IS NOT NULL
  AND jsonb_array_length(validation_warnings) > 0
GROUP BY jsonb_array_elements_text(validation_warnings)
ORDER BY occurrence_count DESC
LIMIT 20;
```

#### Query 4: Agent Performance Comparison

Compare different agent types:

```sql
SELECT
    agent_type,
    COUNT(*) as total_tool_calls,
    SUM(CASE WHEN result_status = 'success' THEN 1 ELSE 0 END) as successful,
    ROUND(AVG(CASE WHEN result_status = 'success' THEN 1.0 ELSE 0.0 END) * 100, 1) as success_rate_pct,
    ROUND(AVG(duration_ms), 1) as avg_latency_ms,
    PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY duration_ms) as p95_latency_ms,
    SUM(actual_cost_usd) as total_cost_usd
FROM mcp_audit_log
WHERE timestamp >= NOW() - INTERVAL '7 days'
GROUP BY agent_type
ORDER BY total_tool_calls DESC;
```

#### Query 5: Prompt-to-Outcome Correlation

Analyze which prompt patterns lead to successful workflows:

```sql
WITH prompt_outcomes AS (
    SELECT
        a.natural_language_prompt,
        a.workflow_id,
        a.validation_passed,
        w.status as workflow_status,
        w.total_cost_usd
    FROM mcp_audit_log a
    LEFT JOIN workflows w ON a.workflow_id = w.id
    WHERE a.tool_name = 'submit_workflow'
      AND a.natural_language_prompt IS NOT NULL
)
SELECT
    CASE
        WHEN natural_language_prompt ILIKE '%research%' THEN 'Research'
        WHEN natural_language_prompt ILIKE '%email%' THEN 'Email/Notification'
        WHEN natural_language_prompt ILIKE '%data%' OR natural_language_prompt ILIKE '%fetch%' THEN 'Data Processing'
        WHEN natural_language_prompt ILIKE '%monitor%' OR natural_language_prompt ILIKE '%alert%' THEN 'Monitoring'
        ELSE 'Other'
    END as prompt_category,
    COUNT(*) as total,
    SUM(CASE WHEN validation_passed THEN 1 ELSE 0 END) as valid_workflows,
    SUM(CASE WHEN workflow_status = 'completed' THEN 1 ELSE 0 END) as completed,
    ROUND(AVG(total_cost_usd), 4) as avg_cost_usd
FROM prompt_outcomes
GROUP BY 1
ORDER BY total DESC;
```

### 9.4 Regression Detection

Automatically detect regressions between test runs:

```python
#!/usr/bin/env python3
"""Detect regressions between test runs."""

import sys
from dataclasses import dataclass
from typing import Optional

@dataclass
class TestRunMetrics:
    run_id: str
    timestamp: str
    tool_success_rate: float
    avg_latency_ms: float
    p95_latency_ms: float
    validation_error_rate: float
    cost_estimation_error_pct: float

def detect_regressions(
    baseline: TestRunMetrics,
    current: TestRunMetrics,
    thresholds: dict
) -> list[dict]:
    """Compare current run against baseline and detect regressions."""

    regressions = []

    # Success rate regression
    if baseline.tool_success_rate - current.tool_success_rate > thresholds.get('success_rate_drop', 0.05):
        regressions.append({
            'metric': 'tool_success_rate',
            'severity': 'critical',
            'baseline': baseline.tool_success_rate,
            'current': current.tool_success_rate,
            'threshold': thresholds.get('success_rate_drop', 0.05),
            'message': f"Success rate dropped from {baseline.tool_success_rate:.1%} to {current.tool_success_rate:.1%}"
        })

    # Latency regression
    latency_increase = (current.p95_latency_ms - baseline.p95_latency_ms) / baseline.p95_latency_ms
    if latency_increase > thresholds.get('latency_increase_pct', 0.20):
        regressions.append({
            'metric': 'p95_latency_ms',
            'severity': 'warning',
            'baseline': baseline.p95_latency_ms,
            'current': current.p95_latency_ms,
            'threshold': thresholds.get('latency_increase_pct', 0.20),
            'message': f"P95 latency increased by {latency_increase:.0%} ({baseline.p95_latency_ms:.0f}ms → {current.p95_latency_ms:.0f}ms)"
        })

    # Validation error rate regression
    if current.validation_error_rate - baseline.validation_error_rate > thresholds.get('validation_error_increase', 0.10):
        regressions.append({
            'metric': 'validation_error_rate',
            'severity': 'warning',
            'baseline': baseline.validation_error_rate,
            'current': current.validation_error_rate,
            'threshold': thresholds.get('validation_error_increase', 0.10),
            'message': f"Validation error rate increased from {baseline.validation_error_rate:.1%} to {current.validation_error_rate:.1%}"
        })

    # Cost estimation accuracy regression
    if current.cost_estimation_error_pct - baseline.cost_estimation_error_pct > thresholds.get('cost_error_increase', 0.15):
        regressions.append({
            'metric': 'cost_estimation_error_pct',
            'severity': 'info',
            'baseline': baseline.cost_estimation_error_pct,
            'current': current.cost_estimation_error_pct,
            'threshold': thresholds.get('cost_error_increase', 0.15),
            'message': f"Cost estimation error increased from {baseline.cost_estimation_error_pct:.1%} to {current.cost_estimation_error_pct:.1%}"
        })

    return regressions

# Default regression thresholds
DEFAULT_THRESHOLDS = {
    'success_rate_drop': 0.05,      # 5% drop triggers alert
    'latency_increase_pct': 0.20,   # 20% increase triggers alert
    'validation_error_increase': 0.10,  # 10% increase triggers alert
    'cost_error_increase': 0.15,    # 15% increase triggers alert
}
```

### 9.5 Test Report Generation

Generate comprehensive test reports after each run:

```markdown
# MCP Server Test Report

**Run ID**: {{run_id}}
**Timestamp**: {{timestamp}}
**Duration**: {{duration_minutes}} minutes
**Environment**: {{environment}}

## Summary

| Metric                    | Value     | Status |
|---------------------------|-----------|--------|
| Total Tool Invocations    | {{total}} | -      |
| Success Rate              | {{success_rate}}% | {{success_status}} |
| Avg Latency               | {{avg_latency}}ms | {{latency_status}} |
| P95 Latency               | {{p95_latency}}ms | {{p95_status}} |
| Validation Error Rate     | {{validation_error_rate}}% | {{validation_status}} |
| Cost Estimation Error     | {{cost_error}}% | {{cost_status}} |

## Regressions Detected

{{#if regressions}}
| Metric | Severity | Baseline | Current | Change |
|--------|----------|----------|---------|--------|
{{#each regressions}}
| {{metric}} | {{severity}} | {{baseline}} | {{current}} | {{change}} |
{{/each}}
{{else}}
✅ No regressions detected.
{{/if}}

## Tool Invocation Breakdown

| Tool Name                   | Calls | Success | Avg Latency |
|-----------------------------|-------|---------|-------------|
{{#each tools}}
| {{name}} | {{calls}} | {{success_rate}}% | {{avg_latency}}ms |
{{/each}}

## Workflow Generation Analysis

- **Total Workflows Generated**: {{workflows_generated}}
- **Unique Prompts**: {{unique_prompts}}
- **Generation Consistency**: {{consistency_rating}}

## Cost Analysis

- **Total Estimated Cost**: ${{total_estimated}}
- **Total Actual Cost**: ${{total_actual}}
- **Estimation Accuracy**: {{estimation_accuracy}}%

## Recommendations

{{#each recommendations}}
- {{this}}
{{/each}}
```

### 9.6 AI-Assisted Log Analysis

Use Claude to analyze test logs and identify issues:

```python
#!/usr/bin/env python3
"""AI-assisted test log analysis."""

import anthropic
import json

def analyze_test_logs_with_ai(
    log_file: str,
    audit_data: list[dict],
    metrics: dict
) -> str:
    """Use Claude to analyze test results and provide insights."""

    client = anthropic.Anthropic()

    prompt = f"""Analyze this MCP server test run data and provide insights.

## Test Metrics Summary
{json.dumps(metrics, indent=2)}

## Sample Audit Log Entries (last 50)
{json.dumps(audit_data[-50:], indent=2)}

## Error Log Excerpts
{open(log_file).read()[-10000:]}  # Last 10KB

Please provide:
1. **Overall Assessment**: Is this test run healthy?
2. **Issues Identified**: List any problems found
3. **Pattern Analysis**: Are there recurring issues?
4. **Recommendations**: What should be fixed or improved?
5. **Regression Risk**: Based on the data, what's likely to regress?

Format your response as a structured report."""

    response = client.messages.create(
        model="claude-sonnet-4-5-20250929",
        max_tokens=2000,
        messages=[{"role": "user", "content": prompt}]
    )

    return response.content[0].text
```

---

## 10. Audit Log Verification Tests

### 10.1 Audit Completeness Tests

Verify that all required fields are logged:

| Test ID | Verification                              | Expected Result                        |
|---------|-------------------------------------------|----------------------------------------|
| AUD-01  | Every tool call has audit entry           | 100% coverage                          |
| AUD-02  | Agent ID present on all entries           | No NULL agent_ids                      |
| AUD-03  | Natural language prompts logged           | All submit_workflow calls have prompts |
| AUD-04  | Generated YAML captured                   | All workflows have YAML stored         |
| AUD-05  | Timestamps are accurate                   | Within 1 second of actual time         |
| AUD-06  | Session continuity maintained             | Session IDs persist across calls       |

### 10.2 Audit Query Tests

```sql
-- AUD-01: Verify complete audit coverage
SELECT
    tool_name,
    COUNT(*) as audit_entries,
    COUNT(*) FILTER (WHERE agent_id IS NULL) as missing_agent_id,
    COUNT(*) FILTER (WHERE timestamp IS NULL) as missing_timestamp
FROM mcp_audit_log
WHERE timestamp >= NOW() - INTERVAL '1 hour'
GROUP BY tool_name;

-- AUD-03: Verify prompts logged for workflow submissions
SELECT
    COUNT(*) as total_submissions,
    COUNT(*) FILTER (WHERE natural_language_prompt IS NOT NULL) as with_prompt,
    COUNT(*) FILTER (WHERE natural_language_prompt IS NULL) as missing_prompt
FROM mcp_audit_log
WHERE tool_name = 'submit_workflow';

-- AUD-04: Verify YAML captured
SELECT
    COUNT(*) as total_submissions,
    COUNT(*) FILTER (WHERE generated_workflow_yaml IS NOT NULL) as with_yaml,
    AVG(LENGTH(generated_workflow_yaml)) as avg_yaml_length
FROM mcp_audit_log
WHERE tool_name IN ('submit_workflow', 'validate_workflow');
```

---

## Appendix A: MCP Tool Schema Reference

```json
{
  "tools": [
    {
      "name": "list_workflow_definitions",
      "description": "List available workflow definitions",
      "inputSchema": {
        "type": "object",
        "properties": {
          "namespace": {"type": "string"},
          "limit": {"type": "integer", "default": 20}
        }
      }
    },
    {
      "name": "get_workflow_status",
      "description": "Get current status of a workflow",
      "inputSchema": {
        "type": "object",
        "properties": {
          "workflow_id": {"type": "string", "format": "uuid"},
          "include_activities": {"type": "boolean", "default": false}
        },
        "required": ["workflow_id"]
      }
    },
    {
      "name": "render_workflow_diagram",
      "description": "Generate Mermaid diagram of workflow",
      "inputSchema": {
        "type": "object",
        "properties": {
          "workflow_id": {"type": "string"},
          "workflow_definition": {"type": "string"},
          "format": {"type": "string", "enum": ["mermaid"], "default": "mermaid"},
          "include_status": {"type": "boolean", "default": false}
        }
      }
    }
  ]
}
```
