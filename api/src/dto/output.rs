//! Output Retrieval DTOs
//!
//! API-layer wrappers for output query types to provide OpenAPI schema generation.

use chrono::{DateTime, Utc};
use kruxiaflow_core::WorkflowStatus;
use kruxiaflow_core::orchestrator::WorkflowActivityStatus;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;
use uuid::Uuid;

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
