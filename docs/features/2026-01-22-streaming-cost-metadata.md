# Streaming Cost Metadata in WebSocket Complete Events

## Summary

Add an optional `metadata` field to the WebSocket `Complete` message that includes cost and token usage data for LLM activities. This enables clients to receive cost information in real-time via the existing WebSocket connection instead of requiring separate polling requests.

## Motivation

Currently, clients that subscribe to activity streaming via WebSocket must poll a separate REST endpoint to retrieve cost data:

| Data              | Delivery              | Endpoint                              |
|-------------------|-----------------------|---------------------------------------|
| LLM tokens        | WebSocket (streaming) | `ws://.../api/v1/activities/{id}/ws`  |
| Cost/token usage  | Polling (REST)        | `GET /api/v1/workflows/{id}/cost`     |

This creates several issues:

1. **Extra round-trips**: Clients need to poll for cost data after receiving the `Complete` message
2. **Timing complexity**: Cost data may not be available immediately after completion (race condition)
3. **Increased server load**: Each workflow execution requires additional HTTP requests
4. **Dashboard latency**: Real-time dashboards (e.g., Pi Zero demos) show stale cost data

The worker already has cost data available when it sends the `Complete` message, so including it is straightforward.

## Proposed Changes

### 1. Add `metadata` Field to `StreamMessage::Complete`

**Current wire format:**
```json
{
  "type": "complete",
  "activity_id": "550e8400-e29b-41d4-a716-446655440000",
  "result": {"content": "The answer is 42."},
  "timestamp": "2026-01-22T10:30:00Z"
}
```

**Proposed wire format:**
```json
{
  "type": "complete",
  "activity_id": "550e8400-e29b-41d4-a716-446655440000",
  "result": {"content": "The answer is 42."},
  "metadata": {
    "cost_usd": "0.00345",
    "usage": {
      "prompt_tokens": 150,
      "output_tokens": 25,
      "total_tokens": 175,
      "cached_tokens": 50
    },
    "model": {
      "provider": "anthropic",
      "name": "claude-3-5-haiku-20241022"
    }
  },
  "timestamp": "2026-01-22T10:30:00Z"
}
```

### 2. Metadata Field Specification

The `metadata` field is **optional** and only present when the activity produces cost/usage data (primarily LLM activities).

| Field                       | Type           | Required | Description                                |
|-----------------------------|----------------|----------|--------------------------------------------|
| `cost_usd`                  | string/decimal | No       | Total cost in USD (string for precision)   |
| `usage`                     | object         | No       | Token usage breakdown                      |
| `usage.prompt_tokens`       | integer        | No       | Number of input/prompt tokens              |
| `usage.output_tokens`       | integer        | No       | Number of output/completion tokens         |
| `usage.total_tokens`        | integer        | No       | Total tokens (prompt + output)             |
| `usage.cached_tokens`       | integer        | No       | Cached tokens (Anthropic only)             |
| `model`                     | object         | No       | Model information                          |
| `model.provider`            | string         | No       | Provider name (anthropic, openai, etc.)    |
| `model.name`                | string         | No       | Model name used for execution              |
| `cached`                    | boolean        | No       | Whether result was served from cache       |
| `cache_key`                 | string         | No       | Cache key (if caching enabled)             |

### 3. Rust Type Changes

**api/src/websocket/messages.rs:**

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamMessage {
    // ... existing variants ...

    Complete {
        activity_id: Uuid,
        result: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<serde_json::Value>,  // NEW FIELD
        timestamp: DateTime<Utc>,
    },
}
```

**api/src/handlers/streaming.rs:**

```rust
#[derive(Debug, Deserialize, ToSchema)]
pub struct StreamCompletePayload {
    pub result: serde_json::Value,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,  // NEW FIELD
}
```

### 4. Worker Changes

The worker already tracks cost data in `ActivityResult`. When publishing the completion event, include the metadata:

```rust
// In worker completion handler
let metadata = activity_result.metadata.clone().or_else(|| {
    activity_result.cost_usd.map(|cost| {
        serde_json::json!({
            "cost_usd": cost.to_string(),
        })
    })
});

client.post(&format!("{}/api/v1/activities/{}/ws/complete", api_url, activity_id))
    .json(&StreamCompletePayload {
        result: activity_result.to_json_value(),
        metadata,
    })
    .send()
    .await?;
```

## Backward Compatibility

This change is **fully backward compatible**:

1. **New field is optional**: The `metadata` field uses `skip_serializing_if = "Option::is_none"`, so it's omitted when not present
2. **Existing clients**: Clients that don't expect `metadata` will simply ignore it (standard JSON behavior)
3. **Non-LLM activities**: Activities without cost data (HTTP, PostgreSQL) won't include metadata unless they have cache information
4. **Gradual adoption**: Clients can opt-in to using metadata when ready

## Client Usage Examples

### JavaScript/TypeScript Dashboard

```typescript
const ws = new WebSocket(`ws://${host}/api/v1/activities/${activityId}/ws`);

ws.onmessage = (event) => {
  const message = JSON.parse(event.data);

  if (message.type === 'complete') {
    // Display result
    displayResult(message.result);

    // Update cost display if metadata present
    if (message.metadata?.cost_usd) {
      updateCostDisplay({
        cost: parseFloat(message.metadata.cost_usd),
        tokens: message.metadata.usage,
        model: message.metadata.model,
      });
    }
  }
};
```

### Python SDK

```python
async for message in websocket:
    data = json.loads(message)

    if data["type"] == "complete":
        result = data["result"]

        # Access cost metadata if available
        if metadata := data.get("metadata"):
            print(f"Cost: ${metadata.get('cost_usd', 'N/A')}")
            if usage := metadata.get("usage"):
                print(f"Tokens: {usage.get('total_tokens', 'N/A')}")
```

## Implementation Plan

### Phase 1: Core Changes

1. **Update `StreamMessage::Complete`** in `api/src/websocket/messages.rs`
   - Add optional `metadata` field
   - Update serialization tests

2. **Update `StreamCompletePayload`** in `api/src/handlers/streaming.rs`
   - Add optional `metadata` field
   - Update handler to pass metadata through

3. **Update `StreamMessage` constructors**
   - Add `complete_with_metadata()` helper method

### Phase 2: Worker Integration

4. **Update worker completion flow**
   - Collect metadata from `ActivityResult`
   - Include in WebSocket completion payload

5. **LLM activity metadata**
   - Ensure token usage is included in metadata
   - Add provider/model information

### Phase 3: Documentation & Testing

6. **Update API documentation**
   - Document new wire format
   - Update cost-dashboard-api.md with streaming option

7. **Add integration tests**
   - Test metadata presence for LLM activities
   - Test metadata absence for non-LLM activities
   - Test backward compatibility

## Testing Strategy

### Unit Tests

```rust
#[test]
fn test_complete_message_with_metadata() {
    let msg = StreamMessage::Complete {
        activity_id: Uuid::now_v7(),
        result: json!({"content": "test"}),
        metadata: Some(json!({
            "cost_usd": "0.00123",
            "usage": {"prompt_tokens": 100, "output_tokens": 50}
        })),
        timestamp: Utc::now(),
    };

    let json = msg.to_json().unwrap();
    assert!(json.contains(r#""metadata""#));
    assert!(json.contains(r#""cost_usd":"0.00123""#));
}

#[test]
fn test_complete_message_without_metadata() {
    let msg = StreamMessage::Complete {
        activity_id: Uuid::now_v7(),
        result: json!({"content": "test"}),
        metadata: None,
        timestamp: Utc::now(),
    };

    let json = msg.to_json().unwrap();
    // metadata field should be omitted entirely
    assert!(!json.contains(r#""metadata""#));
}
```

### Integration Tests

- Execute LLM activity with streaming enabled
- Verify WebSocket `Complete` message contains cost metadata
- Execute HTTP activity with streaming
- Verify `Complete` message has no metadata (or only cache info)

## Performance Considerations

- **Minimal overhead**: Metadata is a small JSON object (~200 bytes)
- **No extra queries**: Cost data is already computed by the worker
- **Reduced polling**: Clients can eliminate cost polling requests

## Future Enhancements

1. **Incremental cost updates**: Add a `CostUpdate` message type for long-running activities
2. **Budget alerts via WebSocket**: Stream budget threshold warnings
3. **Workflow-level cost streaming**: Subscribe to cost updates for entire workflow

## Related Documentation

- [Cost Dashboard API](../cost-dashboard-api.md) - REST endpoints for cost queries
- [Token Streaming](../implementation/US-7.1-token-streaming.md) - WebSocket streaming implementation
- [Semantic Caching](./2026-01-05-semantic-caching.md) - Cache metadata in results
