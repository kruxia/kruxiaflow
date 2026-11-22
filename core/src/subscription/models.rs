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
    /// Loop round of the waiting activity (None outside loops)
    pub iteration: Option<i32>,
}

/// Request to create a new subscription
#[derive(Debug, Clone)]
pub struct NewSubscription {
    pub workflow_id: Uuid,
    pub activity_key: String,
    pub event_name: String,
    pub on_timeout: OnTimeout,
    pub timeout_seconds: u64,
    /// Loop round of the waiting activity (None outside loops); the events
    /// this subscription produces carry it
    pub iteration: Option<i32>,
}

/// Request to signal an activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalRequest {
    pub workflow_id: Uuid,
    pub activity_key: String,
    pub event_name: String,
    pub data: Option<Value>,
}

/// How many signals one waiting activity holds for one event name
/// (`wait_for_signal.hold_early`). A signal past the limit is refused.
pub const MAX_HELD_SIGNALS: i64 = 100;

/// What became of a signal
#[derive(Debug, Clone)]
pub enum SignalOutcome {
    /// The waiting activity took it
    Signaled(ActivitySubscription),
    /// The activity is not waiting, and its wait holds early signals: the
    /// signal is kept and delivered when the activity next waits
    Held,
    /// Not delivered and not kept
    Refused(SignalRefusal),
}

impl SignalOutcome {
    /// Whether a waiting activity took the signal
    pub fn is_signaled(&self) -> bool {
        matches!(self, SignalOutcome::Signaled(_))
    }

    /// Whether the signal is held for the activity's next wait
    pub fn is_held(&self) -> bool {
        matches!(self, SignalOutcome::Held)
    }
}

/// Why a signal was neither delivered nor held
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalRefusal {
    /// The activity waits for this event, but is not waiting now (not yet, or
    /// this round's signal already arrived) and does not hold early signals.
    /// The same signal can succeed later.
    NotWaiting,
    /// No such workflow
    UnknownWorkflow,
    /// The workflow has no activity with this key that waits for this event
    UnknownActivity,
    /// The workflow has completed or failed
    WorkflowEnded,
    /// The activity already holds [`MAX_HELD_SIGNALS`] signals for this event
    HoldLimit,
}

impl SignalRefusal {
    /// Stable code for API responses
    pub fn code(&self) -> &'static str {
        match self {
            SignalRefusal::NotWaiting => "not_waiting",
            SignalRefusal::UnknownWorkflow => "unknown_workflow",
            SignalRefusal::UnknownActivity => "unknown_activity",
            SignalRefusal::WorkflowEnded => "workflow_ended",
            SignalRefusal::HoldLimit => "hold_limit",
        }
    }

    /// One sentence for the caller, about `request`
    pub fn describe(&self, request: &SignalRequest) -> String {
        let (workflow, key, event) = (
            request.workflow_id,
            &request.activity_key,
            &request.event_name,
        );
        match self {
            SignalRefusal::NotWaiting => format!(
                "Activity '{key}' in workflow {workflow} is not waiting for '{event}' right now"
            ),
            SignalRefusal::UnknownWorkflow => format!("No workflow {workflow}"),
            SignalRefusal::UnknownActivity => {
                format!("Workflow {workflow} has no activity '{key}' that waits for '{event}'")
            }
            SignalRefusal::WorkflowEnded => format!("Workflow {workflow} has ended"),
            SignalRefusal::HoldLimit => format!(
                "Activity '{key}' in workflow {workflow} already holds {MAX_HELD_SIGNALS} '{event}' signals"
            ),
        }
    }
}

/// Information about an expired subscription
#[derive(Debug, Clone)]
pub struct ExpiredSubscription {
    pub id: Uuid,
    pub workflow_id: Uuid,
    pub activity_key: String,
    pub event_name: String,
    pub on_timeout: OnTimeout,
    /// Loop round of the waiting activity (None outside loops)
    pub iteration: Option<i32>,
}
