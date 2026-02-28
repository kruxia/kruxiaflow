use crate::websocket::ConnectionId;
use kruxiaflow_core::events::{WorkflowEvent, WorkflowEventType};
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use uuid::Uuid;

use super::messages::WorkflowEventMessage;

/// Subscription filter for workflow events.
///
/// Empty vecs mean "match all" for that dimension.
pub struct SubscriptionFilter {
    /// Filter to specific workflow IDs (empty = all workflows)
    pub workflow_ids: Vec<Uuid>,
    /// Filter to specific event types (empty = all types)
    pub event_types: Vec<WorkflowEventType>,
}

impl SubscriptionFilter {
    /// Check if a workflow event matches this filter.
    pub fn matches(&self, event: &WorkflowEvent) -> bool {
        let workflow_match =
            self.workflow_ids.is_empty() || self.workflow_ids.contains(&event.workflow_id);
        let type_match =
            self.event_types.is_empty() || self.event_types.contains(&event.event_type);
        workflow_match && type_match
    }
}

/// Internal subscription state.
struct Subscription {
    id: ConnectionId,
    filter: SubscriptionFilter,
    sender: mpsc::Sender<String>,
}

/// Manages WebSocket subscriptions for workflow event streaming.
///
/// Subscriptions are filtered per-client. Uses bounded channels (capacity 1000)
/// with `try_send()` for backpressure — slow clients are dropped.
///
/// Thread-safe via `Arc<RwLock<...>>`. Cloning is cheap.
///
/// # Data structure: flat Vec with linear scan
///
/// `broadcast()` iterates all subscriptions for each event — O(events × subscriptions)
/// per poll cycle. Each iteration is cheap: a filter check (~20ns for small
/// `contains` on workflow_ids/event_types vecs) + `try_send` (~50–100ns for
/// mpsc channel atomic ops), so ~100–150ns per subscription per event.
///
/// Per-event broadcast latency at various subscription counts:
///
/// | Subscriptions | Per-event broadcast | 100-event batch |
/// |---------------|---------------------|-----------------|
/// | 100           | ~10–15μs            | ~1–1.5ms        |
/// | 1,000         | ~100–150μs          | ~10–15ms        |
/// | 10,000        | ~1–1.5ms            | ~100–150ms      |
///
/// With the poller's 50ms minimum interval, broadcast overhead stays under 30%
/// up to ~1,000 subscriptions. At ~10,000 subscriptions the broadcast dominates
/// the poll cycle and events queue up — at that scale, switch to a
/// `HashMap<Uuid, Vec<Subscription>>` keyed by workflow_id (with a separate
/// "all workflows" list) to avoid scanning irrelevant subscriptions.
///
/// For MVP, 1,000 concurrent WebSocket subscriptions per API instance is well
/// beyond expected load. The flat Vec also avoids HashMap overhead for the
/// common "subscribe to all workflows" case where keying by workflow_id
/// doesn't help.
#[derive(Clone)]
pub struct WorkflowEventManager {
    subscriptions: Arc<RwLock<Vec<Subscription>>>,
}

impl WorkflowEventManager {
    pub fn new() -> Self {
        Self {
            subscriptions: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Register a new subscription. Returns a ConnectionId for later unregister.
    pub async fn register(
        &self,
        filter: SubscriptionFilter,
        sender: mpsc::Sender<String>,
    ) -> ConnectionId {
        let id = ConnectionId::new();
        let subscription = Subscription { id, filter, sender };

        let mut subs = self.subscriptions.write().await;
        subs.push(subscription);

        tracing::info!(
            connection_id = %id,
            subscriber_count = subs.len(),
            "Workflow event subscription registered"
        );

        id
    }

    /// Unregister a subscription by ConnectionId.
    pub async fn unregister(&self, conn_id: ConnectionId) {
        let mut subs = self.subscriptions.write().await;
        let before = subs.len();
        subs.retain(|s| s.id != conn_id);
        let removed = before - subs.len();

        if removed > 0 {
            tracing::info!(
                connection_id = %conn_id,
                remaining_subscribers = subs.len(),
                "Workflow event subscription unregistered"
            );
        }
    }

    /// Broadcast a workflow event to all matching subscriptions.
    ///
    /// Uses `try_send()` — if a subscription's channel is full (slow client),
    /// it is dropped to prevent memory buildup.
    ///
    /// Returns the number of subscriptions that received the event.
    pub async fn broadcast(&self, event: &WorkflowEvent) -> usize {
        // Fast path: skip serialization when nobody is listening.
        // At 500 events/sec this avoids ~1ms/sec of wasted JSON serialization.
        if !self.has_subscribers().await {
            return 0;
        }

        let message = WorkflowEventMessage::from_workflow_event(event);
        let json = match message.to_json() {
            Ok(j) => j,
            Err(e) => {
                tracing::error!(error = %e, "Failed to serialize workflow event message");
                return 0;
            }
        };

        let mut slow_clients: Vec<ConnectionId> = Vec::new();
        let mut sent_count = 0;

        {
            let subs = self.subscriptions.read().await;
            for sub in subs.iter() {
                if !sub.filter.matches(event) {
                    continue;
                }
                match sub.sender.try_send(json.clone()) {
                    Ok(()) => sent_count += 1,
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        tracing::warn!(
                            connection_id = %sub.id,
                            "Workflow event subscriber too slow, dropping"
                        );
                        slow_clients.push(sub.id);
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        slow_clients.push(sub.id);
                    }
                }
            }
        }

        // Clean up slow/disconnected clients
        if !slow_clients.is_empty() {
            let mut subs = self.subscriptions.write().await;
            subs.retain(|s| !slow_clients.contains(&s.id));
        }

        sent_count
    }

    /// Get the number of active subscriptions.
    pub async fn subscriber_count(&self) -> usize {
        self.subscriptions.read().await.len()
    }

    /// Check if there are any active subscriptions.
    pub async fn has_subscribers(&self) -> bool {
        !self.subscriptions.read().await.is_empty()
    }
}

impl Default for WorkflowEventManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    fn make_event(workflow_id: Uuid, event_type: WorkflowEventType) -> WorkflowEvent {
        WorkflowEvent {
            id: Uuid::now_v7(),
            workflow_id,
            event_type,
            activity_key: None,
            payload: json!({}),
            timestamp: Utc.with_ymd_and_hms(2025, 6, 1, 12, 0, 0).unwrap(),
            iteration: None,
        }
    }

    // --- SubscriptionFilter tests ---

    #[test]
    fn test_filter_empty_matches_all() {
        let filter = SubscriptionFilter {
            workflow_ids: vec![],
            event_types: vec![],
        };
        let event = make_event(Uuid::now_v7(), WorkflowEventType::WorkflowCreated);
        assert!(filter.matches(&event));
    }

    #[test]
    fn test_filter_matching_workflow_id() {
        let wf_id = Uuid::now_v7();
        let filter = SubscriptionFilter {
            workflow_ids: vec![wf_id],
            event_types: vec![],
        };
        let event = make_event(wf_id, WorkflowEventType::WorkflowCreated);
        assert!(filter.matches(&event));
    }

    #[test]
    fn test_filter_non_matching_workflow_id() {
        let filter = SubscriptionFilter {
            workflow_ids: vec![Uuid::now_v7()],
            event_types: vec![],
        };
        let event = make_event(Uuid::now_v7(), WorkflowEventType::WorkflowCreated);
        assert!(!filter.matches(&event));
    }

    #[test]
    fn test_filter_matching_event_type() {
        let filter = SubscriptionFilter {
            workflow_ids: vec![],
            event_types: vec![WorkflowEventType::ActivityCompleted],
        };
        let event = make_event(Uuid::now_v7(), WorkflowEventType::ActivityCompleted);
        assert!(filter.matches(&event));
    }

    #[test]
    fn test_filter_non_matching_event_type() {
        let filter = SubscriptionFilter {
            workflow_ids: vec![],
            event_types: vec![WorkflowEventType::ActivityCompleted],
        };
        let event = make_event(Uuid::now_v7(), WorkflowEventType::WorkflowCreated);
        assert!(!filter.matches(&event));
    }

    #[test]
    fn test_filter_both_dimensions_match() {
        let wf_id = Uuid::now_v7();
        let filter = SubscriptionFilter {
            workflow_ids: vec![wf_id],
            event_types: vec![WorkflowEventType::ActivityCompleted],
        };
        let event = make_event(wf_id, WorkflowEventType::ActivityCompleted);
        assert!(filter.matches(&event));
    }

    #[test]
    fn test_filter_workflow_matches_type_does_not() {
        let wf_id = Uuid::now_v7();
        let filter = SubscriptionFilter {
            workflow_ids: vec![wf_id],
            event_types: vec![WorkflowEventType::ActivityCompleted],
        };
        let event = make_event(wf_id, WorkflowEventType::WorkflowCreated);
        assert!(!filter.matches(&event));
    }

    #[test]
    fn test_filter_multiple_workflow_ids() {
        let wf1 = Uuid::now_v7();
        let wf2 = Uuid::now_v7();
        let filter = SubscriptionFilter {
            workflow_ids: vec![wf1, wf2],
            event_types: vec![],
        };
        assert!(filter.matches(&make_event(wf1, WorkflowEventType::WorkflowCreated)));
        assert!(filter.matches(&make_event(wf2, WorkflowEventType::WorkflowCreated)));
        assert!(!filter.matches(&make_event(Uuid::now_v7(), WorkflowEventType::WorkflowCreated)));
    }

    #[test]
    fn test_filter_multiple_event_types() {
        let filter = SubscriptionFilter {
            workflow_ids: vec![],
            event_types: vec![
                WorkflowEventType::ActivityCompleted,
                WorkflowEventType::ActivityFailed,
            ],
        };
        assert!(filter.matches(&make_event(Uuid::now_v7(), WorkflowEventType::ActivityCompleted)));
        assert!(filter.matches(&make_event(Uuid::now_v7(), WorkflowEventType::ActivityFailed)));
        assert!(!filter.matches(&make_event(Uuid::now_v7(), WorkflowEventType::WorkflowCreated)));
    }

    // --- WorkflowEventManager tests ---

    #[tokio::test]
    async fn test_new_manager_has_no_subscribers() {
        let manager = WorkflowEventManager::new();
        assert_eq!(manager.subscriber_count().await, 0);
        assert!(!manager.has_subscribers().await);
    }

    #[tokio::test]
    async fn test_default_manager() {
        let manager = WorkflowEventManager::default();
        assert_eq!(manager.subscriber_count().await, 0);
    }

    #[tokio::test]
    async fn test_register_increases_count() {
        let manager = WorkflowEventManager::new();
        let (tx, _rx) = mpsc::channel(10);
        let filter = SubscriptionFilter {
            workflow_ids: vec![],
            event_types: vec![],
        };
        manager.register(filter, tx).await;
        assert_eq!(manager.subscriber_count().await, 1);
        assert!(manager.has_subscribers().await);
    }

    #[tokio::test]
    async fn test_unregister_removes_subscription() {
        let manager = WorkflowEventManager::new();
        let (tx, _rx) = mpsc::channel(10);
        let filter = SubscriptionFilter {
            workflow_ids: vec![],
            event_types: vec![],
        };
        let conn_id = manager.register(filter, tx).await;
        assert_eq!(manager.subscriber_count().await, 1);

        manager.unregister(conn_id).await;
        assert_eq!(manager.subscriber_count().await, 0);
    }

    #[tokio::test]
    async fn test_unregister_nonexistent_is_noop() {
        let manager = WorkflowEventManager::new();
        let (tx, _rx) = mpsc::channel(10);
        let filter = SubscriptionFilter {
            workflow_ids: vec![],
            event_types: vec![],
        };
        manager.register(filter, tx).await;
        // Unregister an ID that was never registered
        manager.unregister(ConnectionId::new()).await;
        assert_eq!(manager.subscriber_count().await, 1);
    }

    #[tokio::test]
    async fn test_broadcast_no_subscribers_returns_zero() {
        let manager = WorkflowEventManager::new();
        let event = make_event(Uuid::now_v7(), WorkflowEventType::WorkflowCreated);
        assert_eq!(manager.broadcast(&event).await, 0);
    }

    #[tokio::test]
    async fn test_broadcast_delivers_to_matching_subscriber() {
        let manager = WorkflowEventManager::new();
        let (tx, mut rx) = mpsc::channel(10);
        let filter = SubscriptionFilter {
            workflow_ids: vec![],
            event_types: vec![],
        };
        manager.register(filter, tx).await;

        let event = make_event(Uuid::now_v7(), WorkflowEventType::WorkflowCreated);
        let sent = manager.broadcast(&event).await;
        assert_eq!(sent, 1);

        let msg = rx.try_recv().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["type"], "event");
    }

    #[tokio::test]
    async fn test_broadcast_skips_non_matching_subscriber() {
        let manager = WorkflowEventManager::new();
        let (tx, mut rx) = mpsc::channel(10);
        let filter = SubscriptionFilter {
            workflow_ids: vec![Uuid::now_v7()], // Different ID
            event_types: vec![],
        };
        manager.register(filter, tx).await;

        let event = make_event(Uuid::now_v7(), WorkflowEventType::WorkflowCreated);
        let sent = manager.broadcast(&event).await;
        assert_eq!(sent, 0);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_broadcast_drops_closed_channel() {
        let manager = WorkflowEventManager::new();
        let (tx, rx) = mpsc::channel(10);
        let filter = SubscriptionFilter {
            workflow_ids: vec![],
            event_types: vec![],
        };
        manager.register(filter, tx).await;
        assert_eq!(manager.subscriber_count().await, 1);

        // Drop receiver to close channel
        drop(rx);

        let event = make_event(Uuid::now_v7(), WorkflowEventType::WorkflowCreated);
        let sent = manager.broadcast(&event).await;
        assert_eq!(sent, 0);
        // Closed subscriber should be cleaned up
        assert_eq!(manager.subscriber_count().await, 0);
    }

    #[tokio::test]
    async fn test_broadcast_drops_slow_client() {
        let manager = WorkflowEventManager::new();
        // Channel with capacity 1 — second message will trigger slow client
        let (tx, _rx) = mpsc::channel(1);
        let filter = SubscriptionFilter {
            workflow_ids: vec![],
            event_types: vec![],
        };
        manager.register(filter, tx).await;

        let event = make_event(Uuid::now_v7(), WorkflowEventType::WorkflowCreated);

        // First broadcast fills the channel
        assert_eq!(manager.broadcast(&event).await, 1);
        // Second broadcast finds channel full → drops client
        assert_eq!(manager.broadcast(&event).await, 0);
        assert_eq!(manager.subscriber_count().await, 0);
    }

    #[tokio::test]
    async fn test_broadcast_multiple_subscribers() {
        let manager = WorkflowEventManager::new();

        let (tx1, mut rx1) = mpsc::channel(10);
        let (tx2, mut rx2) = mpsc::channel(10);

        let filter1 = SubscriptionFilter {
            workflow_ids: vec![],
            event_types: vec![],
        };
        let filter2 = SubscriptionFilter {
            workflow_ids: vec![],
            event_types: vec![],
        };
        manager.register(filter1, tx1).await;
        manager.register(filter2, tx2).await;

        let event = make_event(Uuid::now_v7(), WorkflowEventType::WorkflowCreated);
        let sent = manager.broadcast(&event).await;
        assert_eq!(sent, 2);

        assert!(rx1.try_recv().is_ok());
        assert!(rx2.try_recv().is_ok());
    }

    #[tokio::test]
    async fn test_broadcast_mixed_matching() {
        let manager = WorkflowEventManager::new();
        let wf_id = Uuid::now_v7();

        let (tx_match, mut rx_match) = mpsc::channel(10);
        let (tx_nomatch, mut rx_nomatch) = mpsc::channel(10);

        let filter_match = SubscriptionFilter {
            workflow_ids: vec![wf_id],
            event_types: vec![],
        };
        let filter_nomatch = SubscriptionFilter {
            workflow_ids: vec![Uuid::now_v7()],
            event_types: vec![],
        };
        manager.register(filter_match, tx_match).await;
        manager.register(filter_nomatch, tx_nomatch).await;

        let event = make_event(wf_id, WorkflowEventType::WorkflowCreated);
        let sent = manager.broadcast(&event).await;
        assert_eq!(sent, 1);
        assert!(rx_match.try_recv().is_ok());
        assert!(rx_nomatch.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_clone_shares_state() {
        let manager = WorkflowEventManager::new();
        let cloned = manager.clone();
        let (tx, _rx) = mpsc::channel(10);
        let filter = SubscriptionFilter {
            workflow_ids: vec![],
            event_types: vec![],
        };
        manager.register(filter, tx).await;
        // Cloned manager should see the registration
        assert_eq!(cloned.subscriber_count().await, 1);
    }
}
