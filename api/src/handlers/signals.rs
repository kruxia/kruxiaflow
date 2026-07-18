//! Signal API handlers for activities waiting for external signals.

use crate::error::{ApiResult, AppError, ValidationErrors};
use crate::middleware::auth::ValidatedClaims;
use crate::state::AppState;
use axum::{Extension, Json, extract::Path, extract::State};
use kruxiaflow_core::events::{NewWorkflowEvent, WorkflowEventType};
use kruxiaflow_core::subscription::SignalRequest;
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

/// Signal activity request
#[derive(Debug, Deserialize, ToSchema)]
pub struct SignalActivityRequest {
    /// Activity key to signal
    #[schema(example = "wait_for_approval")]
    pub activity_key: String,

    /// Event name that must match the activity's wait_for_signal.event_name
    #[schema(example = "approval_received")]
    pub event_name: String,

    /// Optional data to pass to the activity
    #[schema(example = json!({"approved": true, "approver": "admin@example.com"}))]
    pub data: Option<serde_json::Value>,
}

impl SignalActivityRequest {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();

        if self.activity_key.is_empty() {
            errors.add("activity_key", "Activity key cannot be empty");
        }

        if self.event_name.is_empty() {
            errors.add("event_name", "Event name cannot be empty");
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Signal activity response
#[derive(Debug, Serialize, ToSchema)]
pub struct SignalActivityResponse {
    /// Whether the signal was delivered
    #[schema(example = true)]
    pub signaled: bool,

    /// Message describing the result
    #[schema(example = "Activity signaled successfully")]
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use crate::state::tests::*;
    use axum::extract::State;
    use kruxiaflow_oauth::Claims;
    use sqlx::PgPool;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    // --- SignalActivityRequest validation tests ---

    #[test]
    fn test_signal_request_validate_valid() {
        let request = SignalActivityRequest {
            activity_key: "wait_for_approval".to_string(),
            event_name: "approval_received".to_string(),
            data: Some(json!({"approved": true})),
        };
        assert!(request.validate().is_ok());
    }

    #[test]
    fn test_signal_request_validate_empty_activity_key() {
        let request = SignalActivityRequest {
            activity_key: "".to_string(),
            event_name: "approval_received".to_string(),
            data: None,
        };
        let err = request.validate().unwrap_err();
        assert!(err.field_errors.contains_key("activity_key"));
    }

    #[test]
    fn test_signal_request_validate_empty_event_name() {
        let request = SignalActivityRequest {
            activity_key: "wait_for_approval".to_string(),
            event_name: "".to_string(),
            data: None,
        };
        let err = request.validate().unwrap_err();
        assert!(err.field_errors.contains_key("event_name"));
    }

    #[test]
    fn test_signal_request_validate_both_empty() {
        let request = SignalActivityRequest {
            activity_key: "".to_string(),
            event_name: "".to_string(),
            data: None,
        };
        let err = request.validate().unwrap_err();
        assert!(err.field_errors.contains_key("activity_key"));
        assert!(err.field_errors.contains_key("event_name"));
    }

    #[test]
    fn test_signal_request_validate_no_data() {
        let request = SignalActivityRequest {
            activity_key: "wait".to_string(),
            event_name: "signal".to_string(),
            data: None,
        };
        assert!(request.validate().is_ok());
    }

    #[test]
    fn test_signal_request_deserialize() {
        let json = r#"{"activity_key": "wait", "event_name": "go", "data": {"value": 42}}"#;
        let request: SignalActivityRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.activity_key, "wait");
        assert_eq!(request.event_name, "go");
        assert_eq!(request.data.unwrap()["value"], 42);
    }

    #[test]
    fn test_signal_request_deserialize_without_data() {
        let json = r#"{"activity_key": "wait", "event_name": "go"}"#;
        let request: SignalActivityRequest = serde_json::from_str(json).unwrap();
        assert!(request.data.is_none());
    }

    #[test]
    fn test_signal_response_serialize() {
        let response = SignalActivityResponse {
            signaled: true,
            message: "Activity signaled successfully".to_string(),
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["signaled"], true);
        assert_eq!(json["message"], "Activity signaled successfully");
    }

    #[test]
    fn test_signal_response_serialize_not_found() {
        let response = SignalActivityResponse {
            signaled: false,
            message: "No waiting activity found".to_string(),
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["signaled"], false);
    }

    // --- Handler tests using mock state ---

    fn setup_test_state(pool: PgPool) -> AppState {
        use kruxiaflow_core::cache::NoOpCache;
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

    #[sqlx::test(migrations = "../migrations")]
    async fn test_signal_activity_no_subscription_found(pool: PgPool) {
        // MockSubscriptionService.signal_activity returns None
        let state = setup_test_state(pool);
        let workflow_id = Uuid::now_v7();
        let request = SignalActivityRequest {
            activity_key: "wait_for_approval".to_string(),
            event_name: "approval_received".to_string(),
            data: Some(json!({"approved": true})),
        };

        let result = signal_activity(
            State(state),
            Extension(test_claims()),
            Path(workflow_id),
            Json(request),
        )
        .await;

        assert!(result.is_ok());
        let response = result.unwrap().0;
        assert!(!response.signaled);
        assert!(response.message.contains("No waiting activity found"));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_signal_activity_validation_error(pool: PgPool) {
        let state = setup_test_state(pool);
        let workflow_id = Uuid::now_v7();
        let request = SignalActivityRequest {
            activity_key: "".to_string(),
            event_name: "".to_string(),
            data: None,
        };

        let result = signal_activity(
            State(state),
            Extension(test_claims()),
            Path(workflow_id),
            Json(request),
        )
        .await;

        assert!(result.is_err());
    }

    // Mock subscription service that returns a found subscription
    struct MockFoundSubscriptionService;

    #[async_trait::async_trait]
    impl kruxiaflow_core::subscription::SubscriptionService for MockFoundSubscriptionService {
        async fn create_subscription(
            &self,
            _subscription: kruxiaflow_core::subscription::NewSubscription,
        ) -> kruxiaflow_core::subscription::Result<Uuid> {
            Ok(Uuid::now_v7())
        }

        async fn signal_activity(
            &self,
            request: kruxiaflow_core::subscription::SignalRequest,
        ) -> kruxiaflow_core::subscription::Result<
            Option<kruxiaflow_core::subscription::ActivitySubscription>,
        > {
            Ok(Some(kruxiaflow_core::subscription::ActivitySubscription {
                id: Uuid::now_v7(),
                workflow_id: request.workflow_id,
                activity_key: request.activity_key,
                event_name: request.event_name,
                on_timeout: kruxiaflow_core::workflow::OnTimeout::Fail,
                timeout_at: chrono::Utc::now() + chrono::Duration::hours(1),
                signal_data: request.data,
                created_at: chrono::Utc::now(),
            }))
        }

        async fn get_signal_data(
            &self,
            _workflow_id: Uuid,
            _activity_key: &str,
        ) -> kruxiaflow_core::subscription::Result<Option<serde_json::Value>> {
            Ok(None)
        }

        async fn expire_subscriptions(
            &self,
            _limit: i64,
        ) -> kruxiaflow_core::subscription::Result<
            Vec<kruxiaflow_core::subscription::ExpiredSubscription>,
        > {
            Ok(vec![])
        }

        async fn recover_expired(
            &self,
            _limit: i64,
        ) -> kruxiaflow_core::subscription::Result<
            Vec<kruxiaflow_core::subscription::ExpiredSubscription>,
        > {
            Ok(vec![])
        }

        async fn delete_subscription(
            &self,
            _workflow_id: Uuid,
            _activity_key: &str,
        ) -> kruxiaflow_core::subscription::Result<()> {
            Ok(())
        }
    }

    fn setup_test_state_with_found_subscription(pool: PgPool) -> AppState {
        use kruxiaflow_core::cache::NoOpCache;
        AppState::new(
            pool,
            Arc::new(MockAuthService),
            Arc::new(MockActivityQueue),
            Arc::new(MockEventSource),
            Arc::new(MockWorkflowStorage),
            Arc::new(NoOpCache::new()),
            Arc::new(MockFoundSubscriptionService),
            CancellationToken::new(),
        )
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_signal_activity_subscription_found(pool: PgPool) {
        let state = setup_test_state_with_found_subscription(pool);
        let workflow_id = Uuid::now_v7();
        let request = SignalActivityRequest {
            activity_key: "wait_for_approval".to_string(),
            event_name: "approval_received".to_string(),
            data: Some(json!({"approved": true})),
        };

        let result = signal_activity(
            State(state),
            Extension(test_claims()),
            Path(workflow_id),
            Json(request),
        )
        .await;

        assert!(result.is_ok());
        let response = result.unwrap().0;
        assert!(response.signaled);
        assert!(response.message.contains("signaled successfully"));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_signal_activity_subscription_found_no_data(pool: PgPool) {
        let state = setup_test_state_with_found_subscription(pool);
        let workflow_id = Uuid::now_v7();
        let request = SignalActivityRequest {
            activity_key: "wait_step".to_string(),
            event_name: "continue".to_string(),
            data: None,
        };

        let result = signal_activity(
            State(state),
            Extension(test_claims()),
            Path(workflow_id),
            Json(request),
        )
        .await;

        assert!(result.is_ok());
        let response = result.unwrap().0;
        assert!(response.signaled);
    }

    // Mock subscription service that returns an error
    struct MockErrorSubscriptionService;

    #[async_trait::async_trait]
    impl kruxiaflow_core::subscription::SubscriptionService for MockErrorSubscriptionService {
        async fn create_subscription(
            &self,
            _subscription: kruxiaflow_core::subscription::NewSubscription,
        ) -> kruxiaflow_core::subscription::Result<Uuid> {
            Ok(Uuid::now_v7())
        }

        async fn signal_activity(
            &self,
            _request: kruxiaflow_core::subscription::SignalRequest,
        ) -> kruxiaflow_core::subscription::Result<
            Option<kruxiaflow_core::subscription::ActivitySubscription>,
        > {
            Err(kruxiaflow_core::subscription::SubscriptionError::NotFound)
        }

        async fn get_signal_data(
            &self,
            _workflow_id: Uuid,
            _activity_key: &str,
        ) -> kruxiaflow_core::subscription::Result<Option<serde_json::Value>> {
            Ok(None)
        }

        async fn expire_subscriptions(
            &self,
            _limit: i64,
        ) -> kruxiaflow_core::subscription::Result<
            Vec<kruxiaflow_core::subscription::ExpiredSubscription>,
        > {
            Ok(vec![])
        }

        async fn recover_expired(
            &self,
            _limit: i64,
        ) -> kruxiaflow_core::subscription::Result<
            Vec<kruxiaflow_core::subscription::ExpiredSubscription>,
        > {
            Ok(vec![])
        }

        async fn delete_subscription(
            &self,
            _workflow_id: Uuid,
            _activity_key: &str,
        ) -> kruxiaflow_core::subscription::Result<()> {
            Ok(())
        }
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_signal_activity_subscription_error(pool: PgPool) {
        use kruxiaflow_core::cache::NoOpCache;
        let state = AppState::new(
            pool,
            Arc::new(MockAuthService),
            Arc::new(MockActivityQueue),
            Arc::new(MockEventSource),
            Arc::new(MockWorkflowStorage),
            Arc::new(NoOpCache::new()),
            Arc::new(MockErrorSubscriptionService),
            CancellationToken::new(),
        );

        let workflow_id = Uuid::now_v7();
        let request = SignalActivityRequest {
            activity_key: "wait_step".to_string(),
            event_name: "go".to_string(),
            data: None,
        };

        let result = signal_activity(
            State(state),
            Extension(test_claims()),
            Path(workflow_id),
            Json(request),
        )
        .await;

        assert!(result.is_err());
    }
}

/// Signal an activity waiting for an external event
///
/// Endpoint: POST /api/v1/workflows/{workflow_id}/signal
///
/// Sends a signal to an activity that is in the "waiting" state.
/// The activity must have been configured with `wait_for_signal` setting
/// and must be waiting for the specified event_name.
#[utoipa::path(
    post,
    path = "/api/v1/workflows/{workflow_id}/signal",
    tag = "Workflows",
    params(
        ("workflow_id" = Uuid, Path, description = "Workflow ID")
    ),
    request_body = SignalActivityRequest,
    responses(
        (status = 200, description = "Signal processed", body = SignalActivityResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Workflow or subscription not found"),
        (status = 422, description = "Validation error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn signal_activity(
    State(state): State<AppState>,
    Extension(_claims): Extension<ValidatedClaims>,
    Path(workflow_id): Path<Uuid>,
    Json(request): Json<SignalActivityRequest>,
) -> ApiResult<Json<SignalActivityResponse>> {
    // Validate request
    request.validate().map_err(AppError::ValidationError)?;

    tracing::info!(
        workflow_id = %workflow_id,
        activity_key = %request.activity_key,
        event_name = %request.event_name,
        "Signaling activity"
    );

    // Create signal request
    let signal_request = SignalRequest {
        workflow_id,
        activity_key: request.activity_key.clone(),
        event_name: request.event_name.clone(),
        data: request.data.clone(),
    };

    // Try to signal the subscription
    let subscription = state
        .subscription_service
        .signal_activity(signal_request)
        .await
        .map_err(|e| {
            tracing::error!("Error signaling activity: {:?}", e);
            AppError::InternalError(anyhow::anyhow!(e))
        })?;

    match subscription {
        Some(sub) => {
            // Subscription found and signaled - publish ActivitySignaled event
            let event = NewWorkflowEvent {
                workflow_id,
                event_type: WorkflowEventType::ActivitySignaled,
                activity_key: Some(request.activity_key.clone()),
                payload: json!({
                    "event_name": request.event_name,
                    "signal_data": request.data,
                }),
                iteration: None,
            };

            state.event_source.publish(event).await.map_err(|e| {
                tracing::error!("Failed to publish ActivitySignaled event: {:?}", e);
                AppError::InternalError(anyhow::anyhow!(e))
            })?;

            tracing::info!(
                workflow_id = %workflow_id,
                activity_key = %request.activity_key,
                subscription_id = %sub.id,
                "Activity signaled successfully"
            );

            Ok(Json(SignalActivityResponse {
                signaled: true,
                message: "Activity signaled successfully".to_string(),
            }))
        }
        None => {
            // No matching subscription found
            tracing::warn!(
                workflow_id = %workflow_id,
                activity_key = %request.activity_key,
                event_name = %request.event_name,
                "No matching subscription found for signal"
            );

            Ok(Json(SignalActivityResponse {
                signaled: false,
                message: format!(
                    "No waiting activity found for workflow {} with key '{}' and event '{}'",
                    workflow_id, request.activity_key, request.event_name
                ),
            }))
        }
    }
}
