use crate::events::models::{WorkflowEventType, WorkflowStatus};
use crate::workflow::repository::{RepositoryError, WorkflowDefinitionRepository};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

/// Workflow service error
#[derive(Debug, Error)]
pub enum WorkflowServiceError {
    #[error("Workflow definition not found: {name} version {version}")]
    DefinitionNotFound { name: String, version: String },

    #[error("Workflow definition not found: {name} (no version specified)")]
    DefinitionNotFoundLatest { name: String },

    #[error("Duplicate workflow submission: unique_key '{0}' already exists")]
    DuplicateSubmission(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),

    #[error("Repository error: {0}")]
    RepositoryError(#[from] RepositoryError),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
}

pub type WorkflowServiceResult<T> = Result<T, WorkflowServiceError>;

/// Created workflow record
#[derive(Debug, Clone)]
pub struct CreatedWorkflow {
    pub id: Uuid,
    pub definition_name: String,      // Workflow definition name
    pub workflow_definition_id: Uuid, // FK to workflow_definitions
    pub definition_version: String,   // Formatted as YYYYmmdd.HHMMSS.uuuuuu
    pub input: Value,
    pub unique_key: Option<String>,
    pub status: String, // Will be 'created'
    pub created_at: DateTime<Utc>,
}

/// Workflow service
///
/// Orchestrates workflow creation, validation, and event publishing.
/// Following the repository pattern - holds PgPool and is cloneable.
#[derive(Clone)]
pub struct WorkflowService {
    pool: PgPool,
}

impl WorkflowService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Submit a new workflow
    ///
    /// This method:
    /// 1. Resolves workflow definition (by name and optional version)
    /// 2. Validates input structure (basic JSON validation)
    /// 3. Checks for duplicate unique_key (idempotency)
    /// 4. Creates workflow record in database
    /// 5. Publishes WorkflowCreated event
    /// 6. Returns workflow ID immediately (async execution)
    ///
    /// All operations are atomic (transaction).
    pub async fn submit_workflow(
        &self,
        definition_name: &str,
        version: Option<&str>,
        input: Value,
        unique_key: Option<String>,
        budget_limit_usd: Option<Decimal>,
    ) -> WorkflowServiceResult<CreatedWorkflow> {
        // Create repository for definition lookup
        let repo = WorkflowDefinitionRepository::new(self.pool.clone());

        // Resolve workflow definition (latest if version not specified)
        let definition = if let Some(v) = version {
            repo.get(definition_name, v)
                .await
                .map_err(WorkflowServiceError::RepositoryError)?
                .ok_or_else(|| WorkflowServiceError::DefinitionNotFound {
                    name: definition_name.to_string(),
                    version: v.to_string(),
                })?
        } else {
            repo.get_latest(definition_name)
                .await
                .map_err(WorkflowServiceError::RepositoryError)?
                .ok_or_else(|| WorkflowServiceError::DefinitionNotFoundLatest {
                    name: definition_name.to_string(),
                })?
        };

        // Basic input validation (activities will validate semantically)
        Self::validate_input(&input)?;

        // Start transaction for atomic workflow + event creation
        let mut tx = self.pool.begin().await?;

        // Check for duplicate unique_key (idempotency)
        if let Some(ref key) = unique_key {
            let existing = sqlx::query!(
                r#"
                SELECT id, definition_name, created_at
                FROM workflows
                WHERE unique_key = $1
                "#,
                key
            )
            .fetch_optional(&mut *tx)
            .await?;

            if existing.is_some() {
                return Err(WorkflowServiceError::DuplicateSubmission(key.clone()));
            }
        }

        // Create workflow record
        let workflow_id = Uuid::now_v7();
        let status = WorkflowStatus::Created;

        // Initialize with empty activities and state_data
        // Orchestrator will populate activities when processing WorkflowCreated event
        // Table columns map 1:1 to WorkflowState struct fields
        let initial_activities = serde_json::json!({});
        let initial_state_data = serde_json::json!({});

        // Persist the workflow-level budget limit so budget checks and cost
        // endpoints see it. An explicit per-submission limit overrides the
        // definition's settings.budget default.
        let budget_limit_usd = budget_limit_usd.or_else(|| {
            definition
                .settings
                .as_ref()
                .and_then(|s| s.budget.as_ref())
                .map(|b| b.limit)
        });

        let row = sqlx::query!(
            r#"
            INSERT INTO workflows (
                id, definition_name, workflow_definition_id,
                input, unique_key, status, activities, state_data,
                budget_limit_usd, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NOW(), NOW())
            RETURNING id, definition_name, workflow_definition_id,
                      input, unique_key, status AS "status: WorkflowStatus", created_at
            "#,
            workflow_id,
            definition.name, // definition_name
            definition.id,   // workflow_definition_id
            input,
            unique_key,
            status as WorkflowStatus,
            initial_activities,
            initial_state_data,
            budget_limit_usd
        )
        .fetch_one(&mut *tx)
        .await?;

        // Publish WorkflowCreated event
        let event_id = Uuid::now_v7();
        let event_type = WorkflowEventType::WorkflowCreated;
        let event_payload = serde_json::json!({
            "definition_name": definition.name,
            "definition_version": definition.version,
            "input": input
        });

        sqlx::query!(
            r#"
            INSERT INTO workflow_events (id, workflow_id, event_type, activity_key, payload, timestamp)
            VALUES ($1, $2, $3, $4, $5, NOW())
            "#,
            event_id,
            workflow_id,
            event_type as WorkflowEventType,
            None::<String>, // activity_key is None for WorkflowCreated events
            event_payload
        )
        .execute(&mut *tx)
        .await?;

        // Commit transaction
        tx.commit().await?;

        tracing::debug!(
            workflow_id = %workflow_id,
            definition_name = %definition.name,
            definition_version = %definition.version,
            "Workflow submitted successfully"
        );

        Ok(CreatedWorkflow {
            id: row.id,
            definition_name: row.definition_name,
            workflow_definition_id: row.workflow_definition_id,
            definition_version: definition.version, // From resolved definition
            input: row.input,
            unique_key: row.unique_key,
            status: row.status.to_string(),
            created_at: row.created_at,
        })
    }

    /// Validate input structure
    ///
    /// For MVP, this only validates that input is a JSON object.
    /// Activities will validate specific parameter requirements at execution time.
    fn validate_input(input: &Value) -> WorkflowServiceResult<()> {
        if !input.is_object() {
            return Err(WorkflowServiceError::InvalidInput(
                "Input must be a JSON object".to_string(),
            ));
        }

        Ok(())
    }
}
