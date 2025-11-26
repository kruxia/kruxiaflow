//! Connection manager for WebSocket activity streaming.
//!
//! Manages active WebSocket connections per activity, enabling broadcast
//! of streaming messages (particularly LLM tokens) to all connected clients.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{RwLock, mpsc};
use uuid::Uuid;

use super::StreamMessage;

/// Unique identifier for a WebSocket connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(u64);

impl ConnectionId {
    fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed))
    }
}

impl std::fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "conn-{}", self.0)
    }
}

/// A registered WebSocket connection.
struct Connection {
    id: ConnectionId,
    sender: mpsc::UnboundedSender<String>,
}

/// Manages WebSocket connections for activity streaming.
///
/// # Thread Safety
///
/// This type is thread-safe and can be shared across tasks via `Arc`.
/// All operations use internal locking and are safe to call concurrently.
///
/// # Example
///
/// ```ignore
/// let manager = ConnectionManager::new();
///
/// // Register a connection
/// let (tx, rx) = mpsc::unbounded_channel();
/// let conn_id = manager.register(activity_id, tx).await;
///
/// // Broadcast to all connections for an activity
/// manager.broadcast(activity_id, StreamMessage::token("hello", 0)).await;
///
/// // Unregister when done
/// manager.unregister(activity_id, conn_id).await;
/// ```
#[derive(Clone, Default)]
pub struct ConnectionManager {
    /// Map: activity_id -> list of connections
    connections: Arc<RwLock<HashMap<Uuid, Vec<Connection>>>>,
}

impl ConnectionManager {
    /// Create a new connection manager.
    pub fn new() -> Self {
        Self {
            connections: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a new WebSocket connection for an activity.
    ///
    /// Returns a unique connection ID that should be used for unregistration.
    ///
    /// # Arguments
    ///
    /// * `activity_id` - The activity this connection is streaming
    /// * `sender` - Channel sender for delivering messages to this connection
    pub async fn register(
        &self,
        activity_id: Uuid,
        sender: mpsc::UnboundedSender<String>,
    ) -> ConnectionId {
        let conn_id = ConnectionId::new();
        let connection = Connection {
            id: conn_id,
            sender,
        };

        let mut conns = self.connections.write().await;
        let activity_conns = conns.entry(activity_id).or_default();
        activity_conns.push(connection);

        tracing::info!(
            activity_id = %activity_id,
            connection_id = %conn_id,
            connection_count = activity_conns.len(),
            "WebSocket connection registered"
        );

        conn_id
    }

    /// Unregister a connection by its ID.
    ///
    /// Called when a WebSocket connection closes.
    pub async fn unregister(&self, activity_id: Uuid, conn_id: ConnectionId) {
        let mut conns = self.connections.write().await;
        if let Some(activity_conns) = conns.get_mut(&activity_id) {
            let before_len = activity_conns.len();
            activity_conns.retain(|c| c.id != conn_id);
            let removed = before_len - activity_conns.len();

            if removed > 0 {
                tracing::info!(
                    activity_id = %activity_id,
                    connection_id = %conn_id,
                    remaining_connections = activity_conns.len(),
                    "WebSocket connection unregistered"
                );
            }

            // Clean up empty entries
            if activity_conns.is_empty() {
                conns.remove(&activity_id);
            }
        }
    }

    /// Broadcast a message to all connections for an activity.
    ///
    /// Failed sends (disconnected clients) are automatically cleaned up.
    ///
    /// # Arguments
    ///
    /// * `activity_id` - The activity to broadcast to
    /// * `message` - The message to send
    ///
    /// # Returns
    ///
    /// Number of connections that successfully received the message.
    pub async fn broadcast(&self, activity_id: Uuid, message: StreamMessage) -> usize {
        let json = match message.to_json() {
            Ok(j) => j,
            Err(e) => {
                tracing::error!(error = %e, "Failed to serialize WebSocket message");
                return 0;
            }
        };

        // First pass: send to all connections, collect failed IDs
        let failed_ids: Vec<ConnectionId> = {
            let conns = self.connections.read().await;
            if let Some(activity_conns) = conns.get(&activity_id) {
                activity_conns
                    .iter()
                    .filter_map(|conn| {
                        if conn.sender.send(json.clone()).is_err() {
                            Some(conn.id)
                        } else {
                            None
                        }
                    })
                    .collect()
            } else {
                return 0;
            }
        };

        // Second pass: cleanup failed connections
        let success_count = {
            let conns = self.connections.read().await;
            conns
                .get(&activity_id)
                .map(|c| c.len())
                .unwrap_or(0)
                .saturating_sub(failed_ids.len())
        };

        if !failed_ids.is_empty() {
            tracing::debug!(
                activity_id = %activity_id,
                failed_count = failed_ids.len(),
                "Cleaning up failed WebSocket connections"
            );
            for conn_id in failed_ids {
                self.unregister(activity_id, conn_id).await;
            }
        }

        success_count
    }

    /// Close all connections for an activity.
    ///
    /// Called when an activity completes or fails. All connections
    /// will be dropped, causing the WebSocket handlers to close.
    pub async fn close_all(&self, activity_id: Uuid) {
        let mut conns = self.connections.write().await;
        if let Some(activity_conns) = conns.remove(&activity_id) {
            tracing::info!(
                activity_id = %activity_id,
                connection_count = activity_conns.len(),
                "Closing all WebSocket connections for activity"
            );
            // Connections are dropped here, closing channels
        }
    }

    /// Get the number of active connections for an activity.
    ///
    /// Useful for metrics and debugging.
    pub async fn connection_count(&self, activity_id: Uuid) -> usize {
        let conns = self.connections.read().await;
        conns.get(&activity_id).map(|v| v.len()).unwrap_or(0)
    }

    /// Get total number of active connections across all activities.
    ///
    /// Useful for metrics and capacity planning.
    pub async fn total_connection_count(&self) -> usize {
        let conns = self.connections.read().await;
        conns.values().map(|v| v.len()).sum()
    }

    /// Get number of activities with active connections.
    ///
    /// Useful for metrics.
    pub async fn active_activity_count(&self) -> usize {
        let conns = self.connections.read().await;
        conns.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[tokio::test]
    async fn test_register_and_count() {
        let manager = ConnectionManager::new();
        let activity_id = Uuid::now_v7();
        let (tx, _rx) = mpsc::unbounded_channel();

        let _conn_id = manager.register(activity_id, tx).await;
        assert_eq!(manager.connection_count(activity_id).await, 1);
        assert_eq!(manager.total_connection_count().await, 1);
        assert_eq!(manager.active_activity_count().await, 1);
    }

    #[tokio::test]
    async fn test_register_multiple_connections() {
        let manager = ConnectionManager::new();
        let activity_id = Uuid::now_v7();

        let (tx1, _rx1) = mpsc::unbounded_channel();
        let (tx2, _rx2) = mpsc::unbounded_channel();
        let (tx3, _rx3) = mpsc::unbounded_channel();

        manager.register(activity_id, tx1).await;
        manager.register(activity_id, tx2).await;
        manager.register(activity_id, tx3).await;

        assert_eq!(manager.connection_count(activity_id).await, 3);
    }

    #[tokio::test]
    async fn test_unregister_connection() {
        let manager = ConnectionManager::new();
        let activity_id = Uuid::now_v7();

        let (tx1, _rx1) = mpsc::unbounded_channel();
        let (tx2, _rx2) = mpsc::unbounded_channel();

        let conn_id1 = manager.register(activity_id, tx1).await;
        let _conn_id2 = manager.register(activity_id, tx2).await;

        assert_eq!(manager.connection_count(activity_id).await, 2);

        manager.unregister(activity_id, conn_id1).await;
        assert_eq!(manager.connection_count(activity_id).await, 1);
    }

    #[tokio::test]
    async fn test_unregister_last_connection_removes_activity() {
        let manager = ConnectionManager::new();
        let activity_id = Uuid::now_v7();

        let (tx, _rx) = mpsc::unbounded_channel();
        let conn_id = manager.register(activity_id, tx).await;

        assert_eq!(manager.active_activity_count().await, 1);

        manager.unregister(activity_id, conn_id).await;

        assert_eq!(manager.connection_count(activity_id).await, 0);
        assert_eq!(manager.active_activity_count().await, 0);
    }

    #[tokio::test]
    async fn test_broadcast_to_single_connection() {
        let manager = ConnectionManager::new();
        let activity_id = Uuid::now_v7();

        let (tx, mut rx) = mpsc::unbounded_channel();
        manager.register(activity_id, tx).await;

        let msg = StreamMessage::Token {
            text: "hello".to_string(),
            index: 0,
            timestamp: Utc::now(),
        };

        let count = manager.broadcast(activity_id, msg).await;
        assert_eq!(count, 1);

        let received = rx.recv().await.unwrap();
        assert!(received.contains(r#""type":"token"#));
        assert!(received.contains(r#""text":"hello"#));
    }

    #[tokio::test]
    async fn test_broadcast_to_multiple_connections() {
        let manager = ConnectionManager::new();
        let activity_id = Uuid::now_v7();

        let (tx1, mut rx1) = mpsc::unbounded_channel();
        let (tx2, mut rx2) = mpsc::unbounded_channel();

        manager.register(activity_id, tx1).await;
        manager.register(activity_id, tx2).await;

        let msg = StreamMessage::Token {
            text: "world".to_string(),
            index: 1,
            timestamp: Utc::now(),
        };

        let count = manager.broadcast(activity_id, msg).await;
        assert_eq!(count, 2);

        assert!(rx1.recv().await.is_some());
        assert!(rx2.recv().await.is_some());
    }

    #[tokio::test]
    async fn test_broadcast_to_nonexistent_activity() {
        let manager = ConnectionManager::new();
        let activity_id = Uuid::now_v7();

        let msg = StreamMessage::token("test", 0);
        let count = manager.broadcast(activity_id, msg).await;

        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_broadcast_cleans_up_dropped_receivers() {
        let manager = ConnectionManager::new();
        let activity_id = Uuid::now_v7();

        let (tx1, rx1) = mpsc::unbounded_channel();
        let (tx2, mut rx2) = mpsc::unbounded_channel();

        manager.register(activity_id, tx1).await;
        manager.register(activity_id, tx2).await;

        assert_eq!(manager.connection_count(activity_id).await, 2);

        // Drop rx1 to simulate disconnected client
        drop(rx1);

        let msg = StreamMessage::token("test", 0);
        let count = manager.broadcast(activity_id, msg).await;

        // Only one connection received the message
        assert_eq!(count, 1);

        // Failed connection was cleaned up
        assert_eq!(manager.connection_count(activity_id).await, 1);

        // Remaining connection still works
        assert!(rx2.recv().await.is_some());
    }

    #[tokio::test]
    async fn test_close_all() {
        let manager = ConnectionManager::new();
        let activity_id = Uuid::now_v7();

        let (tx1, mut rx1) = mpsc::unbounded_channel();
        let (tx2, mut rx2) = mpsc::unbounded_channel();

        manager.register(activity_id, tx1).await;
        manager.register(activity_id, tx2).await;

        assert_eq!(manager.connection_count(activity_id).await, 2);

        manager.close_all(activity_id).await;

        assert_eq!(manager.connection_count(activity_id).await, 0);
        assert_eq!(manager.active_activity_count().await, 0);

        // Receivers should get None (channel closed)
        assert!(rx1.recv().await.is_none());
        assert!(rx2.recv().await.is_none());
    }

    #[tokio::test]
    async fn test_multiple_activities() {
        let manager = ConnectionManager::new();
        let activity1 = Uuid::now_v7();
        let activity2 = Uuid::now_v7();

        let (tx1, mut rx1) = mpsc::unbounded_channel();
        let (tx2, mut rx2) = mpsc::unbounded_channel();

        manager.register(activity1, tx1).await;
        manager.register(activity2, tx2).await;

        assert_eq!(manager.active_activity_count().await, 2);
        assert_eq!(manager.total_connection_count().await, 2);

        // Broadcast to activity1 only
        let msg = StreamMessage::token("for-activity-1", 0);
        manager.broadcast(activity1, msg).await;

        // Only rx1 should receive
        let received = rx1.recv().await.unwrap();
        assert!(received.contains("for-activity-1"));

        // rx2 should have nothing (use try_recv to avoid blocking)
        assert!(rx2.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_connection_id_uniqueness() {
        let id1 = ConnectionId::new();
        let id2 = ConnectionId::new();
        let id3 = ConnectionId::new();

        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert_ne!(id1, id3);
    }

    #[tokio::test]
    async fn test_clone_manager() {
        let manager = ConnectionManager::new();
        let cloned = manager.clone();
        let activity_id = Uuid::now_v7();

        let (tx, mut rx) = mpsc::unbounded_channel();
        manager.register(activity_id, tx).await;

        // Cloned manager should see the same connections
        assert_eq!(cloned.connection_count(activity_id).await, 1);

        // Broadcasting via clone should work
        let msg = StreamMessage::token("via-clone", 0);
        cloned.broadcast(activity_id, msg).await;

        let received = rx.recv().await.unwrap();
        assert!(received.contains("via-clone"));
    }
}
