//! Output Retrieval DTOs
//!
//! API-layer wrappers for output query types to provide OpenAPI schema generation.

use chrono::{DateTime, Utc};
use kruxiaflow_core::WorkflowStatus;
use kruxiaflow_core::orchestrator::WorkflowActivityStatus;
use kruxiaflow_core::storage::FileMetadata;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;
use uuid::Uuid;

/// Response for POST /api/v1/workflows/{workflow_id}/activities/{activity_key}/files/{filename}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UploadActivityFileResponse {
    /// Workflow ID
    #[schema(example = "550e8400-e29b-41d4-a716-446655440000")]
    pub workflow_id: Uuid,

    /// Activity key
    #[schema(example = "extract_content")]
    pub activity_key: String,

    /// Uploaded filename
    #[schema(example = "passages.jsonl")]
    pub filename: String,

    /// File size in bytes
    #[schema(example = 102400)]
    pub size: i64,

    /// MIME content type
    #[schema(example = "application/x-ndjson")]
    pub content_type: Option<String>,

    /// When the file was uploaded
    #[schema(example = "2025-11-27T10:30:00Z")]
    pub created_at: DateTime<Utc>,
}

impl From<FileMetadata> for UploadActivityFileResponse {
    fn from(metadata: FileMetadata) -> Self {
        Self {
            workflow_id: metadata.workflow_id,
            activity_key: metadata.activity_key,
            filename: metadata.filename,
            size: metadata.size,
            content_type: metadata.content_type,
            created_at: metadata.created_at,
        }
    }
}

/// File information for activity output
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FileInfo {
    /// Filename
    #[schema(example = "analysis_report.pdf")]
    pub filename: String,

    /// File size in bytes
    #[schema(example = 102400)]
    pub size: i64,

    /// MIME content type
    #[schema(example = "application/pdf")]
    pub content_type: Option<String>,

    /// Download URL for the file
    #[schema(
        example = "/api/v1/workflows/550e8400-e29b-41d4-a716-446655440000/activities/analyze_document/files/analysis_report.pdf"
    )]
    pub download_url: String,
}

impl From<kruxiaflow_core::workflow::FileInfo> for FileInfo {
    fn from(info: kruxiaflow_core::workflow::FileInfo) -> Self {
        Self {
            filename: info.filename,
            size: info.size,
            content_type: info.content_type,
            download_url: info.download_url,
        }
    }
}

/// Response for GET /api/v1/workflows/{workflow_id}/activities/{activity_key}/output
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GetActivityOutputResponse {
    /// Workflow ID
    #[schema(example = "550e8400-e29b-41d4-a716-446655440000")]
    pub workflow_id: Uuid,

    /// Activity key
    #[schema(example = "analyze_document")]
    pub activity_key: String,

    /// Activity status (will be "completed")
    #[schema(value_type = String, example = "completed")]
    pub status: WorkflowActivityStatus,

    /// Activity output (JSON value)
    #[schema(example = json!({"summary": "Document analysis complete", "categories": ["finance", "legal"], "confidence": 0.95}))]
    pub output: Option<serde_json::Value>,

    /// Total cost in USD for this activity
    #[schema(example = "0.0023")]
    pub cost_usd: Decimal,

    /// When the activity completed
    #[schema(example = "2025-11-27T10:30:00Z")]
    pub completed_at: Option<DateTime<Utc>>,

    /// Files produced by this activity
    pub files: Vec<FileInfo>,
}

impl From<kruxiaflow_core::workflow::ActivityOutputResult> for GetActivityOutputResponse {
    fn from(result: kruxiaflow_core::workflow::ActivityOutputResult) -> Self {
        Self {
            workflow_id: result.workflow_id,
            activity_key: result.activity_key,
            status: result.status,
            output: result.output,
            cost_usd: result.cost_usd,
            completed_at: result.completed_at,
            files: result.files.into_iter().map(FileInfo::from).collect(),
        }
    }
}

/// Activity output summary for workflow output response
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ActivityOutputSummary {
    /// Activity status
    #[schema(value_type = String, example = "completed")]
    pub status: WorkflowActivityStatus,

    /// Activity output (JSON value)
    #[schema(example = json!({"result": "success"}))]
    pub output: Option<serde_json::Value>,

    /// Total cost in USD for this activity
    #[schema(example = "0.0023")]
    pub cost_usd: Decimal,

    /// When the activity completed
    #[schema(example = "2025-11-27T10:30:00Z")]
    pub completed_at: Option<DateTime<Utc>>,

    /// Whether this is a terminal activity (final output)
    #[schema(example = true)]
    pub is_terminal: bool,
}

impl From<kruxiaflow_core::workflow::ActivityOutputSummary> for ActivityOutputSummary {
    fn from(summary: kruxiaflow_core::workflow::ActivityOutputSummary) -> Self {
        Self {
            status: summary.status,
            output: summary.output,
            cost_usd: summary.cost_usd,
            completed_at: summary.completed_at,
            is_terminal: summary.is_terminal,
        }
    }
}

/// Response for GET /api/v1/workflows/{workflow_id}/output
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GetWorkflowOutputResponse {
    /// Workflow ID
    #[schema(example = "550e8400-e29b-41d4-a716-446655440000")]
    pub workflow_id: Uuid,

    /// Workflow status (will be "completed")
    #[schema(value_type = String, example = "completed")]
    pub status: WorkflowStatus,

    /// Total cost in USD for all activities
    #[schema(example = "0.0145")]
    pub total_cost_usd: Decimal,

    /// When the workflow completed (latest terminal activity completion)
    #[schema(example = "2025-11-27T10:35:00Z")]
    pub completed_at: Option<DateTime<Utc>>,

    /// Outputs from all activities, keyed by activity key
    pub outputs: HashMap<String, ActivityOutputSummary>,

    /// List of terminal activity keys (those whose outputs are final workflow outputs)
    #[schema(example = json!(["generate_report"]))]
    pub terminal_outputs: Vec<String>,
}

impl From<kruxiaflow_core::workflow::WorkflowOutputResult> for GetWorkflowOutputResponse {
    fn from(result: kruxiaflow_core::workflow::WorkflowOutputResult) -> Self {
        Self {
            workflow_id: result.workflow_id,
            status: result.status,
            total_cost_usd: result.total_cost_usd,
            completed_at: result.completed_at,
            outputs: result
                .outputs
                .into_iter()
                .map(|(k, v)| (k, ActivityOutputSummary::from(v)))
                .collect(),
            terminal_outputs: result.terminal_outputs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use kruxiaflow_core::storage::FileMetadata;
    use rust_decimal::Decimal;
    use std::str::FromStr;
    use uuid::Uuid;

    #[test]
    fn test_upload_activity_file_response_from_metadata() {
        let now = Utc::now();
        let wf_id = Uuid::now_v7();
        let metadata = FileMetadata {
            workflow_id: wf_id,
            activity_key: "extract".to_string(),
            filename: "output.json".to_string(),
            size: 1024,
            content_type: Some("application/json".to_string()),
            created_at: now,
        };
        let response = UploadActivityFileResponse::from(metadata);
        assert_eq!(response.workflow_id, wf_id);
        assert_eq!(response.activity_key, "extract");
        assert_eq!(response.filename, "output.json");
        assert_eq!(response.size, 1024);
        assert_eq!(response.content_type, Some("application/json".to_string()));
    }

    #[test]
    fn test_upload_activity_file_response_no_content_type() {
        let metadata = FileMetadata {
            workflow_id: Uuid::now_v7(),
            activity_key: "step".to_string(),
            filename: "data.bin".to_string(),
            size: 0,
            content_type: None,
            created_at: Utc::now(),
        };
        let response = UploadActivityFileResponse::from(metadata);
        assert!(response.content_type.is_none());
    }

    #[test]
    fn test_file_info_from_core() {
        let core = kruxiaflow_core::workflow::FileInfo {
            filename: "report.pdf".to_string(),
            size: 5000,
            content_type: Some("application/pdf".to_string()),
            download_url: "/api/v1/workflows/abc/activities/gen/files/report.pdf".to_string(),
        };
        let api = FileInfo::from(core);
        assert_eq!(api.filename, "report.pdf");
        assert_eq!(api.size, 5000);
        assert!(api.download_url.contains("report.pdf"));
    }

    #[test]
    fn test_activity_output_summary_from_core() {
        let now = Utc::now();
        let core = kruxiaflow_core::workflow::ActivityOutputSummary {
            status: WorkflowActivityStatus::Completed,
            output: Some(serde_json::json!({"result": "ok"})),
            cost_usd: Decimal::from_str("0.05").unwrap(),
            completed_at: Some(now),
            is_terminal: true,
        };
        let api = ActivityOutputSummary::from(core);
        assert!(api.is_terminal);
        assert_eq!(api.cost_usd, Decimal::from_str("0.05").unwrap());
        assert!(api.output.is_some());
    }

    #[test]
    fn test_get_activity_output_response_from_core() {
        let wf_id = Uuid::now_v7();
        let core = kruxiaflow_core::workflow::ActivityOutputResult {
            workflow_id: wf_id,
            activity_key: "analyze".to_string(),
            status: WorkflowActivityStatus::Completed,
            output: Some(serde_json::json!({"summary": "done"})),
            cost_usd: Decimal::from_str("0.01").unwrap(),
            completed_at: Some(Utc::now()),
            files: vec![kruxiaflow_core::workflow::FileInfo {
                filename: "out.txt".to_string(),
                size: 100,
                content_type: None,
                download_url: "/files/out.txt".to_string(),
            }],
        };
        let api = GetActivityOutputResponse::from(core);
        assert_eq!(api.workflow_id, wf_id);
        assert_eq!(api.activity_key, "analyze");
        assert_eq!(api.files.len(), 1);
        assert_eq!(api.files[0].filename, "out.txt");
    }

    #[test]
    fn test_get_workflow_output_response_from_core() {
        let wf_id = Uuid::now_v7();
        let mut outputs = HashMap::new();
        outputs.insert(
            "step1".to_string(),
            kruxiaflow_core::workflow::ActivityOutputSummary {
                status: WorkflowActivityStatus::Completed,
                output: Some(serde_json::json!({"x": 1})),
                cost_usd: Decimal::from_str("0.02").unwrap(),
                completed_at: Some(Utc::now()),
                is_terminal: false,
            },
        );
        let core = kruxiaflow_core::workflow::WorkflowOutputResult {
            workflow_id: wf_id,
            status: WorkflowStatus::Completed,
            total_cost_usd: Decimal::from_str("0.02").unwrap(),
            completed_at: Some(Utc::now()),
            outputs,
            terminal_outputs: vec!["step1".to_string()],
        };
        let api = GetWorkflowOutputResponse::from(core);
        assert_eq!(api.workflow_id, wf_id);
        assert_eq!(api.outputs.len(), 1);
        assert!(api.outputs.contains_key("step1"));
        assert_eq!(api.terminal_outputs, vec!["step1"]);
    }

    #[test]
    fn test_upload_response_serialization() {
        let response = UploadActivityFileResponse {
            workflow_id: Uuid::nil(),
            activity_key: "test".to_string(),
            filename: "file.txt".to_string(),
            size: 42,
            content_type: Some("text/plain".to_string()),
            created_at: Utc::now(),
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["size"], 42);
        assert_eq!(json["content_type"], "text/plain");
    }
}
