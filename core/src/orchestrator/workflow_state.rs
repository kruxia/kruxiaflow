use super::{OrchestratorError, Result};
use crate::events::{WorkflowDefinition, WorkflowEvent, WorkflowEventType, WorkflowStatus};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgConnection;
use std::collections::HashMap;
use uuid::Uuid;

/// Complete workflow state (stored in workflows.state_data JSONB)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowState {
    pub workflow_id: Uuid,
    pub workflow_type: String,
    pub status: WorkflowStatus,
    pub activities: HashMap<String, ActivityState>,
    pub state_data: serde_json::Value,
}

/// State of individual activity
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivityState {
    pub key: String,
    pub status: WorkflowActivityStatus,
    pub outputs: Option<serde_json::Value>,
    pub error: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub retry_count: u32,
}

/// Activity status in workflow (different from queue status)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkflowActivityStatus {
    NotScheduled, // Not yet in queue
    Pending,      // In queue, waiting for worker
    Running,      // Worker executing
    Completed,    // Finished successfully
    Failed,       // Failed permanently
}

/// Load workflow definition from database
pub async fn load_workflow_definition(
    tx: &mut PgConnection,
    workflow_id: Uuid,
) -> Result<WorkflowDefinition> {
    let row = sqlx::query!(
        r#"SELECT wd.id, wd.name, wd.activities, wd.created_at
           FROM workflows w
           JOIN workflow_definitions wd ON wd.id = w.workflow_definition_id
           WHERE w.id = $1"#,
        workflow_id
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| OrchestratorError::WorkflowDefinitionNotFound(workflow_id.to_string()))?;

    // Parse activities JSONB array
    let activities = serde_json::from_value(row.activities)
        .map_err(|e| OrchestratorError::StateDeserialization(e.to_string()))?;

    Ok(WorkflowDefinition {
        id: row.id,
        name: row.name,
        version: crate::workflow::definition::format_version(&row.created_at),
        activities,
    })
}

/// Load materialized state from workflows table (O(1))
pub async fn load_materialized_state(
    tx: &mut PgConnection,
    workflow_id: Uuid,
) -> Result<WorkflowState> {
    let row = sqlx::query!(
        r#"SELECT state_data FROM workflows WHERE id = $1"#,
        workflow_id
    )
    .fetch_one(&mut *tx)
    .await?;

    serde_json::from_value(row.state_data)
        .map_err(|e| OrchestratorError::StateDeserialization(e.to_string()))
}

/// Save updated state to workflows table
pub async fn save_materialized_state(
    tx: &mut PgConnection,
    workflow_id: Uuid,
    state: &WorkflowState,
) -> Result<()> {
    let state_json = serde_json::to_value(state)
        .map_err(|e| OrchestratorError::StateSerialization(e.to_string()))?;

    sqlx::query!(
        r#"UPDATE workflows
           SET state_data = $1, updated_at = NOW()
           WHERE id = $2"#,
        state_json,
        workflow_id
    )
    .execute(&mut *tx)
    .await?;

    Ok(())
}

/// Initialize workflow state when WorkflowCreated event is consumed
pub async fn initialize_workflow_state(
    tx: &mut PgConnection,
    workflow_id: Uuid,
    definition: &WorkflowDefinition,
    initial_state_data: Option<serde_json::Value>,
) -> Result<WorkflowState> {
    let mut activities = HashMap::new();

    // Initialize all activities as NotScheduled
    for activity in &definition.activities {
        activities.insert(
            activity.key.clone(),
            ActivityState {
                key: activity.key.clone(),
                status: WorkflowActivityStatus::NotScheduled,
                outputs: None,
                error: None,
                started_at: None,
                completed_at: None,
                retry_count: 0,
            },
        );
    }

    let state = WorkflowState {
        workflow_id,
        workflow_type: definition.name.clone(),
        status: WorkflowStatus::Running,
        activities,
        state_data: initial_state_data.unwrap_or_else(|| json!({})),
    };

    // Save initial state to database
    save_materialized_state(tx, workflow_id, &state).await?;

    Ok(state)
}

/// Apply a single event to update state incrementally (not full reconstruction)
pub fn apply_event_to_state(state: &mut WorkflowState, event: &WorkflowEvent) -> Result<()> {
    match event.event_type {
        WorkflowEventType::WorkflowCreated => {
            // Initial state setup (if any custom state data in payload)
            if let Some(initial_state) = event.payload.get("state_data") {
                state.state_data = initial_state.clone();
            }
        }
        WorkflowEventType::ActivityScheduled => {
            let activity_key = event
                .activity_key
                .as_ref()
                .ok_or(OrchestratorError::MissingActivityKey)?;

            if let Some(activity) = state.activities.get_mut(activity_key) {
                activity.status = WorkflowActivityStatus::Pending;
                activity.started_at = Some(Utc::now());
            }
        }
        WorkflowEventType::ActivityCompleted => {
            let activity_key = event
                .activity_key
                .as_ref()
                .ok_or(OrchestratorError::MissingActivityKey)?;

            if let Some(activity) = state.activities.get_mut(activity_key) {
                activity.status = WorkflowActivityStatus::Completed;
                activity.outputs = event.payload.get("outputs").cloned();
                activity.completed_at = Some(Utc::now());
            }
        }
        WorkflowEventType::ActivityFailed => {
            let activity_key = event
                .activity_key
                .as_ref()
                .ok_or(OrchestratorError::MissingActivityKey)?;

            if let Some(activity) = state.activities.get_mut(activity_key) {
                activity.status = WorkflowActivityStatus::Failed;
                activity.error = event
                    .payload
                    .get("error")
                    .and_then(|e| e.as_str())
                    .map(String::from);
                activity.completed_at = Some(Utc::now());
            }
        }
        WorkflowEventType::WorkflowCompleted => {
            state.status = WorkflowStatus::Completed;
        }
        WorkflowEventType::WorkflowFailed => {
            state.status = WorkflowStatus::Failed;
        }
        WorkflowEventType::WorkflowUpdated => {
            // WorkflowUpdated doesn't modify state directly
            // State is updated by individual activity events
        }
    }

    Ok(())
}
