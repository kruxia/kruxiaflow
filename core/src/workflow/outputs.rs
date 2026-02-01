use serde::{Deserialize, Deserializer, Serialize};

/// Output type enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Hash)]
#[serde(rename_all = "lowercase")]
pub enum OutputType {
    /// Default: JSON value
    #[default]
    Value,

    /// File reference
    File,

    /// Folder reference (post-MVP)
    Folder,
}

/// Activity output definition (user-provided in workflow YAML)
///
/// Supports three formats:
/// 1. Shorthand (string): `"response"` → `{ name: "response", type: "value" }`
/// 2. Object without type: `{ name: "response" }` → `{ name: "response", type: "value" }`
/// 3. Full object: `{ name: "response", type: "file" }` → `{ name: "response", type: "file" }`
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ActivityOutputDefinition {
    /// Output name (key for referencing in templates)
    pub name: String,

    /// Output type (default: value)
    #[serde(default)]
    #[serde(rename = "type")]
    pub output_type: OutputType,
}

impl<'de> Deserialize<'de> for ActivityOutputDefinition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum OutputDefHelper {
            String(String),
            Object {
                name: String,
                #[serde(default)]
                #[serde(rename = "type")]
                output_type: OutputType,
            },
        }

        match OutputDefHelper::deserialize(deserializer)? {
            OutputDefHelper::String(name) => Ok(ActivityOutputDefinition {
                name,
                output_type: OutputType::Value,
            }),
            OutputDefHelper::Object { name, output_type } => {
                Ok(ActivityOutputDefinition { name, output_type })
            }
        }
    }
}

/// Activity output with value and type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivityOutput {
    /// Output name
    pub name: String,

    /// Output type
    #[serde(rename = "type")]
    pub output_type: OutputType,

    /// Output value
    /// - For Value: JSON data
    /// - For File: file reference string (e.g., "postgres://workflow_id/activity_key/filename")
    /// - For Folder: folder reference string
    pub value: serde_json::Value,
}

impl ActivityOutput {
    pub fn value(name: impl Into<String>, value: serde_json::Value) -> Self {
        Self {
            name: name.into(),
            output_type: OutputType::Value,
            value,
        }
    }

    pub fn file(name: impl Into<String>, reference: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            output_type: OutputType::File,
            value: serde_json::Value::String(reference.into()),
        }
    }

    pub fn folder(name: impl Into<String>, reference: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            output_type: OutputType::Folder,
            value: serde_json::Value::String(reference.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_output_type_default() {
        assert_eq!(OutputType::default(), OutputType::Value);
    }

    #[test]
    fn test_output_type_serde() {
        assert_eq!(
            serde_json::to_string(&OutputType::Value).unwrap(),
            "\"value\""
        );
        assert_eq!(
            serde_json::to_string(&OutputType::File).unwrap(),
            "\"file\""
        );
        assert_eq!(
            serde_json::to_string(&OutputType::Folder).unwrap(),
            "\"folder\""
        );

        let v: OutputType = serde_json::from_str("\"value\"").unwrap();
        assert_eq!(v, OutputType::Value);
        let f: OutputType = serde_json::from_str("\"file\"").unwrap();
        assert_eq!(f, OutputType::File);
        let d: OutputType = serde_json::from_str("\"folder\"").unwrap();
        assert_eq!(d, OutputType::Folder);
    }

    #[test]
    fn test_activity_output_value() {
        let output = ActivityOutput::value("result", json!({"status": "ok"}));
        assert_eq!(output.name, "result");
        assert_eq!(output.output_type, OutputType::Value);
        assert_eq!(output.value, json!({"status": "ok"}));
    }

    #[test]
    fn test_activity_output_file() {
        let output = ActivityOutput::file("doc", "postgres://wf/act/file.pdf");
        assert_eq!(output.name, "doc");
        assert_eq!(output.output_type, OutputType::File);
        assert_eq!(output.value, json!("postgres://wf/act/file.pdf"));
    }

    #[test]
    fn test_activity_output_folder() {
        let output = ActivityOutput::folder("out_dir", "postgres://wf/act/output/");
        assert_eq!(output.name, "out_dir");
        assert_eq!(output.output_type, OutputType::Folder);
        assert_eq!(output.value, json!("postgres://wf/act/output/"));
    }

    #[test]
    fn test_activity_output_definition_shorthand() {
        let def: ActivityOutputDefinition = serde_json::from_str("\"response\"").unwrap();
        assert_eq!(def.name, "response");
        assert_eq!(def.output_type, OutputType::Value);
    }

    #[test]
    fn test_activity_output_definition_object_no_type() {
        let def: ActivityOutputDefinition = serde_json::from_str("{\"name\": \"result\"}").unwrap();
        assert_eq!(def.name, "result");
        assert_eq!(def.output_type, OutputType::Value);
    }

    #[test]
    fn test_activity_output_definition_object_with_type() {
        let def: ActivityOutputDefinition =
            serde_json::from_str("{\"name\": \"doc\", \"type\": \"file\"}").unwrap();
        assert_eq!(def.name, "doc");
        assert_eq!(def.output_type, OutputType::File);
    }

    #[test]
    fn test_activity_output_definition_folder_type() {
        let def: ActivityOutputDefinition =
            serde_json::from_str("{\"name\": \"out\", \"type\": \"folder\"}").unwrap();
        assert_eq!(def.name, "out");
        assert_eq!(def.output_type, OutputType::Folder);
    }

    #[test]
    fn test_activity_output_serde_roundtrip() {
        let output = ActivityOutput::value("test", json!(42));
        let json_str = serde_json::to_string(&output).unwrap();
        let deserialized: ActivityOutput = serde_json::from_str(&json_str).unwrap();
        assert_eq!(output, deserialized);
    }
}
