//! Models for activity event subscriptions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::workflow::OnTimeout;

/// An activity subscription waiting for a signal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivitySubscription {
    pub id: Uuid,
    pub workflow_id: Uuid,
    pub activity_key: String,
    pub event_name: String,
    pub on_timeout: OnTimeout,
    pub timeout_at: DateTime<Utc>,
    pub signal_data: Option<Value>,
    pub created_at: DateTime<Utc>,
}

/// Request to create a new subscription
#[derive(Debug, Clone)]
pub struct NewSubscription {
    pub workflow_id: Uuid,
    pub activity_key: String,
    pub event_name: String,
    pub on_timeout: OnTimeout,
    pub timeout_seconds: u64,
}

/// Request to signal an activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalRequest {
    pub workflow_id: Uuid,
    pub activity_key: String,
    pub event_name: String,
    pub data: Option<Value>,
}

/// Information about an expired subscription
#[derive(Debug, Clone)]
pub struct ExpiredSubscription {
    pub id: Uuid,
    pub workflow_id: Uuid,
    pub activity_key: String,
    pub event_name: String,
    pub on_timeout: OnTimeout,
}
