use super::{OrchestratorError, Result, WorkflowActivityStatus, WorkflowState};
use crate::events::{ActivityDefinition, WorkflowDefinition};
use crate::workflow::template::{TemplateContext, resolve_template_value};
use serde_json::Value;
use std::collections::HashMap;

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
        tracing::trace!(
            "Activity {} is a root activity (no dependencies)",
            activity.key
        );
        return Ok(true);
    }

    tracing::trace!(
        "Checking {} dependencies for activity {}: [{}]",
        preceding_keys.len(),
        activity.key,
        preceding_keys
            .iter()
            .map(|(k, _)| k.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    // Check all preceding activities are satisfied (ALL must be satisfied - AND semantics)
    // Track if we found at least one dependency that needs to be satisfied
    let mut found_applicable_dependency = false;

    for (preceding_key, conditions) in &preceding_keys {
        let preceding_state = state
            .activities
            .get(preceding_key)
            .ok_or_else(|| OrchestratorError::ActivityNotFound(preceding_key.clone()))?;

        // Check if dependency is in terminal state FIRST
        // (must check this before evaluating conditions since conditions may reference outputs)
        if !matches!(
            preceding_state.status,
            WorkflowActivityStatus::Completed | WorkflowActivityStatus::Failed
        ) {
            tracing::trace!(
                "Activity {} not ready: dependency {} is in state {:?}",
                activity.key,
                preceding_key,
                preceding_state.status
            );
            return Ok(false); // Dependency not in terminal state yet
        }

        // Now check conditions (if any)
        if let Some(condition_list) = conditions {
            // Build template context for condition evaluation
            let context = build_condition_context(state);

            // Check if conditions are satisfied
            let mut conditions_satisfied = true;
            for condition in condition_list {
                if !evaluate_condition(condition, &context)? {
                    conditions_satisfied = false;
                    tracing::trace!(
                        "Activity {} dependency {} skipped: condition '{}' not satisfied",
                        activity.key,
                        preceding_key,
                        condition
                    );
                    break;
                }
            }

            // If conditions are not satisfied, skip this dependency entirely
            if !conditions_satisfied {
                continue;
            }

            // Conditions are satisfied, so this dependency is applicable
            found_applicable_dependency = true;
        } else {
            // No explicit conditions - this dependency is always applicable
            found_applicable_dependency = true;

            // Preceding activity must be Completed (not just Failed)
            if preceding_state.status != WorkflowActivityStatus::Completed {
                tracing::trace!(
                    "Activity {} not ready: dependency {} is {:?} (expected Completed)",
                    activity.key,
                    preceding_key,
                    preceding_state.status
                );
                return Ok(false); // Only run following activities on success
            }
        }
    }

    // If activity has dependencies but none were applicable (all conditions false),
    // then this activity should not be scheduled
    if !preceding_keys.is_empty() && !found_applicable_dependency {
        tracing::trace!(
            "Activity {} not ready: has {} dependencies but none have satisfied conditions",
            activity.key,
            preceding_keys.len()
        );
        return Ok(false);
    }

    tracing::trace!(
        "Activity {} is ready: all applicable dependencies satisfied",
        activity.key
    );
    Ok(true)
}

/// Get list of preceding activities with their conditions
/// Deduplicates entries to handle cases where both `preceding` and `following` are defined
fn get_preceding_activities(
    activity: &ActivityDefinition,
    definition: &WorkflowDefinition,
) -> Vec<(String, Option<Vec<String>>)> {
    use std::collections::HashMap;

    // Use HashMap to track unique preceding activities by key
    // If same activity appears twice, keep first occurrence (explicit `preceding` takes priority)
    let mut preceding_map: HashMap<String, Option<Vec<String>>> = HashMap::new();

    // Check explicit `preceding` list (higher priority)
    if let Some(preceding_list) = &activity.preceding {
        for item in preceding_list {
            preceding_map.insert(item.activity_key.clone(), item.conditions.clone());
        }
    }

    // Check if other activities list this one in `following` (only add if not already present)
    for other_activity in &definition.activities {
        if let Some(following_list) = &other_activity.following {
            for item in following_list {
                if item.activity_key == activity.key {
                    // Only insert if not already present (explicit `preceding` takes priority)
                    preceding_map
                        .entry(other_activity.key.clone())
                        .or_insert_with(|| item.conditions.clone());
                }
            }
        }
    }

    // Convert HashMap back to Vec
    preceding_map.into_iter().collect()
}

/// Build template context from workflow state for condition evaluation
pub fn build_condition_context(state: &WorkflowState) -> TemplateContext {
    let mut context = TemplateContext::new();

    // Add workflow inputs from state_data
    if let Value::Object(state_obj) = &state.state_data {
        let inputs: HashMap<String, Value> = state_obj
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        context = context.with_inputs(inputs);
    }

    // Add activity outputs
    for (activity_key, activity_state) in &state.activities {
        if let Some(outputs) = &activity_state.outputs {
            if let Value::Object(outputs_obj) = outputs {
                let outputs_map: HashMap<String, Value> = outputs_obj
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                context.add_activity_output(activity_key.clone(), outputs_map);
            }
        }
    }

    context
}

/// Evaluate a condition expression using MiniJinja template engine
/// Supports full expression syntax: ==, !=, >, <, AND, OR, etc.
/// Example: "{{check_email.valid == true}}"
pub fn evaluate_condition(condition: &str, context: &TemplateContext) -> Result<bool> {
    // Resolve the condition expression as a template
    let condition_value = Value::String(condition.to_string());

    match resolve_template_value(&condition_value, context) {
        Ok(resolved) => {
            // The result should be a boolean
            match resolved {
                Value::Bool(b) => Ok(b),
                Value::String(s) => {
                    // Handle string boolean representations
                    let s_lower = s.to_lowercase();
                    if s_lower == "true" {
                        Ok(true)
                    } else if s_lower == "false" {
                        Ok(false)
                    } else {
                        // Non-empty string is truthy
                        Ok(!s.is_empty())
                    }
                }
                Value::Number(n) => {
                    // Non-zero is truthy
                    Ok(n.as_f64().map(|f| f != 0.0).unwrap_or(false))
                }
                Value::Null => Ok(false),
                _ => {
                    // Arrays/objects are truthy if non-empty
                    Ok(true)
                }
            }
        }
        Err(e) => Err(OrchestratorError::TemplateFailed(format!(
            "Failed to evaluate condition '{}': {}",
            condition, e
        ))),
    }
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
