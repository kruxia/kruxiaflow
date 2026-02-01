use crate::workflow::outputs::{ActivityOutput, ActivityOutputDefinition};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// Re-export ActivitySettings from workflow module for backward compatibility
pub use crate::workflow::ActivitySettings;

/// Activity to be scheduled
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Activity {
    pub key: String,
    pub worker: String,
    pub activity_name: String,
    pub parameters: serde_json::Value,
    pub settings: Option<ActivitySettings>,
    pub scheduled_for: Option<DateTime<Utc>>,
    /// Output definitions from workflow definition (for file handling)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_definitions: Option<Vec<ActivityOutputDefinition>>,
    /// Iteration number for looping activities (0-based)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iteration: Option<i32>,
    /// Signal data for activities that were waiting for an external signal
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_data: Option<serde_json::Value>,
}

/// Queued activity returned to worker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedActivity {
    pub id: Uuid,
    pub workflow_id: Uuid,
    pub activity_key: String,
    pub worker: String,
    pub activity_name: String,
    pub parameters: serde_json::Value,
    pub settings: Option<ActivitySettings>,
    pub retry_count: i32,
    pub claimed_at: DateTime<Utc>,
    /// Output definitions from workflow definition (for file handling)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_definitions: Option<Vec<ActivityOutputDefinition>>,
    /// Iteration number for looping activities (0-based)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iteration: Option<i32>,
    /// Signal data for activities that were waiting for an external signal
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_data: Option<serde_json::Value>,
}

/// Activity result from worker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityResult {
    pub success: bool,
    /// Structured outputs with types (Value, File, or Folder)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outputs: Option<Vec<ActivityOutput>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_usage: Option<TokenUsage>,
}

impl ActivityResult {
    /// Create a successful result with value outputs
    pub fn success(outputs: serde_json::Value) -> Self {
        // Convert JSON object to Vec<ActivityOutput> with type Value
        let outputs_vec = if let serde_json::Value::Object(map) = outputs {
            Some(
                map.into_iter()
                    .map(|(k, v)| ActivityOutput::value(k, v))
                    .collect(),
            )
        } else {
            None
        };

        Self {
            success: true,
            outputs: outputs_vec,
            error: None,
            cost_usd: None,
            token_usage: None,
        }
    }

    /// Create a failure result
    pub fn failure(error: impl Into<String>) -> Self {
        Self {
            success: false,
            outputs: None,
            error: Some(error.into()),
            cost_usd: None,
            token_usage: None,
        }
    }

    /// Create a successful result with structured outputs
    pub fn with_outputs(outputs: Vec<ActivityOutput>) -> Self {
        Self {
            success: true,
            outputs: Some(outputs),
            error: None,
            cost_usd: None,
            token_usage: None,
        }
    }

    /// Add cost tracking
    pub fn with_cost(mut self, cost_usd: Decimal, token_usage: Option<TokenUsage>) -> Self {
        self.cost_usd = Some(cost_usd);
        self.token_usage = token_usage;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

/// Activity status enum (matches database enum)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "activity_status", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum ActivityStatus {
    Waiting,
    Pending,
    Running,
    Completed,
    Failed,
}

impl std::fmt::Display for ActivityStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActivityStatus::Waiting => write!(f, "waiting"),
            ActivityStatus::Pending => write!(f, "pending"),
            ActivityStatus::Running => write!(f, "running"),
            ActivityStatus::Completed => write!(f, "completed"),
            ActivityStatus::Failed => write!(f, "failed"),
        }
    }
}

/// Result of reclaiming a stale activity
#[derive(Debug, Clone)]
pub enum StaleActivityAction {
    /// Activity was reset to pending for retry
    ResetToPending,
    /// Activity was marked as failed (retries exhausted)
    MarkedFailed,
}

/// Information about a stale activity that was reclaimed
#[derive(Debug, Clone)]
pub struct StaleActivityInfo {
    pub workflow_id: Uuid,
    pub activity_key: String,
    pub iteration: Option<i32>,
    pub action: StaleActivityAction,
    pub retry_count: i32,
    pub max_retries: i32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // =========================================================================
    // ActivityResult tests
    // =========================================================================

    #[test]
    fn test_activity_result_success() {
        let result = ActivityResult::success(json!({"answer": 42}));

        assert!(result.success);
        assert!(result.outputs.is_some());
        assert!(result.error.is_none());
        assert!(result.cost_usd.is_none());
        assert!(result.token_usage.is_none());
    }

    #[test]
    fn test_activity_result_success_converts_to_outputs() {
        let result = ActivityResult::success(json!({
            "name": "test",
            "value": 123
        }));

        let outputs = result.outputs.unwrap();
        assert_eq!(outputs.len(), 2);

        // Check both outputs are created with Value type
        let output_names: Vec<&str> = outputs.iter().map(|o| o.name.as_str()).collect();
        assert!(output_names.contains(&"name"));
        assert!(output_names.contains(&"value"));
    }

    #[test]
    fn test_activity_result_success_non_object_input() {
        // Non-object JSON should result in None outputs
        let result = ActivityResult::success(json!(["array", "value"]));

        assert!(result.success);
        assert!(result.outputs.is_none()); // Non-object doesn't convert
    }

    #[test]
    fn test_activity_result_failure() {
        let result = ActivityResult::failure("Something went wrong");

        assert!(!result.success);
        assert!(result.outputs.is_none());
        assert_eq!(result.error, Some("Something went wrong".to_string()));
        assert!(result.cost_usd.is_none());
    }

    #[test]
    fn test_activity_result_failure_with_string() {
        let result = ActivityResult::failure(String::from("Error message"));

        assert!(!result.success);
        assert_eq!(result.error, Some("Error message".to_string()));
    }

    #[test]
    fn test_activity_result_with_outputs() {
        let outputs = vec![
            ActivityOutput::value("result", json!("success")),
            ActivityOutput::file("report", "postgres://uuid/step/file.pdf"),
        ];

        let result = ActivityResult::with_outputs(outputs);

        assert!(result.success);
        assert!(result.outputs.is_some());
        assert_eq!(result.outputs.unwrap().len(), 2);
        assert!(result.error.is_none());
    }

    #[test]
    fn test_activity_result_with_cost() {
        let token_usage = TokenUsage {
            prompt_tokens: 100,
            output_tokens: 50,
            total_tokens: 150,
        };

        let result = ActivityResult::success(json!({"answer": "response"}))
            .with_cost(Decimal::new(1234, 6), Some(token_usage));

        assert!(result.success);
        assert_eq!(result.cost_usd, Some(Decimal::new(1234, 6))); // 0.001234
        assert!(result.token_usage.is_some());

        let usage = result.token_usage.unwrap();
        assert_eq!(usage.prompt_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.total_tokens, 150);
    }

    #[test]
    fn test_activity_result_with_cost_no_tokens() {
        let result =
            ActivityResult::success(json!({"data": "value"})).with_cost(Decimal::new(50, 4), None);

        assert!(result.success);
        assert_eq!(result.cost_usd, Some(Decimal::new(50, 4))); // 0.0050
        assert!(result.token_usage.is_none());
    }

    #[test]
    fn test_activity_result_serialization_success() {
        let result = ActivityResult::success(json!({"key": "value"}));
        let json = serde_json::to_string(&result).unwrap();

        assert!(json.contains("\"success\":true"));
        assert!(!json.contains("error")); // Should skip null fields
    }

    #[test]
    fn test_activity_result_serialization_failure() {
        let result = ActivityResult::failure("Test error");
        let json = serde_json::to_string(&result).unwrap();

        assert!(json.contains("\"success\":false"));
        assert!(json.contains("Test error"));
    }

    #[test]
    fn test_activity_result_deserialization() {
        let json = r#"{
            "success": true,
            "outputs": [{"name": "result", "value": "data", "type": "value"}],
            "cost_usd": "0.001"
        }"#;

        let result: ActivityResult = serde_json::from_str(json).unwrap();
        assert!(result.success);
        assert!(result.outputs.is_some());
        assert_eq!(result.cost_usd, Some(Decimal::new(1, 3)));
    }

    // =========================================================================
    // ActivityStatus tests
    // =========================================================================

    #[test]
    fn test_activity_status_display_waiting() {
        assert_eq!(ActivityStatus::Waiting.to_string(), "waiting");
    }

    #[test]
    fn test_activity_status_display_pending() {
        assert_eq!(ActivityStatus::Pending.to_string(), "pending");
    }

    #[test]
    fn test_activity_status_display_running() {
        assert_eq!(ActivityStatus::Running.to_string(), "running");
    }

    #[test]
    fn test_activity_status_display_completed() {
        assert_eq!(ActivityStatus::Completed.to_string(), "completed");
    }

    #[test]
    fn test_activity_status_display_failed() {
        assert_eq!(ActivityStatus::Failed.to_string(), "failed");
    }

    #[test]
    fn test_activity_status_serialization() {
        assert_eq!(
            serde_json::to_string(&ActivityStatus::Waiting).unwrap(),
            "\"waiting\""
        );
        assert_eq!(
            serde_json::to_string(&ActivityStatus::Pending).unwrap(),
            "\"pending\""
        );
        assert_eq!(
            serde_json::to_string(&ActivityStatus::Running).unwrap(),
            "\"running\""
        );
        assert_eq!(
            serde_json::to_string(&ActivityStatus::Completed).unwrap(),
            "\"completed\""
        );
        assert_eq!(
            serde_json::to_string(&ActivityStatus::Failed).unwrap(),
            "\"failed\""
        );
    }

    #[test]
    fn test_activity_status_deserialization() {
        assert_eq!(
            serde_json::from_str::<ActivityStatus>("\"waiting\"").unwrap(),
            ActivityStatus::Waiting
        );
        assert_eq!(
            serde_json::from_str::<ActivityStatus>("\"pending\"").unwrap(),
            ActivityStatus::Pending
        );
        assert_eq!(
            serde_json::from_str::<ActivityStatus>("\"running\"").unwrap(),
            ActivityStatus::Running
        );
        assert_eq!(
            serde_json::from_str::<ActivityStatus>("\"completed\"").unwrap(),
            ActivityStatus::Completed
        );
        assert_eq!(
            serde_json::from_str::<ActivityStatus>("\"failed\"").unwrap(),
            ActivityStatus::Failed
        );
    }

    #[test]
    fn test_activity_status_equality() {
        assert_eq!(ActivityStatus::Pending, ActivityStatus::Pending);
        assert_ne!(ActivityStatus::Pending, ActivityStatus::Running);
    }

    #[test]
    fn test_activity_status_copy() {
        let status = ActivityStatus::Completed;
        let copied = status; // Copy
        assert_eq!(status, copied);
    }

    // =========================================================================
    // TokenUsage tests
    // =========================================================================

    #[test]
    fn test_token_usage_serialization() {
        let usage = TokenUsage {
            prompt_tokens: 500,
            output_tokens: 200,
            total_tokens: 700,
        };

        let json = serde_json::to_string(&usage).unwrap();
        assert!(json.contains("\"prompt_tokens\":500"));
        assert!(json.contains("\"output_tokens\":200"));
        assert!(json.contains("\"total_tokens\":700"));
    }

    #[test]
    fn test_token_usage_deserialization() {
        let json = r#"{"prompt_tokens": 1000, "output_tokens": 300, "total_tokens": 1300}"#;
        let usage: TokenUsage = serde_json::from_str(json).unwrap();

        assert_eq!(usage.prompt_tokens, 1000);
        assert_eq!(usage.output_tokens, 300);
        assert_eq!(usage.total_tokens, 1300);
    }

    #[test]
    fn test_token_usage_clone() {
        let usage = TokenUsage {
            prompt_tokens: 100,
            output_tokens: 50,
            total_tokens: 150,
        };

        let cloned = usage.clone();
        assert_eq!(cloned.prompt_tokens, usage.prompt_tokens);
        assert_eq!(cloned.output_tokens, usage.output_tokens);
        assert_eq!(cloned.total_tokens, usage.total_tokens);
    }

    // =========================================================================
    // Activity and QueuedActivity tests
    // =========================================================================

    #[test]
    fn test_activity_serialization() {
        let activity = Activity {
            key: "process_data".to_string(),
            worker: "data".to_string(),
            activity_name: "transform".to_string(),
            parameters: json!({"input": "value"}),
            settings: None,
            scheduled_for: None,
            output_definitions: None,
            iteration: None,
            signal_data: None,
        };

        let json = serde_json::to_string(&activity).unwrap();
        assert!(json.contains("process_data"));
        assert!(json.contains("transform"));
    }

    #[test]
    fn test_activity_with_iteration() {
        let activity = Activity {
            key: "loop_step".to_string(),
            worker: "processor".to_string(),
            activity_name: "batch".to_string(),
            parameters: json!({}),
            settings: None,
            scheduled_for: None,
            output_definitions: None,
            iteration: Some(5),
            signal_data: None,
        };

        let json = serde_json::to_string(&activity).unwrap();
        assert!(json.contains("\"iteration\":5"));
    }

    #[test]
    fn test_queued_activity_serialization() {
        let queued = QueuedActivity {
            id: Uuid::nil(),
            workflow_id: Uuid::nil(),
            activity_key: "step1".to_string(),
            worker: "test".to_string(),
            activity_name: "action".to_string(),
            parameters: json!({"key": "value"}),
            settings: None,
            retry_count: 2,
            claimed_at: Utc::now(),
            output_definitions: None,
            iteration: None,
            signal_data: None,
        };

        let json = serde_json::to_string(&queued).unwrap();
        assert!(json.contains("step1"));
        assert!(json.contains("\"retry_count\":2"));
    }
}
