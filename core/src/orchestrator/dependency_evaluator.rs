use super::{OrchestratorError, Result, WorkflowActivityStatus, WorkflowState};
use crate::events::{ActivityDefinition, WorkflowDefinition};

/// Find activities that are ready to be scheduled
/// An activity is ready when:
/// - Status is NotScheduled (not already in queue)
/// - All activities in `preceding` list are Completed
/// - All conditions on `preceding` relationships are satisfied
pub fn find_ready_activities<'a>(
    definition: &'a WorkflowDefinition,
    state: &WorkflowState,
) -> Result<Vec<&'a ActivityDefinition>> {
    let mut ready = Vec::new();

    for activity in &definition.activities {
        // Skip if already scheduled/completed/failed
        if let Some(activity_state) = state.activities.get(&activity.key) {
            if activity_state.status != WorkflowActivityStatus::NotScheduled {
                continue;
            }
        }

        // Check if all dependencies satisfied
        if is_activity_ready(activity, definition, state)? {
            ready.push(activity);
        }
    }

    Ok(ready)
}

/// Check if an activity is ready to be scheduled
fn is_activity_ready(
    activity: &ActivityDefinition,
    definition: &WorkflowDefinition,
    state: &WorkflowState,
) -> Result<bool> {
    // Get list of preceding activities from definition
    let preceding_keys = get_preceding_activities(activity, definition);

    // If no preceding activities, it's a root activity (always ready initially)
    if preceding_keys.is_empty() {
        return Ok(true);
    }

    // Check all preceding activities are in terminal state (Completed or Failed)
    for (preceding_key, conditions) in &preceding_keys {
        let preceding_state = state
            .activities
            .get(preceding_key)
            .ok_or_else(|| OrchestratorError::ActivityNotFound(preceding_key.clone()))?;

        // Preceding activity must be in terminal state before we can proceed
        if !matches!(
            preceding_state.status,
            WorkflowActivityStatus::Completed | WorkflowActivityStatus::Failed
        ) {
            return Ok(false); // Dependency not in terminal state yet
        }

        // Check conditions on this relationship
        if let Some(condition_list) = conditions {
            // Explicit conditions - evaluate them (can handle Failed case)
            for condition in condition_list {
                if !evaluate_condition(condition, state)? {
                    return Ok(false); // Condition not satisfied
                }
            }
        } else {
            // No explicit conditions - default to success path only
            if preceding_state.status != WorkflowActivityStatus::Completed {
                return Ok(false); // Only run following activities on success
            }
        }
    }

    Ok(true)
}

/// Get list of preceding activities with their conditions
fn get_preceding_activities(
    activity: &ActivityDefinition,
    definition: &WorkflowDefinition,
) -> Vec<(String, Option<Vec<String>>)> {
    let mut preceding = Vec::new();

    // Check explicit `preceding` list
    if let Some(preceding_list) = &activity.preceding {
        for item in preceding_list {
            preceding.push((item.activity_key.clone(), item.conditions.clone()));
        }
    }

    // Check if other activities list this one in `following`
    for other_activity in &definition.activities {
        if let Some(following_list) = &other_activity.following {
            for item in following_list {
                if item.activity_key == activity.key {
                    preceding.push((other_activity.key.clone(), item.conditions.clone()));
                }
            }
        }
    }

    preceding
}

/// Evaluate a condition expression
/// For MVP: Simple string-based evaluation
/// Supports: {{activity.field}} template substitution and == comparison
pub fn evaluate_condition(condition: &str, state: &WorkflowState) -> Result<bool> {
    // Resolve template variables like {{activity.field}}
    let resolved = resolve_template_variables(condition, state)?;
    let resolved = resolved.trim();

    // Handle boolean literals
    if resolved == "true" {
        return Ok(true);
    }
    if resolved == "false" {
        return Ok(false);
    }

    // Handle == comparisons
    if resolved.contains("==") {
        let parts: Vec<&str> = resolved.split("==").collect();
        if parts.len() == 2 {
            let left = parts[0].trim();
            let right = parts[1].trim();
            return Ok(left == right);
        }
    }

    // Handle != comparisons
    if resolved.contains("!=") {
        let parts: Vec<&str> = resolved.split("!=").collect();
        if parts.len() == 2 {
            let left = parts[0].trim();
            let right = parts[1].trim();
            return Ok(left != right);
        }
    }

    // Default: treat non-empty string as true
    Ok(!resolved.is_empty())
}

/// Resolve template variables like {{activity.field}} with actual values
fn resolve_template_variables(template: &str, state: &WorkflowState) -> Result<String> {
    let mut result = template.to_string();

    // Find and replace all {{activity.field}} patterns
    for (activity_key, activity_state) in &state.activities {
        if let Some(outputs) = &activity_state.outputs {
            // Replace {{activity_key.field}} with outputs[field]
            if let Some(obj) = outputs.as_object() {
                for (field, value) in obj {
                    let pattern = format!("{{{{{}.{}}}}}", activity_key, field);
                    let replacement = match value {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Bool(b) => b.to_string(),
                        serde_json::Value::Number(n) => n.to_string(),
                        _ => value.to_string().trim_matches('"').to_string(),
                    };
                    result = result.replace(&pattern, &replacement);
                }
            }
        }
    }

    Ok(result)
}

/// Check if workflow is complete (all activities in terminal state)
pub fn is_workflow_complete(state: &WorkflowState) -> bool {
    state.activities.values().all(|activity| {
        matches!(
            activity.status,
            WorkflowActivityStatus::Completed | WorkflowActivityStatus::Failed
        )
    })
}

/// Check if workflow has failed (any activity permanently failed)
pub fn is_workflow_failed(state: &WorkflowState) -> bool {
    state
        .activities
        .values()
        .any(|activity| matches!(activity.status, WorkflowActivityStatus::Failed))
}
