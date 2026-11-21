//! Subscription service for activities waiting for external signals.
//!
//! This module provides the infrastructure for activities to wait for external signals
//! before being scheduled. When an activity has `settings.wait_for_signal` configured,
//! it enters a "waiting" state instead of being scheduled immediately. A signal API
//! allows external systems to send signals that transition activities to "pending".

mod delivery;
mod models;
mod postgres_subscription;
mod service;

pub use delivery::{
    SignalDeliveryError, SignalFieldError, activity_signaled_event, deliver_signal,
};
pub use models::{ActivitySubscription, ExpiredSubscription, NewSubscription, SignalRequest};
pub use postgres_subscription::PostgresSubscriptionService;
pub use service::{Result, SubscriptionError, SubscriptionService};
