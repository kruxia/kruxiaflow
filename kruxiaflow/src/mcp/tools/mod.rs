pub mod control;
/// MCP tool registry
///
/// Tools are organised into categories, each in its own module.
/// Each module uses `tool_box!` to generate an enum that provides:
///   - `::tools()` → Vec<Tool> for the list_tools MCP response
///   - `TryFrom<CallToolRequestParams>` → parse an incoming call (succeeds only if the name matches)
pub mod discovery;
pub mod execution;
pub mod observability;
pub mod visualization;

pub use control::ControlTools;
pub use discovery::DiscoveryTools;
pub use execution::ExecutionTools;
pub use observability::ObservabilityTools;
pub use visualization::VisualizationTools;

use rust_mcp_sdk::schema::{CallToolResult, TextContent, schema_utils::CallToolError};

/// A transparent wrapper for `serde_json::Value` that generates a valid JSON Schema.
///
/// `serde_json::Value` is not recognized by the `rust-mcp-macros` `JsonSchema` derive macro
/// and falls through to its unknown-type fallback, producing `{"type": "unknown"}` — an
/// invalid JSON Schema draft 2020-12 value. This newtype provides a `json_schema()` method
/// (called by the macro for any single-segment struct type) that returns `{}`, the correct
/// draft 2020-12 representation of "any JSON value is accepted".
///
/// Use this as a drop-in replacement for `serde_json::Value` in tool structs that derive
/// `JsonSchema`. Access the inner value via `.0` or the `Into<serde_json::Value>` impl.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(transparent)]
pub(crate) struct AnyJson(pub serde_json::Value);

impl AnyJson {
    /// Returns an empty JSON Schema object, which accepts any JSON value.
    /// Called by the `#[derive(JsonSchema)]` macro for fields typed `AnyJson`.
    pub fn json_schema() -> serde_json::Map<String, serde_json::Value> {
        serde_json::Map::new()
    }
}

impl From<AnyJson> for serde_json::Value {
    fn from(v: AnyJson) -> Self {
        v.0
    }
}

/// Wrap a JSON value as a pretty-printed text response.
pub(crate) fn text_response(value: &serde_json::Value) -> Result<CallToolResult, CallToolError> {
    Ok(CallToolResult::text_content(vec![TextContent::from(
        serde_json::to_string_pretty(value)
            .map_err(|e| CallToolError::from_message(e.to_string()))?,
    )]))
}

/// Wrap a JSON value as a pretty-printed text response with `is_error` set.
///
/// Use this for application-level errors (not found, invalid input, stubs)
/// so MCP clients can detect errors via the protocol rather than parsing JSON.
pub(crate) fn error_response(value: &serde_json::Value) -> Result<CallToolResult, CallToolError> {
    let mut result = text_response(value)?;
    result.is_error = Some(true);
    Ok(result)
}

/// Parse a string as UUID, returning a tool error if invalid.
pub(crate) fn parse_uuid(s: &str) -> Result<uuid::Uuid, CallToolError> {
    uuid::Uuid::parse_str(s).map_err(|_| {
        CallToolError::from_message(format!("Invalid workflow_id '{}': must be a valid UUID", s))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Extract the JSON payload from a CallToolResult's first content block.
    fn extract_json(result: &CallToolResult) -> serde_json::Value {
        let text_content = result.content[0].as_text_content().unwrap();
        serde_json::from_str(&text_content.text).unwrap()
    }

    // =========================================================================
    // text_response tests
    // =========================================================================

    #[test]
    fn test_text_response_simple_object() {
        let val = serde_json::json!({"key": "value", "count": 42});
        let result = text_response(&val).unwrap();
        assert!(result.is_error.is_none() || result.is_error == Some(false));
        let parsed = extract_json(&result);
        assert_eq!(parsed["key"], "value");
        assert_eq!(parsed["count"], 42);
    }

    #[test]
    fn test_text_response_empty_object() {
        let val = serde_json::json!({});
        let result = text_response(&val).unwrap();
        let parsed = extract_json(&result);
        assert!(parsed.as_object().unwrap().is_empty());
    }

    #[test]
    fn test_text_response_array() {
        let val = serde_json::json!(["a", "b", "c"]);
        let result = text_response(&val).unwrap();
        let parsed = extract_json(&result);
        assert_eq!(parsed.as_array().unwrap().len(), 3);
    }

    // =========================================================================
    // error_response tests
    // =========================================================================

    #[test]
    fn test_error_response_sets_is_error() {
        let val = serde_json::json!({"error": "something went wrong"});
        let result = error_response(&val).unwrap();
        assert_eq!(result.is_error, Some(true));
    }

    #[test]
    fn test_error_response_preserves_content() {
        let val = serde_json::json!({"error": "not found", "workflow_id": "abc-123"});
        let result = error_response(&val).unwrap();
        let parsed = extract_json(&result);
        assert_eq!(parsed["error"], "not found");
        assert_eq!(parsed["workflow_id"], "abc-123");
    }

    // =========================================================================
    // parse_uuid tests
    // =========================================================================

    #[test]
    fn test_parse_uuid_valid() {
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        let result = parse_uuid(uuid_str);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().to_string(), uuid_str);
    }

    #[test]
    fn test_parse_uuid_invalid() {
        let result = parse_uuid("not-a-uuid");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_uuid_empty() {
        let result = parse_uuid("");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_uuid_v7() {
        let uuid = uuid::Uuid::now_v7();
        let result = parse_uuid(&uuid.to_string());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), uuid);
    }

    // =========================================================================
    // Tool schema validation
    // =========================================================================

    /// Valid JSON Schema draft 2020-12 type values.
    const VALID_SCHEMA_TYPES: &[&str] = &[
        "null", "boolean", "object", "array", "number", "string", "integer",
    ];

    /// Recursively check that every `"type"` value in a JSON schema is valid.
    fn assert_no_invalid_types(tool_name: &str, prop_name: &str, schema: &serde_json::Value) {
        if let Some(obj) = schema.as_object() {
            if let Some(type_val) = obj.get("type") {
                if let Some(s) = type_val.as_str() {
                    assert!(
                        VALID_SCHEMA_TYPES.contains(&s),
                        "Tool '{tool_name}', property '{prop_name}': invalid schema type \"{s}\""
                    );
                }
            }
            // Recurse into nested schemas (properties, items, additionalProperties, etc.)
            for (key, val) in obj {
                if matches!(key.as_str(), "properties" | "items" | "additionalProperties") {
                    assert_no_invalid_types(tool_name, prop_name, val);
                }
            }
            if let Some(props) = obj.get("properties").and_then(|p| p.as_object()) {
                for (nested_name, nested_schema) in props {
                    assert_no_invalid_types(tool_name, nested_name, nested_schema);
                }
            }
        }
    }

    /// The assertion helper correctly rejects `"type": "unknown"`.
    #[test]
    #[should_panic(expected = "invalid schema type \"unknown\"")]
    fn test_schema_validator_catches_unknown_type() {
        let bad_schema = serde_json::json!({"type": "unknown"});
        assert_no_invalid_types("fake_tool", "bad_field", &bad_schema);
    }

    /// Verify that all MCP tool schemas are valid JSON Schema draft 2020-12.
    ///
    /// This catches the `rust-mcp-macros` fallback where unrecognised types
    /// produce `{"type": "unknown"}`, which is rejected by JSON Schema validators
    /// (e.g., Claude's tool loader).
    #[test]
    fn test_all_tool_schemas_have_valid_types() {
        let all_tools = [
            DiscoveryTools::tools(),
            ExecutionTools::tools(),
            ObservabilityTools::tools(),
            VisualizationTools::tools(),
            ControlTools::tools(),
        ]
        .concat();

        assert!(!all_tools.is_empty(), "Expected at least one MCP tool");

        for tool in &all_tools {
            let schema_json = serde_json::to_value(&tool.input_schema)
                .expect("Failed to serialize tool input_schema");

            if let Some(props) = schema_json.get("properties").and_then(|p| p.as_object()) {
                for (prop_name, prop_schema) in props {
                    assert_no_invalid_types(&tool.name, prop_name, prop_schema);
                }
            }
        }
    }
}
