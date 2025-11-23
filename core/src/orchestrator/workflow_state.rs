use super::{OrchestratorError, Result};
use crate::events::{WorkflowEvent, WorkflowEventType, WorkflowStatus};
use crate::workflow::{ActivityOutput, WorkflowDefinition};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgConnection;
use std::collections::HashMap;
use uuid::Uuid;

/// Complete workflow state (stored in workflows table)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowState {
    pub workflow_id: Uuid,
    pub definition_name: String,
    pub status: WorkflowStatus,
    pub activities: HashMap<String, ActivityState>,
    pub state_data: serde_json::Value,
    pub input: serde_json::Value,
}

/// State of individual activity
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivityState {
    pub key: String,
    pub status: WorkflowActivityStatus,
    /// Structured outputs with type information (Value, File, or Folder)
    pub outputs: Option<Vec<ActivityOutput>>,
    pub error: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,

    /// Current attempt number (1-based)
    #[serde(default = "default_attempt")]
    pub attempt: u32,

    /// Last error message from failed attempt
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,

    /// Accumulated cost in USD across all attempts
    #[serde(default)]
    pub accumulated_cost_usd: Decimal,

    /// Current iteration number (0-based) - tracked for ALL loop activities
    #[serde(default)]
    pub iteration: u32,

    /// History of outputs from all iterations (only for iteration_scoped activities)
    /// Outputs are grouped by name: { "output_name": [value0, value1, value2, ...] }
    /// This matches the template access pattern: {{activity.output_name}} returns the array
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iteration_outputs: Option<HashMap<String, Vec<Value>>>,
}

fn default_attempt() -> u32 {
    1
}

impl ActivityState {
    /// Increment the attempt counter for retry
    pub fn increment_attempt(&mut self) {
        self.attempt += 1;
    }

    /// Set the last error message
    pub fn set_error(&mut self, error: String) {
        self.last_error = Some(error.clone());
        self.error = Some(error);
    }

    /// Add cost to accumulated total
    pub fn add_cost(&mut self, cost: Decimal) {
        self.accumulated_cost_usd += cost;
    }

    /// Increment iteration counter (for ALL looping activities, regardless of iteration_scoped)
    /// NOTE: accumulated_cost_usd is NOT reset - it tracks total across all iterations
    pub fn increment_iteration(&mut self) {
        self.iteration += 1;
    }

    /// Archive outputs to iteration history (only for iteration_scoped activities)
    pub fn archive_iteration_outputs(&mut self, current_outputs: Vec<ActivityOutput>) {
        // Only archive if iteration_outputs is initialized (iteration_scoped activities)
        if let Some(history) = &mut self.iteration_outputs {
            for output in current_outputs {
                history
                    .entry(output.name.clone())
                    .or_insert_with(Vec::new)
                    .push(output.value);
            }
        }
    }

    /// Get the latest value for a specific output across all iterations
    pub fn get_latest_output_value(&self, output_name: &str) -> Option<&Value> {
        self.iteration_outputs.as_ref()?.get(output_name)?.last()
    }

    /// Get all values for a specific output across all iterations
    pub fn get_output_values(&self, output_name: &str) -> Option<&Vec<Value>> {
        self.iteration_outputs.as_ref()?.get(output_name)
    }
}

/// Activity status in workflow (different from queue status)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowActivityStatus {
    NotScheduled, // Not yet in queue
    Pending,      // In queue, waiting for worker
    Running,      // Worker executing
    Completed,    // Finished successfully
    Failed,       // Failed permanently
    Skipped,      // Skipped due to unsatisfied conditional dependencies
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
        name: row.name,
        activities,
    })
}

/// Load workflow definition by definition ID (for WorkflowCreated events in tests)
pub async fn load_workflow_definition_by_id(
    tx: &mut PgConnection,
    definition_id: Uuid,
) -> Result<WorkflowDefinition> {
    let row = sqlx::query!(
        r#"SELECT id, name, activities, created_at
           FROM workflow_definitions
           WHERE id = $1"#,
        definition_id
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| OrchestratorError::WorkflowDefinitionNotFound(definition_id.to_string()))?;

    // Parse activities JSONB array
    let activities = serde_json::from_value(row.activities)
        .map_err(|e| OrchestratorError::StateDeserialization(e.to_string()))?;

    Ok(WorkflowDefinition {
        name: row.name,
        activities,
    })
}

/// Load materialized state from workflows table (O(1))
/// Reconstructs WorkflowState from table columns (1:1 mapping with struct)
pub async fn load_materialized_state(
    tx: &mut PgConnection,
    workflow_id: Uuid,
) -> Result<WorkflowState> {
    let row = sqlx::query!(
        r#"SELECT id, definition_name, status AS "status: WorkflowStatus", activities, state_data, input
           FROM workflows WHERE id = $1"#,
        workflow_id
    )
    .fetch_one(&mut *tx)
    .await?;

    // Deserialize activities from its own column
    let activities: HashMap<String, ActivityState> = serde_json::from_value(row.activities)
        .map_err(|e| OrchestratorError::StateDeserialization(e.to_string()))?;

    // Reconstruct WorkflowState from columns (1:1 mapping)
    Ok(WorkflowState {
        workflow_id: row.id,
        definition_name: row.definition_name,
        status: row.status,
        activities,
        state_data: row.state_data,
        input: row.input,
    })
}

/// Save updated state to workflows table
/// Stores activities and state_data in separate JSONB columns (1:1 with WorkflowState struct)
pub async fn save_materialized_state(
    tx: &mut PgConnection,
    workflow_id: Uuid,
    state: &WorkflowState,
) -> Result<()> {
    // Serialize activities and state_data separately to their respective columns
    let activities_json = serde_json::to_value(&state.activities)
        .map_err(|e| OrchestratorError::StateSerialization(e.to_string()))?;

    sqlx::query!(
        r#"UPDATE workflows
           SET activities = $1, state_data = $2, status = $3, updated_at = NOW()
           WHERE id = $4"#,
        activities_json,
        state.state_data,
        state.status as WorkflowStatus,
        workflow_id
    )
    .execute(&mut *tx)
    .await?;

    Ok(())
}

/// Initialize workflow state when WorkflowCreated event is consumed
/// Handles both production (workflow row exists) and test (row doesn't exist) scenarios
pub async fn initialize_workflow_state(
    tx: &mut PgConnection,
    workflow_id: Uuid,
    definition: &WorkflowDefinition,
    initial_state_data: Option<serde_json::Value>,
    workflow_definition_id: Option<Uuid>,
    input: Option<serde_json::Value>,
) -> Result<WorkflowState> {
    // Initialize all activities as NotScheduled, with iteration_outputs for iteration_scoped activities
    let mut activities = HashMap::new();
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
                attempt: 1,
                last_error: None,
                accumulated_cost_usd: Decimal::ZERO,
                iteration: 0,
                // Initialize iteration_outputs for iteration_scoped activities
                iteration_outputs: if activity.iteration_scoped {
                    Some(HashMap::new())
                } else {
                    None
                },
            },
        );
    }

    let state = WorkflowState {
        workflow_id,
        definition_name: definition.name.clone(),
        status: WorkflowStatus::Running,
        activities: activities.clone(),
        state_data: initial_state_data.unwrap_or_else(|| json!({})),
        input: input.clone().unwrap_or_else(|| json!({})),
    };

    // Serialize activities and state_data for storage
    let activities_json = serde_json::to_value(&state.activities)
        .map_err(|e| OrchestratorError::StateSerialization(e.to_string()))?;

    // UPSERT: Insert if workflow doesn't exist (test scenario), update if it does (production)
    // In production, WorkflowService creates the row before publishing the event
    // In tests, the row might not exist yet
    if let Some(def_id) = workflow_definition_id {
        sqlx::query!(
            r#"INSERT INTO workflows (id, definition_name, workflow_definition_id, input, status, 
                                      activities, state_data, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, NOW(), NOW())
               ON CONFLICT (id) DO UPDATE
               SET activities = $6, state_data = $7, status = $5, updated_at = NOW()"#,
            workflow_id,
            state.definition_name,
            def_id,
            input.unwrap_or_else(|| json!({})),
            state.status as WorkflowStatus,
            activities_json,
            state.state_data
        )
        .execute(&mut *tx)
        .await?;
    } else {
        // Fallback to UPDATE only (production path where row already exists)
        save_materialized_state(tx, workflow_id, &state).await?;
    }

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

                // Convert flat outputs object to Vec<ActivityOutput>
                // Event payload has: {"outputs": {"response": {...}, "data": {...}}}
                // We need: [ActivityOutput { name: "response", type: "value", value: {...} }, ...]
                activity.outputs = event.payload.get("outputs").and_then(|v| {
                    if let Value::Object(outputs_map) = v {
                        let outputs: Vec<ActivityOutput> = outputs_map
                            .iter()
                            .map(|(name, value)| ActivityOutput::value(name.clone(), value.clone()))
                            .collect();
                        Some(outputs)
                    } else {
                        None
                    }
                });

                // Accumulate cost if present in event payload
                if let Some(cost_value) = event.payload.get("cost_usd") {
                    if let Some(cost_str) = cost_value.as_str() {
                        if let Ok(cost) = cost_str.parse::<Decimal>() {
                            activity.add_cost(cost);
                        }
                    } else if let Some(cost_num) = cost_value.as_f64()
                        && let Some(cost) = Decimal::from_f64_retain(cost_num)
                    {
                        activity.add_cost(cost);
                    }
                }

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_increment_iteration() {
        let mut state = ActivityState {
            key: "test".to_string(),
            status: WorkflowActivityStatus::NotScheduled,
            outputs: None,
            error: None,
            started_at: None,
            completed_at: None,
            attempt: 1,
            last_error: None,
            accumulated_cost_usd: Decimal::from(5),
            iteration: 0,
            iteration_outputs: Some(HashMap::new()),
        };

        assert_eq!(state.iteration, 0);
        assert_eq!(state.accumulated_cost_usd, Decimal::from(5));

        state.increment_iteration();

        assert_eq!(state.iteration, 1);
        // Cost should NOT be reset
        assert_eq!(state.accumulated_cost_usd, Decimal::from(5));
    }

    #[test]
    fn test_archive_iteration_outputs() {
        let mut state = ActivityState {
            key: "test".to_string(),
            status: WorkflowActivityStatus::NotScheduled,
            outputs: None,
            error: None,
            started_at: None,
            completed_at: None,
            attempt: 1,
            last_error: None,
            accumulated_cost_usd: Decimal::ZERO,
            iteration: 0,
            iteration_outputs: Some(HashMap::new()),
        };

        // Archive first iteration
        state.archive_iteration_outputs(vec![
            ActivityOutput::value("result".to_string(), json!("value1")),
            ActivityOutput::value("score".to_string(), json!(10)),
        ]);

        // Archive second iteration
        state.archive_iteration_outputs(vec![
            ActivityOutput::value("result".to_string(), json!("value2")),
            ActivityOutput::value("score".to_string(), json!(20)),
        ]);

        // Check that outputs are grouped by name as arrays
        let history = state.iteration_outputs.as_ref().unwrap();
        assert_eq!(
            history.get("result").unwrap(),
            &vec![json!("value1"), json!("value2")]
        );
        assert_eq!(history.get("score").unwrap(), &vec![json!(10), json!(20)]);
    }

    #[test]
    fn test_get_latest_output_value() {
        let mut state = ActivityState {
            key: "test".to_string(),
            status: WorkflowActivityStatus::NotScheduled,
            outputs: None,
            error: None,
            started_at: None,
            completed_at: None,
            attempt: 1,
            last_error: None,
            accumulated_cost_usd: Decimal::ZERO,
            iteration: 0,
            iteration_outputs: Some(HashMap::new()),
        };

        state.archive_iteration_outputs(vec![ActivityOutput::value(
            "result".to_string(),
            json!("value1"),
        )]);
        state.archive_iteration_outputs(vec![ActivityOutput::value(
            "result".to_string(),
            json!("value2"),
        )]);

        let latest = state.get_latest_output_value("result");
        assert_eq!(latest, Some(&json!("value2")));
    }

    #[test]
    fn test_get_output_values() {
        let mut state = ActivityState {
            key: "test".to_string(),
            status: WorkflowActivityStatus::NotScheduled,
            outputs: None,
            error: None,
            started_at: None,
            completed_at: None,
            attempt: 1,
            last_error: None,
            accumulated_cost_usd: Decimal::ZERO,
            iteration: 0,
            iteration_outputs: Some(HashMap::new()),
        };

        state.archive_iteration_outputs(vec![ActivityOutput::value(
            "result".to_string(),
            json!("value1"),
        )]);
        state.archive_iteration_outputs(vec![ActivityOutput::value(
            "result".to_string(),
            json!("value2"),
        )]);

        let all_values = state.get_output_values("result");
        assert_eq!(all_values, Some(&vec![json!("value1"), json!("value2")]));
    }

    #[test]
    fn test_non_iteration_scoped_state() {
        let mut state = ActivityState {
            key: "test".to_string(),
            status: WorkflowActivityStatus::NotScheduled,
            outputs: None,
            error: None,
            started_at: None,
            completed_at: None,
            attempt: 1,
            last_error: None,
            accumulated_cost_usd: Decimal::ZERO,
            iteration: 0,
            iteration_outputs: None, // Not iteration-scoped
        };

        // Archive should do nothing for non-iteration-scoped activities
        state.archive_iteration_outputs(vec![ActivityOutput::value(
            "result".to_string(),
            json!("value1"),
        )]);

        assert!(state.iteration_outputs.is_none());
        assert_eq!(state.get_latest_output_value("result"), None);
        assert_eq!(state.get_output_values("result"), None);
    }

    #[test]
    fn test_iteration_counter_without_scoping() {
        let mut state = ActivityState {
            key: "test".to_string(),
            status: WorkflowActivityStatus::NotScheduled,
            outputs: None,
            error: None,
            started_at: None,
            completed_at: None,
            attempt: 1,
            last_error: None,
            accumulated_cost_usd: Decimal::ZERO,
            iteration: 0,
            iteration_outputs: None, // Not iteration-scoped
        };

        // Iteration counter should still work even without iteration_outputs
        state.increment_iteration();
        assert_eq!(state.iteration, 1);

        state.increment_iteration();
        assert_eq!(state.iteration, 2);
    }
}
