//! Activity execution result.

use crate::types::{ActivityOutput, OutputType, UsageEntry};
use rust_decimal::Decimal;
use serde_json::Value;

/// Successful activity result: named outputs plus optional cost tracking.
///
/// # Cost reporting
///
/// Cost governance is enforced in the engine, and external activities count
/// against workflow budgets through what they report here:
///
/// - [`with_usage`](Self::with_usage) / [`push_usage`](Self::push_usage) —
///   one [`UsageEntry`] per LLM call made inside the activity. The server
///   prices entries from its model catalog (unless an entry carries an
///   explicit cost) and records them with the same fidelity as built-in LLM
///   activities: they appear in `/cost/history` and `/cost/analytics` and
///   count against workflow budgets.
/// - [`with_cost`](Self::with_cost) — cost NOT covered by usage entries
///   (e.g., a paid non-LLM API). With no usage entries it is the total
///   activity cost. Never repeat entry costs here.
#[derive(Debug, Clone, Default)]
pub struct ActivityResult {
    /// Structured outputs with type information
    pub outputs: Vec<ActivityOutput>,

    /// Cost in USD not covered by `usage` entries
    pub cost_usd: Option<Decimal>,

    /// Per-LLM-call usage made inside the activity
    pub usage: Vec<UsageEntry>,

    /// Optional metadata (e.g., cache information, execution context)
    pub metadata: Option<Value>,
}

impl ActivityResult {
    /// Create a result with a single value output.
    ///
    /// ```
    /// use kruxiaflow_worker::ActivityResult;
    /// use serde_json::json;
    ///
    /// let result = ActivityResult::value("result", json!({"status": "success"}));
    /// ```
    pub fn value(name: impl Into<String>, value: Value) -> Self {
        Self {
            outputs: vec![ActivityOutput::value(name, value)],
            ..Default::default()
        }
    }

    /// Create a result with multiple named outputs.
    ///
    /// ```
    /// use kruxiaflow_worker::{ActivityOutput, ActivityResult};
    /// use serde_json::json;
    ///
    /// let result = ActivityResult::values(vec![
    ///     ActivityOutput::value("status", json!("success")),
    ///     ActivityOutput::value("count", json!(42)),
    /// ]);
    /// ```
    pub fn values(outputs: Vec<ActivityOutput>) -> Self {
        Self {
            outputs,
            ..Default::default()
        }
    }

    /// Attach cost not covered by usage entries.
    ///
    /// ```
    /// use kruxiaflow_worker::ActivityResult;
    /// use rust_decimal::Decimal;
    /// use serde_json::json;
    /// use std::str::FromStr;
    ///
    /// let result = ActivityResult::value("result", json!({"data": "..."}))
    ///     .with_cost(Decimal::from_str("0.05").unwrap());
    /// ```
    pub fn with_cost(mut self, cost_usd: Decimal) -> Self {
        self.cost_usd = Some(cost_usd);
        self
    }

    /// Attach per-LLM-call usage entries.
    ///
    /// ```
    /// use kruxiaflow_worker::{ActivityResult, UsageEntry};
    /// use serde_json::json;
    ///
    /// let result = ActivityResult::value("summary", json!("..."))
    ///     .with_usage(vec![
    ///         UsageEntry::new("anthropic", "claude-sonnet-5")
    ///             .input_tokens(12034)
    ///             .output_tokens(512),
    ///     ]);
    /// ```
    pub fn with_usage(mut self, usage: Vec<UsageEntry>) -> Self {
        self.usage = usage;
        self
    }

    /// Append a single usage entry.
    pub fn push_usage(mut self, entry: UsageEntry) -> Self {
        self.usage.push(entry);
        self
    }

    /// Attach metadata.
    pub fn with_metadata(mut self, metadata: Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Convert value-type outputs to the flat JSON object the completion API
    /// expects. File and folder outputs are omitted.
    pub fn to_json_value(&self) -> Value {
        let mut map = serde_json::Map::new();
        for output in &self.outputs {
            if output.output_type == OutputType::Value {
                map.insert(output.name.clone(), output.value.clone());
            }
        }
        Value::Object(map)
    }

    /// Get the number of outputs.
    pub fn output_count(&self) -> usize {
        self.outputs.len()
    }

    /// Check if the result has an output with this name.
    pub fn has_output(&self, name: &str) -> bool {
        self.outputs.iter().any(|o| o.name == name)
    }

    /// Get an output by name.
    pub fn get_output(&self, name: &str) -> Option<&ActivityOutput> {
        self.outputs.iter().find(|o| o.name == name)
    }

    /// Get all value-type outputs.
    pub fn value_outputs(&self) -> Vec<&ActivityOutput> {
        self.outputs
            .iter()
            .filter(|o| o.output_type == OutputType::Value)
            .collect()
    }

    /// Get all file-type outputs.
    pub fn file_outputs(&self) -> Vec<&ActivityOutput> {
        self.outputs
            .iter()
            .filter(|o| o.output_type == OutputType::File)
            .collect()
    }

    /// Get all folder-type outputs.
    pub fn folder_outputs(&self) -> Vec<&ActivityOutput> {
        self.outputs
            .iter()
            .filter(|o| o.output_type == OutputType::Folder)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use serde_json::json;

    #[test]
    fn value_output() {
        let result = ActivityResult::value("result", json!({"status": "success"}));
        assert_eq!(result.outputs.len(), 1);
        assert_eq!(result.outputs[0].name, "result");
        assert_eq!(result.cost_usd, None);
        assert!(result.usage.is_empty());
    }

    #[test]
    fn to_json_value_filters_files() {
        let result = ActivityResult::values(vec![
            ActivityOutput::value("status", json!("success")),
            ActivityOutput::file("document", "postgres://wf/act/file.pdf"),
            ActivityOutput::value("count", json!(42)),
        ]);
        assert_eq!(
            result.to_json_value(),
            json!({"status": "success", "count": 42})
        );
    }

    #[test]
    fn with_cost_and_usage() {
        let result = ActivityResult::value("out", json!(1))
            .with_cost(dec!(0.02))
            .with_usage(vec![UsageEntry::new("anthropic", "claude-sonnet-5")])
            .push_usage(UsageEntry::new("openai", "gpt-5"));

        assert_eq!(result.cost_usd, Some(dec!(0.02)));
        assert_eq!(result.usage.len(), 2);
        assert_eq!(result.usage[1].provider, "openai");
    }

    #[test]
    fn output_accessors() {
        let result = ActivityResult::values(vec![
            ActivityOutput::value("status", json!("success")),
            ActivityOutput::file("document", "ref"),
            ActivityOutput::folder("out_dir", "ref/"),
        ]);
        assert_eq!(result.output_count(), 3);
        assert!(result.has_output("status"));
        assert!(!result.has_output("missing"));
        assert_eq!(result.get_output("document").unwrap().name, "document");
        assert_eq!(result.value_outputs().len(), 1);
        assert_eq!(result.file_outputs().len(), 1);
        assert_eq!(result.folder_outputs().len(), 1);
    }
}
