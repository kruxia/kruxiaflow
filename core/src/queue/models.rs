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
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

/// Activity status enum (matches database enum)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "activity_status", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum ActivityStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

impl std::fmt::Display for ActivityStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActivityStatus::Pending => write!(f, "pending"),
            ActivityStatus::Running => write!(f, "running"),
            ActivityStatus::Completed => write!(f, "completed"),
            ActivityStatus::Failed => write!(f, "failed"),
        }
    }
}
