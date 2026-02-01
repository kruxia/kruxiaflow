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
    pub content_hash: Option<Vec<u8>>,
    pub created_at: DateTime<Utc>,
}

/// Result of storing a workflow definition
#[derive(Debug, Clone)]
pub struct StoreResult {
    /// The stored workflow definition
    pub definition: StoredWorkflowDefinition,
    /// True if a new version was created, false if an existing identical version was returned
    pub is_new: bool,
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

    /// Store workflow definition (idempotent)
    ///
    /// If an identical definition (same name and content hash) already exists,
    /// returns the existing version instead of creating a new one.
    ///
    /// Uses created_at timestamp as the version (microsecond precision prevents collisions).
    /// Returns error if definition with same (name, created_at) already exists (virtually impossible).
    pub async fn store(
        &self,
        mut definition: WorkflowDefinition,
    ) -> Result<StoreResult, RepositoryError> {
        // Validate definition before storing (validation is mutable to cache metadata)
        definition
            .validate()
            .map_err(RepositoryError::ValidationError)?;

        let name = definition.name.clone();
        let content_hash = definition.content_hash();

        // Check if an identical definition already exists
        if let Some(existing) = self.find_by_content_hash(&name, &content_hash).await? {
            return Ok(StoreResult {
                definition: existing,
                is_new: false,
            });
        }

        let id = Uuid::now_v7();

        // Store only the activities array (not the full definition)
        let activities_json = serde_json::to_value(&definition.activities)?;

        let row = sqlx::query!(
            r#"
            INSERT INTO workflow_definitions (id, name, activities, content_hash, created_at)
            VALUES ($1, $2, $3, $4, NOW())
            RETURNING id, name, activities, content_hash, created_at
            "#,
            id,
            name,
            activities_json,
            content_hash
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

        Ok(StoreResult {
            definition: StoredWorkflowDefinition {
                id: row.id,
                name: row.name,
                version: format_version(&row.created_at),
                activities: serde_json::from_value(row.activities)?,
                content_hash: row.content_hash,
                created_at: row.created_at,
            },
            is_new: true,
        })
    }

    /// Find a workflow definition by name and content hash.
    /// Returns the existing definition if found, None otherwise.
    ///
    /// Used for idempotent deployment: if identical content already exists,
    /// we return the existing version instead of creating a new one.
    pub async fn find_by_content_hash(
        &self,
        name: &str,
        content_hash: &[u8],
    ) -> Result<Option<StoredWorkflowDefinition>, RepositoryError> {
        let row = sqlx::query!(
            r#"
            SELECT id, name, activities, content_hash, created_at
            FROM workflow_definitions
            WHERE name = $1 AND content_hash = $2
            ORDER BY created_at DESC
            LIMIT 1
            "#,
            name,
            content_hash
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| StoredWorkflowDefinition {
            id: r.id,
            name: r.name,
            version: format_version(&r.created_at),
            activities: serde_json::from_value(r.activities)
                .expect("Failed to deserialize activities"),
            content_hash: r.content_hash,
            created_at: r.created_at,
        }))
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
            SELECT id, name, activities, content_hash, created_at
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
            content_hash: r.content_hash,
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
            SELECT id, name, activities, content_hash, created_at
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
            content_hash: r.content_hash,
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
            SELECT id, name, activities, content_hash, created_at
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
                content_hash: r.content_hash,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_repository_error_display_validation() {
        let err = RepositoryError::ValidationError(ValidationError::SingleError(
            "invalid activity".to_string(),
        ));
        let msg = format!("{}", err);
        assert!(msg.contains("Validation error"));
        assert!(msg.contains("invalid activity"));
    }

    #[test]
    fn test_repository_error_display_duplicate_version() {
        let err = RepositoryError::DuplicateVersion {
            name: "my-workflow".to_string(),
            version: "20250105.143022.123456".to_string(),
        };
        let msg = format!("{}", err);
        assert!(msg.contains("already exists"));
        assert!(msg.contains("my-workflow"));
        assert!(msg.contains("20250105.143022.123456"));
    }

    #[test]
    fn test_repository_error_display_invalid_version() {
        let err = RepositoryError::InvalidVersion {
            version: "bad-version".to_string(),
            error: "expected YYYYmmdd.HHMMSS.uuuuuu".to_string(),
        };
        let msg = format!("{}", err);
        assert!(msg.contains("Invalid version format"));
        assert!(msg.contains("bad-version"));
    }

    #[test]
    fn test_repository_error_display_serialization() {
        let json_err = serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
        let err = RepositoryError::SerializationError(json_err);
        let msg = format!("{}", err);
        assert!(msg.contains("Serialization error"));
    }

    #[test]
    fn test_stored_workflow_definition_clone() {
        let stored = StoredWorkflowDefinition {
            id: Uuid::nil(),
            name: "test-workflow".to_string(),
            version: "20250105.143022.123456".to_string(),
            activities: vec![],
            content_hash: Some(vec![1, 2, 3]),
            created_at: Utc::now(),
        };
        let cloned = stored.clone();
        assert_eq!(cloned.name, stored.name);
        assert_eq!(cloned.version, stored.version);
        assert_eq!(cloned.content_hash, stored.content_hash);
    }

    #[test]
    fn test_stored_workflow_definition_debug() {
        let stored = StoredWorkflowDefinition {
            id: Uuid::nil(),
            name: "test".to_string(),
            version: "20250105.143022.000000".to_string(),
            activities: vec![],
            content_hash: None,
            created_at: Utc::now(),
        };
        let debug = format!("{:?}", stored);
        assert!(debug.contains("test"));
    }

    #[test]
    fn test_store_result_is_new_flag() {
        let result = StoreResult {
            definition: StoredWorkflowDefinition {
                id: Uuid::nil(),
                name: "wf".to_string(),
                version: "20250105.143022.000000".to_string(),
                activities: vec![],
                content_hash: None,
                created_at: Utc::now(),
            },
            is_new: true,
        };
        assert!(result.is_new);

        let result2 = StoreResult {
            definition: result.definition.clone(),
            is_new: false,
        };
        assert!(!result2.is_new);
    }

    #[test]
    fn test_store_result_debug() {
        let result = StoreResult {
            definition: StoredWorkflowDefinition {
                id: Uuid::nil(),
                name: "wf".to_string(),
                version: "v1".to_string(),
                activities: vec![],
                content_hash: None,
                created_at: Utc::now(),
            },
            is_new: true,
        };
        let debug = format!("{:?}", result);
        assert!(debug.contains("is_new"));
        assert!(debug.contains("true"));
    }

    #[test]
    fn test_repository_error_from_validation_error() {
        let validation_err = ValidationError::SingleError("test error".to_string());
        let repo_err: RepositoryError = validation_err.into();
        assert!(matches!(repo_err, RepositoryError::ValidationError(_)));
    }

    #[test]
    fn test_repository_error_from_serde_error() {
        let serde_err = serde_json::from_str::<String>("not valid").unwrap_err();
        let repo_err: RepositoryError = serde_err.into();
        assert!(matches!(repo_err, RepositoryError::SerializationError(_)));
    }
}
