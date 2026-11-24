//! Signal delivery: the one code path for signaling a waiting activity.
//!
//! Every entry point that signals an activity (the REST signal endpoint and
//! the MCP `send_workflow_signal` tool) calls [`deliver_signal`], so
//! validation and the outcomes can't drift between them. The subscription
//! service marks the wait and writes the `ActivitySignaled` event the
//! orchestrator reads, in one transaction.

use super::models::{ActivitySubscription, SignalOutcome, SignalRequest};
use super::service::{SubscriptionError, SubscriptionService};
use crate::events::{NewWorkflowEvent, WorkflowEventType};
use serde_json::json;
use thiserror::Error;

/// A request field that failed validation
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalFieldError {
    /// Field name as in [`SignalRequest`] (`activity_key`, `event_name`)
    pub field: &'static str,
    pub message: &'static str,
}

/// Errors from [`deliver_signal`]
#[derive(Debug, Error)]
pub enum SignalDeliveryError {
    #[error("Invalid signal request: {}", format_field_errors(.0))]
    Invalid(Vec<SignalFieldError>),

    /// Nothing was recorded: neither the signal nor its event
    #[error("Error signaling activity: {0}")]
    Subscription(#[from] SubscriptionError),
}

fn format_field_errors(errors: &[SignalFieldError]) -> String {
    errors
        .iter()
        .map(|e| format!("{}: {}", e.field, e.message))
        .collect::<Vec<_>>()
        .join("; ")
}

impl SignalRequest {
    /// Check the request fields (non-empty activity key and event name)
    pub fn validate(&self) -> Result<(), Vec<SignalFieldError>> {
        let mut errors = Vec::new();
        if self.activity_key.is_empty() {
            errors.push(SignalFieldError {
                field: "activity_key",
                message: "Activity key cannot be empty",
            });
        }
        if self.event_name.is_empty() {
            errors.push(SignalFieldError {
                field: "event_name",
                message: "Event name cannot be empty",
            });
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// The `ActivitySignaled` event for a signaled subscription.
///
/// The payload is `{"event_name", "signal_data"}`: the orchestrator reads the
/// signal's data from `signal_data` only, so keys inside the data can never be
/// read as event fields (such as `on_timeout`).
pub fn activity_signaled_event(
    subscription: &ActivitySubscription,
    request: &SignalRequest,
) -> NewWorkflowEvent {
    NewWorkflowEvent {
        workflow_id: request.workflow_id,
        event_type: WorkflowEventType::ActivitySignaled,
        activity_key: Some(subscription.activity_key.clone()),
        payload: json!({
            "event_name": request.event_name,
            "signal_data": request.data,
        }),
        // The loop round this wait belongs to: without it, the event dedup
        // index drops every round's signal after the first (#29)
        iteration: subscription.iteration,
    }
}

/// Signal a waiting activity.
///
/// Validates the request, then signals the activity's open wait through
/// [`SubscriptionService::signal_activity`], which writes the
/// `ActivitySignaled` event in the same transaction.
///
/// Returns what became of the signal:
/// - [`SignalOutcome::Signaled`]: the waiting activity took it.
/// - [`SignalOutcome::Held`]: the activity is not waiting and its wait is
///   configured with `hold_early`; the signal is kept for its next wait
///   ([`SubscriptionService::hold_signal`]). No event is written: the
///   orchestrator takes the held signal when the activity next waits.
/// - [`SignalOutcome::Refused`]: neither, with the reason (no such activity,
///   a different event name, not waiting, the workflow has ended, or the
///   activity holds as many signals as it may).
///
/// An error records nothing, so the signal can be sent again.
pub async fn deliver_signal(
    subscriptions: &dyn SubscriptionService,
    request: SignalRequest,
) -> Result<SignalOutcome, SignalDeliveryError> {
    request.validate().map_err(SignalDeliveryError::Invalid)?;

    match subscriptions.signal_activity(request.clone()).await? {
        Some(subscription) => Ok(SignalOutcome::Signaled(subscription)),
        // No open wait took it: hold it for the next one, or say why not
        None => Ok(subscriptions.hold_signal(request).await?),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subscription::{NewSubscription, PostgresSubscriptionService};
    use crate::workflow::OnTimeout;
    use serde_json::Value;
    use uuid::Uuid;

    fn request(activity_key: &str, event_name: &str, data: Option<Value>) -> SignalRequest {
        SignalRequest {
            workflow_id: Uuid::now_v7(),
            activity_key: activity_key.to_string(),
            event_name: event_name.to_string(),
            data,
        }
    }

    #[test]
    fn test_validate_valid() {
        assert!(request("approve", "go", None).validate().is_ok());
    }

    #[test]
    fn test_validate_empty_fields() {
        let errors = request("", "", None).validate().unwrap_err();
        let fields: Vec<_> = errors.iter().map(|e| e.field).collect();
        assert_eq!(fields, vec!["activity_key", "event_name"]);
    }

    #[test]
    fn test_event_payload_wraps_data() {
        // Keys that are also event fields stay inside signal_data
        let data = json!({"approved": true, "on_timeout": "skip", "signal_data": 1});
        let req = request("approve", "go", Some(data.clone()));
        let sub = ActivitySubscription {
            id: Uuid::now_v7(),
            workflow_id: req.workflow_id,
            activity_key: "approve".to_string(),
            event_name: "go".to_string(),
            on_timeout: OnTimeout::Fail,
            timeout_at: chrono::Utc::now(),
            signal_data: Some(data.clone()),
            created_at: chrono::Utc::now(),
            // The wait's loop round, which the event must carry (#29)
            iteration: Some(2),
        };
        let event = activity_signaled_event(&sub, &req);
        assert_eq!(event.workflow_id, req.workflow_id);
        assert_eq!(event.event_type, WorkflowEventType::ActivitySignaled);
        assert_eq!(event.activity_key.as_deref(), Some("approve"));
        assert_eq!(
            event.payload,
            json!({"event_name": "go", "signal_data": data})
        );
        assert_eq!(event.iteration, Some(2));
    }

    /// A running workflow whose `approve` step waits for `go`
    async fn waiting_workflow(pool: &sqlx::PgPool) -> Uuid {
        let def_id: Uuid = sqlx::query_scalar(
            "INSERT INTO workflow_definitions (name, activities) VALUES ('w', '[]') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        let workflow_id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO workflows (id, definition_name, workflow_definition_id, input, status, activities, state_data) \
             VALUES ($1, 'w', $2, '{}', 'running', '{}', '{}')",
        )
        .bind(workflow_id)
        .bind(def_id)
        .execute(pool)
        .await
        .unwrap();
        PostgresSubscriptionService::new(pool.clone())
            .create_subscription(NewSubscription {
                workflow_id,
                activity_key: "approve".to_string(),
                event_name: "go".to_string(),
                on_timeout: OnTimeout::Fail,
                timeout_seconds: 300,
                iteration: None,
            })
            .await
            .unwrap();
        workflow_id
    }

    fn approve(workflow_id: Uuid, data: Option<Value>) -> SignalRequest {
        SignalRequest {
            workflow_id,
            activity_key: "approve".to_string(),
            event_name: "go".to_string(),
            data,
        }
    }

    /// The payloads of the workflow's `ActivitySignaled` events
    async fn signaled_events(pool: &sqlx::PgPool, workflow_id: Uuid) -> Vec<Value> {
        sqlx::query_scalar(
            "SELECT payload FROM workflow_events \
             WHERE workflow_id = $1 AND event_type = 'ActivitySignaled'",
        )
        .bind(workflow_id)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    /// The first signal for a wait is taken and its event written with its
    /// data; the second matches nothing and writes nothing
    #[sqlx::test(migrations = "../migrations")]
    async fn test_deliver_signal_once(pool: sqlx::PgPool) {
        let workflow_id = waiting_workflow(&pool).await;
        let subs = PostgresSubscriptionService::new(pool.clone());

        let first = deliver_signal(&subs, approve(workflow_id, Some(json!({"n": 1}))))
            .await
            .unwrap();
        assert!(first.is_signaled());
        let second = deliver_signal(&subs, approve(workflow_id, Some(json!({"n": 2}))))
            .await
            .unwrap();
        assert!(!second.is_signaled());
        assert_eq!(
            signaled_events(&pool, workflow_id).await,
            vec![json!({"event_name": "go", "signal_data": {"n": 1}})]
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_deliver_signal_without_data(pool: sqlx::PgPool) {
        let workflow_id = waiting_workflow(&pool).await;
        let subs = PostgresSubscriptionService::new(pool.clone());
        deliver_signal(&subs, approve(workflow_id, None))
            .await
            .unwrap();
        assert_eq!(
            signaled_events(&pool, workflow_id).await,
            vec![json!({"event_name": "go", "signal_data": null})]
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_deliver_signal_invalid_request(pool: sqlx::PgPool) {
        let workflow_id = waiting_workflow(&pool).await;
        let subs = PostgresSubscriptionService::new(pool.clone());
        let mut invalid = approve(workflow_id, None);
        invalid.event_name = String::new();
        let result = deliver_signal(&subs, invalid).await;
        assert!(matches!(result, Err(SignalDeliveryError::Invalid(_))));
        assert!(signaled_events(&pool, workflow_id).await.is_empty());
    }

    /// A signal whose event can't be written records nothing: it is an
    /// error, the wait is still open, and the signal sent again is taken
    #[sqlx::test(migrations = "../migrations")]
    async fn test_deliver_signal_event_failure_leaves_the_wait_open(pool: sqlx::PgPool) {
        let workflow_id = waiting_workflow(&pool).await;
        let subs = PostgresSubscriptionService::new(pool.clone());
        // The next event insert fails, once (a sequence isn't rolled back)
        for statement in [
            "CREATE SEQUENCE fail_event_once",
            "CREATE FUNCTION fail_event_once() RETURNS trigger LANGUAGE plpgsql AS $$ \
             BEGIN \
               IF nextval('fail_event_once') = 1 THEN RAISE EXCEPTION 'insert failed'; END IF; \
               RETURN NEW; \
             END $$",
            "CREATE TRIGGER fail_event_once BEFORE INSERT ON workflow_events \
             FOR EACH ROW EXECUTE FUNCTION fail_event_once()",
        ] {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }
        let data = Some(json!({"approved": true}));

        let failed = deliver_signal(&subs, approve(workflow_id, data.clone())).await;
        assert!(matches!(failed, Err(SignalDeliveryError::Subscription(_))));
        let signaled_at: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
            "SELECT signaled_at FROM activity_event_subscriptions WHERE workflow_id = $1",
        )
        .bind(workflow_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(signaled_at, None, "the wait is still marked signaled");

        let retry = deliver_signal(&subs, approve(workflow_id, data))
            .await
            .unwrap();
        assert!(retry.is_signaled(), "the signal sent again was refused");
        assert_eq!(signaled_events(&pool, workflow_id).await.len(), 1);
    }
}
