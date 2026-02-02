//! Output Query Service
//!
//! Provides dedicated endpoints for retrieving activity outputs and workflow results
//! without returning the full workflow state.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use thiserror::Error;
use uuid::Uuid;

use crate::WorkflowStatus;
use crate::orchestrator::WorkflowActivityStatus;
use crate::storage::{FileMetadata, StorageError, WorkflowStorage};

/// Output query service error
#[derive(Debug, Error)]
pub enum OutputQueryError {
    #[error("Workflow not found: {0}")]
    WorkflowNotFound(Uuid),

    #[error("Activity not found: {0}")]
    ActivityNotFound(String),

    #[error("Activity not completed: {0}")]
    ActivityNotCompleted(String),

    #[error("Workflow not completed")]
    WorkflowNotCompleted,

    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),

    #[error("Storage error: {0}")]
    StorageError(#[from] StorageError),

    #[error("Deserialization error: {0}")]
    DeserializationError(String),
}

pub type OutputQueryResult<T> = Result<T, OutputQueryError>;

/// File information for activity output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub filename: String,
    pub size: i64,
    pub content_type: Option<String>,
    pub download_url: String,
}

impl From<FileMetadata> for FileInfo {
    fn from(metadata: FileMetadata) -> Self {
        Self {
            download_url: format!(
                "/api/v1/workflows/{}/activities/{}/files/{}",
                metadata.workflow_id, metadata.activity_key, metadata.filename
            ),
            filename: metadata.filename,
            size: metadata.size,
            content_type: metadata.content_type,
        }
    }
}

/// Activity output with cost and file information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityOutputResult {
    pub workflow_id: Uuid,
    pub activity_key: String,
    pub status: WorkflowActivityStatus,
    pub output: Option<serde_json::Value>,
    pub cost_usd: Decimal,
    pub completed_at: Option<DateTime<Utc>>,
    pub files: Vec<FileInfo>,
}

/// Activity output summary for workflow output response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityOutputSummary {
    pub status: WorkflowActivityStatus,
    pub output: Option<serde_json::Value>,
    pub cost_usd: Decimal,
    pub completed_at: Option<DateTime<Utc>>,
    pub is_terminal: bool,
}

/// Workflow output with all activity outputs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowOutputResult {
    pub workflow_id: Uuid,
    pub status: WorkflowStatus,
    pub total_cost_usd: Decimal,
    pub completed_at: Option<DateTime<Utc>>,
    pub outputs: HashMap<String, ActivityOutputSummary>,
    pub terminal_outputs: Vec<String>,
}

/// Output query service
///
/// Provides read-only access to activity and workflow outputs.
/// Follows the repository pattern - holds PgPool and is cloneable.
#[derive(Clone)]
pub struct OutputQueryService {
    pool: PgPool,
}

impl OutputQueryService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Get activity output for a specific activity
    ///
    /// Returns activity output with cost information and file list.
    /// Returns error if activity doesn't exist or is not completed.
    pub async fn get_activity_output(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
        storage: &dyn WorkflowStorage,
    ) -> OutputQueryResult<ActivityOutputResult> {
        // Query workflow and activity state
        let row = sqlx::query!(
            r#"
            SELECT
                w.id,
                w.status AS "status: String",
                w.activities->>$2 AS activity_data,
                COALESCE(
                    (SELECT SUM(cost_usd)
                     FROM activity_costs
                     WHERE workflow_id = $1 AND activity_key = $2),
                    0.0
                ) AS "cost_usd!"
            FROM workflows w
            WHERE w.id = $1
            "#,
            workflow_id,
            activity_key
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(OutputQueryError::WorkflowNotFound(workflow_id))?;

        // Parse activity data
        let activity_data = row
            .activity_data
            .ok_or_else(|| OutputQueryError::ActivityNotFound(activity_key.to_string()))?;

        let activity_json: serde_json::Value = serde_json::from_str(&activity_data)
            .map_err(|e| OutputQueryError::DeserializationError(e.to_string()))?;

        // Extract status
        let status_str = activity_json
            .get("status")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                OutputQueryError::DeserializationError("Missing status field".to_string())
            })?;

        let status: WorkflowActivityStatus =
            serde_json::from_value(serde_json::Value::String(status_str.to_string()))
                .map_err(|e| OutputQueryError::DeserializationError(e.to_string()))?;

        // Check if activity is completed
        if status != WorkflowActivityStatus::Completed {
            return Err(OutputQueryError::ActivityNotCompleted(
                activity_key.to_string(),
            ));
        }

        // Extract output
        let output = activity_json.get("outputs").cloned();

        // Extract completed_at
        let completed_at = activity_json
            .get("completed_at")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));

        // Get file list from storage
        let files = storage
            .list_files(workflow_id, activity_key)
            .await?
            .into_iter()
            .map(FileInfo::from)
            .collect();

        Ok(ActivityOutputResult {
            workflow_id,
            activity_key: activity_key.to_string(),
            status,
            output,
            cost_usd: row.cost_usd,
            completed_at,
            files,
        })
    }

    /// Get workflow output with all activity outputs
    ///
    /// Returns aggregated outputs from all completed activities,
    /// with terminal activities marked.
    pub async fn get_workflow_output(
        &self,
        workflow_id: Uuid,
    ) -> OutputQueryResult<WorkflowOutputResult> {
        // Query workflow state
        let row = sqlx::query!(
            r#"
            SELECT
                w.id,
                w.status AS "status: WorkflowStatus",
                w.activities,
                w.updated_at,
                COALESCE(
                    (SELECT SUM(cost_usd)
                     FROM activity_costs
                     WHERE workflow_id = $1),
                    0.0
                ) AS "total_cost_usd!"
            FROM workflows w
            WHERE w.id = $1
            "#,
            workflow_id
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(OutputQueryError::WorkflowNotFound(workflow_id))?;

        // Check if workflow is completed
        if row.status != WorkflowStatus::Completed {
            return Err(OutputQueryError::WorkflowNotCompleted);
        }

        // Parse activities
        let activities_map = row.activities.as_object().ok_or_else(|| {
            OutputQueryError::DeserializationError("Invalid activities".to_string())
        })?;

        // Get per-activity costs
        let activity_costs = self.get_activity_costs(workflow_id).await?;

        // Determine terminal activities (those with no dependents)
        let terminal_activities = self.find_terminal_activities(&row.activities)?;

        // Build outputs map
        let mut outputs = HashMap::new();
        let mut workflow_completed_at: Option<DateTime<Utc>> = None;

        for (key, activity_state) in activities_map {
            let status_str = activity_state
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("not_scheduled");

            let status: WorkflowActivityStatus =
                serde_json::from_value(serde_json::Value::String(status_str.to_string()))
                    .unwrap_or(WorkflowActivityStatus::NotScheduled);

            let output = activity_state.get("outputs").cloned();

            let completed_at = activity_state
                .get("completed_at")
                .and_then(|v| v.as_str())
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc));

            let is_terminal = terminal_activities.contains(key.as_str());

            // Track the latest completion time for terminal activities
            if is_terminal && let Some(ca) = completed_at {
                match workflow_completed_at {
                    None => workflow_completed_at = Some(ca),
                    Some(existing) if ca > existing => workflow_completed_at = Some(ca),
                    _ => {}
                }
            }

            let cost_usd = activity_costs.get(key).cloned().unwrap_or(Decimal::ZERO);

            outputs.insert(
                key.clone(),
                ActivityOutputSummary {
                    status,
                    output,
                    cost_usd,
                    completed_at,
                    is_terminal,
                },
            );
        }

        Ok(WorkflowOutputResult {
            workflow_id,
            status: row.status,
            total_cost_usd: row.total_cost_usd,
            completed_at: workflow_completed_at,
            outputs,
            terminal_outputs: terminal_activities.into_iter().collect(),
        })
    }

    /// Get per-activity cost aggregations
    async fn get_activity_costs(
        &self,
        workflow_id: Uuid,
    ) -> OutputQueryResult<HashMap<String, Decimal>> {
        let rows = sqlx::query!(
            r#"
            SELECT activity_key, SUM(cost_usd) AS "total_cost!"
            FROM activity_costs
            WHERE workflow_id = $1
            GROUP BY activity_key
            "#,
            workflow_id
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| (r.activity_key, r.total_cost))
            .collect())
    }

    /// Find terminal activities (those that no other activity depends on)
    fn find_terminal_activities(
        &self,
        activities: &serde_json::Value,
    ) -> OutputQueryResult<HashSet<String>> {
        let activities_map = activities.as_object().ok_or_else(|| {
            OutputQueryError::DeserializationError("Invalid activities".to_string())
        })?;

        let all_keys: HashSet<String> = activities_map.keys().cloned().collect();
        let mut has_dependents: HashSet<String> = HashSet::new();

        // Collect all activities that are depended upon
        for (_key, activity_state) in activities_map {
            if let Some(depends_on) = activity_state.get("depends_on")
                && let Some(deps) = depends_on.as_array()
            {
                for dep in deps {
                    if let Some(dep_key) = dep.get("activity_key").and_then(|v| v.as_str()) {
                        has_dependents.insert(dep_key.to_string());
                    }
                }
            }
        }

        // Terminal activities have no dependents
        Ok(all_keys.difference(&has_dependents).cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Helper function to find terminal activities without needing a service instance.
    /// This mirrors the logic in OutputQueryService::find_terminal_activities.
    fn find_terminal_activities_pure(
        activities: &serde_json::Value,
    ) -> OutputQueryResult<HashSet<String>> {
        let activities_map = activities.as_object().ok_or_else(|| {
            OutputQueryError::DeserializationError("Invalid activities".to_string())
        })?;

        let all_keys: HashSet<String> = activities_map.keys().cloned().collect();
        let mut has_dependents: HashSet<String> = HashSet::new();

        for (_key, activity_state) in activities_map {
            if let Some(depends_on) = activity_state.get("depends_on")
                && let Some(deps) = depends_on.as_array()
            {
                for dep in deps {
                    if let Some(dep_key) = dep.get("activity_key").and_then(|v| v.as_str()) {
                        has_dependents.insert(dep_key.to_string());
                    }
                }
            }
        }

        Ok(all_keys.difference(&has_dependents).cloned().collect())
    }

    #[test]
    fn test_find_terminal_activities_single_activity() {
        let activities = json!({
            "step1": {
                "status": "completed"
            }
        });

        let result = find_terminal_activities_pure(&activities).unwrap();

        assert_eq!(result.len(), 1);
        assert!(result.contains("step1"));
    }

    #[test]
    fn test_find_terminal_activities_linear_chain() {
        // step1 -> step2 -> step3
        // step3 is terminal (no one depends on it)
        let activities = json!({
            "step1": {
                "status": "completed"
            },
            "step2": {
                "status": "completed",
                "depends_on": [{"activity_key": "step1"}]
            },
            "step3": {
                "status": "completed",
                "depends_on": [{"activity_key": "step2"}]
            }
        });

        let result = find_terminal_activities_pure(&activities).unwrap();

        assert_eq!(result.len(), 1);
        assert!(result.contains("step3"));
    }

    #[test]
    fn test_find_terminal_activities_fan_out() {
        // step1 -> step2
        // step1 -> step3
        // Both step2 and step3 are terminal
        let activities = json!({
            "step1": {
                "status": "completed"
            },
            "step2": {
                "status": "completed",
                "depends_on": [{"activity_key": "step1"}]
            },
            "step3": {
                "status": "completed",
                "depends_on": [{"activity_key": "step1"}]
            }
        });

        let result = find_terminal_activities_pure(&activities).unwrap();

        assert_eq!(result.len(), 2);
        assert!(result.contains("step2"));
        assert!(result.contains("step3"));
    }

    #[test]
    fn test_find_terminal_activities_fan_in() {
        // step1 -> step3
        // step2 -> step3
        // step3 is the only terminal
        let activities = json!({
            "step1": {
                "status": "completed"
            },
            "step2": {
                "status": "completed"
            },
            "step3": {
                "status": "completed",
                "depends_on": [
                    {"activity_key": "step1"},
                    {"activity_key": "step2"}
                ]
            }
        });

        let result = find_terminal_activities_pure(&activities).unwrap();

        assert_eq!(result.len(), 1);
        assert!(result.contains("step3"));
    }

    #[test]
    fn test_find_terminal_activities_diamond_pattern() {
        // Diamond pattern:
        //     step1
        //    /     \
        // step2   step3
        //    \     /
        //     step4
        let activities = json!({
            "step1": {
                "status": "completed"
            },
            "step2": {
                "status": "completed",
                "depends_on": [{"activity_key": "step1"}]
            },
            "step3": {
                "status": "completed",
                "depends_on": [{"activity_key": "step1"}]
            },
            "step4": {
                "status": "completed",
                "depends_on": [
                    {"activity_key": "step2"},
                    {"activity_key": "step3"}
                ]
            }
        });

        let result = find_terminal_activities_pure(&activities).unwrap();

        assert_eq!(result.len(), 1);
        assert!(result.contains("step4"));
    }

    #[test]
    fn test_find_terminal_activities_multiple_independent_chains() {
        // Two independent chains:
        // chain1: a -> b
        // chain2: c -> d
        // Both b and d are terminal
        let activities = json!({
            "a": {
                "status": "completed"
            },
            "b": {
                "status": "completed",
                "depends_on": [{"activity_key": "a"}]
            },
            "c": {
                "status": "completed"
            },
            "d": {
                "status": "completed",
                "depends_on": [{"activity_key": "c"}]
            }
        });

        let result = find_terminal_activities_pure(&activities).unwrap();

        assert_eq!(result.len(), 2);
        assert!(result.contains("b"));
        assert!(result.contains("d"));
    }

    #[test]
    fn test_find_terminal_activities_empty_depends_on() {
        // Empty depends_on array should not cause issues
        let activities = json!({
            "step1": {
                "status": "completed",
                "depends_on": []
            },
            "step2": {
                "status": "completed",
                "depends_on": [{"activity_key": "step1"}]
            }
        });

        let result = find_terminal_activities_pure(&activities).unwrap();

        assert_eq!(result.len(), 1);
        assert!(result.contains("step2"));
    }

    #[test]
    fn test_find_terminal_activities_invalid_json() {
        let activities = json!("not an object");

        let result = find_terminal_activities_pure(&activities);

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            OutputQueryError::DeserializationError(_)
        ));
    }

    #[test]
    fn test_file_info_from_metadata() {
        use crate::storage::FileMetadata;
        use chrono::Utc;

        let workflow_id = Uuid::now_v7();
        let metadata = FileMetadata {
            workflow_id,
            activity_key: "process_data".to_string(),
            filename: "output.json".to_string(),
            size: 1024,
            content_type: Some("application/json".to_string()),
            created_at: Utc::now(),
        };

        let file_info = FileInfo::from(metadata);

        assert_eq!(file_info.filename, "output.json");
        assert_eq!(file_info.size, 1024);
        assert_eq!(file_info.content_type, Some("application/json".to_string()));
        assert_eq!(
            file_info.download_url,
            format!(
                "/api/v1/workflows/{}/activities/process_data/files/output.json",
                workflow_id
            )
        );
    }

    #[test]
    fn test_file_info_from_metadata_no_content_type() {
        use crate::storage::FileMetadata;
        use chrono::Utc;

        let workflow_id = Uuid::now_v7();
        let metadata = FileMetadata {
            workflow_id,
            activity_key: "step1".to_string(),
            filename: "data.bin".to_string(),
            size: 0,
            content_type: None,
            created_at: Utc::now(),
        };

        let file_info = FileInfo::from(metadata);
        assert_eq!(file_info.filename, "data.bin");
        assert_eq!(file_info.size, 0);
        assert!(file_info.content_type.is_none());
    }

    // =========================================================================
    // OutputQueryError Display Tests
    // =========================================================================

    #[test]
    fn test_output_query_error_workflow_not_found() {
        let id = Uuid::now_v7();
        let err = OutputQueryError::WorkflowNotFound(id);
        assert!(err.to_string().contains(&id.to_string()));
        assert!(err.to_string().contains("Workflow not found"));
    }

    #[test]
    fn test_output_query_error_activity_not_found() {
        let err = OutputQueryError::ActivityNotFound("step1".to_string());
        assert_eq!(err.to_string(), "Activity not found: step1");
    }

    #[test]
    fn test_output_query_error_activity_not_completed() {
        let err = OutputQueryError::ActivityNotCompleted("step2".to_string());
        assert_eq!(err.to_string(), "Activity not completed: step2");
    }

    #[test]
    fn test_output_query_error_workflow_not_completed() {
        let err = OutputQueryError::WorkflowNotCompleted;
        assert_eq!(err.to_string(), "Workflow not completed");
    }

    #[test]
    fn test_output_query_error_file_not_found() {
        let err = OutputQueryError::FileNotFound("missing.pdf".to_string());
        assert_eq!(err.to_string(), "File not found: missing.pdf");
    }

    #[test]
    fn test_output_query_error_deserialization() {
        let err = OutputQueryError::DeserializationError("bad json".to_string());
        assert_eq!(err.to_string(), "Deserialization error: bad json");
    }

    // =========================================================================
    // ActivityOutputResult / WorkflowOutputResult Serde Tests
    // =========================================================================

    #[test]
    fn test_activity_output_result_serialization() {
        let result = ActivityOutputResult {
            workflow_id: Uuid::nil(),
            activity_key: "step1".to_string(),
            status: WorkflowActivityStatus::Completed,
            output: Some(json!({"answer": 42})),
            cost_usd: rust_decimal::Decimal::new(50, 4),
            completed_at: None,
            files: vec![],
        };

        let json_str = serde_json::to_string(&result).unwrap();
        assert!(json_str.contains("step1"));
        assert!(json_str.contains("42"));

        let deserialized: ActivityOutputResult = serde_json::from_str(&json_str).unwrap();
        assert_eq!(deserialized.activity_key, "step1");
    }

    #[test]
    fn test_workflow_output_result_serialization() {
        let mut outputs = HashMap::new();
        outputs.insert(
            "step1".to_string(),
            ActivityOutputSummary {
                status: WorkflowActivityStatus::Completed,
                output: Some(json!({"result": "ok"})),
                cost_usd: rust_decimal::Decimal::new(10, 2),
                completed_at: None,
                is_terminal: true,
            },
        );

        let result = WorkflowOutputResult {
            workflow_id: Uuid::nil(),
            status: WorkflowStatus::Completed,
            total_cost_usd: rust_decimal::Decimal::new(10, 2),
            completed_at: None,
            outputs,
            terminal_outputs: vec!["step1".to_string()],
        };

        let json_str = serde_json::to_string(&result).unwrap();
        assert!(json_str.contains("step1"));
        assert!(json_str.contains("terminal_outputs"));

        let deserialized: WorkflowOutputResult = serde_json::from_str(&json_str).unwrap();
        assert_eq!(deserialized.terminal_outputs, vec!["step1".to_string()]);
    }

    #[test]
    fn test_activity_output_summary_serialization() {
        let summary = ActivityOutputSummary {
            status: WorkflowActivityStatus::Completed,
            output: None,
            cost_usd: rust_decimal::Decimal::ZERO,
            completed_at: None,
            is_terminal: false,
        };

        let json_str = serde_json::to_string(&summary).unwrap();
        let deserialized: ActivityOutputSummary = serde_json::from_str(&json_str).unwrap();
        assert!(!deserialized.is_terminal);
        assert!(deserialized.output.is_none());
    }
}
