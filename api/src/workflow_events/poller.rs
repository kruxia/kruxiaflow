use kruxiaflow_core::events::{EventSource, Wake};
use kruxiaflow_core::orchestrator::AdaptiveBackoff;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::manager::WorkflowEventManager;

/// Consumer ID prefix for the WebSocket broadcast poller.
/// Each API server instance gets a unique consumer_id (prefix + UUIDv7) to ensure
/// every instance independently polls all events and broadcasts to its own local
/// subscribers. Without per-instance IDs, multiple instances sharing a single
/// consumer position would cause subscribers on one instance to miss events
/// consumed by another.
const CONSUMER_ID_PREFIX: &str = "ws";

/// Backoff range for the broadcast poller. The idle maximum is longer when
/// the event source has a wake signal, because the timer is then only a
/// safety net for lost notifications.
const POLL_INTERVAL_MIN: Duration = Duration::from_millis(50);
const POLL_INTERVAL_MAX: Duration = Duration::from_secs(1);
const WAKE_POLL_INTERVAL_MAX: Duration = Duration::from_secs(5);

/// Run the event broadcast poller.
///
/// Polls the EventSource for new workflow events and broadcasts them to all
/// registered WebSocket subscriptions via the WorkflowEventManager.
///
/// Runs unconditionally at API server boot. Drains, then waits: after a
/// non-empty batch it polls again at once; after an empty poll it waits for
/// the event source's wake signal (if any) or the backoff timer (50ms up to
/// 1s idle, or 5s with a wake signal), whichever comes first.
///
/// Each API server instance uses a unique consumer_id (prefix + UUIDv7)
/// so that in multi-instance deployments, every instance independently tracks
/// its position and delivers all events to its local WebSocket subscribers.
///
/// Shuts down gracefully when the CancellationToken is cancelled.
pub async fn run_event_broadcast_poller(
    event_source: Arc<dyn EventSource>,
    manager: WorkflowEventManager,
    shutdown_token: CancellationToken,
) {
    let consumer_id = format!("{}-{}", CONSUMER_ID_PREFIX, Uuid::now_v7());

    let mut wake = event_source.wake();
    let max_interval = if wake.is_some() {
        WAKE_POLL_INTERVAL_MAX
    } else {
        POLL_INTERVAL_MAX
    };
    let mut backoff = AdaptiveBackoff::new(POLL_INTERVAL_MIN, max_interval, 2.0);

    tracing::info!(
        consumer_id = %consumer_id,
        event_wake = if wake.is_some() { "notify" } else { "poll" },
        "Workflow event broadcast poller started"
    );

    loop {
        if shutdown_token.is_cancelled() {
            tracing::info!("Workflow event broadcast poller shutting down");
            return;
        }

        // Wait before the next poll only when caught up or failing
        let wait = match event_source.poll(&consumer_id).await {
            Ok(events) if events.is_empty() => true,
            Ok(events) => {
                let event_count = events.len();
                let mut total_sent = 0;

                for event in &events {
                    total_sent += manager.broadcast(event).await;
                }

                if total_sent > 0 {
                    tracing::debug!(
                        event_count,
                        total_sent,
                        "Broadcast workflow events to WebSocket subscribers"
                    );
                }

                // Update consumer position after broadcasting all events
                let cursor = events.last().map(|event| event.cursor());
                match cursor {
                    Some(cursor) => {
                        match event_source.update_position(&consumer_id, cursor).await {
                            Ok(()) => {
                                backoff.reset();
                                false
                            }
                            Err(e) => {
                                tracing::error!(
                                    error = %e,
                                    "Failed to update websocket broadcast consumer position"
                                );
                                backoff.increase();
                                true
                            }
                        }
                    }
                    None => true,
                }
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "Workflow event broadcast poller error"
                );
                backoff.increase();
                true
            }
        };

        if !wait {
            // Drain: poll again at once
            continue;
        }

        let interval = backoff.current();
        backoff.increase();
        match wake.as_mut() {
            Some(wake) => tokio::select! {
                woken = wake.wait(interval) => {
                    if woken == Wake::Woken {
                        backoff.reset();
                    }
                }
                _ = shutdown_token.cancelled() => {
                    tracing::info!("Workflow event broadcast poller shutting down");
                    return;
                }
            },
            None => tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                _ = shutdown_token.cancelled() => {
                    tracing::info!("Workflow event broadcast poller shutting down");
                    return;
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use chrono::{TimeZone, Utc};
    use kruxiaflow_core::events::{
        EventCursor, EventError, NewWorkflowEvent, WorkflowEvent, WorkflowEventType,
    };
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::mpsc;

    /// Mock EventSource that returns events from a pre-configured list, then empty.
    struct MockPollerEventSource {
        events: tokio::sync::Mutex<Vec<WorkflowEvent>>,
        poll_count: AtomicUsize,
        update_position_count: AtomicUsize,
    }

    impl MockPollerEventSource {
        fn new(events: Vec<WorkflowEvent>) -> Self {
            Self {
                events: tokio::sync::Mutex::new(events),
                poll_count: AtomicUsize::new(0),
                update_position_count: AtomicUsize::new(0),
            }
        }

        fn poll_count(&self) -> usize {
            self.poll_count.load(Ordering::SeqCst)
        }

        fn update_position_count(&self) -> usize {
            self.update_position_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl EventSource for MockPollerEventSource {
        async fn publish(&self, _event: NewWorkflowEvent) -> kruxiaflow_core::events::Result<()> {
            Ok(())
        }

        async fn poll(
            &self,
            _consumer_id: &str,
        ) -> kruxiaflow_core::events::Result<Vec<WorkflowEvent>> {
            self.poll_count.fetch_add(1, Ordering::SeqCst);
            let mut events = self.events.lock().await;
            let result = std::mem::take(&mut *events);
            Ok(result)
        }

        async fn update_position(
            &self,
            _consumer_id: &str,
            _cursor: EventCursor,
        ) -> kruxiaflow_core::events::Result<()> {
            self.update_position_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    /// Mock EventSource that always returns an error on poll.
    struct ErrorEventSource;

    #[async_trait]
    impl EventSource for ErrorEventSource {
        async fn publish(&self, _event: NewWorkflowEvent) -> kruxiaflow_core::events::Result<()> {
            Ok(())
        }

        async fn poll(
            &self,
            _consumer_id: &str,
        ) -> kruxiaflow_core::events::Result<Vec<WorkflowEvent>> {
            Err(EventError::Invalid("test error".to_string()))
        }

        async fn update_position(
            &self,
            _consumer_id: &str,
            _cursor: EventCursor,
        ) -> kruxiaflow_core::events::Result<()> {
            Ok(())
        }
    }

    /// Mock EventSource where update_position fails.
    struct UpdateFailEventSource {
        events: tokio::sync::Mutex<Vec<WorkflowEvent>>,
    }

    impl UpdateFailEventSource {
        fn new(events: Vec<WorkflowEvent>) -> Self {
            Self {
                events: tokio::sync::Mutex::new(events),
            }
        }
    }

    #[async_trait]
    impl EventSource for UpdateFailEventSource {
        async fn publish(&self, _event: NewWorkflowEvent) -> kruxiaflow_core::events::Result<()> {
            Ok(())
        }

        async fn poll(
            &self,
            _consumer_id: &str,
        ) -> kruxiaflow_core::events::Result<Vec<WorkflowEvent>> {
            let mut events = self.events.lock().await;
            Ok(std::mem::take(&mut *events))
        }

        async fn update_position(
            &self,
            _consumer_id: &str,
            _cursor: EventCursor,
        ) -> kruxiaflow_core::events::Result<()> {
            Err(EventError::Invalid("update failed".to_string()))
        }
    }

    fn make_event() -> WorkflowEvent {
        WorkflowEvent {
            id: Uuid::now_v7(),
            position: 0,
            tx: 0,
            workflow_id: Uuid::now_v7(),
            event_type: WorkflowEventType::WorkflowCreated,
            activity_key: None,
            payload: json!({}),
            timestamp: Utc.with_ymd_and_hms(2025, 6, 1, 12, 0, 0).unwrap(),
            iteration: None,
        }
    }

    #[tokio::test]
    async fn test_poller_shuts_down_on_cancellation() {
        let source = Arc::new(MockPollerEventSource::new(vec![]));
        let manager = WorkflowEventManager::new();
        let token = CancellationToken::new();

        // Cancel immediately
        token.cancel();

        // Should return quickly
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            run_event_broadcast_poller(source, manager, token),
        )
        .await;
        assert!(result.is_ok(), "Poller did not shut down in time");
    }

    #[tokio::test]
    async fn test_poller_broadcasts_events_to_subscribers() {
        let event = make_event();
        let source = Arc::new(MockPollerEventSource::new(vec![event]));
        let manager = WorkflowEventManager::new();
        let token = CancellationToken::new();

        // Register a subscriber
        let (tx, mut rx) = mpsc::channel(100);
        let filter = super::super::manager::SubscriptionFilter {
            workflow_ids: vec![],
            event_types: vec![],
        };
        manager.register(filter, tx).await;

        let token_clone = token.clone();
        let handle = tokio::spawn(async move {
            run_event_broadcast_poller(source, manager, token_clone).await;
        });

        // Wait for the message to arrive
        let msg = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("Timeout waiting for event")
            .expect("Channel closed");

        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["type"], "event");

        token.cancel();
        let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
    }

    #[tokio::test]
    async fn test_poller_updates_position_after_events() {
        let event = make_event();
        let source = Arc::new(MockPollerEventSource::new(vec![event]));
        let source_ref = source.clone();
        let manager = WorkflowEventManager::new();
        let token = CancellationToken::new();

        let token_clone = token.clone();
        let handle = tokio::spawn(async move {
            run_event_broadcast_poller(source, manager, token_clone).await;
        });

        // Wait for at least one poll cycle
        tokio::time::sleep(Duration::from_millis(200)).await;
        token.cancel();
        let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;

        assert!(source_ref.poll_count() >= 1);
        assert_eq!(source_ref.update_position_count(), 1);
    }

    #[tokio::test]
    async fn test_poller_handles_poll_errors() {
        let source: Arc<dyn EventSource> = Arc::new(ErrorEventSource);
        let manager = WorkflowEventManager::new();
        let token = CancellationToken::new();

        let token_clone = token.clone();
        let handle = tokio::spawn(async move {
            run_event_broadcast_poller(source, manager, token_clone).await;
        });

        // Let the poller run a couple cycles with errors
        tokio::time::sleep(Duration::from_millis(200)).await;
        token.cancel();

        let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(result.is_ok(), "Poller did not shut down after errors");
    }

    #[tokio::test]
    async fn test_poller_handles_update_position_failure() {
        let event = make_event();
        let source: Arc<dyn EventSource> = Arc::new(UpdateFailEventSource::new(vec![event]));
        let manager = WorkflowEventManager::new();
        let token = CancellationToken::new();

        // Register a subscriber so broadcast actually runs
        let (tx, _rx) = mpsc::channel(100);
        let filter = super::super::manager::SubscriptionFilter {
            workflow_ids: vec![],
            event_types: vec![],
        };
        manager.register(filter, tx).await;

        let token_clone = token.clone();
        let handle = tokio::spawn(async move {
            run_event_broadcast_poller(source, manager, token_clone).await;
        });

        // Let it process
        tokio::time::sleep(Duration::from_millis(200)).await;
        token.cancel();

        let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(
            result.is_ok(),
            "Poller did not shut down after update_position failure"
        );
    }

    #[tokio::test]
    async fn test_poller_does_not_update_position_on_empty() {
        let source = Arc::new(MockPollerEventSource::new(vec![]));
        let source_ref = source.clone();
        let manager = WorkflowEventManager::new();
        let token = CancellationToken::new();

        let token_clone = token.clone();
        let handle = tokio::spawn(async move {
            run_event_broadcast_poller(source, manager, token_clone).await;
        });

        tokio::time::sleep(Duration::from_millis(200)).await;
        token.cancel();
        let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;

        assert!(source_ref.poll_count() >= 1);
        assert_eq!(source_ref.update_position_count(), 0);
    }

    #[tokio::test]
    async fn test_poller_shuts_down_during_sleep() {
        // Test that the poller exits even during the backoff sleep
        let source: Arc<dyn EventSource> = Arc::new(MockPollerEventSource::new(vec![]));
        let manager = WorkflowEventManager::new();
        let token = CancellationToken::new();

        let token_clone = token.clone();
        let handle = tokio::spawn(async move {
            run_event_broadcast_poller(source, manager, token_clone).await;
        });

        // Let it start polling, then cancel
        tokio::time::sleep(Duration::from_millis(100)).await;
        token.cancel();

        let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(result.is_ok(), "Poller did not shut down during sleep");
    }
}
