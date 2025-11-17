use super::{OrchestratorError, Result, WorkflowActivityStatus, WorkflowState};
use crate::events::{ActivityDefinition, WorkflowDefinition};
use crate::workflow::template::{TemplateContext, resolve_template_value};
use serde_json::Value;
use std::collections::HashMap;

/// Find activities that are ready to be scheduled
/// An activity is ready when:
/// - Status is NotScheduled (not already in queue)
/// - All activities in `depends_on` list are Completed
/// - All conditions on `depends_on` relationships are satisfied
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
    _definition: &WorkflowDefinition,
    state: &WorkflowState,
) -> Result<bool> {
    // Get list of dependencies from definition
    let dependencies = get_dependencies(activity);

    // If no dependencies, it's a root activity (always ready initially)
    if dependencies.is_empty() {
        tracing::trace!(
            "Activity {} is a root activity (no dependencies)",
            activity.key
        );
        return Ok(true);
    }

    tracing::trace!(
        "Checking {} dependencies for activity {}: [{}]",
        dependencies.len(),
        activity.key,
        dependencies
            .iter()
            .map(|(k, _)| k.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    // Check all dependencies are satisfied (ALL must be satisfied - AND semantics)
    // Track if we found at least one dependency that needs to be satisfied
    let mut found_applicable_dependency = false;

    for (dependency_key, conditions) in &dependencies {
        let dependency_state = state
            .activities
            .get(dependency_key)
            .ok_or_else(|| OrchestratorError::ActivityNotFound(dependency_key.clone()))?;

        // Check if dependency is in terminal state FIRST
        // (must check this before evaluating conditions since conditions may reference outputs)
        if !matches!(
            dependency_state.status,
            WorkflowActivityStatus::Completed | WorkflowActivityStatus::Failed
        ) {
            tracing::trace!(
                "Activity {} not ready: dependency {} is in state {:?}",
                activity.key,
                dependency_key,
                dependency_state.status
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
                        dependency_key,
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

            // Dependency activity must be Completed (not just Failed)
            if dependency_state.status != WorkflowActivityStatus::Completed {
                tracing::trace!(
                    "Activity {} not ready: dependency {} is {:?} (expected Completed)",
                    activity.key,
                    dependency_key,
                    dependency_state.status
                );
                return Ok(false); // Only run dependent activities on success
            }
        }
    }

    // If activity has dependencies but none were applicable (all conditions false),
    // then this activity should not be scheduled
    if !dependencies.is_empty() && !found_applicable_dependency {
        tracing::trace!(
            "Activity {} not ready: has {} dependencies but none have satisfied conditions",
            activity.key,
            dependencies.len()
        );
        return Ok(false);
    }

    tracing::trace!(
        "Activity {} is ready: all applicable dependencies satisfied",
        activity.key
    );
    Ok(true)
}

/// Get list of dependencies (activities this activity depends on) with their conditions
/// After normalization, only depends_on is populated, so this is a simple extraction
fn get_dependencies(activity: &ActivityDefinition) -> Vec<(String, Option<Vec<String>>)> {
    // After normalization, only depends_on is populated
    if let Some(depends_on_list) = &activity.depends_on {
        depends_on_list
            .iter()
            .map(|item| (item.activity_key.clone(), item.conditions.clone()))
            .collect()
    } else {
        Vec::new()
    }
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
///
/// A workflow is complete when all activities have reached a terminal state:
/// - Completed: Successfully executed
/// - Failed: Permanently failed
/// - Skipped: Not executed due to unsatisfied conditional dependencies
pub fn is_workflow_complete(state: &WorkflowState) -> bool {
    state.activities.values().all(|activity| {
        matches!(
            activity.status,
            WorkflowActivityStatus::Completed
                | WorkflowActivityStatus::Failed
                | WorkflowActivityStatus::Skipped
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

/// Find activities that should be marked as Skipped
///
/// An activity should be skipped when:
/// - Status is NotScheduled (not yet processed)
/// - All dependencies are in terminal states (Completed, Failed, or Skipped)
/// - No applicable dependencies exist (all conditional dependencies have false conditions)
pub fn find_skipped_activities<'a>(
    definition: &'a WorkflowDefinition,
    state: &WorkflowState,
) -> Result<Vec<&'a ActivityDefinition>> {
    let mut skipped = Vec::new();

    for activity in &definition.activities {
        // Only consider NotScheduled activities
        if let Some(activity_state) = state.activities.get(&activity.key) {
            if activity_state.status != WorkflowActivityStatus::NotScheduled {
                continue;
            }
        }

        // Get dependencies
        let dependencies = get_dependencies(activity);

        // If no dependencies, it's a root activity and should not be skipped
        if dependencies.is_empty() {
            continue;
        }

        // Check if all dependencies are in terminal states
        let mut all_dependencies_terminal = true;
        for (dependency_key, _) in &dependencies {
            if let Some(dependency_state) = state.activities.get(dependency_key) {
                if !matches!(
                    dependency_state.status,
                    WorkflowActivityStatus::Completed
                        | WorkflowActivityStatus::Failed
                        | WorkflowActivityStatus::Skipped
                ) {
                    all_dependencies_terminal = false;
                    break;
                }
            }
        }

        if !all_dependencies_terminal {
            continue;
        }

        // Check if activity is ready (has applicable dependencies)
        // If not ready and all dependencies are terminal, it should be skipped
        if !is_activity_ready(activity, definition, state)? {
            skipped.push(activity);
        }
    }

    Ok(skipped)
}
