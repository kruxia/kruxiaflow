//! Protocol types shared between workers and the Kruxia Flow server.
//!
//! These mirror the worker HTTP API contract (`/api/v1/workers/poll`,
//! `/api/v1/activities/{id}/…`). Field names are frozen as part of the
//! public worker-API contract.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Activity output type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Hash)]
#[serde(rename_all = "lowercase")]
pub enum OutputType {
    /// Default: JSON value
    #[default]
    Value,

    /// File reference
    File,

    /// Folder reference
    Folder,
}

/// A single named activity output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivityOutput {
    /// Output name
    pub name: String,

    /// Output type
    #[serde(rename = "type")]
    pub output_type: OutputType,

    /// Output value
    /// - For `Value`: JSON data
    /// - For `File`: file reference string (e.g., `postgres://workflow_id/activity_key/filename`)
    /// - For `Folder`: folder reference string
    pub value: Value,
}

impl ActivityOutput {
    /// Create a JSON-value output.
    pub fn value(name: impl Into<String>, value: Value) -> Self {
        Self {
            name: name.into(),
            output_type: OutputType::Value,
            value,
        }
    }

    /// Create a file-reference output.
    pub fn file(name: impl Into<String>, reference: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            output_type: OutputType::File,
            value: Value::String(reference.into()),
        }
    }

    /// Create a folder-reference output.
    pub fn folder(name: impl Into<String>, reference: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            output_type: OutputType::Folder,
            value: Value::String(reference.into()),
        }
    }
}

/// An activity claimed from the queue, as returned by `POST /api/v1/workers/poll`.
#[derive(Debug, Clone, Deserialize)]
pub struct PendingActivity {
    /// Unique identifier for this activity execution
    pub activity_id: Uuid,
    /// Workflow instance this activity belongs to
    pub workflow_id: Uuid,
    /// Activity key from the workflow definition
    pub activity_key: String,
    /// Worker type (the `worker:` field in the workflow definition)
    pub worker: String,
    /// Activity name (the `name:` field in the workflow definition)
    pub activity_name: String,
    /// Activity input parameters
    pub parameters: Value,
    /// Raw activity settings from the workflow definition
    pub settings: Option<Value>,
    /// Per-activity timeout override (seconds)
    pub timeout_seconds: Option<i64>,
    /// Declared output definitions (used for file outputs)
    pub output_definitions: Option<Value>,
    /// Signal data for activities that were waiting for an external signal
    pub signal_data: Option<Value>,
}

/// Response envelope for `POST /api/v1/workers/poll`.
#[derive(Debug, Deserialize)]
pub struct PollActivitiesResponse {
    /// Claimed activities
    pub activities: Vec<PendingActivity>,
    /// Number of activities claimed
    pub count: usize,
}

/// Per-LLM-call usage made inside an activity, reported on completion or
/// failure so external activities appear in cost history/analytics and count
/// against workflow budgets with the same fidelity as built-in LLM activities.
///
/// When `cost_usd` is omitted the server computes the cost from its
/// `llm_models` pricing catalog (cache reads at the cached-input price, cache
/// creation at the cache-write price, cache storage at the cache-storage
/// price per million token-hours). An unknown provider/model records the
/// entry at cost 0 with a warning — completion never fails because of usage
/// metadata.
///
/// Time-based cache *storage* (e.g., Gemini explicit caching, billed per
/// token-hour) is reported via `cache_storage_token_hours` (engine 0.8+;
/// models without a catalog storage price record that component at 0 with a
/// warning). Reporting the spend via explicit `cost_usd` instead remains
/// fully supported.
///
/// Double-reporting rule: an entry's `cost_usd`, when set, is used verbatim
/// and REPLACES all server-side computation for that entry — never report the
/// same spend both ways.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageEntry {
    /// LLM provider name (matches the server's `llm_models` catalog, e.g. "anthropic")
    pub provider: String,

    /// Model name (matches the server's `llm_models` catalog)
    pub model: String,

    /// Prompt tokens, including cache reads
    #[serde(default)]
    pub input_tokens: u32,

    /// Completion tokens
    #[serde(default)]
    pub output_tokens: u32,

    /// Prompt tokens served from cache (billed at the cached-input price)
    #[serde(default)]
    pub cache_read_tokens: u32,

    /// Tokens written to cache (billed at the catalog's cache-write price,
    /// falling back to the input price for models without one)
    #[serde(default)]
    pub cache_creation_tokens: u32,

    /// Context-cache storage consumed, in token-hours (tokens held x hours
    /// held; fractional). Billed at the catalog's cache-storage price; models
    /// without one record the component at 0 with a warning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_storage_token_hours: Option<Decimal>,

    /// Explicit cost for this call; overrides server-side computation
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<Decimal>,
}

impl UsageEntry {
    /// Create a usage entry for one LLM call with all token counts zero.
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            cache_storage_token_hours: None,
            cost_usd: None,
        }
    }

    /// Set prompt tokens (including cache reads).
    pub fn input_tokens(mut self, tokens: u32) -> Self {
        self.input_tokens = tokens;
        self
    }

    /// Set completion tokens.
    pub fn output_tokens(mut self, tokens: u32) -> Self {
        self.output_tokens = tokens;
        self
    }

    /// Set prompt tokens served from cache.
    pub fn cache_read_tokens(mut self, tokens: u32) -> Self {
        self.cache_read_tokens = tokens;
        self
    }

    /// Set tokens written to cache.
    pub fn cache_creation_tokens(mut self, tokens: u32) -> Self {
        self.cache_creation_tokens = tokens;
        self
    }

    /// Set context-cache storage consumed, in token-hours (tokens held x
    /// hours held; fractional).
    pub fn cache_storage_token_hours(mut self, token_hours: Decimal) -> Self {
        self.cache_storage_token_hours = Some(token_hours);
        self
    }

    /// Set an explicit cost, overriding server-side catalog pricing.
    pub fn cost_usd(mut self, cost: Decimal) -> Self {
        self.cost_usd = Some(cost);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn output_type_serde() {
        assert_eq!(
            serde_json::to_string(&OutputType::Value).unwrap(),
            "\"value\""
        );
        assert_eq!(
            serde_json::to_string(&OutputType::File).unwrap(),
            "\"file\""
        );
        assert_eq!(
            serde_json::to_string(&OutputType::Folder).unwrap(),
            "\"folder\""
        );
    }

    #[test]
    fn activity_output_constructors() {
        let v = ActivityOutput::value("result", json!({"ok": true}));
        assert_eq!(v.output_type, OutputType::Value);

        let f = ActivityOutput::file("doc", "postgres://wf/act/file.pdf");
        assert_eq!(f.output_type, OutputType::File);
        assert_eq!(f.value, json!("postgres://wf/act/file.pdf"));

        let d = ActivityOutput::folder("out", "postgres://wf/act/out/");
        assert_eq!(d.output_type, OutputType::Folder);
    }

    #[test]
    fn pending_activity_deserializes() {
        let activity: PendingActivity = serde_json::from_value(json!({
            "activity_id": "01890a5d-ac96-774b-bcce-b302099a8057",
            "workflow_id": "01890a5d-ac96-774b-bcce-b302099a8058",
            "activity_key": "step_one",
            "worker": "demo",
            "activity_name": "echo",
            "parameters": {"message": "hi"},
            "settings": null,
            "timeout_seconds": 120,
            "output_definitions": null,
            "signal_data": null
        }))
        .unwrap();
        assert_eq!(activity.worker, "demo");
        assert_eq!(activity.activity_name, "echo");
        assert_eq!(activity.timeout_seconds, Some(120));
    }

    #[test]
    fn usage_entry_builder_and_serde() {
        use rust_decimal_macros::dec;

        let entry = UsageEntry::new("anthropic", "claude-sonnet-5")
            .input_tokens(12034)
            .output_tokens(512)
            .cache_read_tokens(9800);

        let value = serde_json::to_value(&entry).unwrap();
        assert_eq!(
            value,
            json!({
                "provider": "anthropic",
                "model": "claude-sonnet-5",
                "input_tokens": 12034,
                "output_tokens": 512,
                "cache_read_tokens": 9800,
                "cache_creation_tokens": 0
            })
        );

        let priced = UsageEntry::new("google", "gemini-2.5-pro").cost_usd(dec!(0.015));
        let value = serde_json::to_value(&priced).unwrap();
        assert_eq!(value["cost_usd"], json!("0.015"));
    }
}
