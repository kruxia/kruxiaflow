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

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // FileReference tests
    // =========================================================================

    #[test]
    fn test_file_reference_new() {
        let workflow_id = Uuid::now_v7();
        let file_ref = FileReference::new(workflow_id, "my_activity", "output.json");

        assert_eq!(file_ref.workflow_id, workflow_id);
        assert_eq!(file_ref.activity_key, "my_activity");
        assert_eq!(file_ref.filename, "output.json");
    }

    #[test]
    fn test_file_reference_to_string() {
        let workflow_id = Uuid::parse_str("019353a1-b0c1-7000-8000-000000000001").unwrap();
        let file_ref = FileReference::new(workflow_id, "step1", "result.txt");

        let ref_string = file_ref.to_string("postgres");

        assert_eq!(
            ref_string,
            "postgres://019353a1-b0c1-7000-8000-000000000001/step1/result.txt"
        );
    }

    #[test]
    fn test_file_reference_to_string_s3() {
        let workflow_id = Uuid::parse_str("019353a1-b0c1-7000-8000-000000000001").unwrap();
        let file_ref = FileReference::new(workflow_id, "process", "data.csv");

        let ref_string = file_ref.to_string("s3");

        assert_eq!(
            ref_string,
            "s3://019353a1-b0c1-7000-8000-000000000001/process/data.csv"
        );
    }

    #[test]
    fn test_file_reference_from_string_valid() {
        let ref_string = "postgres://019353a1-b0c1-7000-8000-000000000001/step1/result.txt";
        let file_ref = FileReference::from_string(ref_string).unwrap();

        assert_eq!(
            file_ref.workflow_id,
            Uuid::parse_str("019353a1-b0c1-7000-8000-000000000001").unwrap()
        );
        assert_eq!(file_ref.activity_key, "step1");
        assert_eq!(file_ref.filename, "result.txt");
    }

    #[test]
    fn test_file_reference_from_string_s3() {
        let ref_string = "s3://019353a1-b0c1-7000-8000-000000000001/process/data.csv";
        let file_ref = FileReference::from_string(ref_string).unwrap();

        assert_eq!(file_ref.activity_key, "process");
        assert_eq!(file_ref.filename, "data.csv");
    }

    #[test]
    fn test_file_reference_roundtrip() {
        let workflow_id = Uuid::now_v7();
        let original = FileReference::new(workflow_id, "my_activity", "test_file.json");

        let ref_string = original.to_string("postgres");
        let parsed = FileReference::from_string(&ref_string).unwrap();

        assert_eq!(parsed.workflow_id, original.workflow_id);
        assert_eq!(parsed.activity_key, original.activity_key);
        assert_eq!(parsed.filename, original.filename);
    }

    #[test]
    fn test_file_reference_from_string_no_protocol() {
        let ref_string = "019353a1-b0c1-7000-8000-000000000001/step1/result.txt";
        let result = FileReference::from_string(ref_string);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("Invalid file reference format")
        );
    }

    #[test]
    fn test_file_reference_from_string_invalid_path() {
        let ref_string = "postgres://019353a1-b0c1-7000-8000-000000000001/step1";
        let result = FileReference::from_string(ref_string);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid file reference path"));
    }

    #[test]
    fn test_file_reference_from_string_invalid_uuid() {
        let ref_string = "postgres://not-a-uuid/step1/result.txt";
        let result = FileReference::from_string(ref_string);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid workflow ID"));
    }

    #[test]
    fn test_file_reference_from_string_too_many_parts() {
        let ref_string = "postgres://019353a1-b0c1-7000-8000-000000000001/step1/subdir/result.txt";
        let result = FileReference::from_string(ref_string);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid file reference path"));
    }

    #[test]
    fn test_file_reference_debug() {
        let workflow_id = Uuid::nil();
        let file_ref = FileReference::new(workflow_id, "test", "file.txt");

        let debug_str = format!("{:?}", file_ref);
        assert!(debug_str.contains("FileReference"));
        assert!(debug_str.contains("test"));
        assert!(debug_str.contains("file.txt"));
    }

    #[test]
    fn test_file_reference_clone() {
        let workflow_id = Uuid::now_v7();
        let file_ref = FileReference::new(workflow_id, "activity", "output.bin");

        let cloned = file_ref.clone();
        assert_eq!(cloned.workflow_id, file_ref.workflow_id);
        assert_eq!(cloned.activity_key, file_ref.activity_key);
        assert_eq!(cloned.filename, file_ref.filename);
    }

    // =========================================================================
    // FileMetadata tests
    // =========================================================================

    #[test]
    fn test_file_metadata_serialization() {
        let metadata = FileMetadata {
            workflow_id: Uuid::nil(),
            activity_key: "test_activity".to_string(),
            filename: "output.json".to_string(),
            size: 1024,
            content_type: Some("application/json".to_string()),
            created_at: Utc::now(),
        };

        let json = serde_json::to_string(&metadata).unwrap();
        assert!(json.contains("test_activity"));
        assert!(json.contains("output.json"));
        assert!(json.contains("1024"));
        assert!(json.contains("application/json"));
    }

    #[test]
    fn test_file_metadata_deserialization() {
        let json = r#"{
            "workflow_id": "00000000-0000-0000-0000-000000000000",
            "activity_key": "step1",
            "filename": "data.csv",
            "size": 2048,
            "content_type": "text/csv",
            "created_at": "2025-11-26T10:00:00Z"
        }"#;

        let metadata: FileMetadata = serde_json::from_str(json).unwrap();
        assert_eq!(metadata.activity_key, "step1");
        assert_eq!(metadata.filename, "data.csv");
        assert_eq!(metadata.size, 2048);
        assert_eq!(metadata.content_type, Some("text/csv".to_string()));
    }

    #[test]
    fn test_file_metadata_optional_content_type() {
        let json = r#"{
            "workflow_id": "00000000-0000-0000-0000-000000000000",
            "activity_key": "step1",
            "filename": "unknown.bin",
            "size": 512,
            "content_type": null,
            "created_at": "2025-11-26T10:00:00Z"
        }"#;

        let metadata: FileMetadata = serde_json::from_str(json).unwrap();
        assert!(metadata.content_type.is_none());
    }

    #[test]
    fn test_file_metadata_clone() {
        let metadata = FileMetadata {
            workflow_id: Uuid::nil(),
            activity_key: "test".to_string(),
            filename: "file.txt".to_string(),
            size: 100,
            content_type: None,
            created_at: Utc::now(),
        };

        let cloned = metadata.clone();
        assert_eq!(cloned.workflow_id, metadata.workflow_id);
        assert_eq!(cloned.activity_key, metadata.activity_key);
        assert_eq!(cloned.filename, metadata.filename);
        assert_eq!(cloned.size, metadata.size);
    }
}
