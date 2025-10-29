use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Activity to be scheduled
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Activity {
    pub key: String,
    pub namespace: String,
    pub name: String,
    pub parameters: serde_json::Value,
    pub settings: Option<ActivitySettings>,
    pub scheduled_for: Option<DateTime<Utc>>,
}

/// Activity settings (retry, timeout, budget config)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivitySettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_config: Option<RetryConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_config: Option<TimeoutConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_config: Option<BudgetConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_config: Option<CacheConfig>,
    #[serde(default = "default_deterministic")]
    pub deterministic: bool,
}

fn default_deterministic() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    pub max_attempts: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backoff_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeoutConfig {
    pub timeout_seconds: u64,
    #[serde(default)]
    pub enable_heartbeat: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heartbeat_interval_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetConfig {
    pub max_cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_seconds: Option<u64>,
}

/// Queued activity returned to worker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedActivity {
    pub id: Uuid,
    pub workflow_id: Uuid,
    pub activity_key: String,
    pub namespace: String,
    pub name: String,
    pub parameters: serde_json::Value,
    pub settings: Option<ActivitySettings>,
    pub retry_count: i32,
    pub claimed_at: DateTime<Utc>,
}

/// Activity result from worker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outputs: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_usage: Option<TokenUsage>,
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
