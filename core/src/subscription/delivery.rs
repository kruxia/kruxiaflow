//! Signal delivery: the one code path for signaling a waiting activity.
//!
//! Every entry point that signals an activity (the REST signal endpoint and
//! the MCP `send_workflow_signal` tool) calls [`deliver_signal`], so the
//! subscription marking and the `ActivitySignaled` event the orchestrator
//! reads can't drift between them.

use super::models::{ActivitySubscription, SignalRequest};
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
        // The subscription carries no loop round yet; when it does, the event
        // must carry it, or the event dedup index drops later rounds' signals
        iteration: None,
    }
}

/// Signal a waiting activity and publish the event the orchestrator acts on.
///
/// Validates the request, marks the subscription signaled through
/// [`SubscriptionService::signal_activity`], then publishes `ActivitySignaled`.
///
/// Returns `Ok(Some(subscription))` when the activity was signaled, and
/// `Ok(None)` when no subscription matched (no such activity, a different
/// event name, already signaled, or not waiting). A publish failure is an
/// error: nothing else delivers the signal to the orchestrator.
pub async fn deliver_signal(
    subscriptions: &dyn SubscriptionService,
    events: &dyn EventSource,
    request: SignalRequest,
) -> Result<Option<ActivitySubscription>, SignalDeliveryError> {
    request.validate().map_err(SignalDeliveryError::Invalid)?;

    let Some(subscription) = subscriptions.signal_activity(request.clone()).await? else {
        return Ok(None);
    };

    events
        .publish(activity_signaled_event(&subscription, &request))
        .await?;

    Ok(Some(subscription))
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
        };
        let event = activity_signaled_event(&sub, &req);
        assert_eq!(event.workflow_id, req.workflow_id);
        assert_eq!(event.event_type, WorkflowEventType::ActivitySignaled);
        assert_eq!(event.activity_key.as_deref(), Some("approve"));
        assert_eq!(
            event.payload,
            json!({"event_name": "go", "signal_data": data})
        );
        assert_eq!(event.iteration, None);
    }

    #[tokio::test]
    async fn test_deliver_signal_publishes_once() {
        let subs = OnceSubscriptions::new();
        let events = RecordingEvents::new(false);
        let req = request("approve", "go", Some(json!({"approved": true})));

        let first = deliver_signal(&subs, &events, req.clone()).await.unwrap();
        assert!(first.is_some());
        let second = deliver_signal(&subs, &events, req).await.unwrap();
        assert!(second.is_none());

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
}
