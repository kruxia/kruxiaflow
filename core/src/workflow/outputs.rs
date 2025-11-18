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
