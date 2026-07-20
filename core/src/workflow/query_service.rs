use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

/// Workflow query service error
#[derive(Debug, Error)]
pub enum WorkflowQueryError {
    #[error("Workflow not found: {0}")]
    WorkflowNotFound(Uuid),

    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),

    #[error("State deserialization error: {0}")]
    DeserializationError(String),
}

pub type WorkflowQueryResult<T> = Result<T, WorkflowQueryError>;

/// Workflow record (full detail)
#[derive(Debug, Clone)]
pub struct WorkflowRecord {
    pub id: Uuid,
    pub definition_name: String,
    pub status: String,
    pub activities: Value,
    pub state_data: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Workflow summary for list view
#[derive(Debug, Clone)]
pub struct WorkflowSummaryRecord {
    pub id: Uuid,
    pub definition_name: String,
    pub status: crate::WorkflowStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// A failed activity's error message (dead-letter visibility for list views)
    pub error_message: Option<String>,
}

/// Workflow query filters
#[derive(Debug, Clone, Default)]
pub struct WorkflowFilters {
    pub status: Option<String>,
    pub definition_name: Option<String>,
    pub created_after: Option<DateTime<Utc>>,
    pub created_before: Option<DateTime<Utc>>,
}

/// Workflow query service
///
/// Provides read-only access to workflow state and activity information.
/// Follows the repository pattern - holds PgPool and is cloneable.
#[derive(Clone)]
pub struct WorkflowQueryService {
    pool: PgPool,
}

impl WorkflowQueryService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Get workflow by ID
    ///
    /// Returns workflow record with activities and state_data.
    pub async fn get_workflow(&self, workflow_id: Uuid) -> WorkflowQueryResult<WorkflowRecord> {
        let row = sqlx::query!(
            r#"
            SELECT id, definition_name, status AS "status: String",
                   activities, state_data, created_at, updated_at
            FROM workflows
            WHERE id = $1
            "#,
            workflow_id
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(WorkflowQueryError::WorkflowNotFound(workflow_id))?;

        Ok(WorkflowRecord {
            id: row.id,
            definition_name: row.definition_name,
            status: row.status,
            activities: row.activities,
            state_data: row.state_data,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    /// List workflows with filters and pagination
    ///
    /// Returns workflows matching filter criteria with pagination support.
    pub async fn list_workflows(
        &self,
        filters: WorkflowFilters,
        limit: i64,
        offset: i64,
    ) -> WorkflowQueryResult<(Vec<WorkflowSummaryRecord>, i64)> {
        // Query workflows with filters
        let rows = sqlx::query!(
            r#"
            SELECT id, definition_name, status AS "status: crate::WorkflowStatus",
                   created_at, updated_at,
                   (SELECT a.value->>'error'
                      FROM jsonb_each(activities) AS a
                     WHERE a.value->>'status' = 'failed'
                       AND a.value->>'error' IS NOT NULL
                     LIMIT 1) AS "error_message?"
            FROM workflows
            WHERE ($1::TEXT IS NULL OR status::TEXT = $1)
              AND ($2::TEXT IS NULL OR definition_name = $2)
              AND ($3::TIMESTAMPTZ IS NULL OR created_at >= $3)
              AND ($4::TIMESTAMPTZ IS NULL OR created_at < $4)
            ORDER BY created_at DESC
            LIMIT $5 OFFSET $6
            "#,
            filters.status.as_deref(),
            filters.definition_name.as_deref(),
            filters.created_after,
            filters.created_before,
            limit,
            offset
        )
        .fetch_all(&self.pool)
        .await?;

        // Get total count (for pagination)
        let total = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*)
            FROM workflows
            WHERE ($1::TEXT IS NULL OR status::TEXT = $1)
              AND ($2::TEXT IS NULL OR definition_name = $2)
              AND ($3::TIMESTAMPTZ IS NULL OR created_at >= $3)
              AND ($4::TIMESTAMPTZ IS NULL OR created_at < $4)
            "#,
            filters.status.as_deref(),
            filters.definition_name.as_deref(),
            filters.created_after,
            filters.created_before
        )
        .fetch_one(&self.pool)
        .await?
        .unwrap_or(0);

        let workflows = rows
            .into_iter()
            .map(|row| WorkflowSummaryRecord {
                id: row.id,
                definition_name: row.definition_name,
                status: row.status,
                created_at: row.created_at,
                updated_at: row.updated_at,
                error_message: row.error_message,
            })
            .collect();

        Ok((workflows, total))
    }
}
