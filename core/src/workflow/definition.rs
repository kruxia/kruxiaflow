use super::outputs::ActivityOutputDefinition;
use chrono::{DateTime, NaiveDateTime, Utc};
use rust_decimal::Decimal;
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

    /// Compute a SHA-256 hash of the normalized definition content.
    /// Used for idempotent deployment comparison.
    ///
    /// The hash is computed from a normalized JSON representation that:
    /// - Excludes version and created_at metadata (these vary per deployment)
    /// - Uses sorted keys for deterministic serialization
    /// - Includes name and activities
    ///
    /// Returns raw 32-byte hash (stored as BYTEA in PostgreSQL).
    /// Two definitions with the same content_hash are considered identical.
    pub fn content_hash(&self) -> Vec<u8> {
        use sha2::{Digest, Sha256};

        // Create normalized representation (excludes version, created_at)
        // Note: serde_json serializes keys in their struct definition order,
        // and we use BTreeMap-like ordering for the JSON object
        let normalized = serde_json::json!({
            "activities": &self.activities,
            "name": &self.name,
        });

        // Serialize deterministically (serde_json sorts object keys)
        let canonical =
            serde_json::to_string(&normalized).expect("WorkflowDefinition should always serialize");

        // Compute SHA-256 hash
        let mut hasher = Sha256::new();
        hasher.update(canonical.as_bytes());
        hasher.finalize().to_vec()
    }

    /// Validate workflow definition structure and compute metadata
    pub fn validate(&mut self) -> Result<(), ValidationError> {
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

        // Validate graph structure (references, then check for loops vs cycles)
        if let Err(e) = self.validate_graph(&activity_keys) {
            errors.merge(e);
        }

        // Detect loops (back-edges) and mark activities/dependencies
        // This happens after basic validation passes
        if errors.is_empty() {
            match self.detect_and_validate_loops() {
                Ok(_) => {}
                Err(e) => errors.merge(e),
            }
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

        // Validate scheduling settings
        if let Some(settings) = &activity.settings
            && let Err(e) = validate_activity_settings(settings, &activity.key)
        {
            errors.merge(e);
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
            graph.entry(activity.key.as_str()).or_default();
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

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Detect loops (back-edges) in the workflow graph and validate them
    /// Also marks is_loop_activity and is_back_edge metadata for orchestrator performance
    fn detect_and_validate_loops(&mut self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();

        // Perform topological sort to identify back-edges
        // Back-edges are edges from later to earlier in topological order
        let loops = match self.detect_loops() {
            Ok(loops) => loops,
            Err(e) => {
                errors.add("activities", &e);
                return Err(errors);
            }
        };

        // Mark activities and dependencies that participate in loops (cache for orchestrator hot path)
        for loop_edge in &loops {
            // Mark both activities in the loop
            for activity in &mut self.activities {
                if activity.key == loop_edge.from || activity.key == loop_edge.to {
                    activity.is_loop_activity = true;
                }

                // Mark the specific dependency that is the back-edge
                // LoopEdge represents the graph edge from -> to
                // In dependency notation: edge A->B means "B depends_on A"
                // So the back-edge should be marked on activity.key==to, dep.activity_key==from
                if activity.key == loop_edge.to {
                    // Check both depends_on and dependency_of (before normalization)
                    if let Some(depends_on) = &mut activity.depends_on {
                        for dep in depends_on {
                            if dep.activity_key == loop_edge.from {
                                dep.is_back_edge = true;
                            }
                        }
                    }
                    if let Some(dependency_of) = &mut activity.dependency_of {
                        for dep in dependency_of {
                            if dep.activity_key == loop_edge.from {
                                dep.is_back_edge = true;
                            }
                        }
                    }
                }
            }
        }

        // Validate that loops have proper configuration
        for loop_edge in loops {
            if let Err(e) = self.validate_loop_edge(&loop_edge) {
                errors.merge(e);
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Detect back-edges (loops) in the workflow graph via topological sort
    fn detect_loops(&self) -> Result<Vec<LoopEdge>, String> {
        // Build adjacency list
        let mut graph: HashMap<&str, Vec<&str>> = HashMap::new();
        for activity in &self.activities {
            graph.entry(activity.key.as_str()).or_default();
        }

        // Add edges from dependencies (both depends_on and dependency_of, before normalization)
        for activity in &self.activities {
            if let Some(depends_on) = &activity.depends_on {
                for dep in depends_on {
                    graph
                        .get_mut(dep.activity_key.as_str())
                        .unwrap()
                        .push(activity.key.as_str());
                }
            }
            if let Some(dependency_of) = &activity.dependency_of {
                for dep in dependency_of {
                    graph
                        .get_mut(activity.key.as_str())
                        .unwrap()
                        .push(dep.activity_key.as_str());
                }
            }
        }

        // Perform topological sort using Kahn's algorithm
        let sorted = match topological_sort_kahn(&graph) {
            Ok(sorted) => sorted,
            Err(_cycle) => {
                // There's a cycle - but we need to check if it's a valid loop
                // For now, if there's a cycle with no exit mechanism, it will be caught
                // during loop validation. We'll continue and try to identify back-edges.
                // If we can't do topological sort, fall back to DFS-based detection
                return self.detect_loops_via_dfs(&graph);
            }
        };

        let mut loops = Vec::new();

        // Build position map for quick lookup
        let position_map: HashMap<&str, usize> = sorted
            .iter()
            .enumerate()
            .map(|(idx, key)| (key.as_str(), idx))
            .collect();

        // Any edge from later to earlier in topo order is a back-edge
        // If activity depends_on dep, the dependency edge is dep → activity
        // A back-edge occurs when dep comes AFTER activity in topo order (creating a cycle)
        for activity in &self.activities {
            if let Some(depends_on) = &activity.depends_on {
                for dep in depends_on {
                    let from_pos = position_map.get(activity.key.as_str());
                    let to_pos = position_map.get(dep.activity_key.as_str());

                    if let (Some(&from_idx), Some(&to_idx)) = (from_pos, to_pos) {
                        // Back-edge: dependency comes AFTER the dependent activity in topo order
                        if to_idx > from_idx {
                            // Back-edge found (dependency points backward in topo order)
                            // The edge in the graph is dep → activity, so from should be dep
                            loops.push(LoopEdge {
                                from: dep.activity_key.clone(),
                                to: activity.key.clone(),
                                condition: dep.conditions.clone(),
                            });
                        }
                    }
                }
            }
        }

        Ok(loops)
    }

    /// Fallback loop detection using DFS when topological sort fails
    fn detect_loops_via_dfs(
        &self,
        graph: &HashMap<&str, Vec<&str>>,
    ) -> Result<Vec<LoopEdge>, String> {
        let mut loops = Vec::new();
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();

        // Find all back-edges using DFS
        // Sort keys for deterministic iteration order (HashMap iteration is non-deterministic)
        let mut nodes: Vec<&str> = graph.keys().copied().collect();
        nodes.sort();
        for node in nodes {
            if !visited.contains(node) {
                self.dfs_find_loops(node, graph, &mut visited, &mut rec_stack, &mut loops);
            }
        }

        if loops.is_empty() {
            // No loops found, but there was a cycle - this is an invalid cycle
            if let Some(cycle) = detect_cycle(graph) {
                return Err(format!(
                    "Workflow contains an invalid cycle (not a valid loop): {}",
                    cycle.join(" -> ")
                ));
            }
        }

        Ok(loops)
    }

    /// DFS helper to find back-edges (loops)
    fn dfs_find_loops<'a>(
        &'a self,
        node: &'a str,
        graph: &HashMap<&'a str, Vec<&'a str>>,
        visited: &mut HashSet<&'a str>,
        rec_stack: &mut HashSet<&'a str>,
        loops: &mut Vec<LoopEdge>,
    ) {
        visited.insert(node);
        rec_stack.insert(node);

        if let Some(neighbors) = graph.get(node) {
            for &neighbor in neighbors {
                if !visited.contains(neighbor) {
                    self.dfs_find_loops(neighbor, graph, visited, rec_stack, loops);
                } else if rec_stack.contains(neighbor) {
                    // Found a back-edge (loop)
                    // In a cycle, check both directions for conditions
                    // The back-edge is the one with the loop condition

                    // Check current direction: node depends_on neighbor (edge: neighbor -> node)
                    // This means "neighbor.depends_on.node" has the condition
                    let mut condition_forward = None;
                    if let Some(activity) = self.activities.iter().find(|a| a.key == neighbor)
                        && let Some(depends_on) = &activity.depends_on
                    {
                        for dep in depends_on {
                            if dep.activity_key == node {
                                condition_forward = dep.conditions.clone();
                            }
                        }
                    }

                    // Check reverse direction: neighbor depends_on node (edge: node -> neighbor)
                    // This means "node.depends_on.neighbor" has the condition
                    let mut condition_reverse = None;
                    if let Some(activity) = self.activities.iter().find(|a| a.key == node)
                        && let Some(depends_on) = &activity.depends_on
                    {
                        for dep in depends_on {
                            if dep.activity_key == neighbor {
                                condition_reverse = dep.conditions.clone();
                            }
                        }
                    }

                    // Determine back-edge direction based on which edge has the condition
                    // The back-edge is the one that loops back (has the condition)
                    // condition_reverse: node depends_on neighbor → edge is neighbor -> node
                    // condition_forward: neighbor depends_on node → edge is node -> neighbor
                    let (from, to, condition) = if condition_reverse.is_some() {
                        // node depends_on neighbor with condition → back-edge: neighbor -> node
                        (neighbor.to_string(), node.to_string(), condition_reverse)
                    } else if condition_forward.is_some() {
                        // neighbor depends_on node with condition → back-edge: node -> neighbor
                        (node.to_string(), neighbor.to_string(), condition_forward)
                    } else {
                        // No condition found - use DFS traversal direction as default
                        // DFS edge node -> neighbor means we're at node going to neighbor
                        (node.to_string(), neighbor.to_string(), None)
                    };

                    loops.push(LoopEdge {
                        from,
                        to,
                        condition,
                    });
                }
            }
        }

        rec_stack.remove(node);
    }

    /// Validate that a loop edge has proper configuration (exit mechanism)
    fn validate_loop_edge(&self, loop_edge: &LoopEdge) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();

        let from_activity = self.activities.iter().find(|a| a.key == loop_edge.from);
        let to_activity = self.activities.iter().find(|a| a.key == loop_edge.to);

        let from_activity = match from_activity {
            Some(a) => a,
            None => return Ok(()), // Activity not found, already caught by earlier validation
        };
        let to_activity = match to_activity {
            Some(a) => a,
            None => return Ok(()),
        };

        // Check if loop has exit condition
        let has_condition = loop_edge
            .condition
            .as_ref()
            .map(|c| !c.is_empty())
            .unwrap_or(false);

        // Check if loop has iteration limit (check both activity-level and settings-level)
        let has_iteration_limit = from_activity.iteration_limit.is_some()
            || to_activity.iteration_limit.is_some()
            || from_activity
                .settings
                .as_ref()
                .and_then(|s| s.iteration_limit)
                .is_some()
            || to_activity
                .settings
                .as_ref()
                .and_then(|s| s.iteration_limit)
                .is_some();

        // Loop MUST have at least one exit mechanism: condition OR iteration_limit OR both
        if !has_condition && !has_iteration_limit {
            errors.add(
                "activities",
                &format!(
                    "Workflow contains a cycle from '{}' to '{}' that must have at least one exit mechanism:\n\
                    - Exit condition: conditions: [\"{{{{evaluate.done == true}}}}\"]\n\
                    - Iteration limit: iteration_limit: 10\n\
                    - Or both (recommended for production)\n\n\
                    Examples:\n\
                    1) Fixed iterations: iteration_limit: 12\n\
                    2) Conditional: conditions: [\"{{{{status.canceled == false}}}}\"]\n\
                    3) Bounded conditional: iteration_limit: 10 AND conditions",
                    loop_edge.from, loop_edge.to
                ),
            );
        }

        // Recommend iteration_scoped for loop activities (warning only, not error)
        if !from_activity.iteration_scoped && !to_activity.iteration_scoped {
            tracing::warn!(
                "Loop from '{}' to '{}' does not use iteration_scoped. \
                Iteration limits will still be enforced, but only the latest outputs will be available. \
                Consider setting iteration_scoped: true to track results per iteration.",
                loop_edge.from,
                loop_edge.to
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
                        .or_default()
                        .push(ActivityRelationship {
                            activity_key: activity.key.clone(),
                            conditions: relationship.conditions.clone(),
                            is_back_edge: false, // Always computed during validation
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

/// Helper for serde skip_serializing_if
fn is_false(b: &bool) -> bool {
    !b
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

    /// Activity output definitions (name and type)
    #[serde(rename = "outputs")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_definitions: Option<Vec<ActivityOutputDefinition>>,

    /// Activities that must complete before this one
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<Vec<ActivityRelationship>>,

    /// Activities that depend on this one (input only, cleared after normalization)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependency_of: Option<Vec<ActivityRelationship>>,

    /// Activity-level settings (timeout, retry, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings: Option<ActivitySettings>,

    /// Whether to store separate outputs for each iteration
    #[serde(default)]
    pub iteration_scoped: bool,

    /// Maximum number of iterations (prevents infinite loops)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iteration_limit: Option<u32>,

    /// Cached metadata: whether this activity is part of a loop (has back-edge)
    /// Computed during validation, stored in database
    /// NOT specified in YAML (ignored if present)
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_loop_activity: bool,

    /// Token streaming configuration for LLM activities
    /// Supports shorthand `streaming: true` or detailed `streaming: { enabled: true }`
    #[serde(default, skip_serializing_if = "StreamingConfig::is_disabled")]
    pub streaming: StreamingConfig,
}

/// Relationship between activities (edge in the directed graph)
#[derive(Debug, Clone, Serialize)]
pub struct ActivityRelationship {
    /// Key of the related activity
    pub activity_key: String,

    /// Optional conditions that must be satisfied for this edge to activate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conditions: Option<Vec<String>>,

    /// Cached metadata: whether this dependency is a back-edge (loop)
    /// Computed during validation, stored in database
    /// NOT specified in YAML (ignored if present)
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_back_edge: bool,
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
            // Full object: {activity_key: "key", conditions: [...], is_back_edge: bool}
            // is_back_edge is computed during validation and stored in the database
            Full {
                activity_key: String,
                #[serde(alias = "condition")]
                conditions: Option<ConditionOrConditions>,
                #[serde(default)]
                is_back_edge: bool,
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
                is_back_edge: false, // Defaults to false for simple string syntax
            }),
            ActivityRelationshipHelper::Full {
                activity_key,
                conditions,
                is_back_edge,
            } => {
                let conditions = conditions.map(|c| match c {
                    ConditionOrConditions::Single(s) => vec![s],
                    ConditionOrConditions::Multiple(v) => v,
                });
                Ok(ActivityRelationship {
                    activity_key,
                    conditions,
                    is_back_edge, // Use the deserialized value (computed during validation)
                })
            }
        }
    }
}

/// Workflow-level settings
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkflowSettings {
    /// Maximum workflow execution time in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,

    /// Maximum retry attempts for transient failures
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,

    /// Workflow-level budget limit
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget: Option<BudgetSettings>,
}

/// Activity-level settings
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ActivitySettings {
    /// Timeout in seconds for activity execution
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,

    /// Retry policy
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryPolicy>,

    /// Budget limits
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<BudgetSettings>,

    /// Enable result caching
    #[serde(default)]
    pub cache: bool,

    /// Cache TTL in seconds
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_ttl: Option<u64>,

    /// Per-activity iteration limit (can override activity-level iteration_limit)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iteration_limit: Option<u32>,

    /// Relative delay: "500ms", "5s", "30m", "30mi", "2h", "7d", "1w", "2mo", "1y"
    /// Template-aware: "{{INPUT.delay_amount}}m"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay: Option<String>,

    /// Absolute ISO 8601 timestamp for scheduling (template-aware)
    /// Example: "2025-12-01T09:00:00-08:00" or "{{INPUT.report_deadline}}"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled_for: Option<String>,
}

impl ActivitySettings {
    /// Calculate backoff delay for retry attempt
    pub fn calculate_backoff(&self, attempt: u32) -> Option<u64> {
        let retry = self.retry.as_ref()?;

        if attempt >= retry.max_attempts {
            return None; // Max attempts reached
        }

        let delay = match retry.strategy {
            BackoffStrategy::Fixed => retry.base_seconds,
            BackoffStrategy::Exponential => {
                let exponential = retry.base_seconds as f64 * retry.factor.powi(attempt as i32 - 1);
                exponential.min(retry.max_seconds as f64) as u64
            }
        };

        Some(delay.min(retry.max_seconds))
    }

    /// Check if activity should be retried
    pub fn should_retry(&self, attempt: u32) -> bool {
        if let Some(retry) = &self.retry {
            attempt < retry.max_attempts
        } else {
            false
        }
    }

    /// Get timeout duration
    pub fn timeout_duration(&self) -> Option<std::time::Duration> {
        self.timeout_seconds.map(std::time::Duration::from_secs)
    }
}

/// Retry policy configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Maximum retry attempts (default: 1 = no retries)
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,

    /// Backoff strategy: exponential or fixed
    #[serde(default)]
    pub strategy: BackoffStrategy,

    /// Base delay in seconds between retries
    #[serde(default = "default_base_seconds")]
    pub base_seconds: u64,

    /// Exponential multiplier (for exponential strategy)
    #[serde(default = "default_factor")]
    pub factor: f64,

    /// Maximum backoff delay cap in seconds
    #[serde(default = "default_max_seconds")]
    pub max_seconds: u64,
}

fn default_max_attempts() -> u32 {
    1
}

fn default_base_seconds() -> u64 {
    2
}

fn default_factor() -> f64 {
    2.0
}

fn default_max_seconds() -> u64 {
    300
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: default_max_attempts(),
            strategy: BackoffStrategy::default(),
            base_seconds: default_base_seconds(),
            factor: default_factor(),
            max_seconds: default_max_seconds(),
        }
    }
}

/// Backoff strategy for retries
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum BackoffStrategy {
    /// Exponential backoff (delay doubles each retry)
    #[default]
    Exponential,
    /// Fixed delay between retries
    Fixed,
}

/// Budget configuration for activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetSettings {
    /// Budget limit in USD
    pub limit: Decimal,

    /// Action when budget exceeded
    #[serde(default)]
    pub action: BudgetAction,
}

/// Action to take when budget is exceeded
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum BudgetAction {
    /// Abort workflow execution
    #[default]
    Abort,

    /// Continue with warning
    Continue,
}

/// Token streaming configuration for LLM activities
/// Supports both shorthand `streaming: true` and detailed `streaming: { enabled: true }`
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum StreamingConfig {
    /// Streaming disabled (default)
    Disabled,
    /// Shorthand: `streaming: true` or `streaming: false`
    Simple(bool),
    /// Detailed: `streaming: { enabled: true }`
    Detailed(StreamingOptions),
}

impl Default for StreamingConfig {
    fn default() -> Self {
        StreamingConfig::Disabled
    }
}

impl<'de> Deserialize<'de> for StreamingConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;

        let value = serde_json::Value::deserialize(deserializer)?;

        match value {
            serde_json::Value::Bool(b) => Ok(StreamingConfig::Simple(b)),
            serde_json::Value::Object(_) => {
                let options: StreamingOptions =
                    serde_json::from_value(value).map_err(D::Error::custom)?;
                Ok(StreamingConfig::Detailed(options))
            }
            serde_json::Value::Null => Ok(StreamingConfig::Disabled),
            _ => Err(D::Error::custom(
                "streaming must be a boolean or object with 'enabled' field",
            )),
        }
    }
}

impl StreamingConfig {
    /// Check if streaming is enabled
    pub fn is_enabled(&self) -> bool {
        match self {
            StreamingConfig::Disabled => false,
            StreamingConfig::Simple(enabled) => *enabled,
            StreamingConfig::Detailed(options) => options.enabled,
        }
    }

    /// Check if streaming is disabled (for serde skip_serializing_if)
    pub fn is_disabled(&self) -> bool {
        !self.is_enabled()
    }
}

/// Detailed streaming options
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingOptions {
    /// Whether streaming is enabled
    #[serde(default)]
    pub enabled: bool,
}

// Legacy type aliases for backward compatibility
pub type RetrySettings = RetryPolicy;

/// Loop edge structure for loop detection
#[derive(Debug, Clone)]
struct LoopEdge {
    from: String,
    to: String,
    condition: Option<Vec<String>>,
}

/// Check if string is valid identifier (alphanumeric, hyphens, underscores)
fn is_valid_identifier(s: &str) -> bool {
    let re = regex::Regex::new(r"^[a-zA-Z0-9_-]+$").unwrap();
    re.is_match(s)
}

/// Validate activity settings (scheduling constraints)
fn validate_activity_settings(
    settings: &ActivitySettings,
    activity_key: &str,
) -> Result<(), ValidationErrors> {
    let mut errors = ValidationErrors::new();

    // Mutually exclusive check: cannot specify both delay and scheduled_for
    if settings.delay.is_some() && settings.scheduled_for.is_some() {
        errors.add(
            &format!("activity.{}.settings", activity_key),
            "Cannot specify both 'delay' and 'scheduled_for' - these fields are mutually exclusive",
        );
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Parse duration string and add to a given DateTime
///
/// Supports units: ms, s, m, mi, h, d, w, mo, y
/// Examples: "500ms", "5s", "30m", "30mi", "2h", "7d", "1w", "2mo", "1y"
pub fn apply_duration(
    base_time: DateTime<Utc>,
    duration_str: &str,
) -> Result<DateTime<Utc>, String> {
    use chrono::{Duration, Months};
    use regex::Regex;

    let re = Regex::new(r"^(\d+)(ms|s|m|mi|h|d|w|mo|y)$").unwrap();

    let caps = re.captures(duration_str).ok_or_else(|| {
        format!(
            "Invalid duration format: '{}'. Expected format: <number><unit> (e.g., 500ms, 5s, 30m, 2h, 7d, 2mo)",
            duration_str
        )
    })?;

    let amount: i64 = caps[1]
        .parse()
        .map_err(|e| format!("Invalid number: {}", e))?;
    let unit = &caps[2];

    let result = match unit {
        "ms" => base_time + Duration::milliseconds(amount),
        "s" => base_time + Duration::seconds(amount),
        "m" | "mi" => base_time + Duration::minutes(amount),
        "h" => base_time + Duration::hours(amount),
        "d" => base_time + Duration::days(amount),
        "w" => base_time + Duration::weeks(amount),
        "mo" => {
            let months = Months::new(amount as u32);
            base_time
                .checked_add_months(months)
                .ok_or_else(|| format!("Month addition overflow: {}", duration_str))?
        }
        "y" => {
            let months = Months::new((amount * 12) as u32);
            base_time
                .checked_add_months(months)
                .ok_or_else(|| format!("Year addition overflow: {}", duration_str))?
        }
        _ => return Err(format!("Unknown unit: {}", unit)),
    };

    Ok(result)
}

/// Parse ISO 8601 timestamp string to DateTime<Utc>
///
/// Example: "2025-12-01T09:00:00-08:00"
pub fn parse_scheduled_for(timestamp_str: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(timestamp_str)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| format!("Invalid ISO 8601 timestamp: {}", e))
}

/// Topological sort using Kahn's algorithm
/// Returns Ok(sorted_keys) if successful, Err(()) if cycle detected
fn topological_sort_kahn<'a>(graph: &HashMap<&'a str, Vec<&'a str>>) -> Result<Vec<String>, ()> {
    // Calculate in-degrees
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    for &node in graph.keys() {
        in_degree.entry(node).or_insert(0);
    }
    for neighbors in graph.values() {
        for &neighbor in neighbors {
            *in_degree.entry(neighbor).or_insert(0) += 1;
        }
    }

    // Queue of nodes with in-degree 0
    let mut queue: Vec<&str> = in_degree
        .iter()
        .filter(|(_, deg)| **deg == 0)
        .map(|(&node, _)| node)
        .collect();

    let mut sorted = Vec::new();

    while let Some(node) = queue.pop() {
        sorted.push(node.to_string());

        if let Some(neighbors) = graph.get(node) {
            for &neighbor in neighbors {
                if let Some(deg) = in_degree.get_mut(neighbor) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push(neighbor);
                    }
                }
            }
        }
    }

    // If sorted doesn't contain all nodes, there's a cycle
    if sorted.len() != graph.len() {
        Err(())
    } else {
        Ok(sorted)
    }
}

/// Detect cycles in directed graph using DFS
fn detect_cycle<'a>(graph: &HashMap<&'a str, Vec<&'a str>>) -> Option<Vec<String>> {
    let mut visited = HashSet::new();
    let mut rec_stack = HashSet::new();
    let mut path = Vec::new();

    for node in graph.keys() {
        if !visited.contains(node)
            && let Some(cycle) = dfs_cycle(node, graph, &mut visited, &mut rec_stack, &mut path)
        {
            return Some(cycle);
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

impl Default for ValidationErrors {
    fn default() -> Self {
        Self::new()
    }
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
            .or_default()
            .push(message.to_string());
    }

    pub fn merge(&mut self, other: ValidationErrors) {
        for (field, messages) in other.errors {
            self.errors.entry(field).or_default().extend(messages);
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
        let mut definition = WorkflowDefinition {
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
                        is_back_edge: false,
                    }]),
                    settings: None,
                    output_definitions: None,
                    iteration_scoped: false,
                    iteration_limit: None,
                    is_loop_activity: false,
                    streaming: Default::default(),
                },
                ActivityDefinition {
                    key: "authorize".to_string(),
                    worker: "payments".to_string(),
                    activity_name: Some("authorize_card".to_string()),
                    parameters: None,
                    depends_on: None,
                    dependency_of: None,
                    settings: None,
                    output_definitions: None,
                    iteration_scoped: false,
                    iteration_limit: None,
                    is_loop_activity: false,
                    streaming: Default::default(),
                },
            ],
        };

        assert!(definition.validate().is_ok());
    }

    #[test]
    fn test_validate_duplicate_activity_keys() {
        let mut definition = WorkflowDefinition {
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
                    output_definitions: None,
                    iteration_scoped: false,
                    iteration_limit: None,
                    is_loop_activity: false,
                    streaming: Default::default(),
                },
                ActivityDefinition {
                    key: "step1".to_string(), // Duplicate!
                    worker: "test".to_string(),
                    activity_name: None,
                    parameters: None,
                    depends_on: None,
                    dependency_of: None,
                    settings: None,
                    output_definitions: None,
                    iteration_scoped: false,
                    iteration_limit: None,
                    is_loop_activity: false,
                    streaming: Default::default(),
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
        let mut definition = WorkflowDefinition {
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
                    is_back_edge: false,
                }]),
                settings: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
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
        let mut definition = WorkflowDefinition {
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
                        is_back_edge: false,
                    }]),
                    settings: None,
                    output_definitions: None,
                    iteration_scoped: false,
                    iteration_limit: None,
                    is_loop_activity: false,
                    streaming: Default::default(),
                },
                ActivityDefinition {
                    key: "step2".to_string(),
                    worker: "test".to_string(),
                    activity_name: None,
                    parameters: None,
                    depends_on: None,
                    dependency_of: Some(vec![ActivityRelationship {
                        activity_key: "step1".to_string(), // Cycle without exit mechanism!
                        conditions: None,
                        is_back_edge: false,
                    }]),
                    settings: None,
                    output_definitions: None,
                    iteration_scoped: false,
                    iteration_limit: None,
                    is_loop_activity: false,
                    streaming: Default::default(),
                },
            ],
        };

        let result = definition.validate();
        assert!(result.is_err());
        match result.unwrap_err() {
            ValidationError::MultipleErrors(errors) => {
                let json = errors.to_json();
                let json_str = json.to_string();
                // Should fail because loop has no exit mechanism
                assert!(json_str.contains("exit mechanism") || json_str.contains("cycle"));
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
        let mut definition = WorkflowDefinition {
            name: "".to_string(),
            activities: vec![ActivityDefinition {
                key: "step1".to_string(),
                worker: "test".to_string(),
                activity_name: None,
                parameters: None,
                depends_on: None,
                dependency_of: None,
                settings: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            }],
        };

        let result = definition.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_no_activities() {
        let mut definition = WorkflowDefinition {
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

    // Loop validation tests

    #[test]
    fn test_loop_with_iteration_limit_only() {
        // Pattern 1: iteration_limit only
        let mut definition = WorkflowDefinition {
            name: "test_loop".to_string(),
            activities: vec![
                ActivityDefinition {
                    key: "search".to_string(),
                    worker: "test".to_string(),
                    activity_name: Some("search".to_string()),
                    parameters: None,
                    depends_on: Some(vec![ActivityRelationship {
                        activity_key: "evaluate".to_string(),
                        conditions: None, // No condition
                        is_back_edge: false,
                    }]),
                    dependency_of: None,
                    settings: None,
                    output_definitions: None,
                    iteration_scoped: true,
                    iteration_limit: Some(10), // Has iteration_limit
                    is_loop_activity: false,
                    streaming: Default::default(),
                },
                ActivityDefinition {
                    key: "evaluate".to_string(),
                    worker: "test".to_string(),
                    activity_name: Some("evaluate".to_string()),
                    parameters: None,
                    depends_on: Some(vec![ActivityRelationship {
                        activity_key: "search".to_string(),
                        conditions: None,
                        is_back_edge: false,
                    }]),
                    dependency_of: None,
                    settings: None,
                    output_definitions: None,
                    iteration_scoped: true,
                    iteration_limit: None,
                    is_loop_activity: false,
                    streaming: Default::default(),
                },
            ],
        };

        // Should pass - has iteration_limit
        let result = definition.validate();
        assert!(
            result.is_ok(),
            "Pattern 1 (iteration_limit only) should pass: {:?}",
            result
        );

        // Check that metadata was set
        let search = definition
            .activities
            .iter()
            .find(|a| a.key == "search")
            .unwrap();
        assert!(
            search.is_loop_activity,
            "search should be marked as loop activity"
        );
    }

    #[test]
    fn test_loop_with_condition_only() {
        // Pattern 2: condition only
        let mut definition = WorkflowDefinition {
            name: "test_loop".to_string(),
            activities: vec![
                ActivityDefinition {
                    key: "search".to_string(),
                    worker: "test".to_string(),
                    activity_name: Some("search".to_string()),
                    parameters: None,
                    depends_on: Some(vec![ActivityRelationship {
                        activity_key: "evaluate".to_string(),
                        conditions: Some(vec!["{{evaluate.done == false}}".to_string()]), // Has condition
                        is_back_edge: false,
                    }]),
                    dependency_of: None,
                    settings: None,
                    output_definitions: None,
                    iteration_scoped: true,
                    iteration_limit: None, // No iteration_limit
                    is_loop_activity: false,
                    streaming: Default::default(),
                },
                ActivityDefinition {
                    key: "evaluate".to_string(),
                    worker: "test".to_string(),
                    activity_name: Some("evaluate".to_string()),
                    parameters: None,
                    depends_on: Some(vec![ActivityRelationship {
                        activity_key: "search".to_string(),
                        conditions: None,
                        is_back_edge: false,
                    }]),
                    dependency_of: None,
                    settings: None,
                    output_definitions: None,
                    iteration_scoped: true,
                    iteration_limit: None,
                    is_loop_activity: false,
                    streaming: Default::default(),
                },
            ],
        };

        // Should pass - has condition
        let result = definition.validate();
        assert!(
            result.is_ok(),
            "Pattern 2 (condition only) should pass: {:?}",
            result
        );
    }

    #[test]
    fn test_loop_with_both() {
        // Pattern 3: both condition and iteration_limit
        let mut definition = WorkflowDefinition {
            name: "test_loop".to_string(),
            activities: vec![
                ActivityDefinition {
                    key: "search".to_string(),
                    worker: "test".to_string(),
                    activity_name: Some("search".to_string()),
                    parameters: None,
                    depends_on: Some(vec![ActivityRelationship {
                        activity_key: "evaluate".to_string(),
                        conditions: Some(vec!["{{evaluate.done == false}}".to_string()]), // Has condition
                        is_back_edge: false,
                    }]),
                    dependency_of: None,
                    settings: None,
                    output_definitions: None,
                    iteration_scoped: true,
                    iteration_limit: Some(10), // Has iteration_limit
                    is_loop_activity: false,
                    streaming: Default::default(),
                },
                ActivityDefinition {
                    key: "evaluate".to_string(),
                    worker: "test".to_string(),
                    activity_name: Some("evaluate".to_string()),
                    parameters: None,
                    depends_on: Some(vec![ActivityRelationship {
                        activity_key: "search".to_string(),
                        conditions: None,
                        is_back_edge: false,
                    }]),
                    dependency_of: None,
                    settings: None,
                    output_definitions: None,
                    iteration_scoped: true,
                    iteration_limit: None,
                    is_loop_activity: false,
                    streaming: Default::default(),
                },
            ],
        };

        // Should pass - has both
        let result = definition.validate();
        assert!(result.is_ok(), "Pattern 3 (both) should pass: {:?}", result);
    }

    #[test]
    fn test_loop_requires_exit_mechanism() {
        // Loop with neither condition nor iteration_limit
        let mut definition = WorkflowDefinition {
            name: "test_loop".to_string(),
            activities: vec![
                ActivityDefinition {
                    key: "search".to_string(),
                    worker: "test".to_string(),
                    activity_name: Some("search".to_string()),
                    parameters: None,
                    depends_on: Some(vec![ActivityRelationship {
                        activity_key: "evaluate".to_string(),
                        conditions: None, // No condition
                        is_back_edge: false,
                    }]),
                    dependency_of: None,
                    settings: None,
                    output_definitions: None,
                    iteration_scoped: true,
                    iteration_limit: None, // No iteration_limit
                    is_loop_activity: false,
                    streaming: Default::default(),
                },
                ActivityDefinition {
                    key: "evaluate".to_string(),
                    worker: "test".to_string(),
                    activity_name: Some("evaluate".to_string()),
                    parameters: None,
                    depends_on: Some(vec![ActivityRelationship {
                        activity_key: "search".to_string(),
                        conditions: None,
                        is_back_edge: false,
                    }]),
                    dependency_of: None,
                    settings: None,
                    output_definitions: None,
                    iteration_scoped: true,
                    iteration_limit: None,
                    is_loop_activity: false,
                    streaming: Default::default(),
                },
            ],
        };

        // Should fail - no exit mechanism
        let result = definition.validate();
        assert!(result.is_err(), "Loop without exit mechanism should fail");
        match result.unwrap_err() {
            ValidationError::MultipleErrors(errors) => {
                let json = errors.to_json();
                let json_str = json.to_string();
                assert!(
                    json_str.contains("exit mechanism"),
                    "Error should mention exit mechanism"
                );
            }
            _ => panic!("Expected MultipleErrors"),
        }
    }

    // Activity scheduling tests (US-3.7 Phase 1)

    #[test]
    fn test_parse_delay_milliseconds() {
        let yaml = r#"
name: test_delay
activities:
  - key: task1
    worker: builtin
    settings:
      delay: "500ms"
"#;
        let result = WorkflowDefinition::from_yaml(yaml);
        assert!(result.is_ok(), "Should parse delay with milliseconds");
        let workflow = result.unwrap();
        let settings = workflow.activities[0].settings.as_ref().unwrap();
        assert_eq!(settings.delay, Some("500ms".to_string()));
    }

    #[test]
    fn test_parse_delay_seconds() {
        let yaml = r#"
name: test_delay
activities:
  - key: task1
    worker: builtin
    settings:
      delay: "5s"
"#;
        let result = WorkflowDefinition::from_yaml(yaml);
        assert!(result.is_ok(), "Should parse delay with seconds");
        let workflow = result.unwrap();
        let settings = workflow.activities[0].settings.as_ref().unwrap();
        assert_eq!(settings.delay, Some("5s".to_string()));
    }

    #[test]
    fn test_parse_delay_minutes_m() {
        let yaml = r#"
name: test_delay
activities:
  - key: task1
    worker: builtin
    settings:
      delay: "30m"
"#;
        let result = WorkflowDefinition::from_yaml(yaml);
        assert!(result.is_ok(), "Should parse delay with minutes (m)");
        let workflow = result.unwrap();
        let settings = workflow.activities[0].settings.as_ref().unwrap();
        assert_eq!(settings.delay, Some("30m".to_string()));
    }

    #[test]
    fn test_parse_delay_minutes_mi() {
        let yaml = r#"
name: test_delay
activities:
  - key: task1
    worker: builtin
    settings:
      delay: "30mi"
"#;
        let result = WorkflowDefinition::from_yaml(yaml);
        assert!(result.is_ok(), "Should parse delay with minutes (mi)");
        let workflow = result.unwrap();
        let settings = workflow.activities[0].settings.as_ref().unwrap();
        assert_eq!(settings.delay, Some("30mi".to_string()));
    }

    #[test]
    fn test_parse_delay_hours() {
        let yaml = r#"
name: test_delay
activities:
  - key: task1
    worker: builtin
    settings:
      delay: "2h"
"#;
        let result = WorkflowDefinition::from_yaml(yaml);
        assert!(result.is_ok(), "Should parse delay with hours");
        let workflow = result.unwrap();
        let settings = workflow.activities[0].settings.as_ref().unwrap();
        assert_eq!(settings.delay, Some("2h".to_string()));
    }

    #[test]
    fn test_parse_delay_days() {
        let yaml = r#"
name: test_delay
activities:
  - key: task1
    worker: builtin
    settings:
      delay: "7d"
"#;
        let result = WorkflowDefinition::from_yaml(yaml);
        assert!(result.is_ok(), "Should parse delay with days");
        let workflow = result.unwrap();
        let settings = workflow.activities[0].settings.as_ref().unwrap();
        assert_eq!(settings.delay, Some("7d".to_string()));
    }

    #[test]
    fn test_parse_delay_weeks() {
        let yaml = r#"
name: test_delay
activities:
  - key: task1
    worker: builtin
    settings:
      delay: "1w"
"#;
        let result = WorkflowDefinition::from_yaml(yaml);
        assert!(result.is_ok(), "Should parse delay with weeks");
        let workflow = result.unwrap();
        let settings = workflow.activities[0].settings.as_ref().unwrap();
        assert_eq!(settings.delay, Some("1w".to_string()));
    }

    #[test]
    fn test_parse_delay_months() {
        let yaml = r#"
name: test_delay
activities:
  - key: task1
    worker: builtin
    settings:
      delay: "2mo"
"#;
        let result = WorkflowDefinition::from_yaml(yaml);
        assert!(result.is_ok(), "Should parse delay with months");
        let workflow = result.unwrap();
        let settings = workflow.activities[0].settings.as_ref().unwrap();
        assert_eq!(settings.delay, Some("2mo".to_string()));
    }

    #[test]
    fn test_parse_delay_years() {
        let yaml = r#"
name: test_delay
activities:
  - key: task1
    worker: builtin
    settings:
      delay: "1y"
"#;
        let result = WorkflowDefinition::from_yaml(yaml);
        assert!(result.is_ok(), "Should parse delay with years");
        let workflow = result.unwrap();
        let settings = workflow.activities[0].settings.as_ref().unwrap();
        assert_eq!(settings.delay, Some("1y".to_string()));
    }

    #[test]
    fn test_parse_scheduled_for() {
        let yaml = r#"
name: test_scheduled
activities:
  - key: task1
    worker: builtin
    settings:
      scheduled_for: "2025-12-01T09:00:00Z"
"#;
        let result = WorkflowDefinition::from_yaml(yaml);
        assert!(result.is_ok(), "Should parse scheduled_for with ISO 8601");
        let workflow = result.unwrap();
        let settings = workflow.activities[0].settings.as_ref().unwrap();
        assert_eq!(
            settings.scheduled_for,
            Some("2025-12-01T09:00:00Z".to_string())
        );
    }

    #[test]
    fn test_reject_both_scheduling_fields() {
        let yaml = r#"
name: test_both
activities:
  - key: task1
    worker: builtin
    settings:
      delay: "5s"
      scheduled_for: "2025-12-01T09:00:00Z"
"#;
        let result = WorkflowDefinition::from_yaml(yaml);
        assert!(
            result.is_err(),
            "Should reject workflow with both delay and scheduled_for"
        );
        match result.unwrap_err() {
            ValidationError::MultipleErrors(errors) => {
                let json = errors.to_json();
                let json_str = json.to_string();
                assert!(
                    json_str.contains("mutually exclusive"),
                    "Error should mention mutually exclusive fields"
                );
            }
            _ => panic!("Expected MultipleErrors"),
        }
    }

    #[test]
    fn test_parse_scheduled_for_with_template() {
        let yaml = r#"
name: test_template
activities:
  - key: task1
    worker: builtin
    settings:
      scheduled_for: "{{INPUT.deadline}}"
"#;
        let result = WorkflowDefinition::from_yaml(yaml);
        assert!(result.is_ok(), "Should parse scheduled_for with template");
        let workflow = result.unwrap();
        let settings = workflow.activities[0].settings.as_ref().unwrap();
        assert_eq!(
            settings.scheduled_for,
            Some("{{INPUT.deadline}}".to_string())
        );
    }

    #[test]
    fn test_parse_delay_with_template() {
        let yaml = r#"
name: test_template
activities:
  - key: task1
    worker: builtin
    settings:
      delay: "{{INPUT.delay_minutes}}m"
"#;
        let result = WorkflowDefinition::from_yaml(yaml);
        assert!(result.is_ok(), "Should parse delay with template");
        let workflow = result.unwrap();
        let settings = workflow.activities[0].settings.as_ref().unwrap();
        assert_eq!(settings.delay, Some("{{INPUT.delay_minutes}}m".to_string()));
    }

    // Duration parsing function tests

    #[test]
    fn test_apply_duration_milliseconds() {
        let base = Utc::now();
        let result = apply_duration(base, "500ms");
        assert!(result.is_ok());
        let expected = base + chrono::Duration::milliseconds(500);
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn test_apply_duration_seconds() {
        let base = Utc::now();
        let result = apply_duration(base, "5s");
        assert!(result.is_ok());
        let expected = base + chrono::Duration::seconds(5);
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn test_apply_duration_minutes_m() {
        let base = Utc::now();
        let result = apply_duration(base, "30m");
        assert!(result.is_ok());
        let expected = base + chrono::Duration::minutes(30);
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn test_apply_duration_minutes_mi() {
        let base = Utc::now();
        let result = apply_duration(base, "30mi");
        assert!(result.is_ok());
        let expected = base + chrono::Duration::minutes(30);
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn test_apply_duration_hours() {
        let base = Utc::now();
        let result = apply_duration(base, "2h");
        assert!(result.is_ok());
        let expected = base + chrono::Duration::hours(2);
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn test_apply_duration_days() {
        let base = Utc::now();
        let result = apply_duration(base, "7d");
        assert!(result.is_ok());
        let expected = base + chrono::Duration::days(7);
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn test_apply_duration_weeks() {
        let base = Utc::now();
        let result = apply_duration(base, "1w");
        assert!(result.is_ok());
        let expected = base + chrono::Duration::weeks(1);
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn test_apply_duration_months() {
        use chrono::{Months, NaiveDate};
        // Test month-aware addition: Jan 31 + 1 month = Feb 28/29
        let base = DateTime::<Utc>::from_naive_utc_and_offset(
            chrono::NaiveDateTime::new(
                NaiveDate::from_ymd_opt(2025, 1, 31).unwrap(),
                chrono::NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
            ),
            Utc,
        );
        let result = apply_duration(base, "1mo");
        assert!(result.is_ok());
        let dt = result.unwrap();
        let expected = base
            .checked_add_months(Months::new(1))
            .expect("month addition");
        assert_eq!(dt, expected);
        // Jan 31 + 1 month should be Feb 28 (2025 is not a leap year)
        assert_eq!(dt.day(), 28);
        assert_eq!(dt.month(), 2);
    }

    #[test]
    fn test_apply_duration_years() {
        use chrono::{Months, NaiveDate};
        let base = DateTime::<Utc>::from_naive_utc_and_offset(
            chrono::NaiveDateTime::new(
                NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(), // Leap year
                chrono::NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
            ),
            Utc,
        );
        let result = apply_duration(base, "1y");
        assert!(result.is_ok());
        let dt = result.unwrap();
        let expected = base
            .checked_add_months(Months::new(12))
            .expect("year addition");
        assert_eq!(dt, expected);
        // Feb 29, 2024 + 1 year = Feb 28, 2025 (not a leap year)
        assert_eq!(dt.day(), 28);
        assert_eq!(dt.month(), 2);
        assert_eq!(dt.year(), 2025);
    }

    #[test]
    fn test_reject_invalid_duration_format() {
        let base = Utc::now();
        let result = apply_duration(base, "5x");
        assert!(result.is_err(), "Should reject invalid unit");
        let result = apply_duration(base, "abc");
        assert!(result.is_err(), "Should reject invalid format");
        let result = apply_duration(base, "5");
        assert!(result.is_err(), "Should reject missing unit");
    }

    // ISO 8601 parsing tests

    #[test]
    fn test_parse_scheduled_for_valid() {
        let result = parse_scheduled_for("2025-12-01T09:00:00Z");
        assert!(result.is_ok(), "Should parse valid ISO 8601 timestamp");
        let dt = result.unwrap();
        assert_eq!(dt.year(), 2025);
        assert_eq!(dt.month(), 12);
        assert_eq!(dt.day(), 1);
        assert_eq!(dt.hour(), 9);
        assert_eq!(dt.minute(), 0);
    }

    #[test]
    fn test_parse_scheduled_for_with_timezone() {
        let result = parse_scheduled_for("2025-12-01T09:00:00-08:00");
        assert!(
            result.is_ok(),
            "Should parse ISO 8601 timestamp with timezone"
        );
        let dt = result.unwrap();
        // Should be converted to UTC (9 AM PST = 5 PM UTC)
        assert_eq!(dt.hour(), 17); // 9 AM + 8 hours = 5 PM UTC
    }

    #[test]
    fn test_reject_invalid_iso8601() {
        let result = parse_scheduled_for("2025-12-01");
        assert!(result.is_err(), "Should reject date without time");
        let result = parse_scheduled_for("invalid");
        assert!(result.is_err(), "Should reject invalid format");
        let result = parse_scheduled_for("2025-13-01T09:00:00Z");
        assert!(result.is_err(), "Should reject invalid month");
    }

    #[test]
    fn test_metadata_cached_in_validation() {
        // Test that is_loop_activity and is_back_edge are set correctly
        // Create a proper loop: init -> search -> evaluate -> search (back-edge)
        let mut definition = WorkflowDefinition {
            name: "test_loop".to_string(),
            activities: vec![
                ActivityDefinition {
                    key: "init".to_string(),
                    worker: "test".to_string(),
                    activity_name: Some("init".to_string()),
                    parameters: None,
                    depends_on: None,
                    dependency_of: Some(vec![ActivityRelationship {
                        activity_key: "search".to_string(),
                        conditions: None,
                        is_back_edge: false,
                    }]),
                    settings: None,
                    output_definitions: None,
                    iteration_scoped: false,
                    iteration_limit: None,
                    is_loop_activity: false,
                    streaming: Default::default(),
                },
                ActivityDefinition {
                    key: "search".to_string(),
                    worker: "test".to_string(),
                    activity_name: Some("search".to_string()),
                    parameters: None,
                    depends_on: Some(vec![
                        ActivityRelationship {
                            activity_key: "init".to_string(),
                            conditions: None,
                            is_back_edge: false,
                        },
                        ActivityRelationship {
                            activity_key: "evaluate".to_string(),
                            conditions: Some(vec!["{{evaluate.done == false}}".to_string()]),
                            is_back_edge: false,
                        },
                    ]),
                    dependency_of: None,
                    settings: None,
                    output_definitions: None,
                    iteration_scoped: true,
                    iteration_limit: Some(10),
                    is_loop_activity: false,
                    streaming: Default::default(),
                },
                ActivityDefinition {
                    key: "evaluate".to_string(),
                    worker: "test".to_string(),
                    activity_name: Some("evaluate".to_string()),
                    parameters: None,
                    depends_on: Some(vec![ActivityRelationship {
                        activity_key: "search".to_string(),
                        conditions: None,
                        is_back_edge: false,
                    }]),
                    dependency_of: None,
                    settings: None,
                    output_definitions: None,
                    iteration_scoped: true,
                    iteration_limit: None,
                    is_loop_activity: false,
                    streaming: Default::default(),
                },
            ],
        };

        let result = definition.validate();
        assert!(result.is_ok(), "Validation should succeed: {:?}", result);

        // Check that both loop activities are marked (not init)
        let search = definition
            .activities
            .iter()
            .find(|a| a.key == "search")
            .unwrap();
        let evaluate = definition
            .activities
            .iter()
            .find(|a| a.key == "evaluate")
            .unwrap();
        let init = definition
            .activities
            .iter()
            .find(|a| a.key == "init")
            .unwrap();

        assert!(
            search.is_loop_activity,
            "search should be marked as loop activity"
        );
        assert!(
            evaluate.is_loop_activity,
            "evaluate should be marked as loop activity"
        );
        assert!(
            !init.is_loop_activity,
            "init should NOT be marked as loop activity"
        );

        // Check that the back-edge is marked
        // The graph is: init -> search -> evaluate -> search (back-edge)
        // The back-edge is evaluate -> search, which is represented as search depends_on evaluate
        let search_depends_on = search.depends_on.as_ref().unwrap();
        let back_edge = search_depends_on
            .iter()
            .find(|d| d.activity_key == "evaluate")
            .unwrap();
        assert!(
            back_edge.is_back_edge,
            "Edge from evaluate to search (search depends_on evaluate) should be marked as back-edge"
        );

        // The forward edges (init->search and search->evaluate) should NOT be back-edges
        let search_depends_on = search.depends_on.as_ref().unwrap();
        let forward_edge_init = search_depends_on
            .iter()
            .find(|d| d.activity_key == "init")
            .unwrap();
        assert!(
            !forward_edge_init.is_back_edge,
            "Edge from init to search should NOT be marked as back-edge"
        );

        let evaluate_depends_on = evaluate.depends_on.as_ref().unwrap();
        let forward_edge_search = evaluate_depends_on
            .iter()
            .find(|d| d.activity_key == "search")
            .unwrap();
        assert!(
            !forward_edge_search.is_back_edge,
            "Edge from search to evaluate (evaluate depends_on search) should NOT be marked as back-edge"
        );
    }
}
