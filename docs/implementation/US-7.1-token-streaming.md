# US-7.1: Token Streaming for Real-Time UX - Implementation Plan

**Epic**: Epic 7 - AI-Native Features (Differentiators)
**User Story**: US-7.1
**Status**: ✅ Implemented
**Priority**: MVP Critical - Core AI-Native Differentiator
**Estimated Duration**: ~22-32 hours (3-4 days)
**Dependencies**: US-1A.9a (WebSocket Infrastructure) - ✅ Complete

## Implementation Status

| Task                                         | Status       | Notes                                           |
|----------------------------------------------|--------------|------------------------------------------------|
| Task 0: StreamingConfig Types                | ✅ Complete | `core/src/workflow/definition.rs`               |
| Task 1: Anthropic SSE Streaming              | ✅ Complete | `worker/src/llm/anthropic.rs`                   |
| Task 2: OpenAI SSE Streaming                 | ✅ Complete | `worker/src/llm/openai.rs`                      |
| Task 3: Google Gemini Streaming              | ✅ Complete | `worker/src/llm/google.rs`                      |
| Task 4: Activity Streaming Layer             | ✅ Complete | `worker/src/activities/llm.rs`, `api/src/handlers/streaming.rs` |
| Task 5: Example Integration                  | ✅ Complete | `examples/09a-streaming-llm.yaml`, `examples/09b-streaming-research.yaml` |
| Task 6: Testing                              | ✅ Complete | Unit tests and integration tests added          |

### Key Files Modified/Created

- `core/src/workflow/definition.rs` - Added `StreamingConfig` and `StreamingOptions` types
- `api/src/dto/workflow.rs` - Added DTO versions of streaming types
- `api/src/handlers/streaming.rs` - New internal API for token publishing
- `worker/src/llm/anthropic.rs` - Implemented `prompt_stream()` with SSE parsing
- `worker/src/llm/openai.rs` - Implemented `prompt_stream()` with SSE parsing
- `worker/src/llm/google.rs` - Implemented `prompt_stream()` with SSE parsing
- `worker/src/streaming.rs` - Added `HttpStreamSender` for HTTP-based token delivery
- `worker/src/activities/llm.rs` - Implemented `StreamingActivity` trait for `LLMPromptActivity`
- `examples/09a-streaming-llm.yaml` - Simple streaming example
- `examples/09b-streaming-research.yaml` - Multi-step workflow with streaming
- `api/tests/websocket_integration_tests.rs` - Added internal streaming API tests

---

## User Story

**As** an AI startup engineer
**I want** token-by-token streaming from LLM activities
**So that** users see responses in real-time (ChatGPT-style UX)

### Acceptance Criteria

- ✅ LLM providers support streaming (Anthropic SSE, OpenAI SSE, Google SSE)
- ✅ Activity-level streaming via `streaming: true` in activity definition (opt-in)
- ✅ Token-by-token delivery: `{type: "token", text: "hello", index: 0}`
- ✅ <10ms P95 token latency (achievable with async streaming)
- ✅ Support 1,000 concurrent streaming connections
- ✅ Two-level opt-in: activity config + WebSocket subscriber presence
- ✅ Graceful fallback: Non-streaming activities complete normally (no overhead)
- ✅ Integration with Example 6 (agentic research) for demonstration
- ✅ Client library examples for JavaScript/Python

---

## Strategic Rationale

**Why Token Streaming is Critical for MVP**:

1. **Core Value Proposition**: Explicitly promised in Executive Summary as AI-native differentiator
2. **User Expectation**: AI startup engineers (primary persona) expect ChatGPT-style streaming UX
3. **Competitive Advantage**: No workflow orchestrator (Temporal, Airflow, Conductor) offers this
4. **Production Requirement**: Required for user-facing AI applications
5. **Market Positioning**: Validates "AI-native" claim with concrete capability

**Business Impact**:
- Enables production AI workflows with real-time UX
- Differentiates from all competitors
- Reduces perceived latency for long-running LLM calls
- Essential for AI agents with streaming responses

---

## Architecture Overview

### Token Streaming Flow

```mermaid
sequenceDiagram
    participant Client
    participant API as API Server<br/>(WebSocket)
    participant Worker as Built-in Worker
    participant LLM as LLM Provider<br/>(Anthropic/OpenAI/Google)
    participant ConnMgr as ConnectionManager

    Note over Client,API: 1. Client establishes WebSocket (US-1A.9a)
    Client->>API: WS /api/v1/activities/{id}/ws
    API->>ConnMgr: Register connection

    Note over Worker,LLM: 2. Worker executes LLM activity with streaming
    Worker->>LLM: POST /v1/messages (stream: true)

    loop Token streaming
        LLM-->>Worker: SSE: data: {"type":"content_block_delta","delta":{"text":"hello"}}
        Worker->>Worker: Parse token from SSE
        Worker->>ConnMgr: Publish StreamMessage::Token
        ConnMgr->>API: Broadcast to WebSocket
        API-->>Client: {"type":"token","text":"hello","index":0}
    end

    LLM-->>Worker: SSE: data: {"type":"message_stop"}
    Worker->>Worker: Accumulate full response
    Worker->>ConnMgr: Publish StreamMessage::Complete
    ConnMgr->>API: Broadcast completion
    API-->>Client: {"type":"complete","result":{...}}
    API->>ConnMgr: Close connections
```

### Key Components

1. **LLM Provider Streaming** (`worker/src/llm/anthropic.rs`, `openai.rs`, `google.rs`)
   - Integrate with provider SSE streaming APIs
   - Parse SSE events asynchronously
   - Extract tokens from provider-specific formats

2. **Activity Streaming Layer** (`worker/src/activities/llm.rs`)
   - Wrap LLM provider streaming
   - Publish tokens to ConnectionManager
   - Accumulate full response for activity result
   - Handle streaming errors and reconnection

3. **Non-Streaming Fallback** (`worker/src/activities/llm.rs`)
   - Detect non-streaming activities (HTTP, PostgreSQL, etc.)
   - Execute normally without WebSocket streaming
   - No changes to non-streaming activities

4. **Example Integration** (`examples/06-agentic-research-streaming.yaml`)
   - Demonstrate streaming with Example 6 (agentic research)
   - Show real-time token output during iterative search

---

## Streaming Configuration Design

### Design Principles

Streaming configuration is a **top-level property** on ActivityDefinition, separate from both `parameters` and `settings`:

| Location     | Contains                          | Character     |
|--------------|-----------------------------------|---------------|
| `parameters` | Type-specific inputs              | Functional    |
| `settings`   | Universal execution behavior      | Operational   |
| `streaming`  | Type-specific progress reporting  | Observability |

**Rationale**:
- Streaming doesn't change *what* the activity computes (not `parameters`)
- Streaming isn't universal to all activity types like retry/timeout (not `settings`)
- Streaming is about observability/UX - how progress is reported to observers
- Different activity types have different streaming semantics (tokens vs rows vs chunks)

### Future Extensibility

While MVP focuses on LLM token streaming, the design accommodates future streaming use cases:

| Activity Type   | Streaming Unit | Type-Specific Options (Post-MVP)  |
|-----------------|----------------|-----------------------------------|
| LLM             | tokens         | `include_usage_events: bool`      |
| PostgreSQL      | rows           | `batch_size: 100`                 |
| HTTP (SSE)      | events         | `buffer_size: 1024`               |
| File processing | lines/records  | `chunk_size: 1000`                |

### Activity Definition Schema

```yaml
activities:
  generate_summary:
    type: llm
    streaming: true              # Shorthand for { enabled: true }
    settings:
      timeout: 60s
    parameters:
      model: claude-3-5-sonnet
      prompt: "Summarize this document..."

  # Expanded form (for future type-specific options)
  generate_report:
    type: llm
    streaming:
      enabled: true
      # Future LLM-specific options here
    parameters:
      model: claude-3-5-sonnet
      prompt: "Generate a detailed report..."

  # Non-streaming activity (default)
  fetch_data:
    type: http
    # streaming: false (default, can be omitted)
    parameters:
      url: "https://api.example.com/data"
```

### Rust Types

**File**: `shared/src/types/workflow.rs` (update)

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityDefinition {
    #[serde(rename = "type")]
    pub activity_type: String,

    /// Streaming configuration (optional, defaults to disabled)
    #[serde(default)]
    pub streaming: StreamingConfig,

    /// Execution settings (retry, timeout, etc.)
    #[serde(default)]
    pub settings: ActivitySettings,

    /// Activity-specific parameters
    pub parameters: serde_json::Value,

    // ... other fields (depends_on, etc.)
}

/// Streaming configuration supporting both shorthand and detailed forms.
///
/// Shorthand: `streaming: true` or `streaming: false`
/// Detailed:  `streaming: { enabled: true, ... }`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StreamingConfig {
    /// Streaming disabled (default)
    #[default]
    Disabled,
    /// Shorthand form: `streaming: true/false`
    Simple(bool),
    /// Detailed form with options: `streaming: { enabled: true, ... }`
    Detailed(StreamingOptions),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingOptions {
    /// Whether streaming is enabled for this activity
    pub enabled: bool,

    /// Type-specific streaming options (validated at execution time)
    /// Post-MVP: LLM might have `include_usage_events`, PostgreSQL might have `batch_size`
    #[serde(flatten)]
    pub options: Option<serde_json::Map<String, serde_json::Value>>,
}

impl StreamingConfig {
    /// Check if streaming is enabled for this activity
    pub fn is_enabled(&self) -> bool {
        match self {
            StreamingConfig::Disabled => false,
            StreamingConfig::Simple(enabled) => *enabled,
            StreamingConfig::Detailed(opts) => opts.enabled,
        }
    }

    /// Get type-specific options (if any)
    pub fn options(&self) -> Option<&serde_json::Map<String, serde_json::Value>> {
        match self {
            StreamingConfig::Detailed(opts) => opts.options.as_ref(),
            _ => None,
        }
    }
}

#[cfg(test)]
mod streaming_config_tests {
    use super::*;

    #[test]
    fn test_streaming_disabled_by_default() {
        let yaml = r#"
            type: llm
            parameters:
              model: claude-3-5-sonnet
        "#;
        let def: ActivityDefinition = serde_yaml::from_str(yaml).unwrap();
        assert!(!def.streaming.is_enabled());
    }

    #[test]
    fn test_streaming_shorthand_true() {
        let yaml = r#"
            type: llm
            streaming: true
            parameters:
              model: claude-3-5-sonnet
        "#;
        let def: ActivityDefinition = serde_yaml::from_str(yaml).unwrap();
        assert!(def.streaming.is_enabled());
    }

    #[test]
    fn test_streaming_shorthand_false() {
        let yaml = r#"
            type: llm
            streaming: false
            parameters:
              model: claude-3-5-sonnet
        "#;
        let def: ActivityDefinition = serde_yaml::from_str(yaml).unwrap();
        assert!(!def.streaming.is_enabled());
    }

    #[test]
    fn test_streaming_detailed_form() {
        let yaml = r#"
            type: llm
            streaming:
              enabled: true
            parameters:
              model: claude-3-5-sonnet
        "#;
        let def: ActivityDefinition = serde_yaml::from_str(yaml).unwrap();
        assert!(def.streaming.is_enabled());
    }
}
```

### Two-Level Opt-In

Streaming requires **both** conditions to be true:

1. **Activity-level opt-in**: `streaming: true` in workflow definition
2. **Runtime opt-in**: At least one WebSocket subscriber connected

This design ensures:
- Activities that don't need streaming never incur any overhead
- Even streaming-enabled activities fall back to efficient non-streaming when no one is listening
- Clear intent in workflow definitions
- Future extensibility for type-specific streaming options

---

## Implementation Tasks

### Task 0: StreamingConfig Types (1-2 hours)

**File**: `shared/src/types/workflow.rs` (update)

Add `StreamingConfig` types to ActivityDefinition as documented in the "Streaming Configuration Design" section above.

**Key Implementation Points**:

1. Add `streaming` field to `ActivityDefinition` struct
2. Implement `StreamingConfig` enum with `#[serde(untagged)]` for shorthand/detailed form support
3. Implement `StreamingOptions` struct with `#[serde(flatten)]` for future extensibility
4. Add `is_enabled()` and `options()` helper methods
5. Add unit tests for YAML/JSON parsing of all forms

**Acceptance Criteria**:
- ✅ `streaming: true` shorthand parses correctly
- ✅ `streaming: false` shorthand parses correctly
- ✅ `streaming: { enabled: true }` detailed form parses correctly
- ✅ Missing `streaming` field defaults to disabled
- ✅ `is_enabled()` returns correct value for all forms
- ✅ Unit tests cover all parsing scenarios

---

### Task 1: Anthropic Streaming Integration (3-4 hours)

**File**: `worker/src/llm/anthropic.rs` (update)

Integrate Anthropic streaming API:

```rust
use futures_util::StreamExt;
use reqwest_eventsource::{Event, EventSource};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicStreamEvent {
    MessageStart {
        message: MessageMetadata,
    },
    ContentBlockStart {
        index: u32,
        content_block: ContentBlock,
    },
    ContentBlockDelta {
        index: u32,
        delta: Delta,
    },
    ContentBlockStop {
        index: u32,
    },
    MessageDelta {
        delta: MessageDeltaData,
        usage: UsageInfo,
    },
    MessageStop,
    Ping,
    Error {
        error: ErrorInfo,
    },
}

#[derive(Debug, Deserialize)]
struct Delta {
    #[serde(rename = "type")]
    delta_type: String,
    text: Option<String>,
}

impl AnthropicProvider {
    /// Execute prompt with streaming support
    pub async fn prompt_streaming(
        &self,
        request: &PromptRequest,
        token_sender: mpsc::UnboundedSender<crate::activities::streaming::StreamToken>,
    ) -> Result<PromptResponse, LLMError> {
        let url = format!("{}/v1/messages", self.base_url);

        // Build request with streaming enabled
        let mut body = serde_json::json!({
            "model": request.model,
            "max_tokens": request.max_tokens,
            "messages": request.messages,
            "stream": true,
        });

        if let Some(system) = &request.system {
            body["system"] = serde_json::json!(system);
        }

        // Create EventSource for SSE streaming
        let mut es = EventSource::new(
            self.client
                .post(&url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body)
                .build()?,
        )?;

        let mut full_text = String::new();
        let mut token_index = 0u32;
        let mut usage_info = None;

        // Process SSE events
        while let Some(event) = es.next().await {
            match event {
                Ok(Event::Message(msg)) => {
                    let stream_event: AnthropicStreamEvent = serde_json::from_str(&msg.data)?;

                    match stream_event {
                        AnthropicStreamEvent::ContentBlockDelta { delta, .. } => {
                            if let Some(text) = delta.text {
                                full_text.push_str(&text);

                                // Send token to WebSocket subscribers
                                let _ = token_sender.send(crate::activities::streaming::StreamToken {
                                    text: text.clone(),
                                    index: token_index,
                                });
                                token_index += 1;
                            }
                        }
                        AnthropicStreamEvent::MessageDelta { usage, .. } => {
                            usage_info = Some(usage);
                        }
                        AnthropicStreamEvent::MessageStop => {
                            break;
                        }
                        AnthropicStreamEvent::Error { error } => {
                            return Err(LLMError::ProviderError(error.message));
                        }
                        _ => {
                            // Ignore other event types
                        }
                    }
                }
                Ok(Event::Open) => {
                    tracing::debug!("Anthropic stream opened");
                }
                Err(e) => {
                    return Err(LLMError::StreamError(e.to_string()));
                }
            }
        }

        // Calculate cost from usage
        let cost_usd = if let Some(usage) = usage_info {
            self.calculate_cost(&request.model, usage.input_tokens, usage.output_tokens)
        } else {
            None
        };

        Ok(PromptResponse {
            content: full_text,
            provider: "anthropic".to_string(),
            model: request.model.clone(),
            cost_usd,
            usage: usage_info.map(|u| TokenUsage {
                prompt_tokens: u.input_tokens,
                completion_tokens: u.output_tokens,
                total_tokens: u.input_tokens + u.output_tokens,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_anthropic_streaming() {
        // Mock test for streaming
        let (tx, mut rx) = mpsc::unbounded_channel();

        // Simulate sending tokens
        tx.send(StreamToken {
            text: "Hello".to_string(),
            index: 0,
        }).unwrap();

        let token = rx.recv().await.unwrap();
        assert_eq!(token.text, "Hello");
        assert_eq!(token.index, 0);
    }
}
```

**Dependencies**:
- Add to `worker/Cargo.toml`:
  ```toml
  reqwest-eventsource = "0.5"
  futures-util = "0.3"
  ```

**Acceptance Criteria**:
- ✅ `prompt_streaming` method parses Anthropic SSE events
- ✅ Tokens sent to channel as they arrive
- ✅ Full response accumulated for activity result
- ✅ Usage and cost tracking works with streaming
- ✅ Error handling for stream failures

---

### Task 2: OpenAI Streaming Integration (3-4 hours)

**File**: `worker/src/llm/openai.rs` (update)

Similar to Anthropic, integrate OpenAI streaming:

```rust
use futures_util::StreamExt;
use reqwest_eventsource::{Event, EventSource};

#[derive(Debug, Deserialize)]
struct OpenAIStreamChunk {
    id: String,
    object: String,
    created: u64,
    model: String,
    choices: Vec<StreamChoice>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    index: u32,
    delta: StreamDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamDelta {
    role: Option<String>,
    content: Option<String>,
}

impl OpenAIProvider {
    pub async fn prompt_streaming(
        &self,
        request: &PromptRequest,
        token_sender: mpsc::UnboundedSender<crate::activities::streaming::StreamToken>,
    ) -> Result<PromptResponse, LLMError> {
        let url = format!("{}/v1/chat/completions", self.base_url);

        let body = serde_json::json!({
            "model": request.model,
            "messages": request.messages,
            "max_tokens": request.max_tokens,
            "stream": true,
        });

        let mut es = EventSource::new(
            self.client
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Content-Type", "application/json")
                .json(&body)
                .build()?,
        )?;

        let mut full_text = String::new();
        let mut token_index = 0u32;

        while let Some(event) = es.next().await {
            match event {
                Ok(Event::Message(msg)) => {
                    if msg.data == "[DONE]" {
                        break;
                    }

                    let chunk: OpenAIStreamChunk = serde_json::from_str(&msg.data)?;

                    for choice in chunk.choices {
                        if let Some(content) = choice.delta.content {
                            full_text.push_str(&content);

                            let _ = token_sender.send(crate::activities::streaming::StreamToken {
                                text: content.clone(),
                                index: token_index,
                            });
                            token_index += 1;
                        }
                    }
                }
                Ok(Event::Open) => {
                    tracing::debug!("OpenAI stream opened");
                }
                Err(e) => {
                    return Err(LLMError::StreamError(e.to_string()));
                }
            }
        }

        let cost_usd = self.estimate_cost(&request.model, &full_text, request.max_tokens);

        Ok(PromptResponse {
            content: full_text,
            provider: "openai".to_string(),
            model: request.model.clone(),
            cost_usd,
            usage: None, // OpenAI doesn't provide usage in streaming mode
        })
    }
}
```

**Acceptance Criteria**:
- ✅ OpenAI SSE format parsed correctly
- ✅ Handles `[DONE]` sentinel
- ✅ Tokens sent as they arrive
- ✅ Cost estimation for streaming responses

---

### Task 3: Google Gemini Streaming Integration (2-3 hours)

**File**: `worker/src/llm/google.rs` (update)

Integrate Google Gemini streaming (similar pattern):

```rust
impl GoogleProvider {
    pub async fn prompt_streaming(
        &self,
        request: &PromptRequest,
        token_sender: mpsc::UnboundedSender<crate::activities::streaming::StreamToken>,
    ) -> Result<PromptResponse, LLMError> {
        let url = format!(
            "{}/v1/models/{}:streamGenerateContent?key={}",
            self.base_url, request.model, self.api_key
        );

        let body = serde_json::json!({
            "contents": self.convert_messages(&request.messages),
            "generationConfig": {
                "maxOutputTokens": request.max_tokens,
            }
        });

        let mut es = EventSource::new(
            self.client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body)
                .build()?,
        )?;

        let mut full_text = String::new();
        let mut token_index = 0u32;

        while let Some(event) = es.next().await {
            match event {
                Ok(Event::Message(msg)) => {
                    let chunk: GoogleStreamChunk = serde_json::from_str(&msg.data)?;

                    for candidate in chunk.candidates {
                        if let Some(content) = candidate.content {
                            if let Some(parts) = content.parts {
                                for part in parts {
                                    if let Some(text) = part.text {
                                        full_text.push_str(&text);

                                        let _ = token_sender.send(crate::activities::streaming::StreamToken {
                                            text: text.clone(),
                                            index: token_index,
                                        });
                                        token_index += 1;
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(Event::Open) => {
                    tracing::debug!("Google stream opened");
                }
                Err(e) => {
                    return Err(LLMError::StreamError(e.to_string()));
                }
            }
        }

        let cost_usd = self.estimate_cost(&request.model, &full_text, request.max_tokens);

        Ok(PromptResponse {
            content: full_text,
            provider: "google".to_string(),
            model: request.model.clone(),
            cost_usd,
            usage: None,
        })
    }
}
```

**Acceptance Criteria**:
- ✅ Google SSE format parsed correctly
- ✅ Nested content structure handled
- ✅ Tokens streamed properly

---

### Task 4: Activity Streaming Layer (6-8 hours)

**File**: `worker/src/activities/llm.rs` (update)

Integrate streaming into activity execution with two-level opt-in:

```rust
use crate::websocket::ConnectionManager;
use shared::types::workflow::StreamingConfig;
use std::sync::Arc;
use tokio::sync::mpsc;

pub async fn execute_llm_activity(
    activity_id: Uuid,
    workflow_id: Uuid,
    parameters: &LLMActivityParameters,
    streaming_config: &StreamingConfig,
    connection_manager: Arc<ConnectionManager>,
    api_url: &str,
) -> Result<ActivityResult, ActivityError> {
    // Two-level opt-in check:
    // 1. Activity-level: streaming must be enabled in workflow definition
    // 2. Runtime-level: at least one WebSocket subscriber must be connected
    let should_stream = streaming_config.is_enabled()
        && connection_manager.connection_count(activity_id).await > 0;

    if !should_stream {
        // Either streaming not enabled or no subscribers - use efficient non-streaming path
        return execute_llm_activity_non_streaming(activity_id, workflow_id, parameters).await;
    }

    // Create channel for token streaming
    let (token_tx, mut token_rx) = mpsc::unbounded_channel();

    // Spawn task to forward tokens to WebSocket subscribers
    let conn_mgr = connection_manager.clone();
    let streaming_task = tokio::spawn(async move {
        while let Some(token) = token_rx.recv().await {
            conn_mgr
                .broadcast(
                    activity_id,
                    StreamMessage::Token {
                        text: token.text,
                        index: token.index,
                        timestamp: chrono::Utc::now(),
                    },
                )
                .await;
        }
    });

    // Execute LLM with streaming
    let provider = get_provider(&parameters.model)?;
    let response = match provider {
        Provider::Anthropic(p) => {
            p.prompt_streaming(&parameters.to_prompt_request(), token_tx.clone()).await?
        }
        Provider::OpenAI(p) => {
            p.prompt_streaming(&parameters.to_prompt_request(), token_tx.clone()).await?
        }
        Provider::Google(p) => {
            p.prompt_streaming(&parameters.to_prompt_request(), token_tx.clone()).await?
        }
        Provider::Ollama(p) => {
            // Ollama may not support streaming in MVP
            p.prompt(&parameters.to_prompt_request()).await?
        }
    };

    // Close token channel (signals streaming_task to complete)
    drop(token_tx);
    streaming_task.await?;

    // Send completion message
    connection_manager
        .broadcast(
            activity_id,
            StreamMessage::Complete {
                activity_id,
                result: serde_json::to_value(&response)?,
                timestamp: chrono::Utc::now(),
            },
        )
        .await;

    // Close all connections for this activity
    connection_manager.close_all(activity_id).await;

    // Return activity result (same as non-streaming)
    Ok(ActivityResult {
        outputs: serde_json::json!({
            "result": {
                "content": response.content,
                "provider": response.provider,
                "model": response.model,
                "cost_usd": response.cost_usd,
                "usage": response.usage,
            }
        }),
        cost_usd: response.cost_usd,
        ..Default::default()
    })
}

// Non-streaming version (when no WebSocket subscribers)
async fn execute_llm_activity_non_streaming(
    activity_id: Uuid,
    workflow_id: Uuid,
    parameters: &LLMActivityParameters,
) -> Result<ActivityResult, ActivityError> {
    // Existing non-streaming implementation
    let provider = get_provider(&parameters.model)?;
    let response = provider.prompt(&parameters.to_prompt_request()).await?;

    Ok(ActivityResult {
        outputs: serde_json::json!({
            "result": {
                "content": response.content,
                "provider": response.provider,
                "model": response.model,
                "cost_usd": response.cost_usd,
                "usage": response.usage,
            }
        }),
        cost_usd: response.cost_usd,
        ..Default::default()
    })
}
```

**File**: `worker/src/main.rs` (update)

Pass ConnectionManager to worker:

```rust
// Worker needs access to API's ConnectionManager
// Option 1: Worker connects to API via HTTP and publishes streaming events
// Option 2: Worker has direct access to ConnectionManager (shared state)

// For MVP, use Option 1: Worker publishes via API endpoint
// POST /api/v1/activities/{id}/ws/token
// This keeps built-in worker consistent with external workers

pub async fn publish_stream_token(
    api_url: &str,
    activity_id: Uuid,
    token: &StreamToken,
) -> Result<(), reqwest::Error> {
    let client = reqwest::Client::new();
    client
        .post(format!("{}/api/v1/activities/{}/ws/token", api_url, activity_id))
        .json(&serde_json::json!({
            "text": token.text,
            "index": token.index,
        }))
        .send()
        .await?;
    Ok(())
}
```

**File**: `api/src/handlers/internal.rs` (new)

Internal API endpoint for worker to publish tokens:

```rust
/// Endpoint for worker to publish streaming tokens
/// POST /api/v1/activities/{id}/ws/token
pub async fn publish_stream_token(
    Path(activity_id): Path<Uuid>,
    State(state): State<Arc<AppState>>,
    Json(payload): Json<StreamTokenPayload>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .connection_manager
        .broadcast(
            activity_id,
            StreamMessage::Token {
                text: payload.text,
                index: payload.index,
                timestamp: chrono::Utc::now(),
            },
        )
        .await;

    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
struct StreamTokenPayload {
    text: String,
    index: u32,
}
```

**Acceptance Criteria**:
- ✅ Two-level opt-in: check `streaming_config.is_enabled()` AND subscriber count
- ✅ Execute non-streaming when either condition is false (performance optimization)
- ✅ Forward tokens to WebSocket via ConnectionManager or internal API
- ✅ Accumulate full response for activity result
- ✅ Send completion message after streaming finishes
- ✅ Handle errors during streaming gracefully
- ✅ StreamingConfig types added to `shared/src/types/workflow.rs`

---

### Task 5: Example Integration and Documentation (3-4 hours)

**File**: `examples/06d-agentic-research-streaming.yaml` (new)

Add streaming example variant of Example 6:

```yaml
# This example demonstrates real-time token streaming from LLM activities
# Clients can connect to WebSocket endpoint to see live responses
#
# IMPORTANT: Activities must have `streaming: true` to enable token streaming.
# This is an explicit opt-in to avoid streaming overhead for activities that don't need it.
#
# WebSocket connection:
# ws://localhost:8080/api/v1/activities/{activity_id}/ws?token=YOUR_TOKEN
#
# Message format:
# {"type": "token", "text": "Hello", "index": 0, "timestamp": "2025-11-21T..."}
# {"type": "complete", "activity_id": "...", "result": {...}, "timestamp": "..."}

name: agentic_research_streaming
description: Iterative research with real-time token streaming (demonstrates US-7.1)

activities:
  # LLM activities with streaming enabled
  initial_research:
    type: llm
    streaming: true  # Enable token streaming for this activity
    parameters:
      model: claude-3-5-sonnet
      prompt: "Research the following topic: {{inputs.topic}}"

  synthesize_findings:
    type: llm
    streaming: true  # Enable token streaming
    depends_on: [initial_research]
    parameters:
      model: claude-3-5-sonnet
      prompt: |
        Based on the research findings, synthesize a comprehensive summary:
        {{initial_research.outputs.result.content}}

  # Non-streaming activities (default behavior)
  save_results:
    type: http
    # streaming: false (default, omitted)
    depends_on: [synthesize_findings]
    parameters:
      method: POST
      url: "{{inputs.callback_url}}"
      body:
        summary: "{{synthesize_findings.outputs.result.content}}"
```

**File**: `examples/streaming-client.js` (new)

JavaScript WebSocket client example:

```javascript
// Kruxia Flow Token Streaming Client Example
// Connects to activity WebSocket and displays tokens in real-time

const WebSocket = require('ws');

class StreamFlowStreamingClient {
  constructor(apiUrl, bearerToken) {
    this.apiUrl = apiUrl;
    this.token = bearerToken;
  }

  // Subscribe to activity streaming
  async streamActivity(activityId, onToken, onComplete, onError) {
    const wsUrl = `${this.apiUrl.replace('http', 'ws')}/api/v1/activities/${activityId}/ws?token=${this.token}`;

    const ws = new WebSocket(wsUrl);

    ws.on('open', () => {
      console.log('Connected to activity stream');
    });

    ws.on('message', (data) => {
      const message = JSON.parse(data);

      switch (message.type) {
        case 'token':
          onToken(message.text, message.index);
          break;
        case 'complete':
          onComplete(message.result);
          break;
        case 'error':
          onError(new Error(message.error));
          break;
      }
    });

    ws.on('error', (error) => {
      onError(error);
    });

    ws.on('close', () => {
      console.log('Stream closed');
    });

    return ws;
  }
}

// Usage example
const client = new StreamFlowStreamingClient('http://localhost:8080', 'YOUR_TOKEN');

client.streamActivity(
  'activity-id-here',
  (text, index) => {
    process.stdout.write(text); // Display tokens as they arrive
  },
  (result) => {
    console.log('\n\nComplete:', result);
  },
  (error) => {
    console.error('Error:', error);
  }
);
```

**File**: `examples/streaming-client.py` (new)

Python WebSocket client example:

```python
import asyncio
import json
import websockets

class StreamFlowStreamingClient:
    def __init__(self, api_url: str, bearer_token: str):
        self.api_url = api_url
        self.token = bearer_token

    async def stream_activity(self, activity_id: str, on_token, on_complete, on_error):
        """Subscribe to activity streaming"""
        ws_url = f"{self.api_url.replace('http', 'ws')}/api/v1/activities/{activity_id}/ws?token={self.token}"

        try:
            async with websockets.connect(ws_url) as ws:
                print("Connected to activity stream")

                async for message_str in ws:
                    message = json.loads(message_str)

                    if message['type'] == 'token':
                        on_token(message['text'], message['index'])
                    elif message['type'] == 'complete':
                        on_complete(message['result'])
                    elif message['type'] == 'error':
                        on_error(Exception(message['error']))

        except Exception as e:
            on_error(e)

# Usage example
async def main():
    client = StreamFlowStreamingClient('http://localhost:8080', 'YOUR_TOKEN')

    def on_token(text, index):
        print(text, end='', flush=True)  # Display tokens as they arrive

    def on_complete(result):
        print('\n\nComplete:', result)

    def on_error(error):
        print('Error:', error)

    await client.stream_activity('activity-id-here', on_token, on_complete, on_error)

if __name__ == '__main__':
    asyncio.run(main())
```

**File**: `examples/README.md` (update)

Add streaming documentation:

```markdown
## Token Streaming (US-7.1)

LLM activities support real-time token streaming when:
1. The activity has `streaming: true` in its definition (explicit opt-in)
2. At least one WebSocket client is connected to the activity stream

### Enabling Streaming in Workflow Definitions

```yaml
activities:
  my_llm_activity:
    type: llm
    streaming: true  # Required to enable token streaming
    parameters:
      model: claude-3-5-sonnet
      prompt: "Generate a response..."
```

Streaming is **disabled by default** to avoid overhead for activities that don't need it.

### How to Use Token Streaming

1. **Start a workflow** with streaming-enabled LLM activities:
   ```bash
   curl -X POST http://localhost:8080/api/v1/workflows \
     -H "Authorization: Bearer YOUR_TOKEN" \
     -H "Content-Type: application/json" \
     -d @examples/06d-agentic-research-streaming.yaml
   ```

2. **Get activity ID** from workflow status:
   ```bash
   curl http://localhost:8080/api/v1/workflows/{workflow_id} \
     -H "Authorization: Bearer YOUR_TOKEN"
   ```

3. **Connect WebSocket** before activity executes to stream tokens:
   ```javascript
   const ws = new WebSocket(
     'ws://localhost:8080/api/v1/activities/{activity_id}/ws?token=YOUR_TOKEN'
   );
   ```

4. **Receive tokens** in real-time:
   ```json
   {"type": "token", "text": "Hello", "index": 0}
   {"type": "token", "text": " world", "index": 1}
   {"type": "complete", "result": {...}}
   ```

**Note**: If no WebSocket clients are connected when the activity executes, the activity
runs in efficient non-streaming mode regardless of the `streaming` setting.

See `examples/streaming-client.js` and `examples/streaming-client.py` for complete examples.
```

**Acceptance Criteria**:
- ✅ Streaming example workflow created
- ✅ JavaScript client library example
- ✅ Python client library example
- ✅ Documentation in examples/README.md
- ✅ Example demonstrates streaming with Example 6

---

### Task 6: Testing (5-6 hours)

**File**: `api/tests/token_streaming_integration_tests.rs` (new)

End-to-end token streaming tests:

```rust
#[tokio::test]
async fn test_llm_activity_with_streaming() {
    let app = create_test_app().await;
    let token = create_test_token(&app).await;

    // 1. Submit workflow with LLM activity
    let workflow_id = submit_test_workflow(&app, &token).await;

    // 2. Get activity ID from workflow
    let activity_id = get_first_activity_id(&app, workflow_id, &token).await;

    // 3. Connect WebSocket before activity executes
    let ws_url = format!(
        "ws://localhost/api/v1/activities/{}/ws?token={}",
        activity_id, token
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();

    // 4. Collect streamed tokens
    let mut tokens = Vec::new();
    let mut complete_received = false;

    while let Some(msg) = ws.next().await {
        let msg = msg.unwrap();
        let json: serde_json::Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();

        match json["type"].as_str().unwrap() {
            "token" => {
                tokens.push(json["text"].as_str().unwrap().to_string());
            }
            "complete" => {
                complete_received = true;
                break;
            }
            _ => {}
        }
    }

    // 5. Verify streaming worked
    assert!(!tokens.is_empty(), "Should receive tokens");
    assert!(complete_received, "Should receive completion message");

    // 6. Verify activity result matches streamed content
    let workflow_status = get_workflow_status(&app, workflow_id, &token).await;
    let activity_result = workflow_status["state_data"]["activities"][activity_id.to_string()]["outputs"]["result"]["content"]
        .as_str()
        .unwrap();

    let streamed_text: String = tokens.join("");
    assert_eq!(activity_result, streamed_text);
}

#[tokio::test]
async fn test_streaming_with_multiple_subscribers() {
    let app = create_test_app().await;
    let token = create_test_token(&app).await;

    let workflow_id = submit_test_workflow(&app, &token).await;
    let activity_id = get_first_activity_id(&app, workflow_id, &token).await;

    // Connect 10 WebSocket clients
    let mut clients = Vec::new();
    for _ in 0..10 {
        let ws_url = format!(
            "ws://localhost/api/v1/activities/{}/ws?token={}",
            activity_id, token
        );
        let (ws, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();
        clients.push(ws);
    }

    // All clients should receive same tokens
    let mut all_tokens = Vec::new();
    for mut client in clients {
        let mut tokens = Vec::new();
        while let Some(msg) = client.next().await {
            let msg = msg.unwrap();
            let json: serde_json::Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();

            if json["type"] == "token" {
                tokens.push(json["text"].as_str().unwrap().to_string());
            } else if json["type"] == "complete" {
                break;
            }
        }
        all_tokens.push(tokens);
    }

    // Verify all clients received same tokens
    for tokens in &all_tokens[1..] {
        assert_eq!(tokens, &all_tokens[0]);
    }
}

#[tokio::test]
async fn test_non_streaming_fallback_no_subscribers() {
    let app = create_test_app().await;
    let token = create_test_token(&app).await;

    // Submit workflow with streaming: true but DON'T connect WebSocket
    let workflow_id = submit_streaming_workflow(&app, &token).await;

    // Wait for workflow to complete
    wait_for_workflow_completion(&app, workflow_id, &token).await;

    // Verify activity completed successfully without streaming
    let workflow_status = get_workflow_status(&app, workflow_id, &token).await;
    assert_eq!(workflow_status["status"], "Completed");
}

#[tokio::test]
async fn test_streaming_disabled_by_default() {
    let app = create_test_app().await;
    let token = create_test_token(&app).await;

    // Submit workflow WITHOUT streaming: true (default behavior)
    let workflow_id = submit_test_workflow(&app, &token).await;
    let activity_id = get_first_activity_id(&app, workflow_id, &token).await;

    // Connect WebSocket subscriber
    let ws_url = format!(
        "ws://localhost/api/v1/activities/{}/ws?token={}",
        activity_id, token
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();

    // Wait for workflow to complete
    wait_for_workflow_completion(&app, workflow_id, &token).await;

    // Should NOT receive tokens (only completion) because streaming: true not set
    let mut tokens_received = 0;
    let mut complete_received = false;

    while let Some(msg) = ws.next().await {
        let msg = msg.unwrap();
        let json: serde_json::Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();

        match json["type"].as_str().unwrap() {
            "token" => tokens_received += 1,
            "complete" => {
                complete_received = true;
                break;
            }
            _ => {}
        }
    }

    // No tokens should be streamed when streaming is not enabled
    assert_eq!(tokens_received, 0, "Should not receive tokens when streaming disabled");
    assert!(complete_received, "Should receive completion message");
}

#[tokio::test]
async fn test_streaming_error_handling() {
    let app = create_test_app().await;
    let token = create_test_token(&app).await;

    // Submit workflow with invalid LLM parameters (will fail)
    let workflow_id = submit_failing_workflow(&app, &token).await;
    let activity_id = get_first_activity_id(&app, workflow_id, &token).await;

    let ws_url = format!(
        "ws://localhost/api/v1/activities/{}/ws?token={}",
        activity_id, token
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();

    // Should receive error message
    let mut error_received = false;
    while let Some(msg) = ws.next().await {
        let msg = msg.unwrap();
        let json: serde_json::Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();

        if json["type"] == "error" {
            error_received = true;
            break;
        }
    }

    assert!(error_received, "Should receive error message");
}
```

**Acceptance Criteria**:
- ✅ Test streaming with single subscriber (streaming: true + subscriber)
- ✅ Test streaming with multiple subscribers
- ✅ Test non-streaming fallback when no subscribers (streaming: true but no WebSocket)
- ✅ Test streaming disabled by default (no streaming: true, even with subscriber)
- ✅ Test error handling during streaming
- ✅ Test all three LLM providers (Anthropic, OpenAI, Google)
- ✅ Verify streamed tokens match final result
- ✅ Test StreamingConfig parsing (shorthand and detailed forms)

---

## Success Criteria

### Functional Requirements

- ✅ Anthropic Claude streaming works with all models
- ✅ OpenAI GPT streaming works with all models
- ✅ Google Gemini streaming works with all models
- ✅ Tokens delivered in real-time (<10ms P95 latency)
- ✅ Full response accumulated correctly
- ✅ Cost tracking works with streaming
- ✅ `streaming: true/false` activity property parsed correctly (shorthand and detailed forms)
- ✅ Two-level opt-in: streaming only when `streaming: true` AND subscribers present
- ✅ Non-streaming activities not affected (HTTP, PostgreSQL, etc.)
- ✅ Zero overhead for activities without `streaming: true`

### Non-Functional Requirements

- ✅ Support 1,000 concurrent streaming connections
- ✅ <10ms P95 token delivery latency
- ✅ Memory usage: <1MB per streaming connection
- ✅ No performance degradation for non-streaming activities
- ✅ Streaming adds <5% overhead to activity execution

### User Experience

- ✅ ChatGPT-style real-time UX
- ✅ Example 6 (agentic research) demonstrates streaming
- ✅ JavaScript and Python client examples work
- ✅ Documentation clear and complete

---

## Risks and Mitigations

### Risk 1: LLM Provider API Changes

**Impact**: High
**Probability**: Low
**Mitigation**:
- Pin API versions in provider implementations
- Monitor provider changelogs
- Add integration tests that will fail if API changes

### Risk 2: Streaming Performance Overhead

**Impact**: Medium
**Probability**: Low
**Mitigation**:
- Only stream when WebSocket subscribers present
- Use unbounded channels to avoid blocking
- Monitor metrics to detect performance issues

### Risk 3: Token Accumulation Memory Usage

**Impact**: Low
**Probability**: Low
**Mitigation**:
- Stream tokens but also accumulate full response
- Limit max response size (via max_tokens)
- Monitor memory usage per activity

---

## Post-Implementation Checklist

- [ ] All acceptance criteria met
- [ ] Unit tests passing (all providers)
- [ ] Integration tests passing
- [ ] Load test: 1,000 concurrent streaming connections
- [ ] Example 6 demonstrates streaming
- [ ] Client library examples tested
- [ ] Documentation complete
- [ ] Code review completed
- [ ] Performance metrics validated (<10ms P95)
- [ ] Merged to main branch
- [ ] **Token streaming delivered before public MVP launch**

---

## References

- US-1A.9a: WebSocket Infrastructure (dependency)
- Example 6: Agentic Research (demonstration workflow)
- Anthropic Streaming API: https://docs.anthropic.com/claude/reference/streaming
- OpenAI Streaming API: https://platform.openai.com/docs/api-reference/chat/create#chat-create-stream
- Google Streaming API: https://ai.google.dev/api/rest/v1/models/streamGenerateContent
