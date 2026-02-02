//! Cache key generation for deterministic hashing
//!
//! Generates SHA256-based cache keys from activity name and parameters.
//! Keys are deterministic (same inputs always produce same key) regardless
//! of JSON object key ordering.

use serde_json::Value;
use sha2::{Digest, Sha256};

/// Generate cache key from activity parameters
///
/// Creates a deterministic SHA256 hash from activity name and normalized parameters.
/// The same activity name and parameters (regardless of JSON key order) will always
/// produce the same cache key.
///
/// # Arguments
///
/// * `activity_name` - Full activity name (e.g., "std.llm_prompt")
/// * `parameters` - Activity parameters as JSON value
///
/// # Returns
///
/// Hex-encoded SHA256 hash (64 characters)
///
/// # Examples
///
/// ```
/// use serde_json::json;
/// use kruxiaflow_core::cache::key_generator::generate_cache_key;
///
/// let params = json!({
///     "prompt": "Hello world",
///     "model": "claude-3-haiku",
///     "temperature": 0.0
/// });
///
/// let key = generate_cache_key("std.llm_prompt", &params)?;
/// assert_eq!(key.len(), 64); // SHA256 hex = 64 chars
/// # Ok::<(), anyhow::Error>(())
/// ```
pub fn generate_cache_key(activity_name: &str, parameters: &Value) -> anyhow::Result<String> {
    // Normalize parameters by sorting keys (JSON objects are unordered)
    let normalized = normalize_json(parameters)?;

    // Create hash input: activity_name + normalized_params
    let hash_input = format!("{}:{}", activity_name, normalized);

    // Generate SHA256 hash
    let mut hasher = Sha256::new();
    hasher.update(hash_input.as_bytes());
    let hash = hasher.finalize();

    // Return hex-encoded hash
    Ok(format!("{:x}", hash))
}

/// Normalize JSON for deterministic hashing
///
/// Sorts object keys to ensure consistent hash values regardless of
/// the original key ordering in the JSON object.
fn normalize_json(value: &Value) -> anyhow::Result<String> {
    match value {
        Value::Object(map) => {
            // Sort object keys for deterministic ordering
            let mut sorted: Vec<_> = map.iter().collect();
            sorted.sort_by_key(|(k, _)| *k);

            // Recursively normalize nested values
            let normalized_map: serde_json::Map<String, Value> = sorted
                .into_iter()
                .map(|(k, v)| {
                    // Recursively normalize nested objects
                    let normalized_v = match v {
                        Value::Object(_) => {
                            let normalized_str = normalize_json(v)
                                .unwrap_or_else(|_| serde_json::to_string(v).unwrap_or_default());
                            serde_json::from_str(&normalized_str).unwrap_or_else(|_| v.clone())
                        }
                        Value::Array(arr) => {
                            // Normalize array elements
                            let normalized_arr: Vec<Value> = arr
                                .iter()
                                .map(|item| match item {
                                    Value::Object(_) => {
                                        let normalized_str =
                                            normalize_json(item).unwrap_or_else(|_| {
                                                serde_json::to_string(item).unwrap_or_default()
                                            });
                                        serde_json::from_str(&normalized_str)
                                            .unwrap_or_else(|_| item.clone())
                                    }
                                    _ => item.clone(),
                                })
                                .collect();
                            Value::Array(normalized_arr)
                        }
                        _ => v.clone(),
                    };
                    (k.clone(), normalized_v)
                })
                .collect();

            serde_json::to_string(&normalized_map)
                .map_err(|e| anyhow::anyhow!("Failed to serialize normalized JSON: {}", e))
        }
        _ => {
            // For non-objects, use standard serialization
            serde_json::to_string(value)
                .map_err(|e| anyhow::anyhow!("Failed to serialize JSON: {}", e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_cache_key_deterministic() {
        // Parameters in different orders should produce same key
        let params1 = json!({
            "prompt": "Hello world",
            "model": "claude-3-haiku",
            "temperature": 0.0
        });

        let params2 = json!({
            "temperature": 0.0,
            "prompt": "Hello world",
            "model": "claude-3-haiku"
        });

        let key1 = generate_cache_key("llm_prompt", &params1).unwrap();
        let key2 = generate_cache_key("llm_prompt", &params2).unwrap();

        // Keys should be identical despite different parameter order
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_cache_key_different_params() {
        let params1 = json!({"prompt": "Hello"});
        let params2 = json!({"prompt": "World"});

        let key1 = generate_cache_key("llm_prompt", &params1).unwrap();
        let key2 = generate_cache_key("llm_prompt", &params2).unwrap();

        // Keys should be different for different parameters
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_cache_key_different_activity() {
        let params = json!({"prompt": "Hello"});

        let key1 = generate_cache_key("llm_prompt", &params).unwrap();
        let key2 = generate_cache_key("http_request", &params).unwrap();

        // Keys should be different for different activities
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_cache_key_hex_length() {
        let params = json!({"test": "value"});
        let key = generate_cache_key("test_activity", &params).unwrap();

        // SHA256 hex encoding is 64 characters
        assert_eq!(key.len(), 64);
    }

    #[test]
    fn test_cache_key_nested_objects() {
        let params1 = json!({
            "outer": {
                "inner": "value",
                "number": 42
            }
        });

        let params2 = json!({
            "outer": {
                "number": 42,
                "inner": "value"
            }
        });

        let key1 = generate_cache_key("test", &params1).unwrap();
        let key2 = generate_cache_key("test", &params2).unwrap();

        // Should handle nested object key ordering
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_normalize_json_simple() {
        let value = json!({"b": 2, "a": 1});
        let normalized = normalize_json(&value).unwrap();

        // Keys should be sorted
        assert_eq!(normalized, r#"{"a":1,"b":2}"#);
    }

    #[test]
    fn test_normalize_json_non_object() {
        let value = json!([1, 2, 3]);
        let normalized = normalize_json(&value).unwrap();

        // Arrays should serialize normally
        assert_eq!(normalized, "[1,2,3]");
    }

    #[test]
    fn test_cache_key_empty_object() {
        let params = json!({});
        let key = generate_cache_key("test", &params).unwrap();
        assert_eq!(key.len(), 64);
    }

    #[test]
    fn test_cache_key_null_value() {
        let params = json!(null);
        let key = generate_cache_key("test", &params).unwrap();
        assert_eq!(key.len(), 64);
    }

    #[test]
    fn test_cache_key_string_value() {
        let params = json!("just a string");
        let key = generate_cache_key("test", &params).unwrap();
        assert_eq!(key.len(), 64);
    }

    #[test]
    fn test_cache_key_number_value() {
        let params = json!(42);
        let key = generate_cache_key("test", &params).unwrap();
        assert_eq!(key.len(), 64);
    }

    #[test]
    fn test_cache_key_boolean_value() {
        let params = json!(true);
        let key = generate_cache_key("test", &params).unwrap();
        assert_eq!(key.len(), 64);
    }

    #[test]
    fn test_cache_key_array_with_nested_objects() {
        let params1 = json!([{"b": 2, "a": 1}, {"d": 4, "c": 3}]);
        let params2 = json!([{"a": 1, "b": 2}, {"c": 3, "d": 4}]);

        let key1 = generate_cache_key("test", &params1).unwrap();
        let key2 = generate_cache_key("test", &params2).unwrap();

        // Array element objects should be normalized
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_cache_key_deeply_nested() {
        let params1 = json!({"a": {"b": {"d": 4, "c": 3}}});
        let params2 = json!({"a": {"b": {"c": 3, "d": 4}}});

        let key1 = generate_cache_key("test", &params1).unwrap();
        let key2 = generate_cache_key("test", &params2).unwrap();

        assert_eq!(key1, key2);
    }

    #[test]
    fn test_cache_key_empty_activity_name() {
        let params = json!({"key": "value"});
        let key = generate_cache_key("", &params).unwrap();
        assert_eq!(key.len(), 64);
    }

    #[test]
    fn test_cache_key_same_params_different_null_vs_missing() {
        // null value vs missing key should produce different keys
        let params1 = json!({"a": null});
        let params2 = json!({});

        let key1 = generate_cache_key("test", &params1).unwrap();
        let key2 = generate_cache_key("test", &params2).unwrap();

        assert_ne!(key1, key2);
    }

    #[test]
    fn test_normalize_json_null() {
        let value = json!(null);
        let normalized = normalize_json(&value).unwrap();
        assert_eq!(normalized, "null");
    }

    #[test]
    fn test_normalize_json_string() {
        let value = json!("hello");
        let normalized = normalize_json(&value).unwrap();
        assert_eq!(normalized, "\"hello\"");
    }

    #[test]
    fn test_normalize_json_number() {
        let value = json!(42);
        let normalized = normalize_json(&value).unwrap();
        assert_eq!(normalized, "42");
    }

    #[test]
    fn test_normalize_json_boolean() {
        let value = json!(false);
        let normalized = normalize_json(&value).unwrap();
        assert_eq!(normalized, "false");
    }

    #[test]
    fn test_normalize_json_nested_array_with_objects() {
        let value = json!({"items": [{"z": 1, "a": 2}]});
        let normalized = normalize_json(&value).unwrap();
        // The outer object key "items" is sorted, inner object keys "a","z" are sorted
        assert_eq!(normalized, r#"{"items":[{"a":2,"z":1}]}"#);
    }

    #[test]
    fn test_normalize_json_empty_object() {
        let value = json!({});
        let normalized = normalize_json(&value).unwrap();
        assert_eq!(normalized, "{}");
    }

    #[test]
    fn test_normalize_json_empty_array() {
        let value = json!([]);
        let normalized = normalize_json(&value).unwrap();
        assert_eq!(normalized, "[]");
    }
}
