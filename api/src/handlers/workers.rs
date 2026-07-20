use crate::error::{ApiResult, AppError, ValidationErrors};
use crate::middleware::auth::ValidatedClaims;
use crate::state::AppState;
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use kruxiaflow_core::activity::{ActivityWorkerError, ActivityWorkerService};
use kruxiaflow_core::cost::CostCalculator;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Poll for activities request
///
/// Workers poll for pending activities by specifying which worker type
/// they handle. The API returns activities for that worker, ordered by
/// scheduled_for for fair scheduling across all activity types.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PollActivitiesRequest {
    /// Worker type this worker handles (e.g., "std", "custom")
    #[schema(example = "std")]
    pub worker: String,

    /// Worker instance ID (for tracking which worker claimed the activity)
    #[schema(example = "worker_payments_01")]
    pub worker_id: String,

    /// Maximum number of activities to return (default 1, max 100)
    #[serde(default = "default_max_activities")]
    pub max_activities: usize,
}

fn default_max_activities() -> usize {
    1
}

impl PollActivitiesRequest {
    /// Validate request structure
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();

        if self.worker.is_empty() {
            errors.add("worker", "Worker type cannot be empty");
        }

        if self.worker_id.is_empty() {
            errors.add("worker_id", "Worker ID cannot be empty");
        }

        if self.max_activities == 0 {
            errors.add("max_activities", "max_activities must be at least 1");
        }
        if self.max_activities > 100 {
            errors.add("max_activities", "max_activities must be at most 100");
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Activity for worker execution
#[derive(Debug, Serialize, ToSchema)]
pub struct PendingActivity {
    /// Unique activity ID
    #[schema(example = "550e8400-e29b-41d4-a716-446655440000")]
    pub activity_id: Uuid,

    /// Workflow ID this activity belongs to
    #[schema(example = "660e8400-e29b-41d4-a716-446655440001")]
    pub workflow_id: Uuid,

    /// Activity key (unique within workflow)
    #[schema(example = "authorize_card")]
    pub activity_key: String,

    /// Activity worker type
    #[schema(example = "std")]
    pub worker: String,

    /// Activity name
    #[schema(example = "http_request")]
    pub activity_name: String,

    /// Activity input parameters
    #[schema(example = json!({"card_token": "tok_123", "amount": 100.00}))]
    pub parameters: serde_json::Value,

    /// Activity settings (timeout, retry, etc.)
    #[schema(example = json!({"timeout": 300, "max_retries": 3}))]
    pub settings: Option<serde_json::Value>,

    /// Timeout in seconds (extracted from settings for convenience)
    #[schema(example = 300)]
    pub timeout_seconds: Option<i64>,

    /// Output definitions (for file outputs)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!([{"name": "document", "type": "file"}]))]
    pub output_definitions: Option<serde_json::Value>,

    /// Signal data (for activities that were waiting for a signal)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!({"approved": true, "approver": "admin@example.com"}))]
    pub signal_data: Option<serde_json::Value>,
}

/// Poll for activities response
#[derive(Debug, Serialize, ToSchema)]
pub struct PollActivitiesResponse {
    /// List of pending activities (may be empty if none available)
    pub activities: Vec<PendingActivity>,

    /// Number of activities returned
    pub count: usize,
}

/// Activity heartbeat request
#[derive(Debug, Deserialize, ToSchema)]
pub struct ActivityHeartbeatRequest {
    /// Worker instance ID (must match the worker that claimed the activity)
    #[schema(example = "worker_payments_01")]
    pub worker_id: String,
}

impl ActivityHeartbeatRequest {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();

        if self.worker_id.is_empty() {
            errors.add("worker_id", "Worker ID cannot be empty");
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Activity heartbeat response
#[derive(Debug, Serialize, ToSchema)]
pub struct ActivityHeartbeatResponse {
    /// Heartbeat acknowledged
    #[schema(example = true)]
    pub acknowledged: bool,

    /// Recommended seconds until next heartbeat
    #[schema(example = 30)]
    pub next_heartbeat_seconds: i64,
}

/// Per-LLM-call usage reported by an external activity.
///
/// Field names are frozen as part of the worker-API contract (worker SDK spec).
/// `input_tokens` is the full prompt size including cache reads. When `cost_usd`
/// is omitted the server computes the cost from the `llm_models` catalog
/// (cache reads at the cached-input price, cache creation at the cache-write
/// price, falling back to the input price); an unknown provider/model records
/// the entry at cost 0 with a warning — completion never fails because of
/// usage metadata.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct UsageEntry {
    /// LLM provider name (matches the llm_models catalog)
    #[schema(example = "anthropic")]
    pub provider: String,

    /// Model name (matches the llm_models catalog)
    #[schema(example = "claude-sonnet-5")]
    pub model: String,

    /// Prompt tokens, including cache reads
    #[serde(default)]
    #[schema(example = 12034)]
    pub input_tokens: u32,

    /// Completion tokens
    #[serde(default)]
    #[schema(example = 512)]
    pub output_tokens: u32,

    /// Prompt tokens served from cache (billed at the cached-input price)
    #[serde(default)]
    #[schema(example = 9800)]
    pub cache_read_tokens: u32,

    /// Tokens written to cache (billed at the catalog's cache-write price,
    /// falling back to the input price for models without one)
    #[serde(default)]
    #[schema(example = 0)]
    pub cache_creation_tokens: u32,

    /// Context-cache storage consumed, in token-hours (tokens held x hours
    /// held; fractional). Billed at the catalog's cache-storage price (e.g.,
    /// Gemini explicit-caching storage); models without one record the
    /// component at 0 with a warning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 3.33)]
    pub cache_storage_token_hours: Option<Decimal>,

    /// Explicit cost for this call; overrides server-side computation
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 0.015)]
    pub cost_usd: Option<Decimal>,
}

impl UsageEntry {
    fn validate(&self, index: usize, errors: &mut ValidationErrors) {
        if self.provider.is_empty() {
            errors.add(
                format!("usage[{}].provider", index),
                "Provider cannot be empty",
            );
        }
        if self.model.is_empty() {
            errors.add(format!("usage[{}].model", index), "Model cannot be empty");
        }
        if let Some(cost) = self.cost_usd
            && cost < Decimal::ZERO
        {
            errors.add(
                format!("usage[{}].cost_usd", index),
                "Cost must be non-negative",
            );
        }
        if let Some(token_hours) = self.cache_storage_token_hours
            && token_hours < Decimal::ZERO
        {
            errors.add(
                format!("usage[{}].cache_storage_token_hours", index),
                "Cache storage token-hours must be non-negative",
            );
        }
    }
}

impl From<UsageEntry> for kruxiaflow_core::cost::UsageEntry {
    fn from(entry: UsageEntry) -> Self {
        Self {
            provider: entry.provider,
            model: entry.model,
            input_tokens: entry.input_tokens,
            output_tokens: entry.output_tokens,
            cache_read_tokens: entry.cache_read_tokens,
            cache_creation_tokens: entry.cache_creation_tokens,
            cache_storage_token_hours: entry.cache_storage_token_hours,
            cost_usd: entry.cost_usd,
        }
    }
}

/// Activity completion request
#[derive(Debug, Deserialize, ToSchema)]
pub struct CompleteActivityRequest {
    /// Worker instance ID (must match the worker that claimed the activity)
    #[schema(example = "worker_payments_01")]
    pub worker_id: String,

    /// Activity output (result of execution)
    #[schema(example = json!({"authorization_id": "auth_123", "approved": true}))]
    pub output: serde_json::Value,

    /// Cost in USD. Without `usage` entries: total activity cost.
    /// With `usage` entries: cost NOT covered by the entries (e.g., a paid
    /// non-LLM API) — never repeat entry costs here.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 0.015)]
    pub cost_usd: Option<Decimal>,

    /// Per-LLM-call usage made inside the activity, one entry per call
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Vec<UsageEntry>>,
}

impl CompleteActivityRequest {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();

        if self.worker_id.is_empty() {
            errors.add("worker_id", "Worker ID cannot be empty");
        }

        // Validate output is an object
        if !self.output.is_object() {
            errors.add("output", "Output must be a JSON object");
        }

        // Validate cost_usd if provided
        if let Some(cost) = self.cost_usd
            && cost < Decimal::ZERO
        {
            errors.add("cost_usd", "Cost must be non-negative");
        }

        // Validate usage entries if provided
        if let Some(usage) = &self.usage {
            for (index, entry) in usage.iter().enumerate() {
                entry.validate(index, &mut errors);
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Activity completion response
#[derive(Debug, Serialize, ToSchema)]
pub struct CompleteActivityResponse {
    /// Completion acknowledged
    #[schema(example = true)]
    pub acknowledged: bool,

    /// Non-fatal warnings about reported usage (e.g., unknown provider/model
    /// recorded at cost 0)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Activity error details
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ActivityError {
    /// Error code (for categorization)
    #[schema(example = "PAYMENT_DECLINED")]
    pub code: String,

    /// Error message (human-readable)
    #[schema(example = "Card was declined by the bank")]
    pub message: String,

    /// Whether this error is retryable
    #[schema(example = false)]
    pub retryable: bool,
}

/// Activity failure request
#[derive(Debug, Deserialize, ToSchema)]
pub struct FailActivityRequest {
    /// Worker instance ID (must match the worker that claimed the activity)
    #[schema(example = "worker_payments_01")]
    pub worker_id: String,

    /// Error details
    pub error: ActivityError,

    /// Cost in USD spent before the failure. Without `usage` entries: total
    /// attempt cost. With `usage` entries: cost NOT covered by the entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 0.015)]
    pub cost_usd: Option<Decimal>,

    /// Per-LLM-call usage made inside the activity before it failed —
    /// a failed attempt still spent the money and counts against budgets
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Vec<UsageEntry>>,
}

impl FailActivityRequest {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();

        if self.worker_id.is_empty() {
            errors.add("worker_id", "Worker ID cannot be empty");
        }

        if self.error.code.is_empty() {
            errors.add("error.code", "Error code cannot be empty");
        }

        if self.error.message.is_empty() {
            errors.add("error.message", "Error message cannot be empty");
        }

        if let Some(cost) = self.cost_usd
            && cost < Decimal::ZERO
        {
            errors.add("cost_usd", "Cost must be non-negative");
        }

        if let Some(usage) = &self.usage {
            for (index, entry) in usage.iter().enumerate() {
                entry.validate(index, &mut errors);
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Activity failure response
#[derive(Debug, Serialize, ToSchema)]
pub struct FailActivityResponse {
    /// Failure acknowledged
    #[schema(example = true)]
    pub acknowledged: bool,

    /// Whether the activity will be retried
    #[schema(example = true)]
    pub will_retry: bool,

    /// Non-fatal warnings about reported usage (e.g., unknown provider/model
    /// recorded at cost 0)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    // --- PollActivitiesRequest validation ---

    #[test]
    fn test_poll_request_valid() {
        let req = PollActivitiesRequest {
            worker: "std".to_string(),
            worker_id: "worker_01".to_string(),
            max_activities: 5,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_poll_request_empty_worker() {
        let req = PollActivitiesRequest {
            worker: "".to_string(),
            worker_id: "worker_01".to_string(),
            max_activities: 1,
        };
        let err = req.validate().unwrap_err();
        assert!(err.field_errors.contains_key("worker"));
    }

    #[test]
    fn test_poll_request_empty_worker_id() {
        let req = PollActivitiesRequest {
            worker: "std".to_string(),
            worker_id: "".to_string(),
            max_activities: 1,
        };
        let err = req.validate().unwrap_err();
        assert!(err.field_errors.contains_key("worker_id"));
    }

    #[test]
    fn test_poll_request_zero_max_activities() {
        let req = PollActivitiesRequest {
            worker: "std".to_string(),
            worker_id: "w1".to_string(),
            max_activities: 0,
        };
        let err = req.validate().unwrap_err();
        assert!(err.field_errors.contains_key("max_activities"));
    }

    #[test]
    fn test_poll_request_over_100_max_activities() {
        let req = PollActivitiesRequest {
            worker: "std".to_string(),
            worker_id: "w1".to_string(),
            max_activities: 101,
        };
        let err = req.validate().unwrap_err();
        assert!(err.field_errors.contains_key("max_activities"));
    }

    #[test]
    fn test_poll_request_max_100_valid() {
        let req = PollActivitiesRequest {
            worker: "std".to_string(),
            worker_id: "w1".to_string(),
            max_activities: 100,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_poll_request_default_max_activities() {
        let json = r#"{"worker": "std", "worker_id": "w1"}"#;
        let req: PollActivitiesRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.max_activities, 1);
    }

    #[test]
    fn test_poll_request_multiple_errors() {
        let req = PollActivitiesRequest {
            worker: "".to_string(),
            worker_id: "".to_string(),
            max_activities: 0,
        };
        let err = req.validate().unwrap_err();
        assert!(err.field_errors.contains_key("worker"));
        assert!(err.field_errors.contains_key("worker_id"));
        assert!(err.field_errors.contains_key("max_activities"));
    }

    // --- ActivityHeartbeatRequest validation ---

    #[test]
    fn test_heartbeat_request_valid() {
        let req = ActivityHeartbeatRequest {
            worker_id: "worker_01".to_string(),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_heartbeat_request_empty_worker_id() {
        let req = ActivityHeartbeatRequest {
            worker_id: "".to_string(),
        };
        let err = req.validate().unwrap_err();
        assert!(err.field_errors.contains_key("worker_id"));
    }

    // --- CompleteActivityRequest validation ---

    #[test]
    fn test_complete_request_valid() {
        let req = CompleteActivityRequest {
            worker_id: "worker_01".to_string(),
            output: serde_json::json!({"result": "ok"}),
            cost_usd: Some(Decimal::from_str("0.015").unwrap()),
            usage: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_complete_request_empty_worker_id() {
        let req = CompleteActivityRequest {
            worker_id: "".to_string(),
            output: serde_json::json!({"result": "ok"}),
            cost_usd: None,
            usage: None,
        };
        let err = req.validate().unwrap_err();
        assert!(err.field_errors.contains_key("worker_id"));
    }

    #[test]
    fn test_complete_request_non_object_output() {
        let req = CompleteActivityRequest {
            worker_id: "w1".to_string(),
            output: serde_json::json!("just a string"),
            cost_usd: None,
            usage: None,
        };
        let err = req.validate().unwrap_err();
        assert!(err.field_errors.contains_key("output"));
    }

    #[test]
    fn test_complete_request_negative_cost() {
        let req = CompleteActivityRequest {
            worker_id: "w1".to_string(),
            output: serde_json::json!({"result": "ok"}),
            cost_usd: Some(Decimal::from_str("-1.00").unwrap()),
            usage: None,
        };
        let err = req.validate().unwrap_err();
        assert!(err.field_errors.contains_key("cost_usd"));
    }

    #[test]
    fn test_complete_request_zero_cost_valid() {
        let req = CompleteActivityRequest {
            worker_id: "w1".to_string(),
            output: serde_json::json!({"result": "ok"}),
            cost_usd: Some(Decimal::ZERO),
            usage: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_complete_request_no_cost_valid() {
        let req = CompleteActivityRequest {
            worker_id: "w1".to_string(),
            output: serde_json::json!({"result": "ok"}),
            cost_usd: None,
            usage: None,
        };
        assert!(req.validate().is_ok());
    }

    // --- FailActivityRequest validation ---

    #[test]
    fn test_fail_request_valid() {
        let req = FailActivityRequest {
            worker_id: "worker_01".to_string(),
            error: ActivityError {
                code: "TIMEOUT".to_string(),
                message: "Activity timed out".to_string(),
                retryable: true,
            },
            cost_usd: None,
            usage: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_fail_request_empty_worker_id() {
        let req = FailActivityRequest {
            worker_id: "".to_string(),
            error: ActivityError {
                code: "ERR".to_string(),
                message: "msg".to_string(),
                retryable: false,
            },
            cost_usd: None,
            usage: None,
        };
        let err = req.validate().unwrap_err();
        assert!(err.field_errors.contains_key("worker_id"));
    }

    #[test]
    fn test_fail_request_empty_error_code() {
        let req = FailActivityRequest {
            worker_id: "w1".to_string(),
            error: ActivityError {
                code: "".to_string(),
                message: "msg".to_string(),
                retryable: false,
            },
            cost_usd: None,
            usage: None,
        };
        let err = req.validate().unwrap_err();
        assert!(err.field_errors.contains_key("error.code"));
    }

    #[test]
    fn test_fail_request_empty_error_message() {
        let req = FailActivityRequest {
            worker_id: "w1".to_string(),
            error: ActivityError {
                code: "ERR".to_string(),
                message: "".to_string(),
                retryable: false,
            },
            cost_usd: None,
            usage: None,
        };
        let err = req.validate().unwrap_err();
        assert!(err.field_errors.contains_key("error.message"));
    }

    #[test]
    fn test_fail_request_all_empty() {
        let req = FailActivityRequest {
            worker_id: "".to_string(),
            error: ActivityError {
                code: "".to_string(),
                message: "".to_string(),
                retryable: false,
            },
            cost_usd: None,
            usage: None,
        };
        let err = req.validate().unwrap_err();
        assert!(err.field_errors.contains_key("worker_id"));
        assert!(err.field_errors.contains_key("error.code"));
        assert!(err.field_errors.contains_key("error.message"));
    }

    // --- Serialization tests ---

    #[test]
    fn test_pending_activity_serialize() {
        let activity = PendingActivity {
            activity_id: Uuid::nil(),
            workflow_id: Uuid::nil(),
            activity_key: "step1".to_string(),
            worker: "std".to_string(),
            activity_name: "http_request".to_string(),
            parameters: serde_json::json!({"url": "http://example.com"}),
            settings: None,
            timeout_seconds: Some(300),
            output_definitions: None,
            signal_data: None,
        };
        let json = serde_json::to_value(&activity).unwrap();
        assert_eq!(json["activity_key"], "step1");
        assert_eq!(json["timeout_seconds"], 300);
        // signal_data and output_definitions should be skipped
        assert!(json.get("signal_data").is_none());
        assert!(json.get("output_definitions").is_none());
    }

    #[test]
    fn test_pending_activity_with_signal_data() {
        let activity = PendingActivity {
            activity_id: Uuid::nil(),
            workflow_id: Uuid::nil(),
            activity_key: "step1".to_string(),
            worker: "std".to_string(),
            activity_name: "process".to_string(),
            parameters: serde_json::json!({}),
            settings: None,
            timeout_seconds: None,
            output_definitions: None,
            signal_data: Some(serde_json::json!({"approved": true})),
        };
        let json = serde_json::to_value(&activity).unwrap();
        assert_eq!(json["signal_data"]["approved"], true);
    }

    #[test]
    fn test_poll_response_serialize() {
        let response = PollActivitiesResponse {
            activities: vec![],
            count: 0,
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["count"], 0);
        assert_eq!(json["activities"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_complete_response_serialize() {
        let response = CompleteActivityResponse {
            acknowledged: true,
            warnings: Vec::new(),
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["acknowledged"], true);
    }

    #[test]
    fn test_fail_response_serialize() {
        let response = FailActivityResponse {
            acknowledged: true,
            will_retry: true,
            warnings: Vec::new(),
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["acknowledged"], true);
        assert_eq!(json["will_retry"], true);
    }

    #[test]
    fn test_heartbeat_response_serialize() {
        let response = ActivityHeartbeatResponse {
            acknowledged: true,
            next_heartbeat_seconds: 30,
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["acknowledged"], true);
        assert_eq!(json["next_heartbeat_seconds"], 30);
    }

    #[test]
    fn test_activity_error_serialize() {
        let error = ActivityError {
            code: "PAYMENT_DECLINED".to_string(),
            message: "Card was declined".to_string(),
            retryable: false,
        };
        let json = serde_json::to_value(&error).unwrap();
        assert_eq!(json["code"], "PAYMENT_DECLINED");
        assert_eq!(json["retryable"], false);
    }

    // =========================================================================
    // Handler integration tests
    // =========================================================================

    use crate::middleware::auth::ValidatedClaims;
    use crate::state::tests::*;
    use kruxiaflow_core::activity::ActivityWorkerService;
    use kruxiaflow_oauth::Claims;
    use std::sync::Arc;

    fn test_claims() -> ValidatedClaims {
        ValidatedClaims(Claims {
            sub: "test_user".to_string(),
            jti: "test_jti".to_string(),
            iss: "test".to_string(),
            aud: "test".to_string(),
            exp: 9999999999,
            iat: 1000000000,
        })
    }

    fn test_service() -> ActivityWorkerService {
        ActivityWorkerService::new(Arc::new(MockActivityQueue), Arc::new(MockEventSource))
    }

    fn test_state(pool: sqlx::PgPool) -> AppState {
        use kruxiaflow_core::cache::NoOpCache;
        use tokio_util::sync::CancellationToken;

        AppState::new(
            pool,
            Arc::new(MockAuthService),
            Arc::new(MockActivityQueue),
            Arc::new(MockEventSource),
            Arc::new(MockWorkflowStorage),
            Arc::new(NoOpCache::new()),
            Arc::new(MockSubscriptionService),
            CancellationToken::new(),
        )
    }

    #[tokio::test]
    async fn test_poll_activities_handler_empty() {
        let service = test_service();

        let request = PollActivitiesRequest {
            worker: "std".to_string(),
            worker_id: "worker_01".to_string(),
            max_activities: 5,
        };

        let result = poll_activities(service, Extension(test_claims()), Json(request)).await;

        assert!(result.is_ok());
        let Json(response) = result.unwrap();
        assert_eq!(response.count, 0);
        assert!(response.activities.is_empty());
    }

    #[tokio::test]
    async fn test_poll_activities_handler_validation_error() {
        let service = test_service();

        let request = PollActivitiesRequest {
            worker: "".to_string(),
            worker_id: "".to_string(),
            max_activities: 0,
        };

        let result = poll_activities(service, Extension(test_claims()), Json(request)).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_heartbeat_activity_handler_not_found() {
        let service = test_service();
        let activity_id = Uuid::now_v7();

        let request = ActivityHeartbeatRequest {
            worker_id: "worker_01".to_string(),
        };

        let result = heartbeat_activity(
            service,
            Extension(test_claims()),
            Path(activity_id),
            Json(request),
        )
        .await;

        // MockActivityQueue.heartbeat returns Ok(()) so this should succeed
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_heartbeat_activity_handler_validation_error() {
        let service = test_service();

        let request = ActivityHeartbeatRequest {
            worker_id: "".to_string(),
        };

        let result = heartbeat_activity(
            service,
            Extension(test_claims()),
            Path(Uuid::now_v7()),
            Json(request),
        )
        .await;

        assert!(result.is_err());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_complete_activity_handler(pool: sqlx::PgPool) {
        let service = test_service();
        let activity_id = Uuid::now_v7();

        let request = CompleteActivityRequest {
            worker_id: "worker_01".to_string(),
            output: serde_json::json!({"result": "success"}),
            cost_usd: None,
            usage: None,
        };

        let result = complete_activity(
            service,
            State(test_state(pool)),
            Extension(test_claims()),
            Path(activity_id),
            Json(request),
        )
        .await;

        // MockActivityQueue.complete returns Ok(()) so this should succeed
        assert!(result.is_ok());
        let Json(response) = result.unwrap();
        assert!(response.acknowledged);
        assert!(response.warnings.is_empty());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_complete_activity_handler_validation_error(pool: sqlx::PgPool) {
        let service = test_service();

        let request = CompleteActivityRequest {
            worker_id: "".to_string(),
            output: serde_json::json!("not an object"),
            cost_usd: None,
            usage: None,
        };

        let result = complete_activity(
            service,
            State(test_state(pool)),
            Extension(test_claims()),
            Path(Uuid::now_v7()),
            Json(request),
        )
        .await;

        assert!(result.is_err());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_complete_activity_handler_unknown_model_warning(pool: sqlx::PgPool) {
        let service = test_service();
        let activity_id = Uuid::now_v7();

        let request = CompleteActivityRequest {
            worker_id: "worker_01".to_string(),
            output: serde_json::json!({"result": "success"}),
            cost_usd: None,
            usage: Some(vec![UsageEntry {
                provider: "nonexistent".to_string(),
                model: "not-a-model".to_string(),
                input_tokens: 100,
                output_tokens: 10,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                cache_storage_token_hours: None,
                cost_usd: None,
            }]),
        };

        let result = complete_activity(
            service,
            State(test_state(pool)),
            Extension(test_claims()),
            Path(activity_id),
            Json(request),
        )
        .await;

        // Unknown model must never fail the completion — only warn
        assert!(result.is_ok());
        let Json(response) = result.unwrap();
        assert!(response.acknowledged);
        assert_eq!(response.warnings.len(), 1);
        assert!(response.warnings[0].contains("nonexistent/not-a-model"));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_fail_activity_handler(pool: sqlx::PgPool) {
        let service = test_service();
        let activity_id = Uuid::now_v7();

        let request = FailActivityRequest {
            worker_id: "worker_01".to_string(),
            error: ActivityError {
                code: "TIMEOUT".to_string(),
                message: "Activity timed out".to_string(),
                retryable: true,
            },
            cost_usd: None,
            usage: None,
        };

        let result = fail_activity(
            service,
            State(test_state(pool)),
            Extension(test_claims()),
            Path(activity_id),
            Json(request),
        )
        .await;

        // MockActivityQueue.fail returns Ok(false) so will_retry = false
        assert!(result.is_ok());
        let Json(response) = result.unwrap();
        assert!(response.acknowledged);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_fail_activity_handler_validation_error(pool: sqlx::PgPool) {
        let service = test_service();

        let request = FailActivityRequest {
            worker_id: "".to_string(),
            error: ActivityError {
                code: "".to_string(),
                message: "".to_string(),
                retryable: false,
            },
            cost_usd: None,
            usage: None,
        };

        let result = fail_activity(
            service,
            State(test_state(pool)),
            Extension(test_claims()),
            Path(Uuid::now_v7()),
            Json(request),
        )
        .await;

        assert!(result.is_err());
    }
}

/// Poll for activities
///
/// Endpoint: POST /api/v1/workers/poll
///
/// Workers poll for pending activities matching their capabilities.
/// Activities are claimed atomically using FOR UPDATE SKIP LOCKED.
#[utoipa::path(
    post,
    path = "/api/v1/workers/poll",
    tag = "Workers",
    request_body = PollActivitiesRequest,
    responses(
        (status = 200, description = "Activities claimed (may be empty list)", body = PollActivitiesResponse),
        (status = 401, description = "Unauthorized"),
        (status = 422, description = "Validation error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn poll_activities(
    service: ActivityWorkerService,
    Extension(claims): Extension<ValidatedClaims>,
    Json(request): Json<PollActivitiesRequest>,
) -> ApiResult<Json<PollActivitiesResponse>> {
    // Validate request
    request.validate().map_err(AppError::ValidationError)?;

    tracing::debug!(
        worker_id = %request.worker_id,
        worker = %request.worker,
        max_activities = request.max_activities,
        user = %claims.subject(),
        "Polling for activities"
    );

    // Poll for activities (filters by worker only for fair scheduling)
    let activities = service
        .poll_activities(&request.worker, &request.worker_id, request.max_activities)
        .await
        .map_err(|e| {
            tracing::error!("Error polling activities: {:?}", e);
            AppError::InternalError(anyhow::anyhow!(e))
        })?;

    let count = activities.len();

    if count > 0 {
        tracing::debug!(
            worker_id = %request.worker_id,
            claimed_count = count,
            "Activities claimed"
        );
    }

    Ok(Json(PollActivitiesResponse {
        activities: activities
            .into_iter()
            .map(|a| {
                // Extract timeout from settings
                let timeout_seconds = a
                    .settings
                    .as_ref()
                    .and_then(|s| s.get("timeout_seconds"))
                    .and_then(|t| t.as_i64());

                PendingActivity {
                    activity_id: a.id,
                    workflow_id: a.workflow_id,
                    activity_key: a.activity_key,
                    worker: a.worker,
                    activity_name: a.activity_name,
                    parameters: a.parameters,
                    settings: a.settings,
                    timeout_seconds,
                    output_definitions: a.output_definitions,
                    signal_data: a.signal_data,
                }
            })
            .collect(),
        count,
    }))
}

/// Send activity heartbeat
///
/// Endpoint: POST /api/v1/activities/{activity_id}/heartbeat
///
/// Workers send periodic heartbeats for long-running activities to prevent timeout.
#[utoipa::path(
    post,
    path = "/api/v1/activities/{activity_id}/heartbeat",
    tag = "Workers",
    params(
        ("activity_id" = Uuid, Path, description = "Activity ID")
    ),
    request_body = ActivityHeartbeatRequest,
    responses(
        (status = 200, description = "Heartbeat acknowledged", body = ActivityHeartbeatResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Activity not found"),
        (status = 409, description = "Activity already completed or wrong worker")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn heartbeat_activity(
    service: ActivityWorkerService,
    Extension(_claims): Extension<ValidatedClaims>,
    Path(activity_id): Path<Uuid>,
    Json(request): Json<ActivityHeartbeatRequest>,
) -> ApiResult<Json<ActivityHeartbeatResponse>> {
    // Validate request
    request.validate().map_err(AppError::ValidationError)?;

    tracing::debug!(
        activity_id = %activity_id,
        worker_id = %request.worker_id,
        "Heartbeat received"
    );

    let next_heartbeat_seconds = service
        .heartbeat_activity(activity_id, request.worker_id.clone())
        .await
        .map_err(|e| match e {
            ActivityWorkerError::ActivityNotFound(id) => {
                tracing::warn!("Activity not found: {}", id);
                AppError::NotFound(format!("Activity '{}' not found", id))
            }
            ActivityWorkerError::ActivityAlreadyCompleted => {
                tracing::warn!("Activity already completed: {}", activity_id);
                AppError::Conflict("Activity already completed".to_string())
            }
            ActivityWorkerError::WrongWorker => {
                tracing::warn!("Wrong worker for activity {}", activity_id);
                AppError::Conflict("Activity claimed by different worker".to_string())
            }
            _ => AppError::InternalError(anyhow::anyhow!(e)),
        })?;

    Ok(Json(ActivityHeartbeatResponse {
        acknowledged: true,
        next_heartbeat_seconds,
    }))
}

/// Complete activity successfully
///
/// Endpoint: POST /api/v1/activities/{activity_id}/complete
///
/// Workers report successful activity completion with output.
#[utoipa::path(
    post,
    path = "/api/v1/activities/{activity_id}/complete",
    tag = "Workers",
    params(
        ("activity_id" = Uuid, Path, description = "Activity ID")
    ),
    request_body = CompleteActivityRequest,
    responses(
        (status = 200, description = "Activity completed", body = CompleteActivityResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Activity not found"),
        (status = 409, description = "Activity already completed or wrong worker"),
        (status = 422, description = "Validation error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn complete_activity(
    service: ActivityWorkerService,
    State(state): State<AppState>,
    Extension(_claims): Extension<ValidatedClaims>,
    Path(activity_id): Path<Uuid>,
    Json(request): Json<CompleteActivityRequest>,
) -> ApiResult<Json<CompleteActivityResponse>> {
    // Validate request
    request.validate().map_err(AppError::ValidationError)?;

    tracing::debug!(
        activity_id = %activity_id,
        worker_id = %request.worker_id,
        "Completing activity"
    );

    let usage: Option<Vec<kruxiaflow_core::cost::UsageEntry>> = request
        .usage
        .map(|entries| entries.into_iter().map(Into::into).collect());

    // Surface catalog-freshness problems in the response, but never fail the
    // completion because of them — the work is already done
    let warnings = match &usage {
        Some(entries) => unknown_model_warnings(&state.db_pool, entries, activity_id).await,
        None => Vec::new(),
    };

    service
        .complete_activity(
            activity_id,
            request.worker_id.clone(),
            request.output,
            request.cost_usd,
            usage,
        )
        .await
        .map_err(|e| match e {
            ActivityWorkerError::ActivityNotFound(id) => {
                tracing::warn!("Activity not found: {}", id);
                AppError::NotFound(format!("Activity '{}' not found", id))
            }
            ActivityWorkerError::ActivityAlreadyCompleted => {
                tracing::warn!("Activity already completed: {}", activity_id);
                AppError::Conflict("Activity already completed".to_string())
            }
            ActivityWorkerError::WrongWorker => {
                tracing::warn!("Wrong worker for activity {}", activity_id);
                AppError::Conflict("Activity claimed by different worker".to_string())
            }
            _ => AppError::InternalError(anyhow::anyhow!(e)),
        })?;

    Ok(Json(CompleteActivityResponse {
        acknowledged: true,
        warnings,
    }))
}

/// Check reported usage entries that need server-side cost computation against
/// the llm_models catalog, returning a warning per unknown (provider, model)
/// and per model that reports cache-storage token-hours without a catalog
/// storage price (that component is recorded at 0 — no fallback exists for a
/// time-based dimension).
///
/// Read-only and best-effort: a lookup failure yields no warnings rather than
/// failing the request — the orchestrator's recording step performs the same
/// check when it writes the cost rows.
async fn unknown_model_warnings(
    pool: &sqlx::PgPool,
    usage: &[kruxiaflow_core::cost::UsageEntry],
    activity_id: Uuid,
) -> Vec<String> {
    let to_price: Vec<(String, String)> = usage
        .iter()
        .filter(|e| e.cost_usd.is_none())
        .map(|e| (e.provider.clone(), e.model.clone()))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    if to_price.is_empty() {
        return Vec::new();
    }

    match CostCalculator::new(pool.clone())
        .batch_get_pricing(&to_price)
        .await
    {
        Ok(pricing) => {
            let mut warnings: Vec<String> = to_price
                .iter()
                .filter(|(provider, model)| {
                    !pricing.contains_key(&format!("{}/{}", provider, model))
                })
                .map(|(provider, model)| {
                    tracing::warn!(
                        activity_id = %activity_id,
                        provider = %provider,
                        model = %model,
                        "Unknown provider/model in reported usage; cost will be recorded as 0"
                    );
                    format!(
                        "unknown model '{}/{}': not in the llm_models catalog; \
                         its usage will be recorded with cost 0 — supply cost_usd \
                         per entry or update the catalog",
                        provider, model
                    )
                })
                .collect();

            let storage_unpriced: std::collections::BTreeSet<(String, String)> = usage
                .iter()
                .filter(|e| {
                    e.cost_usd.is_none()
                        && e.cache_storage_token_hours
                            .is_some_and(|h| h > Decimal::ZERO)
                        && pricing
                            .get(&format!("{}/{}", e.provider, e.model))
                            .is_some_and(|p| {
                                p.cache_storage_price_per_million_token_hours.is_none()
                            })
                })
                .map(|e| (e.provider.clone(), e.model.clone()))
                .collect();

            warnings.extend(storage_unpriced.iter().map(|(provider, model)| {
                tracing::warn!(
                    activity_id = %activity_id,
                    provider = %provider,
                    model = %model,
                    "Model has no cache-storage price; reported token-hours will cost 0"
                );
                format!(
                    "model '{}/{}' has no cache-storage price in the llm_models \
                     catalog; reported cache_storage_token_hours will be recorded \
                     with cost 0 — supply cost_usd per entry or update the catalog",
                    provider, model
                )
            }));

            warnings
        }
        Err(e) => {
            tracing::error!(
                activity_id = %activity_id,
                error = %e,
                "Failed to check pricing catalog for reported usage"
            );
            Vec::new()
        }
    }
}

/// Fail activity
///
/// Endpoint: POST /api/v1/activities/{activity_id}/fail
///
/// Workers report activity failure with error details.
#[utoipa::path(
    post,
    path = "/api/v1/activities/{activity_id}/fail",
    tag = "Workers",
    params(
        ("activity_id" = Uuid, Path, description = "Activity ID")
    ),
    request_body = FailActivityRequest,
    responses(
        (status = 200, description = "Activity failure recorded", body = FailActivityResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Activity not found"),
        (status = 409, description = "Activity already completed or wrong worker"),
        (status = 422, description = "Validation error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn fail_activity(
    service: ActivityWorkerService,
    State(state): State<AppState>,
    Extension(_claims): Extension<ValidatedClaims>,
    Path(activity_id): Path<Uuid>,
    Json(request): Json<FailActivityRequest>,
) -> ApiResult<Json<FailActivityResponse>> {
    // Validate request
    request.validate().map_err(AppError::ValidationError)?;

    tracing::warn!(
        activity_id = %activity_id,
        worker_id = %request.worker_id,
        error_code = %request.error.code,
        "Activity failed"
    );

    let usage: Option<Vec<kruxiaflow_core::cost::UsageEntry>> = request
        .usage
        .map(|entries| entries.into_iter().map(Into::into).collect());

    let warnings = match &usage {
        Some(entries) => unknown_model_warnings(&state.db_pool, entries, activity_id).await,
        None => Vec::new(),
    };

    let will_retry = service
        .fail_activity(
            activity_id,
            request.worker_id.clone(),
            request.error.code,
            request.error.message,
            request.error.retryable,
            request.cost_usd,
            usage,
        )
        .await
        .map_err(|e| match e {
            ActivityWorkerError::ActivityNotFound(id) => {
                tracing::warn!("Activity not found: {}", id);
                AppError::NotFound(format!("Activity '{}' not found", id))
            }
            ActivityWorkerError::ActivityAlreadyCompleted => {
                tracing::warn!("Activity already completed: {}", activity_id);
                AppError::Conflict("Activity already completed".to_string())
            }
            ActivityWorkerError::WrongWorker => {
                tracing::warn!("Wrong worker for activity {}", activity_id);
                AppError::Conflict("Activity claimed by different worker".to_string())
            }
            _ => AppError::InternalError(anyhow::anyhow!(e)),
        })?;

    Ok(Json(FailActivityResponse {
        acknowledged: true,
        will_retry,
        warnings,
    }))
}
