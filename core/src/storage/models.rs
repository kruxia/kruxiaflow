use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct FileMetadata {
    pub workflow_id: Uuid,
    pub activity_key: String,
    pub filename: String,
    pub size: i64,
    pub content_type: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileReference {
    pub workflow_id: Uuid,
    pub activity_key: String,
    pub filename: String,
}

impl FileReference {
    pub fn new(
        workflow_id: Uuid,
        activity_key: impl Into<String>,
        filename: impl Into<String>,
    ) -> Self {
        Self {
            workflow_id,
            activity_key: activity_key.into(),
            filename: filename.into(),
        }
    }

    /// Format as a reference string, e.g.: postgres://{workflow_id}/{activity_key}/{filename}
    pub fn to_string(&self, provider: &str) -> String {
        format!(
            "{}://{}/{}/{}",
            provider, self.workflow_id, self.activity_key, self.filename
        )
    }

    /// Parse a reference string back into a FileReference
    pub fn from_string(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.split("://").collect();
        if parts.len() != 2 {
            return Err(format!("Invalid file reference format: {}", s));
        }

        let path_parts: Vec<&str> = parts[1].split('/').collect();
        if path_parts.len() != 3 {
            return Err(format!("Invalid file reference path: {}", parts[1]));
        }

        let workflow_id =
            Uuid::parse_str(path_parts[0]).map_err(|e| format!("Invalid workflow ID: {}", e))?;

        Ok(Self {
            workflow_id,
            activity_key: path_parts[1].to_string(),
            filename: path_parts[2].to_string(),
        })
    }
}
