use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Workflow definition (user-provided, without version)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    /// Workflow name (unique per version)
    pub name: String,

    /// Activities in the workflow
    pub activities: Vec<ActivityDefinition>,
}

impl WorkflowDefinition {
    /// Parse workflow definition from YAML string
    pub fn from_yaml(yaml: &str) -> Result<Self, ValidationError> {
        let mut definition: WorkflowDefinition = serde_yaml::from_str(yaml)
            .map_err(|e| ValidationError::SingleError(format!("Failed to parse YAML: {}", e)))?;

        definition.validate()?;
        definition.normalize();
        Ok(definition)
    }

    /// Parse workflow definition from JSON string
    pub fn from_json(json: &str) -> Result<Self, ValidationError> {
        let mut definition: WorkflowDefinition = serde_json::from_str(json)
            .map_err(|e| ValidationError::SingleError(format!("Failed to parse JSON: {}", e)))?;

        definition.validate()?;
        definition.normalize();
        Ok(definition)
    }

    /// Serialize workflow definition to YAML string
    pub fn to_yaml(&self) -> Result<String, ValidationError> {
        serde_yaml::to_string(self).map_err(|e| {
            ValidationError::SingleError(format!("Failed to serialize to YAML: {}", e))
        })
    }

    /// Serialize workflow definition to JSON string
    pub fn to_json(&self) -> Result<String, ValidationError> {
        serde_json::to_string_pretty(self).map_err(|e| {
            ValidationError::SingleError(format!("Failed to serialize to JSON: {}", e))
        })
    }

    /// Validate workflow definition structure
    pub fn validate(&self) -> Result<(), ValidationError> {
        let mut errors = ValidationErrors::new();

        // Validate workflow name
        if self.name.is_empty() {
            errors.add("name", "Workflow name cannot be empty");
        }
        if !is_valid_identifier(&self.name) {
            errors.add(
                "name",
                "Workflow name must be a valid identifier (alphanumeric, hyphens, underscores)",
            );
        }

        // Validate activities
        if self.activities.is_empty() {
            errors.add("activities", "Workflow must have at least one activity");
        }

        // Check for duplicate activity keys
        let mut activity_keys = HashSet::new();
        for (idx, activity) in self.activities.iter().enumerate() {
            if !activity_keys.insert(&activity.key) {
                errors.add(
                    &format!("activities[{}].key", idx),
                    &format!("Duplicate activity key: {}", activity.key),
                );
            }

            // Validate activity structure
            if let Err(e) = self.validate_activity(activity, idx) {
                errors.merge(e);
            }
        }

        // Validate graph structure (no cycles, valid references)
        if let Err(e) = self.validate_graph(&activity_keys) {
            errors.merge(e);
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(ValidationError::MultipleErrors(errors))
        }
    }

    /// Validate individual activity
    fn validate_activity(
        &self,
        activity: &ActivityDefinition,
        idx: usize,
    ) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();

        // Validate activity key
        if activity.key.is_empty() {
            errors.add(
                &format!("activities[{}].key", idx),
                "Activity key cannot be empty",
            );
        }
        if !is_valid_identifier(&activity.key) {
            errors.add(
                &format!("activities[{}].key", idx),
                "Activity key must be a valid identifier",
            );
        }

        // Validate worker
        if activity.worker.is_empty() {
            errors.add(
                &format!("activities[{}].worker", idx),
                "Activity worker cannot be empty",
            );
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Validate directed graph structure
    fn validate_graph(&self, activity_keys: &HashSet<&String>) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();

        // Build adjacency list for cycle detection
        let mut graph: HashMap<&str, Vec<&str>> = HashMap::new();
        for activity in &self.activities {
            graph.entry(activity.key.as_str()).or_insert_with(Vec::new);
        }

        // Validate all activity references
        // Note: Validation happens before normalization, so we check both fields
        for activity in &self.activities {
            // Validate depends_on references
            if let Some(depends_on) = &activity.depends_on {
                for rel in depends_on {
                    if !activity_keys.contains(&rel.activity_key) {
                        errors.add(
                            &format!("activity.{}.depends_on", activity.key),
                            &format!("Referenced activity not found: {}", rel.activity_key),
                        );
                    } else {
                        // Add edge: dependency -> current
                        graph
                            .get_mut(rel.activity_key.as_str())
                            .unwrap()
                            .push(activity.key.as_str());
                    }
                }
            }

            // Validate dependency_of references (before normalization clears them)
            if let Some(dependency_of) = &activity.dependency_of {
                for rel in dependency_of {
                    if !activity_keys.contains(&rel.activity_key) {
                        errors.add(
                            &format!("activity.{}.dependency_of", activity.key),
                            &format!("Referenced activity not found: {}", rel.activity_key),
                        );
                    } else {
                        // Add edge: current -> dependent
                        graph
                            .get_mut(activity.key.as_str())
                            .unwrap()
                            .push(rel.activity_key.as_str());
                    }
                }
            }
        }

        // Detect cycles using DFS
        if let Some(cycle) = detect_cycle(&graph) {
            errors.add(
                "activities",
                &format!("Workflow contains a cycle: {}", cycle.join(" -> ")),
            );
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Normalize the workflow definition by converting all dependency_of
    /// relationships into depends_on relationships.
    ///
    /// After normalization:
    /// - All activities have only depends_on populated (if they have dependencies)
    /// - All dependency_of fields are cleared
    /// - The dependency graph is represented in a single canonical form
    pub fn normalize(&mut self) {
        use std::collections::HashMap;

        // Build a map of activity_key -> dependencies to add
        let mut dependencies_to_add: HashMap<String, Vec<ActivityRelationship>> = HashMap::new();

        // Pass 1: For each activity with dependency_of,
        // record that those targets should depend on this activity
        for activity in &self.activities {
            if let Some(dependency_of_list) = &activity.dependency_of {
                for relationship in dependency_of_list {
                    // The target activity should depend on this activity
                    dependencies_to_add
                        .entry(relationship.activity_key.clone())
                        .or_insert_with(Vec::new)
                        .push(ActivityRelationship {
                            activity_key: activity.key.clone(),
                            conditions: relationship.conditions.clone(),
                        });
                }
            }
        }

        // Pass 2: Add the computed dependencies to each activity's depends_on
        for activity in &mut self.activities {
            if let Some(new_deps) = dependencies_to_add.remove(&activity.key) {
                let depends_on = activity.depends_on.get_or_insert_with(Vec::new);
                depends_on.extend(new_deps);

                // Deduplicate (same activity might be referenced from both directions)
                depends_on.sort_by(|a, b| {
                    a.activity_key.cmp(&b.activity_key).then_with(|| {
                        format!("{:?}", a.conditions).cmp(&format!("{:?}", b.conditions))
                    })
                });
                depends_on.dedup_by(|a, b| {
                    a.activity_key == b.activity_key && a.conditions == b.conditions
                });
            }
        }

        // Pass 3: Clear all dependency_of fields (no longer needed)
        for activity in &mut self.activities {
            activity.dependency_of = None;
        }
    }
}

/// Activity definition within a workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityDefinition {
    /// Unique key for this activity within the workflow
    pub key: String,

    /// Activity worker type (e.g., "builtin", "custom-python")
    pub worker: String,

    /// Activity name within worker (e.g., "http_request", "postgres_query")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity_name: Option<String>,

    /// Activity parameters (can include template expressions)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<HashMap<String, serde_json::Value>>,

    /// Activities that must complete before this one
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<Vec<ActivityRelationship>>,

    /// Activities that depend on this one (input only, cleared after normalization)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependency_of: Option<Vec<ActivityRelationship>>,

    /// Activity-level settings (timeout, retry, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings: Option<ActivitySettings>,
}

/// Relationship between activities (edge in the directed graph)
#[derive(Debug, Clone, Serialize)]
pub struct ActivityRelationship {
    /// Key of the related activity
    pub activity_key: String,

    /// Optional conditions that must be satisfied for this edge to activate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conditions: Option<Vec<String>>,
}

impl<'de> Deserialize<'de> for ActivityRelationship {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum ActivityRelationshipHelper {
            // Simple string: "activity_key"
            Simple(String),
            // Full object: {activity_key: "key", conditions: [...]}
            Full {
                activity_key: String,
                #[serde(alias = "condition")]
                conditions: Option<ConditionOrConditions>,
            },
        }

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum ConditionOrConditions {
            Single(String),
            Multiple(Vec<String>),
        }

        match ActivityRelationshipHelper::deserialize(deserializer)? {
            ActivityRelationshipHelper::Simple(activity_key) => Ok(ActivityRelationship {
                activity_key,
                conditions: None,
            }),
            ActivityRelationshipHelper::Full {
                activity_key,
                conditions,
            } => {
                let conditions = conditions.map(|c| match c {
                    ConditionOrConditions::Single(s) => vec![s],
                    ConditionOrConditions::Multiple(v) => v,
                });
                Ok(ActivityRelationship {
                    activity_key,
                    conditions,
                })
            }
        }
    }
}

/// Workflow-level settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSettings {
    /// Maximum workflow execution time in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,

    /// Maximum retry attempts for transient failures
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,
}

/// Activity-level settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivitySettings {
    /// Activity timeout in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,

    /// Retry configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetrySettings>,
}

/// Retry configuration for activities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrySettings {
    /// Maximum retry attempts
    pub max_attempts: u32,

    /// Backoff strategy
    #[serde(default = "default_backoff")]
    pub backoff: BackoffStrategy,
}

fn default_backoff() -> BackoffStrategy {
    BackoffStrategy::Exponential
}

/// Backoff strategy for retries
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackoffStrategy {
    /// Fixed delay between retries
    Fixed,
    /// Exponential backoff (delay doubles each retry)
    Exponential,
}

/// Check if string is valid identifier (alphanumeric, hyphens, underscores)
fn is_valid_identifier(s: &str) -> bool {
    let re = regex::Regex::new(r"^[a-zA-Z0-9_-]+$").unwrap();
    re.is_match(s)
}

/// Detect cycles in directed graph using DFS
fn detect_cycle<'a>(graph: &HashMap<&'a str, Vec<&'a str>>) -> Option<Vec<String>> {
    let mut visited = HashSet::new();
    let mut rec_stack = HashSet::new();
    let mut path = Vec::new();

    for node in graph.keys() {
        if !visited.contains(node) {
            if let Some(cycle) = dfs_cycle(node, graph, &mut visited, &mut rec_stack, &mut path) {
                return Some(cycle);
            }
        }
    }

    None
}

/// DFS helper for cycle detection
fn dfs_cycle<'a>(
    node: &'a str,
    graph: &HashMap<&'a str, Vec<&'a str>>,
    visited: &mut HashSet<&'a str>,
    rec_stack: &mut HashSet<&'a str>,
    path: &mut Vec<String>,
) -> Option<Vec<String>> {
    visited.insert(node);
    rec_stack.insert(node);
    path.push(node.to_string());

    if let Some(neighbors) = graph.get(node) {
        for &neighbor in neighbors {
            if !visited.contains(neighbor) {
                if let Some(cycle) = dfs_cycle(neighbor, graph, visited, rec_stack, path) {
                    return Some(cycle);
                }
            } else if rec_stack.contains(neighbor) {
                // Found cycle - return path from neighbor to current node
                let cycle_start = path.iter().position(|n| n == neighbor).unwrap();
                return Some(path[cycle_start..].to_vec());
            }
        }
    }

    rec_stack.remove(node);
    path.pop();
    None
}

/// Validation errors
#[derive(Debug, Clone)]
pub struct ValidationErrors {
    errors: HashMap<String, Vec<String>>,
}

impl ValidationErrors {
    pub fn new() -> Self {
        Self {
            errors: HashMap::new(),
        }
    }

    pub fn add(&mut self, field: &str, message: &str) {
        self.errors
            .entry(field.to_string())
            .or_insert_with(Vec::new)
            .push(message.to_string());
    }

    pub fn merge(&mut self, other: ValidationErrors) {
        for (field, messages) in other.errors {
            self.errors
                .entry(field)
                .or_insert_with(Vec::new)
                .extend(messages);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "field_errors": self.errors
        })
    }

    pub fn errors(&self) -> &HashMap<String, Vec<String>> {
        &self.errors
    }
}

/// Validation error
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("Validation failed: {0}")]
    SingleError(String),

    #[error("Multiple validation errors")]
    MultipleErrors(ValidationErrors),
}

/// Format a timestamp as a compact version string
///
/// Format: YYYYmmdd.HHMMSS.uuuuuu
/// Example: "20250105.143022.123456"
///
/// This format is:
/// - Human-scannable (visual separation with dots)
/// - Lexicographically sortable
/// - Compact for URLs and logs
/// - Microsecond precision (prevents collisions)
/// - UTC timezone implicit
pub fn format_version(timestamp: &DateTime<Utc>) -> String {
    let micros = timestamp.timestamp_subsec_micros();
    format!("{}.{:06}", timestamp.format("%Y%m%d.%H%M%S"), micros)
}

/// Parse a compact version string back to a timestamp
///
/// Format: YYYYmmdd.HHMMSS.uuuuuu
/// Example: "20250105.143022.123456"
///
/// Returns error if the format is invalid.
pub fn parse_version(version: &str) -> Result<DateTime<Utc>, String> {
    // Expected format: YYYYmmdd.HHMMSS.uuuuuu
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 3 {
        return Err(format!(
            "Invalid version format '{}': expected YYYYmmdd.HHMMSS.uuuuuu",
            version
        ));
    }

    let date_part = parts[0];
    let time_part = parts[1];
    let micro_part = parts[2];

    // Validate lengths
    if date_part.len() != 8 || time_part.len() != 6 || micro_part.len() != 6 {
        return Err(format!(
            "Invalid version format '{}': expected YYYYmmdd.HHMMSS.uuuuuu",
            version
        ));
    }

    // Parse components
    let year = date_part[0..4]
        .parse::<i32>()
        .map_err(|_| format!("Invalid year in version '{}'", version))?;
    let month = date_part[4..6]
        .parse::<u32>()
        .map_err(|_| format!("Invalid month in version '{}'", version))?;
    let day = date_part[6..8]
        .parse::<u32>()
        .map_err(|_| format!("Invalid day in version '{}'", version))?;

    let hour = time_part[0..2]
        .parse::<u32>()
        .map_err(|_| format!("Invalid hour in version '{}'", version))?;
    let minute = time_part[2..4]
        .parse::<u32>()
        .map_err(|_| format!("Invalid minute in version '{}'", version))?;
    let second = time_part[4..6]
        .parse::<u32>()
        .map_err(|_| format!("Invalid second in version '{}'", version))?;

    let micros = micro_part
        .parse::<u32>()
        .map_err(|_| format!("Invalid microseconds in version '{}'", version))?;

    // Construct NaiveDateTime
    let naive_date = chrono::NaiveDate::from_ymd_opt(year, month, day)
        .ok_or_else(|| format!("Invalid date in version '{}'", version))?;

    let naive_time = chrono::NaiveTime::from_hms_micro_opt(hour, minute, second, micros)
        .ok_or_else(|| format!("Invalid time in version '{}'", version))?;

    let naive_datetime = NaiveDateTime::new(naive_date, naive_time);

    Ok(DateTime::<Utc>::from_naive_utc_and_offset(
        naive_datetime,
        Utc,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};

    #[test]
    fn test_validate_valid_workflow() {
        let definition = WorkflowDefinition {
            name: "payment_processing".to_string(),
            activities: vec![
                ActivityDefinition {
                    key: "validate".to_string(),
                    worker: "payments".to_string(),
                    activity_name: Some("validate_card".to_string()),
                    parameters: None,
                    depends_on: None,
                    dependency_of: Some(vec![ActivityRelationship {
                        activity_key: "authorize".to_string(),
                        conditions: None,
                    }]),
                    settings: None,
                },
                ActivityDefinition {
                    key: "authorize".to_string(),
                    worker: "payments".to_string(),
                    activity_name: Some("authorize_card".to_string()),
                    parameters: None,
                    depends_on: None,
                    dependency_of: None,
                    settings: None,
                },
            ],
        };

        assert!(definition.validate().is_ok());
    }

    #[test]
    fn test_validate_duplicate_activity_keys() {
        let definition = WorkflowDefinition {
            name: "test".to_string(),
            activities: vec![
                ActivityDefinition {
                    key: "step1".to_string(),
                    worker: "test".to_string(),
                    activity_name: None,
                    parameters: None,
                    depends_on: None,
                    dependency_of: None,
                    settings: None,
                },
                ActivityDefinition {
                    key: "step1".to_string(), // Duplicate!
                    worker: "test".to_string(),
                    activity_name: None,
                    parameters: None,
                    depends_on: None,
                    dependency_of: None,
                    settings: None,
                },
            ],
        };

        let result = definition.validate();
        assert!(result.is_err());
        match result.unwrap_err() {
            ValidationError::MultipleErrors(errors) => {
                let json = errors.to_json();
                let json_str = json.to_string();
                assert!(json_str.contains("Duplicate activity key"));
            }
            _ => panic!("Expected MultipleErrors"),
        }
    }

    #[test]
    fn test_validate_invalid_activity_reference() {
        let definition = WorkflowDefinition {
            name: "test".to_string(),
            activities: vec![ActivityDefinition {
                key: "step1".to_string(),
                worker: "test".to_string(),
                activity_name: None,
                parameters: None,
                depends_on: None,
                dependency_of: Some(vec![ActivityRelationship {
                    activity_key: "step2".to_string(), // Doesn't exist!
                    conditions: None,
                }]),
                settings: None,
            }],
        };

        let result = definition.validate();
        assert!(result.is_err());
        match result.unwrap_err() {
            ValidationError::MultipleErrors(errors) => {
                let json = errors.to_json();
                let json_str = json.to_string();
                assert!(json_str.contains("not found"));
            }
            _ => panic!("Expected MultipleErrors"),
        }
    }

    #[test]
    fn test_validate_cycle_detection() {
        let definition = WorkflowDefinition {
            name: "test".to_string(),
            activities: vec![
                ActivityDefinition {
                    key: "step1".to_string(),
                    worker: "test".to_string(),
                    activity_name: None,
                    parameters: None,
                    depends_on: None,
                    dependency_of: Some(vec![ActivityRelationship {
                        activity_key: "step2".to_string(),
                        conditions: None,
                    }]),
                    settings: None,
                },
                ActivityDefinition {
                    key: "step2".to_string(),
                    worker: "test".to_string(),
                    activity_name: None,
                    parameters: None,
                    depends_on: None,
                    dependency_of: Some(vec![ActivityRelationship {
                        activity_key: "step1".to_string(), // Cycle!
                        conditions: None,
                    }]),
                    settings: None,
                },
            ],
        };

        let result = definition.validate();
        assert!(result.is_err());
        match result.unwrap_err() {
            ValidationError::MultipleErrors(errors) => {
                let json = errors.to_json();
                let json_str = json.to_string();
                assert!(json_str.contains("cycle"));
            }
            _ => panic!("Expected MultipleErrors"),
        }
    }

    #[test]
    fn test_format_version() {
        let timestamp = DateTime::from_naive_utc_and_offset(
            NaiveDateTime::new(
                chrono::NaiveDate::from_ymd_opt(2025, 1, 5).unwrap(),
                chrono::NaiveTime::from_hms_micro_opt(14, 30, 22, 123456).unwrap(),
            ),
            Utc,
        );

        let version = format_version(&timestamp);
        assert_eq!(version, "20250105.143022.123456");
    }

    #[test]
    fn test_parse_version() {
        let version = "20250105.143022.123456";
        let timestamp = parse_version(version).unwrap();

        assert_eq!(timestamp.year(), 2025);
        assert_eq!(timestamp.month(), 1);
        assert_eq!(timestamp.day(), 5);
        assert_eq!(timestamp.hour(), 14);
        assert_eq!(timestamp.minute(), 30);
        assert_eq!(timestamp.second(), 22);
        assert_eq!(timestamp.timestamp_subsec_micros(), 123456);
    }

    #[test]
    fn test_format_parse_roundtrip() {
        let original = Utc::now();
        let version = format_version(&original);
        let parsed = parse_version(&version).unwrap();

        // Should be equal at microsecond precision
        assert_eq!(original.timestamp(), parsed.timestamp());
        assert_eq!(
            original.timestamp_subsec_micros(),
            parsed.timestamp_subsec_micros()
        );
    }

    #[test]
    fn test_parse_version_invalid_format() {
        assert!(parse_version("invalid").is_err());
        assert!(parse_version("20250105").is_err());
        assert!(parse_version("20250105.143022").is_err());
        assert!(parse_version("2025-01-05.14:30:22.123456").is_err());
    }

    #[test]
    fn test_version_lexicographic_ordering() {
        let v1 = "20250105.143022.123456";
        let v2 = "20250105.143022.123457";
        let v3 = "20250106.120000.000000";

        assert!(v1 < v2);
        assert!(v2 < v3);
        assert!(v1 < v3);
    }

    #[test]
    fn test_empty_workflow_name() {
        let definition = WorkflowDefinition {
            name: "".to_string(),
            activities: vec![ActivityDefinition {
                key: "step1".to_string(),
                worker: "test".to_string(),
                activity_name: None,
                parameters: None,
                depends_on: None,
                dependency_of: None,
                settings: None,
            }],
        };

        let result = definition.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_no_activities() {
        let definition = WorkflowDefinition {
            name: "test".to_string(),
            activities: vec![],
        };

        let result = definition.validate();
        assert!(result.is_err());
        match result.unwrap_err() {
            ValidationError::MultipleErrors(errors) => {
                let json = errors.to_json();
                let json_str = json.to_string();
                assert!(json_str.contains("at least one activity"));
            }
            _ => panic!("Expected MultipleErrors"),
        }
    }
}
