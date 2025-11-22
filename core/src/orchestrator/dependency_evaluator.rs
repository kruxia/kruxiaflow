use super::{OrchestratorError, Result, WorkflowActivityStatus, WorkflowState};
use crate::workflow::template::{TemplateContext, resolve_template_value};
use crate::workflow::{ActivityDefinition, ActivityRelationship, WorkflowDefinition};
use serde_json::Value;
use std::collections::HashMap;

/// Default maximum iterations if not specified
/// Provides safety bound for Pattern 2 (conditional-only) loops
const DEFAULT_MAX_ITERATIONS: u32 = 100;

/// Find activities that are ready to be scheduled
/// An activity is ready when:
/// - Status is NotScheduled (not already in queue) OR
/// - Status is Completed and should loop back
/// - All activities in `depends_on` list are Completed
/// - All conditions on `depends_on` relationships are satisfied
pub fn find_ready_activities<'a>(
    definition: &'a WorkflowDefinition,
    state: &WorkflowState,
) -> Result<Vec<&'a ActivityDefinition>> {
    let mut ready = Vec::new();

    for activity in &definition.activities {
        // Check if activity is ready (handles both first execution and loop-back)
        if is_activity_ready(activity, definition, state)? {
            ready.push(activity);
        }
    }

    Ok(ready)
}

/// Check if an activity is ready to be scheduled.
///
/// # Methodology
///
/// This function determines readiness through a **multi-phase evaluation process**:
///
/// ## Phase 1: Status Gate Check
/// Determines if the activity can be scheduled based on its current status:
/// - `NotScheduled` → Proceed to dependency evaluation (first execution)
/// - `Completed` → Check if should loop back (re-execution), otherwise reject
/// - `Running` | `Pending` → **Reject** (already in progress)
/// - `Failed` | `Skipped` → **Reject** (terminal states)
///
/// ## Phase 2: Loop Back Eligibility
/// For `Completed` activities marked with `is_loop_activity = true`:
/// - If iteration limit exceeded → **Reject**
/// - If back-edge conditions will be checked → Proceed to Phase 3
/// - Otherwise → **Reject** (loop is done)
///
/// ## Phase 3: Iteration Limit Enforcement
/// For all loop activities (regardless of status):
/// - Check `activity.iteration >= iteration_limit`
/// - Uses activity-level, settings-level, or `DEFAULT_MAX_ITERATIONS` (100)
/// - If exceeded → **Reject**
///
/// ## Phase 4: Dependency Evaluation
/// Evaluates all dependencies using **AND semantics** (all applicable dependencies must be satisfied):
///
/// ### 4a. Root Activities (No Dependencies)
/// - No `depends_on` relationships → **Accept** (always ready)
///
/// ### 4b. For Each Dependency:
///
/// #### Back-Edge Dependencies (`is_back_edge = true`):
/// Loop back-edges that create iterative execution:
/// - **Iteration 0**: Automatically satisfied (allows first loop entry)
/// - **Iteration 1+**: Evaluate loop conditions:
///   - If conditions exist → All must evaluate to `true`
///   - Dependency must be `Completed`
///   - If conditions fail → **Reject** (exit loop)
/// - Mark as applicable dependency (participates in AND gate)
///
/// #### Forward Dependencies (`is_back_edge = false`):
/// Standard sequential dependencies:
/// - **Terminal State Check**: Dependency must be `Completed` or `Failed`
///   - If not terminal → **Reject** (waiting for dependency)
/// - **Condition Evaluation** (if conditions specified):
///   - Build template context with all activity outputs
///   - Evaluate each condition expression via MiniJinja
///   - **All conditions must be true** → Mark applicable, continue
///   - **Any condition false** → Skip this dependency (not applicable)
/// - **No Conditions** (unconditional dependency):
///   - Mark as applicable
///   - Dependency must be `Completed` (not just `Failed`)
///   - If `Failed` → **Reject**
///
/// ### 4c. Applicable Dependency Check
/// After evaluating all dependencies:
/// - If activity has dependencies but **none are applicable** (all conditions false) → **Reject**
/// - This prevents orphaned activities from running when all paths are conditional
///
/// ## Phase 5: Final Decision
/// If all phases pass:
/// - All applicable dependencies are satisfied (AND semantics)
/// - No iteration limit exceeded
/// - Status allows scheduling
/// - → **Accept** (activity is ready)
///
/// # Loop Behavior
///
/// **First Execution** (iteration 0):
/// - Root activity or all forward dependencies satisfied → Schedule
/// - Back-edge dependencies automatically satisfied → Allows loop entry
///
/// **Subsequent Iterations** (iteration 1+):
/// - Check iteration limit
/// - Evaluate back-edge conditions
/// - If conditions pass → Loop back (re-schedule)
/// - If conditions fail or limit exceeded → Exit loop (don't schedule)
///
/// # Conditional Dependencies
///
/// **Condition Semantics**:
/// - Empty condition list → Dependency always applicable
/// - Non-empty conditions → **ALL** must evaluate to `true` (AND)
/// - Failed conditions → Dependency not applicable (doesn't block activity)
///
/// **Example**:
/// ```yaml
/// depends_on:
///   - activity_key: process_success
///     conditions: ["{{validate.passed == true}}"]  # Only if validation passed
///   - activity_key: process_failed
///     conditions: ["{{validate.passed == false}}"] # Only if validation failed
/// ```
/// If `validate.passed == true`: Only `process_success` is applicable
///
/// # Performance
///
/// - **Loop detection**: O(1) via precomputed `is_loop_activity` flag
/// - **Back-edge detection**: O(1) via precomputed `is_back_edge` flag
/// - **Dependency check**: O(D) where D = number of dependencies
/// - **Condition evaluation**: O(C) per dependency where C = number of conditions
///
/// Total: **O(D × C)** - Linear in dependency count, no graph traversal
fn is_activity_ready(
    activity: &ActivityDefinition,
    definition: &WorkflowDefinition,
    state: &WorkflowState,
) -> Result<bool> {
    let activity_state = state.activities.get(&activity.key);

    // Check current status
    if let Some(state) = activity_state {
        match state.status {
            WorkflowActivityStatus::Completed => {
                // Check if this is a loop that should continue
                if !should_loop_back(activity, state, definition, activity_state)? {
                    return Ok(false); // Already completed, don't re-execute
                }
                // Fall through to check loop conditions and iteration limits
            }
            WorkflowActivityStatus::Running | WorkflowActivityStatus::Pending => {
                return Ok(false); // Already scheduled/running
            }
            WorkflowActivityStatus::Failed | WorkflowActivityStatus::Skipped => {
                return Ok(false); // Terminal states
            }
            WorkflowActivityStatus::NotScheduled => {
                // Continue to dependency check
            }
        }
    }

    // Check iteration limit (for loop activities)
    if let Some(activity_state) = activity_state
        && is_max_iterations_exceeded(activity, activity_state)?
    {
        tracing::debug!(
            "Activity {} not ready: max iterations exceeded (iteration={})",
            activity.key,
            activity_state.iteration
        );
        return Ok(false);
    }

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
            .map(|rel| rel.activity_key.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    // Check all dependencies are satisfied (ALL must be satisfied - AND semantics)
    // Track if we found at least one dependency that needs to be satisfied
    let mut found_applicable_dependency = false;

    for dep_rel in &dependencies {
        let dependency_state = state
            .activities
            .get(&dep_rel.activity_key)
            .ok_or_else(|| OrchestratorError::ActivityNotFound(dep_rel.activity_key.clone()))?;

        // Check if this is a back-edge (loop) - uses precomputed metadata
        if dep_rel.is_back_edge {
            tracing::trace!(
                "Activity {} has back-edge dependency on {}",
                activity.key,
                dep_rel.activity_key
            );

            // For back-edges, evaluate loop condition
            // Activity state should always exist if we're evaluating its dependencies
            let current_activity_state = activity_state
                .ok_or_else(|| OrchestratorError::ActivityNotFound(activity.key.clone()))?;

            if !evaluate_loop_condition(dep_rel, dependency_state, state, current_activity_state)? {
                tracing::trace!(
                    "Activity {} not ready: back-edge condition not met for {}",
                    activity.key,
                    dep_rel.activity_key
                );
                return Ok(false); // Loop condition not met
            }

            // Back-edge condition satisfied
            found_applicable_dependency = true;
        } else {
            // Standard forward dependency check

            // Check if dependency is in terminal state FIRST
            // (must check this before evaluating conditions since conditions may reference outputs)
            if !matches!(
                dependency_state.status,
                WorkflowActivityStatus::Completed | WorkflowActivityStatus::Failed
            ) {
                tracing::trace!(
                    "Activity {} not ready: dependency {} is in state {:?}",
                    activity.key,
                    dep_rel.activity_key,
                    dependency_state.status
                );
                return Ok(false); // Dependency not in terminal state yet
            }

            // Now check conditions (if any)
            if let Some(condition_list) = &dep_rel.conditions {
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
                            dep_rel.activity_key,
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
                        dep_rel.activity_key,
                        dependency_state.status
                    );
                    return Ok(false); // Only run dependent activities on success
                }
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

/// Check if a completed activity should loop back (re-execute)
fn should_loop_back(
    activity: &ActivityDefinition,
    _activity_state: &super::workflow_state::ActivityState,
    _definition: &WorkflowDefinition,
    state: Option<&super::workflow_state::ActivityState>,
) -> Result<bool> {
    // Only loop if activity is marked as part of a loop (precomputed during validation)
    if !activity.is_loop_activity {
        return Ok(false);
    }

    // If we have a back-edge dependency that's satisfied, we should loop
    // This will be determined in the dependency check
    // For now, return true if it's a loop activity - the actual condition
    // will be evaluated in is_activity_ready

    // Check if activity state exists and is completed
    if let Some(s) = state {
        if s.status == WorkflowActivityStatus::Completed {
            return Ok(true);
        }
    }

    Ok(false)
}

/// Check if max iterations has been exceeded
fn is_max_iterations_exceeded(
    activity: &ActivityDefinition,
    activity_state: &super::workflow_state::ActivityState,
) -> Result<bool> {
    // Check activity-level iteration_limit
    let iteration_limit = activity
        .iteration_limit
        .or_else(|| {
            // Check settings-level iteration_limit
            activity.settings.as_ref().and_then(|s| s.iteration_limit)
        })
        .unwrap_or(DEFAULT_MAX_ITERATIONS); // Safety bound for conditional-only loops

    Ok(activity_state.iteration >= iteration_limit)
}

/// Evaluate loop exit/continuation condition
fn evaluate_loop_condition(
    dep: &ActivityRelationship,
    dep_state: &super::workflow_state::ActivityState,
    state: &WorkflowState,
    activity_state: &super::workflow_state::ActivityState,
) -> Result<bool> {
    // For the first iteration (iteration 0), back-edge dependencies are automatically satisfied
    // This allows the activity to enter the loop for the first time
    if activity_state.iteration == 0 {
        tracing::trace!(
            "Back-edge dependency satisfied for {} (first iteration)",
            activity_state.key
        );
        return Ok(true);
    }

    // For subsequent iterations, check if dependency has completed at least once
    // We need the dependency to have run before we can evaluate conditions that reference its outputs
    if dep_state.status != WorkflowActivityStatus::Completed && dep_state.iteration == 0 {
        // Dependency hasn't run yet in this loop cycle, can't loop back
        tracing::trace!(
            "Back-edge dependency {} not completed yet, can't loop back",
            dep_state.key
        );
        return Ok(false);
    }

    // For subsequent iterations, evaluate loop conditions
    // Loop conditions should evaluate to true to continue looping
    // Exit conditions should evaluate to false to stop looping

    if let Some(conditions) = &dep.conditions {
        let context = build_condition_context(state);

        for condition in conditions {
            if !evaluate_condition(condition, &context)? {
                return Ok(false); // Condition not met, don't loop
            }
        }
    }

    // Check if dependency is in Completed state (required for loop-back)
    if dep_state.status != WorkflowActivityStatus::Completed {
        return Ok(false);
    }

    Ok(true) // All conditions met, continue loop
}

/// Get list of dependencies (activities this activity depends on)
/// After normalization, only depends_on is populated, so this is a simple extraction
/// Returns references to ActivityRelationship objects that contain is_back_edge metadata
fn get_dependencies(activity: &ActivityDefinition) -> Vec<&ActivityRelationship> {
    // After normalization, only depends_on is populated
    if let Some(depends_on_list) = &activity.depends_on {
        depends_on_list.iter().collect()
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

    // Add activity outputs and iteration outputs
    for (activity_key, activity_state) in &state.activities {
        // Add all activities to context, even if they don't have outputs yet
        // This ensures iteration-scoped activities are always available as arrays
        let outputs = activity_state.outputs.clone().unwrap_or_default();
        context.add_activity_state(
            activity_key.clone(),
            outputs,
            activity_state.iteration_outputs.clone(),
            activity_state.iteration,
            activity_state.accumulated_cost_usd,
        );
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
        for dep_rel in &dependencies {
            if let Some(dependency_state) = state.activities.get(&dep_rel.activity_key) {
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
