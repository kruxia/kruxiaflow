use rust_decimal::Decimal;
use serde_json::Value;
use streamflow_core::workflow::{ActivityOutput, OutputType};

/// Activity execution result
///
/// This struct wraps the outputs of an activity execution along with
/// optional cost tracking information.
#[derive(Debug, Clone)]
pub struct ActivityResult {
    /// Structured outputs with type information
    pub outputs: Vec<ActivityOutput>,

    /// Optional cost tracking in USD
    pub cost_usd: Option<Decimal>,
}

impl ActivityResult {
    /// Create a result with a single value output
    ///
    /// This is the most common case - an activity returns a single output value.
    ///
    /// # Example
    /// ```
    /// use serde_json::json;
    /// use streamflow_worker::ActivityResult;
    ///
    /// let result = ActivityResult::value("result", json!({"status": "success"}));
    /// ```
    pub fn value(name: impl Into<String>, value: Value) -> Self {
        Self {
            outputs: vec![ActivityOutput::value(name, value)],
            cost_usd: None,
        }
    }

    /// Create a result with multiple outputs
    ///
    /// Use this when an activity produces multiple named outputs.
    ///
    /// # Example
    /// ```
    /// use streamflow_core::workflow::ActivityOutput;
    /// use streamflow_worker::ActivityResult;
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
            cost_usd: None,
        }
    }

    /// Add cost tracking to this result
    ///
    /// # Example
    /// ```
    /// use serde_json::json;
    /// use streamflow_worker::ActivityResult;
    /// use rust_decimal::Decimal;
    /// use std::str::FromStr;
    ///
    /// let result = ActivityResult::value("result", json!({"data": "..."}))
    ///     .with_cost(Decimal::from_str("0.05").unwrap()); // $0.05
    /// ```
    pub fn with_cost(mut self, cost_usd: Decimal) -> Self {
        self.cost_usd = Some(cost_usd);
        self
    }

    /// Convert to JSON value output format (single JSON object)
    ///
    /// This is used for backward compatibility with the current API.
    /// It converts Vec<ActivityOutput> to a single JSON object where
    /// each output becomes a key-value pair.
    ///
    /// Only Value-type outputs are included in the JSON value format.
    /// File and Folder outputs are omitted since the old format doesn't support them.
    ///
    /// # Example
    /// ```
    /// use streamflow_core::workflow::ActivityOutput;
    /// use streamflow_worker::ActivityResult;
    /// use serde_json::json;
    ///
    /// let result = ActivityResult::values(vec![
    ///     ActivityOutput::value("status", json!("success")),
    ///     ActivityOutput::value("count", json!(42)),
    ///     ActivityOutput::file("document", "postgres://wf/act/file.pdf"),
    /// ]);
    ///
    /// let value = result.to_json_value();
    /// // Returns: {"status": "success", "count": 42}
    /// // File output is omitted
    /// ```
    pub fn to_json_value(&self) -> Value {
        let mut map = serde_json::Map::new();

        for output in &self.outputs {
            // Only include Value-type outputs in JSON value format
            if output.output_type == OutputType::Value {
                map.insert(output.name.clone(), output.value.clone());
            }
        }

        Value::Object(map)
    }

    /// Get the number of outputs
    pub fn output_count(&self) -> usize {
        self.outputs.len()
    }

    /// Check if result has a specific output
    pub fn has_output(&self, name: &str) -> bool {
        self.outputs.iter().any(|o| o.name == name)
    }

    /// Get an output by name
    pub fn get_output(&self, name: &str) -> Option<&ActivityOutput> {
        self.outputs.iter().find(|o| o.name == name)
    }

    /// Get all value-type outputs
    pub fn value_outputs(&self) -> Vec<&ActivityOutput> {
        self.outputs
            .iter()
            .filter(|o| o.output_type == OutputType::Value)
            .collect()
    }

    /// Get all file-type outputs
    pub fn file_outputs(&self) -> Vec<&ActivityOutput> {
        self.outputs
            .iter()
            .filter(|o| o.output_type == OutputType::File)
            .collect()
    }

    /// Get all folder-type outputs
    pub fn folder_outputs(&self) -> Vec<&ActivityOutput> {
        self.outputs
            .iter()
            .filter(|o| o.output_type == OutputType::Folder)
            .collect()
    }
}

impl Default for ActivityResult {
    fn default() -> Self {
        Self {
            outputs: Vec::new(),
            cost_usd: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_value_output() {
        let result = ActivityResult::value("result", json!({"status": "success"}));

        assert_eq!(result.outputs.len(), 1);
        assert_eq!(result.outputs[0].name, "result");
        assert_eq!(result.outputs[0].output_type, OutputType::Value);
        assert_eq!(result.outputs[0].value, json!({"status": "success"}));
        assert_eq!(result.cost_usd, None);
    }

    #[test]
    fn test_values_output() {
        let result = ActivityResult::values(vec![
            ActivityOutput::value("status", json!("success")),
            ActivityOutput::value("count", json!(42)),
        ]);

        assert_eq!(result.outputs.len(), 2);
        assert_eq!(result.outputs[0].name, "status");
        assert_eq!(result.outputs[1].name, "count");
    }

    #[test]
    fn test_with_cost() {
        use std::str::FromStr;
        let result = ActivityResult::value("result", json!({}))
            .with_cost(Decimal::from_str("0.05").unwrap());

        assert_eq!(result.cost_usd, Some(Decimal::from_str("0.05").unwrap()));
    }

    #[test]
    fn test_to_legacy_output() {
        let result = ActivityResult::values(vec![
            ActivityOutput::value("status", json!("success")),
            ActivityOutput::value("count", json!(42)),
        ]);

        let value = result.to_json_value();
        assert_eq!(value, json!({"status": "success", "count": 42}));
    }

    #[test]
    fn test_to_legacy_output_filters_files() {
        let result = ActivityResult::values(vec![
            ActivityOutput::value("status", json!("success")),
            ActivityOutput::file("document", "postgres://wf/act/file.pdf"),
            ActivityOutput::value("count", json!(42)),
        ]);

        let value = result.to_json_value();
        // File output should be filtered out in JSON value format
        assert_eq!(value, json!({"status": "success", "count": 42}));
    }

    #[test]
    fn test_has_output() {
        let result = ActivityResult::values(vec![
            ActivityOutput::value("status", json!("success")),
            ActivityOutput::value("count", json!(42)),
        ]);

        assert!(result.has_output("status"));
        assert!(result.has_output("count"));
        assert!(!result.has_output("missing"));
    }

    #[test]
    fn test_get_output() {
        let result = ActivityResult::values(vec![
            ActivityOutput::value("status", json!("success")),
            ActivityOutput::value("count", json!(42)),
        ]);

        let status = result.get_output("status").unwrap();
        assert_eq!(status.value, json!("success"));

        assert!(result.get_output("missing").is_none());
    }

    #[test]
    fn test_output_type_filters() {
        let result = ActivityResult::values(vec![
            ActivityOutput::value("status", json!("success")),
            ActivityOutput::file("document", "postgres://wf/act/file.pdf"),
            ActivityOutput::folder("output_dir", "postgres://wf/act/output/"),
            ActivityOutput::value("count", json!(42)),
        ]);

        assert_eq!(result.value_outputs().len(), 2);
        assert_eq!(result.file_outputs().len(), 1);
        assert_eq!(result.folder_outputs().len(), 1);
    }
}
