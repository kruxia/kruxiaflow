//! Subscription service trait definition.

use super::models::{ActivitySubscription, ExpiredSubscription, NewSubscription, SignalRequest};
use async_trait::async_trait;
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

/// Errors that can occur in subscription operations
#[derive(Debug, Error)]
pub enum SubscriptionError {
    #[error("Subscription not found")]
    NotFound,

    #[error("Subscription already exists for workflow {0} activity {1}")]
    AlreadyExists(Uuid, String),

    #[error("Event name mismatch: expected {expected}, got {actual}")]
    EventNameMismatch { expected: String, actual: String },

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

pub type Result<T> = std::result::Result<T, SubscriptionError>;

/// Service for managing activity event subscriptions
#[async_trait]
pub trait SubscriptionService: Send + Sync {
    /// Create a new subscription for an activity waiting for a signal
    async fn create_subscription(&self, subscription: NewSubscription) -> Result<Uuid>;

    /// Signal an activity, transitioning it from waiting to pending
    /// Returns the subscription if found, event_name matches and the wait is
    /// still open (not already signaled, not expired), None otherwise.
    /// A wait takes one signal, with or without data.
    async fn signal_activity(&self, request: SignalRequest)
    -> Result<Option<ActivitySubscription>>;

    /// Undo a signal whose ActivitySignaled event could not be published, so
    /// the wait is open again: a retried signal is accepted, and the expiry
    /// checker can time the wait out
    async fn release_signal(&self, subscription_id: Uuid) -> Result<()>;

    /// Get signal data for an activity (if it was signaled)
    async fn get_signal_data(&self, workflow_id: Uuid, activity_key: &str)
    -> Result<Option<Value>>;

    /// Mark expired subscriptions (past timeout_at, not yet signaled or expired).
    /// Sets expired_at rather than deleting, so crash recovery can find unprocessed expirations.
    async fn expire_subscriptions(&self, limit: i64) -> Result<Vec<ExpiredSubscription>>;

    /// Recover subscriptions that were marked expired but never fully processed
    /// (e.g., server crashed after expire_subscriptions but before events were published).
    async fn recover_expired(&self, limit: i64) -> Result<Vec<ExpiredSubscription>>;

    /// Delete an activity's subscription (the orchestrator, once a signal ends its wait)
    async fn delete_subscription(&self, workflow_id: Uuid, activity_key: &str) -> Result<()>;

    /// Delete one subscription by id (the expiry checker, once its timeout event
    /// is published). By id, so a later loop round's subscription for the same
    /// activity, opened in the meantime, is left alone.
    async fn delete_subscription_by_id(&self, id: Uuid) -> Result<()>;
}
