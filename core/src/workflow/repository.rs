use crate::workflow::definition::{
    ActivityDefinition, ValidationError, WorkflowDefinition, format_version, parse_version,
};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// Stored workflow definition record
#[derive(Debug, Clone)]
pub struct StoredWorkflowDefinition {
    pub id: Uuid,
    pub name: String,
    pub version: String,
    pub activities: Vec<ActivityDefinition>,
    pub created_at: DateTime<Utc>,
}

/// Workflow definition repository
#[derive(Clone)]
pub struct WorkflowDefinitionRepository {
    pool: PgPool,
}

impl WorkflowDefinitionRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Store workflow definition
    ///
    /// Uses created_at timestamp as the version (microsecond precision prevents collisions).
    /// Returns error if definition with same (name, created_at) already exists (virtually impossible).
    pub async fn store(
        &self,
        mut definition: WorkflowDefinition,
    ) -> Result<StoredWorkflowDefinition, RepositoryError> {
        // Validate definition before storing (validation is mutable to cache metadata)
        definition
            .validate()
            .map_err(RepositoryError::ValidationError)?;

        let id = Uuid::now_v7();
        let name = definition.name.clone();

        // Store only the activities array (not the full definition)
        let activities_json = serde_json::to_value(&definition.activities)?;

        let row = sqlx::query!(
            r#"
            INSERT INTO workflow_definitions (id, name, activities, created_at)
            VALUES ($1, $2, $3, NOW())
            RETURNING id, name, activities, created_at
            "#,
            id,
            name,
            activities_json
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            // Check for unique constraint violation (virtually impossible with microsecond precision)
            if let Some(db_err) = e.as_database_error()
                && db_err.is_unique_violation()
            {
                return RepositoryError::DuplicateVersion {
                    name: name.clone(),
                    version: format_version(&Utc::now()),
                };
            }

            RepositoryError::DatabaseError(e)
        })?;

        Ok(StoredWorkflowDefinition {
            id: row.id,
            name: row.name,
            version: format_version(&row.created_at),
            activities: serde_json::from_value(row.activities)?,
            created_at: row.created_at,
        })
    }

    /// Get workflow definition by name and version
    ///
    /// Version format: YYYYmmdd.HHMMSS.uuuuuu (parsed back to timestamp for query)
    pub async fn get(
        &self,
        name: &str,
        version: &str,
    ) -> Result<Option<StoredWorkflowDefinition>, RepositoryError> {
        // Parse version string to timestamp
        let created_at = parse_version(version).map_err(|e| RepositoryError::InvalidVersion {
            version: version.to_string(),
            error: e,
        })?;

        let row = sqlx::query!(
            r#"
            SELECT id, name, activities, created_at
            FROM workflow_definitions
            WHERE name = $1 AND created_at = $2
            "#,
            name,
            created_at
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| StoredWorkflowDefinition {
            id: r.id,
            name: r.name,
            version: format_version(&r.created_at),
            activities: serde_json::from_value(r.activities)
                .expect("Failed to deserialize activities"),
            created_at: r.created_at,
        }))
    }

    /// Get latest version of workflow definition by name
    pub async fn get_latest(
        &self,
        name: &str,
    ) -> Result<Option<StoredWorkflowDefinition>, RepositoryError> {
        let row = sqlx::query!(
            r#"
            SELECT id, name, activities, created_at
            FROM workflow_definitions
            WHERE name = $1
            ORDER BY created_at DESC
            LIMIT 1
            "#,
            name
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| StoredWorkflowDefinition {
            id: r.id,
            name: r.name,
            version: format_version(&r.created_at),
            activities: serde_json::from_value(r.activities)
                .expect("Failed to deserialize activities"),
            created_at: r.created_at,
        }))
    }

    /// List all workflow definitions
    ///
    /// Returns all versions of all workflows.
    /// Post-MVP: Add filtering, pagination, search.
    pub async fn list(&self) -> Result<Vec<StoredWorkflowDefinition>, RepositoryError> {
        let rows = sqlx::query!(
            r#"
            SELECT id, name, activities, created_at
            FROM workflow_definitions
            ORDER BY name ASC, created_at DESC
            "#
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| StoredWorkflowDefinition {
                id: r.id,
                name: r.name,
                version: format_version(&r.created_at),
                activities: serde_json::from_value(r.activities)
                    .expect("Failed to deserialize activities"),
                created_at: r.created_at,
            })
            .collect())
    }
}

/// Repository errors
#[derive(Debug, thiserror::Error)]
pub enum RepositoryError {
    #[error("Validation error: {0}")]
    ValidationError(#[from] ValidationError),

    #[error("Workflow definition already exists: {name} version {version}")]
    DuplicateVersion { name: String, version: String },

    #[error("Invalid version format '{version}': {error}")]
    InvalidVersion { version: String, error: String },

    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
}
