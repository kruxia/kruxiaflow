//! Signal delivery: the one code path for signaling a waiting activity.
//!
//! Every entry point that signals an activity (the REST signal endpoint and
//! the MCP `send_workflow_signal` tool) calls [`deliver_signal`], so the
//! subscription marking and the `ActivitySignaled` event the orchestrator
//! reads can't drift between them.

use super::models::{ActivitySubscription, SignalOutcome, SignalRequest};
use super::service::{SubscriptionError, SubscriptionService};
use crate::events::{EventError, EventSource, NewWorkflowEvent, WorkflowEventType};
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

    #[error("Error signaling activity: {0}")]
    Subscription(#[from] SubscriptionError),

    /// The subscription was marked signaled but the event was not published
    #[error("Failed to publish ActivitySignaled event: {0}")]
    Publish(#[from] EventError),
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

/// Signal a waiting activity and publish the event the orchestrator acts on.
///
/// Validates the request, marks the subscription signaled through
/// [`SubscriptionService::signal_activity`], then publishes `ActivitySignaled`.
///
/// Returns what became of the signal:
/// - [`SignalOutcome::Signaled`]: the waiting activity took it.
/// - [`SignalOutcome::Held`]: the activity is not waiting and its wait is
///   configured with `hold_early`; the signal is kept for its next wait
///   ([`SubscriptionService::hold_signal`]). No event is published: the
///   orchestrator takes the held signal when the activity next waits.
/// - [`SignalOutcome::Refused`]: neither, with the reason (no such activity,
///   a different event name, not waiting, the workflow has ended, or the
///   activity holds as many signals as it may).
///
/// A publish failure is an error: nothing else delivers the signal to the
/// orchestrator, so the subscription's mark is undone first
/// ([`SubscriptionService::release_signal`]) and the signal can be retried.
pub async fn deliver_signal(
    subscriptions: &dyn SubscriptionService,
    events: &dyn EventSource,
    request: SignalRequest,
) -> Result<SignalOutcome, SignalDeliveryError> {
    request.validate().map_err(SignalDeliveryError::Invalid)?;

    let subscription = match subscriptions.signal_activity(request.clone()).await? {
        Some(subscription) => subscription,
        // No open wait took it: hold it for the next one, or say why not
        None => match subscriptions.hold_signal(request.clone()).await? {
            SignalOutcome::Signaled(subscription) => subscription,
            other => return Ok(other),
        },
    };

    if let Err(e) = events
        .publish(activity_signaled_event(&subscription, &request))
        .await
    {
        if let Err(release) = subscriptions.release_signal(subscription.id).await {
            tracing::error!(
                subscription_id = %subscription.id,
                "Failed to reopen the wait after a failed publish: {:?}",
                release
            );
        }
        return Err(SignalDeliveryError::Publish(e));
    }

    Ok(SignalOutcome::Signaled(subscription))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{EventCursor, WorkflowEvent};
    use crate::subscription::{ExpiredSubscription, NewSubscription};
    use crate::workflow::OnTimeout;
    use async_trait::async_trait;
    use serde_json::Value;
    use std::sync::Mutex;
    use uuid::Uuid;

    /// Subscription service that matches once per activity key
    struct OnceSubscriptions {
        signaled: Mutex<Vec<String>>,
    }

    impl OnceSubscriptions {
        fn new() -> Self {
            Self {
                signaled: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl SubscriptionService for OnceSubscriptions {
        async fn create_subscription(
            &self,
            _subscription: NewSubscription,
        ) -> crate::subscription::Result<Uuid> {
            Ok(Uuid::now_v7())
        }

        async fn signal_activity(
            &self,
            request: SignalRequest,
        ) -> crate::subscription::Result<Option<ActivitySubscription>> {
            let mut signaled = self.signaled.lock().unwrap();
            if signaled.contains(&request.activity_key) {
                return Ok(None);
            }
            signaled.push(request.activity_key.clone());
            Ok(Some(ActivitySubscription {
                id: Uuid::now_v7(),
                workflow_id: request.workflow_id,
                activity_key: request.activity_key,
                event_name: request.event_name,
                on_timeout: OnTimeout::Fail,
                timeout_at: chrono::Utc::now(),
                signal_data: request.data,
                created_at: chrono::Utc::now(),
                iteration: None,
            }))
        }

        async fn get_signal_data(
            &self,
            _workflow_id: Uuid,
            _activity_key: &str,
        ) -> crate::subscription::Result<Option<Value>> {
            Ok(None)
        }

        async fn expire_subscriptions(
            &self,
            _limit: i64,
        ) -> crate::subscription::Result<Vec<ExpiredSubscription>> {
            Ok(vec![])
        }

        async fn recover_expired(
            &self,
            _limit: i64,
        ) -> crate::subscription::Result<Vec<ExpiredSubscription>> {
            Ok(vec![])
        }

        async fn delete_subscription(
            &self,
            _workflow_id: Uuid,
            _activity_key: &str,
        ) -> crate::subscription::Result<()> {
            Ok(())
        }

        async fn release_signal(&self, _subscription_id: Uuid) -> crate::subscription::Result<()> {
            Ok(())
        }

        async fn delete_subscription_by_id(&self, _id: Uuid) -> crate::subscription::Result<()> {
            Ok(())
        }
    }

    /// Event source that records published events, or fails every publish
    struct RecordingEvents {
        published: Mutex<Vec<NewWorkflowEvent>>,
        fail: bool,
    }

    impl RecordingEvents {
        fn new(fail: bool) -> Self {
            Self {
                published: Mutex::new(Vec::new()),
                fail,
            }
        }
    }

    #[async_trait]
    impl EventSource for RecordingEvents {
        async fn publish(&self, event: NewWorkflowEvent) -> crate::events::Result<()> {
            if self.fail {
                return Err(EventError::Invalid("publish failed".to_string()));
            }
            self.published.lock().unwrap().push(event);
            Ok(())
        }

        async fn poll(&self, _consumer_id: &str) -> crate::events::Result<Vec<WorkflowEvent>> {
            Ok(vec![])
        }

        async fn update_position(
            &self,
            _consumer_id: &str,
            _cursor: EventCursor,
        ) -> crate::events::Result<()> {
            Ok(())
        }
    }

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

    #[tokio::test]
    async fn test_deliver_signal_publishes_only_when_signaled() {
        let subs = OnceSubscriptions::new();
        let events = RecordingEvents::new(false);
        let req = request("approve", "go", Some(json!({"approved": true})));

        let first = deliver_signal(&subs, &events, req.clone()).await.unwrap();
        assert!(first.is_signaled());
        let second = deliver_signal(&subs, &events, req).await.unwrap();
        assert!(!second.is_signaled());

        let published = events.published.lock().unwrap();
        assert_eq!(published.len(), 1);
        assert_eq!(
            published[0].payload,
            json!({"event_name": "go", "signal_data": {"approved": true}})
        );
    }

    #[tokio::test]
    async fn test_deliver_signal_without_data() {
        let subs = OnceSubscriptions::new();
        let events = RecordingEvents::new(false);
        deliver_signal(&subs, &events, request("approve", "go", None))
            .await
            .unwrap();
        let published = events.published.lock().unwrap();
        assert_eq!(
            published[0].payload,
            json!({"event_name": "go", "signal_data": null})
        );
    }

    #[tokio::test]
    async fn test_deliver_signal_invalid_request() {
        let subs = OnceSubscriptions::new();
        let events = RecordingEvents::new(false);
        let result = deliver_signal(&subs, &events, request("approve", "", None)).await;
        assert!(matches!(result, Err(SignalDeliveryError::Invalid(_))));
        assert!(subs.signaled.lock().unwrap().is_empty());
        assert!(events.published.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_deliver_signal_publish_failure_is_an_error() {
        let subs = OnceSubscriptions::new();
        let events = RecordingEvents::new(true);
        let result = deliver_signal(&subs, &events, request("approve", "go", None)).await;
        assert!(matches!(result, Err(SignalDeliveryError::Publish(_))));
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
        crate::subscription::PostgresSubscriptionService::new(pool.clone())
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

    /// Against PostgreSQL: the second signal for a wait matches nothing and
    /// publishes nothing
    #[sqlx::test(migrations = "../migrations")]
    async fn test_deliver_signal_once_against_postgres(pool: sqlx::PgPool) {
        let workflow_id = waiting_workflow(&pool).await;
        let subs = crate::subscription::PostgresSubscriptionService::new(pool.clone());
        let events = RecordingEvents::new(false);

        let first = deliver_signal(&subs, &events, approve(workflow_id, Some(json!({"n": 1}))))
            .await
            .unwrap();
        assert!(first.is_signaled());
        let second = deliver_signal(&subs, &events, approve(workflow_id, Some(json!({"n": 2}))))
            .await
            .unwrap();
        assert!(!second.is_signaled());
        assert_eq!(events.published.lock().unwrap().len(), 1);
    }

    /// A signal whose event fails to publish leaves the wait open: a retry is
    /// accepted and publishes the event
    #[sqlx::test(migrations = "../migrations")]
    async fn test_deliver_signal_publish_failure_reopens_the_wait(pool: sqlx::PgPool) {
        let workflow_id = waiting_workflow(&pool).await;
        let subs = crate::subscription::PostgresSubscriptionService::new(pool.clone());
        let data = Some(json!({"approved": true}));

        let failed = deliver_signal(
            &subs,
            &RecordingEvents::new(true),
            approve(workflow_id, data.clone()),
        )
        .await;
        assert!(matches!(failed, Err(SignalDeliveryError::Publish(_))));
        let stored: Option<Value> = sqlx::query_scalar(
            "SELECT signal_data FROM activity_event_subscriptions WHERE workflow_id = $1",
        )
        .bind(workflow_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(stored, None, "the wait is still marked signaled");

        let events = RecordingEvents::new(false);
        let retry = deliver_signal(&subs, &events, approve(workflow_id, data))
            .await
            .unwrap();
        assert!(
            retry.is_signaled(),
            "a retry after a failed publish was refused"
        );
        assert_eq!(events.published.lock().unwrap().len(), 1);
    }
}
