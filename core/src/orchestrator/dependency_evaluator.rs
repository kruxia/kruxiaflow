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

/// Activities ready to schedule, and activities whose conditions could not
/// be evaluated (with the error)
pub type ReadyOrUnresolvable<'a> = (
    Vec<&'a ActivityDefinition>,
    Vec<(&'a ActivityDefinition, String)>,
);

/// Like [`find_ready_activities`], but an activity whose dependency
/// conditions cannot be evaluated (a template error, such as a reference to
/// an undefined value) is returned separately with the error, instead of
/// failing the whole evaluation (#19). The orchestrator fails that activity;
/// a deterministic template error must not fail the event being processed.
pub fn find_ready_or_unresolvable_activities<'a>(
    definition: &'a WorkflowDefinition,
    state: &WorkflowState,
) -> Result<ReadyOrUnresolvable<'a>> {
    let mut ready = Vec::new();
    let mut unresolvable = Vec::new();
    for activity in &definition.activities {
        match is_activity_ready(activity, definition, state) {
            Ok(true) => ready.push(activity),
            Ok(false) => {}
            Err(OrchestratorError::TemplateFailed(error)) => unresolvable.push((activity, error)),
            Err(other) => return Err(other),
        }
    }
    Ok((ready, unresolvable))
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
/// - If no dependency completed since this activity last did (no new input,
///   see [`has_new_input`]; a self back-edge counts) → **Reject** (#18)
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
/// ### 4b. Every Forward Dependency Terminal
/// - Any forward dependency not `Completed`, `Failed` or `Skipped` →
///   **Reject** (waiting for it), before any condition is evaluated. A
///   condition may read any dependency's result, so the order dependencies
///   complete in must not decide whether it can be evaluated.
///
/// ### 4c. For Each Dependency:
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
/// Standard sequential dependencies (all terminal, see 4b):
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
/// ### 4d. Applicable Dependency Check
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
    if let Some(current) = activity_state {
        match current.status {
            WorkflowActivityStatus::Completed => {
                // Check if this is a loop that should continue
                if !should_loop_back(activity, current, definition, activity_state)? {
                    return Ok(false); // Already completed, don't re-execute
                }
                // Run again only on new input: a dependency that is still
                // Completed from an earlier round must not re-trigger it
                if !has_new_input(activity, current, state) {
                    tracing::trace!(
                        "Activity {} not ready: no dependency completed since it last ran",
                        activity.key
                    );
                    return Ok(false);
                }
                // Fall through to check loop conditions and iteration limits
            }
            WorkflowActivityStatus::Running
            | WorkflowActivityStatus::Pending
            | WorkflowActivityStatus::Waiting => {
                return Ok(false); // Already scheduled/running/waiting
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

    // Every forward dependency must be terminal (Completed, Failed or
    // Skipped) before any condition is evaluated: a condition may read
    // another dependency's result, which a dependency still running doesn't
    // have yet
    for dep_rel in dependencies.iter().filter(|dep| !dep.is_back_edge) {
        let dependency_state = state
            .activities
            .get(&dep_rel.activity_key)
            .ok_or_else(|| OrchestratorError::ActivityNotFound(dep_rel.activity_key.clone()))?;
        if !matches!(
            dependency_state.status,
            WorkflowActivityStatus::Completed
                | WorkflowActivityStatus::Failed
                | WorkflowActivityStatus::Skipped
        ) {
            tracing::trace!(
                "Activity {} not ready: dependency {} is in state {:?}",
                activity.key,
                dep_rel.activity_key,
                dependency_state.status
            );
            return Ok(false);
        }
    }

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
            // Standard forward dependency check (terminal, checked above)

            // Check conditions (if any)
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
    if let Some(s) = state
        && s.status == WorkflowActivityStatus::Completed
    {
        return Ok(true);
    }

    Ok(false)
}

/// Whether a completed loop activity has new input to run on: at least one of
/// its dependencies completed after it last did (#18).
///
/// A dependency still `Completed` from an earlier round is not new input, so a
/// loop member never runs twice on the same input.
///
/// - A self back-edge (`depends_on` itself) counts as new input: the
///   activity's own completion is what it loops on.
/// - Any dependency that completed after this activity counts, wherever it
///   is. A dependency that completes only once, before the loop starts (an
///   `initialize` step), never counts as new, but doesn't block either; the
///   back-edge or an in-loop dependency does. A step of an earlier loop
///   completes again each round of that loop, so it does count.
/// - `completion_seq == 0` carries no ordering (state saved without it), so
///   it counts as new input rather than stalling the activity.
fn has_new_input(
    activity: &ActivityDefinition,
    activity_state: &super::workflow_state::ActivityState,
    state: &WorkflowState,
) -> bool {
    if activity_state.completion_seq == 0 {
        return true;
    }
    get_dependencies(activity).iter().any(|dep| {
        dep.activity_key == activity.key
            || state.activities.get(&dep.activity_key).is_some_and(|d| {
                d.status == WorkflowActivityStatus::Completed
                    && d.completion_seq > activity_state.completion_seq
            })
    })
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

    // All conditions met: loop back only if the dependency has completed
    Ok(dep_state.status == WorkflowActivityStatus::Completed)
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

/// Convert WorkflowActivityStatus to snake_case string for template context
pub fn status_to_string(status: WorkflowActivityStatus) -> String {
    match status {
        WorkflowActivityStatus::NotScheduled => "not_scheduled".to_string(),
        WorkflowActivityStatus::Waiting => "waiting".to_string(),
        WorkflowActivityStatus::Pending => "pending".to_string(),
        WorkflowActivityStatus::Running => "running".to_string(),
        WorkflowActivityStatus::Completed => "completed".to_string(),
        WorkflowActivityStatus::Failed => "failed".to_string(),
        WorkflowActivityStatus::Skipped => "skipped".to_string(),
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

    // Add activity outputs, iteration outputs, and status
    for (activity_key, activity_state) in &state.activities {
        // Add all activities to context, even if they don't have outputs yet
        // This ensures iteration-scoped activities are always available as arrays
        // and status is always accessible for conditional dependencies
        let outputs = activity_state.outputs.clone().unwrap_or_default();
        context.add_activity_state(
            activity_key.clone(),
            outputs,
            activity_state.iteration_outputs.clone(),
            activity_state.iteration,
            activity_state.accumulated_cost_usd,
            status_to_string(activity_state.status),
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
/// - It is not an activity after a loop whose loop is still running: one that
///   depends on a loop member without being one waits until no loop member is
///   scheduled, running or waiting (#27)
pub fn find_skipped_activities<'a>(
    definition: &'a WorkflowDefinition,
    state: &WorkflowState,
) -> Result<Vec<&'a ActivityDefinition>> {
    Ok(find_skipped_or_unresolvable_activities(definition, state)?.0)
}

/// Activities to skip, and activities whose dependency conditions could not
/// be evaluated (with the error)
pub type SkippedOrUnresolvable<'a> = ReadyOrUnresolvable<'a>;

/// Like [`find_skipped_activities`], but an activity whose dependencies are
/// all terminal and whose conditions cannot be evaluated is returned
/// separately with the error, so the orchestrator can fail it (#19). A
/// condition can first be evaluated here: when its dependency was itself
/// skipped earlier in the same skip pass, such as a condition on a skipped
/// step's result. Left out, the activity stays NotScheduled with nothing to
/// wake it, and the workflow hangs.
///
/// Skipping one activity can decide others (what depends on it); the
/// orchestrator calls this again after marking them, until it finds none.
pub fn find_skipped_or_unresolvable_activities<'a>(
    definition: &'a WorkflowDefinition,
    state: &WorkflowState,
) -> Result<SkippedOrUnresolvable<'a>> {
    let mut skipped = Vec::new();
    let mut unresolvable = Vec::new();
    // Computed at most once, only when an activity after a loop would be skipped
    let mut loop_running: Option<bool> = None;

    for activity in &definition.activities {
        // Only consider NotScheduled activities
        if let Some(activity_state) = state.activities.get(&activity.key)
            && activity_state.status != WorkflowActivityStatus::NotScheduled
        {
            continue;
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
            if let Some(dependency_state) = state.activities.get(&dep_rel.activity_key)
                && !matches!(
                    dependency_state.status,
                    WorkflowActivityStatus::Completed
                        | WorkflowActivityStatus::Failed
                        | WorkflowActivityStatus::Skipped
                )
            {
                all_dependencies_terminal = false;
                break;
            }
        }

        if !all_dependencies_terminal {
            continue;
        }

        // Check if activity is ready (has applicable dependencies)
        // If not ready and all dependencies are terminal, it should be skipped.
        // An activity whose conditions can't be evaluated is not skippable:
        // it is returned to be failed instead
        match is_activity_ready(activity, definition, state) {
            Ok(false) => {}
            Ok(true) => continue,
            Err(OrchestratorError::TemplateFailed(error)) => {
                unresolvable.push((activity, error));
                continue;
            }
            Err(other) => return Err(other),
        }

        // An activity after a loop (it depends on a loop member but isn't one,
        // like the loop's exit step) waits while the loop still runs: a later
        // round completes that dependency again and may satisfy its
        // condition. Skipped is terminal, so skipping it now would be for good
        // (#27). Once no loop member is scheduled, running or waiting, the
        // loop is over and the activity is skipped as usual.
        if !activity.is_loop_activity
            && dependencies
                .iter()
                .any(|dep| is_loop_member(definition, &dep.activity_key))
            && *loop_running.get_or_insert_with(|| is_a_loop_running(definition, state))
        {
            tracing::trace!(
                "Activity {} not skipped yet: a loop it depends on is still running",
                activity.key
            );
            continue;
        }

        skipped.push(activity);
    }

    Ok((skipped, unresolvable))
}

/// Whether `key` names a loop member (`is_loop_activity`, set on every
/// activity of a cycle by validation and on load)
fn is_loop_member(definition: &WorkflowDefinition, key: &str) -> bool {
    definition
        .activities
        .iter()
        .any(|a| a.key == key && a.is_loop_activity)
}

/// Whether any loop member is scheduled, running or waiting: the loop can
/// still go another round. The orchestrator schedules the next step of a
/// loop in the same pass that handles the previous step's completion, before
/// it looks for activities to skip, so a loop that goes on always has a
/// member in one of these states, and one that has ended has none.
fn is_a_loop_running(definition: &WorkflowDefinition, state: &WorkflowState) -> bool {
    definition.activities.iter().any(|a| {
        a.is_loop_activity
            && state.activities.get(&a.key).is_some_and(|s| {
                matches!(
                    s.status,
                    WorkflowActivityStatus::Pending
                        | WorkflowActivityStatus::Running
                        | WorkflowActivityStatus::Waiting
                )
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::{ActivityDefinition, ActivityRelationship, StreamingConfig};
    use rust_decimal::Decimal;

    /// Helper to create a minimal ActivityState
    fn make_activity_state(
        key: &str,
        status: WorkflowActivityStatus,
    ) -> (String, super::super::workflow_state::ActivityState) {
        (
            key.to_string(),
            super::super::workflow_state::ActivityState {
                key: key.to_string(),
                status,
                outputs: None,
                error: None,
                started_at: None,
                completed_at: None,
                attempt: 1,
                last_error: None,
                accumulated_cost_usd: Decimal::ZERO,
                iteration: 0,
                iteration_outputs: None,
                signal_data: None,
                completion_seq: 0,
            },
        )
    }

    /// Helper to create a WorkflowState with given activities
    fn make_workflow_state(
        activities: Vec<(String, super::super::workflow_state::ActivityState)>,
    ) -> WorkflowState {
        WorkflowState {
            workflow_id: uuid::Uuid::now_v7(),
            definition_name: "test".to_string(),
            status: crate::events::WorkflowStatus::Running,
            activities: activities.into_iter().collect(),
            state_data: serde_json::json!({}),
            input: serde_json::json!({}),
            error_message: None,
        }
    }

    /// Helper to create a minimal ActivityDefinition
    fn make_activity_def(
        key: &str,
        depends_on: Option<Vec<ActivityRelationship>>,
    ) -> ActivityDefinition {
        ActivityDefinition {
            key: key.to_string(),
            worker: "test".to_string(),
            activity_name: None,
            parameters: None,
            output_definitions: None,
            depends_on,
            dependency_of: None,
            settings: None,
            iteration_scoped: false,
            iteration_limit: None,
            is_loop_activity: false,
            streaming: StreamingConfig::default(),
        }
    }

    // ============================================================================
    // Status-Based Conditional Dependency Tests
    // Regression tests for: docs/bugs/2026-01-10-orchestrator-logic.md
    // ============================================================================

    #[test]
    fn test_status_condition_completed_activity() {
        // When activity status is 'completed', condition {{activity.status == 'completed'}} is true
        let activities = vec![make_activity_state(
            "dep_activity",
            WorkflowActivityStatus::Completed,
        )];
        let state = make_workflow_state(activities);
        let context = build_condition_context(&state);

        let result =
            evaluate_condition("{{dep_activity.status == 'completed'}}", &context).unwrap();
        assert!(result, "Condition should be true for completed activity");
    }

    #[test]
    fn test_status_condition_skipped_activity() {
        // When activity status is 'skipped', condition {{activity.status == 'completed'}} is false
        let activities = vec![make_activity_state(
            "dep_activity",
            WorkflowActivityStatus::Skipped,
        )];
        let state = make_workflow_state(activities);
        let context = build_condition_context(&state);

        let result =
            evaluate_condition("{{dep_activity.status == 'completed'}}", &context).unwrap();
        assert!(!result, "Condition should be false for skipped activity");

        let result = evaluate_condition("{{dep_activity.status == 'skipped'}}", &context).unwrap();
        assert!(result, "Condition should be true for skipped activity");
    }

    #[test]
    fn test_status_condition_failed_activity() {
        // When activity status is 'failed', condition {{activity.status == 'completed'}} is false
        let activities = vec![make_activity_state(
            "dep_activity",
            WorkflowActivityStatus::Failed,
        )];
        let state = make_workflow_state(activities);
        let context = build_condition_context(&state);

        let result =
            evaluate_condition("{{dep_activity.status == 'completed'}}", &context).unwrap();
        assert!(!result, "Condition should be false for failed activity");

        let result = evaluate_condition("{{dep_activity.status == 'failed'}}", &context).unwrap();
        assert!(result, "Condition should be true for failed activity");
    }

    #[test]
    fn test_converging_paths_one_completed() {
        // Scenario: Two mutually exclusive paths (path_a and path_b) converge on finalize
        // path_a completed, path_b skipped
        // Finalize should see path_a.status == 'completed' as true

        let activities = vec![
            make_activity_state("path_a", WorkflowActivityStatus::Completed),
            make_activity_state("path_b", WorkflowActivityStatus::Skipped),
        ];
        let state = make_workflow_state(activities);
        let context = build_condition_context(&state);

        // Condition for path_a dependency (should be true - applicable)
        let result = evaluate_condition("{{path_a.status == 'completed'}}", &context).unwrap();
        assert!(result, "path_a should be applicable");

        // Condition for path_b dependency (should be false - not applicable)
        let result = evaluate_condition("{{path_b.status == 'completed'}}", &context).unwrap();
        assert!(!result, "path_b should not be applicable");
    }

    #[test]
    fn test_converging_paths_other_completed() {
        // Scenario: path_b completed, path_a skipped

        let activities = vec![
            make_activity_state("path_a", WorkflowActivityStatus::Skipped),
            make_activity_state("path_b", WorkflowActivityStatus::Completed),
        ];
        let state = make_workflow_state(activities);
        let context = build_condition_context(&state);

        // Condition for path_a dependency (should be false - not applicable)
        let result = evaluate_condition("{{path_a.status == 'completed'}}", &context).unwrap();
        assert!(!result, "path_a should not be applicable");

        // Condition for path_b dependency (should be true - applicable)
        let result = evaluate_condition("{{path_b.status == 'completed'}}", &context).unwrap();
        assert!(result, "path_b should be applicable");
    }

    #[test]
    fn test_converging_paths_both_skipped() {
        // Scenario: Both paths skipped
        // When all paths are skipped, no dependency is applicable

        let activities = vec![
            make_activity_state("path_a", WorkflowActivityStatus::Skipped),
            make_activity_state("path_b", WorkflowActivityStatus::Skipped),
        ];
        let state = make_workflow_state(activities);
        let context = build_condition_context(&state);

        // Neither path is applicable
        let result = evaluate_condition("{{path_a.status == 'completed'}}", &context).unwrap();
        assert!(!result, "path_a should not be applicable");

        let result = evaluate_condition("{{path_b.status == 'completed'}}", &context).unwrap();
        assert!(!result, "path_b should not be applicable");
    }

    #[test]
    fn test_unconditional_dependency_on_skipped_cascades() {
        // An unconditional dependency on a skipped activity should cause the dependent to be skipped
        // This tests the semantic: depends_on: [A] where A is skipped → dependent is skipped

        let definition = WorkflowDefinition {
            name: "test".to_string(),
            settings: None,
            activities: vec![
                make_activity_def("activity_a", None),
                make_activity_def(
                    "activity_b",
                    Some(vec![ActivityRelationship {
                        activity_key: "activity_a".to_string(),
                        conditions: None, // Unconditional dependency
                        is_back_edge: false,
                    }]),
                ),
            ],
        };

        // activity_a is skipped, activity_b is NotScheduled
        let activities = vec![
            make_activity_state("activity_a", WorkflowActivityStatus::Skipped),
            make_activity_state("activity_b", WorkflowActivityStatus::NotScheduled),
        ];
        let state = make_workflow_state(activities);

        // activity_b should be found as needing to be skipped
        let skipped = find_skipped_activities(&definition, &state).unwrap();
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].key, "activity_b");
    }

    #[test]
    fn test_unconditional_dependency_on_failed_cascades() {
        // An unconditional dependency on a failed activity should cause the dependent to be skipped

        let definition = WorkflowDefinition {
            name: "test".to_string(),
            settings: None,
            activities: vec![
                make_activity_def("activity_a", None),
                make_activity_def(
                    "activity_b",
                    Some(vec![ActivityRelationship {
                        activity_key: "activity_a".to_string(),
                        conditions: None, // Unconditional dependency
                        is_back_edge: false,
                    }]),
                ),
            ],
        };

        // activity_a failed, activity_b is NotScheduled
        let activities = vec![
            make_activity_state("activity_a", WorkflowActivityStatus::Failed),
            make_activity_state("activity_b", WorkflowActivityStatus::NotScheduled),
        ];
        let state = make_workflow_state(activities);

        // activity_b should be found as needing to be skipped
        let skipped = find_skipped_activities(&definition, &state).unwrap();
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].key, "activity_b");
    }

    #[test]
    fn test_conditional_dependency_with_status_check() {
        // A conditional dependency with status check should work correctly
        // depends_on:
        //   - activity_key: path_a
        //     condition: "{{path_a.status == 'completed'}}"

        let definition = WorkflowDefinition {
            name: "test".to_string(),
            settings: None,
            activities: vec![
                make_activity_def("path_a", None),
                make_activity_def(
                    "finalize",
                    Some(vec![ActivityRelationship {
                        activity_key: "path_a".to_string(),
                        conditions: Some(vec!["{{path_a.status == 'completed'}}".to_string()]),
                        is_back_edge: false,
                    }]),
                ),
            ],
        };

        // path_a completed, finalize is NotScheduled
        let activities = vec![
            make_activity_state("path_a", WorkflowActivityStatus::Completed),
            make_activity_state("finalize", WorkflowActivityStatus::NotScheduled),
        ];
        let state = make_workflow_state(activities);

        // finalize should be ready (condition satisfied)
        let ready = find_ready_activities(&definition, &state).unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].key, "finalize");
    }

    #[test]
    fn test_conditional_dependency_skipped_not_applicable() {
        // When dependency is skipped and condition checks status == 'completed',
        // the dependency is not applicable

        let definition = WorkflowDefinition {
            name: "test".to_string(),
            settings: None,
            activities: vec![
                make_activity_def("path_a", None),
                make_activity_def(
                    "finalize",
                    Some(vec![ActivityRelationship {
                        activity_key: "path_a".to_string(),
                        conditions: Some(vec!["{{path_a.status == 'completed'}}".to_string()]),
                        is_back_edge: false,
                    }]),
                ),
            ],
        };

        // path_a skipped, finalize is NotScheduled
        let activities = vec![
            make_activity_state("path_a", WorkflowActivityStatus::Skipped),
            make_activity_state("finalize", WorkflowActivityStatus::NotScheduled),
        ];
        let state = make_workflow_state(activities);

        // finalize should not be ready (condition not satisfied)
        let ready = find_ready_activities(&definition, &state).unwrap();
        assert!(
            ready.is_empty(),
            "finalize should not be ready when only dependency is skipped"
        );

        // finalize should be marked as skipped (no applicable dependencies)
        let skipped = find_skipped_activities(&definition, &state).unwrap();
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].key, "finalize");
    }

    #[test]
    fn test_converging_conditional_dependencies_one_path() {
        // Converging paths with status conditions: one path completes
        // depends_on:
        //   - activity_key: path_a
        //     condition: "{{path_a.status == 'completed'}}"
        //   - activity_key: path_b
        //     condition: "{{path_b.status == 'completed'}}"

        let definition = WorkflowDefinition {
            name: "test".to_string(),
            settings: None,
            activities: vec![
                make_activity_def("path_a", None),
                make_activity_def("path_b", None),
                make_activity_def(
                    "finalize",
                    Some(vec![
                        ActivityRelationship {
                            activity_key: "path_a".to_string(),
                            conditions: Some(vec!["{{path_a.status == 'completed'}}".to_string()]),
                            is_back_edge: false,
                        },
                        ActivityRelationship {
                            activity_key: "path_b".to_string(),
                            conditions: Some(vec!["{{path_b.status == 'completed'}}".to_string()]),
                            is_back_edge: false,
                        },
                    ]),
                ),
            ],
        };

        // path_a completed, path_b skipped, finalize is NotScheduled
        let activities = vec![
            make_activity_state("path_a", WorkflowActivityStatus::Completed),
            make_activity_state("path_b", WorkflowActivityStatus::Skipped),
            make_activity_state("finalize", WorkflowActivityStatus::NotScheduled),
        ];
        let state = make_workflow_state(activities);

        // finalize should be ready (path_a condition satisfied)
        let ready = find_ready_activities(&definition, &state).unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].key, "finalize");
    }

    #[test]
    fn test_converging_conditional_dependencies_both_skipped() {
        // Converging paths with status conditions: both paths skipped

        let definition = WorkflowDefinition {
            name: "test".to_string(),
            settings: None,
            activities: vec![
                make_activity_def("path_a", None),
                make_activity_def("path_b", None),
                make_activity_def(
                    "finalize",
                    Some(vec![
                        ActivityRelationship {
                            activity_key: "path_a".to_string(),
                            conditions: Some(vec!["{{path_a.status == 'completed'}}".to_string()]),
                            is_back_edge: false,
                        },
                        ActivityRelationship {
                            activity_key: "path_b".to_string(),
                            conditions: Some(vec!["{{path_b.status == 'completed'}}".to_string()]),
                            is_back_edge: false,
                        },
                    ]),
                ),
            ],
        };

        // Both paths skipped, finalize is NotScheduled
        let activities = vec![
            make_activity_state("path_a", WorkflowActivityStatus::Skipped),
            make_activity_state("path_b", WorkflowActivityStatus::Skipped),
            make_activity_state("finalize", WorkflowActivityStatus::NotScheduled),
        ];
        let state = make_workflow_state(activities);

        // finalize should not be ready (no conditions satisfied)
        let ready = find_ready_activities(&definition, &state).unwrap();
        assert!(
            ready.is_empty(),
            "finalize should not be ready when all paths are skipped"
        );

        // finalize should be marked as skipped (no applicable dependencies)
        let skipped = find_skipped_activities(&definition, &state).unwrap();
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].key, "finalize");
    }

    #[test]
    fn test_status_to_string_all_variants() {
        // Verify status_to_string returns correct snake_case strings
        assert_eq!(
            status_to_string(WorkflowActivityStatus::NotScheduled),
            "not_scheduled"
        );
        assert_eq!(status_to_string(WorkflowActivityStatus::Waiting), "waiting");
        assert_eq!(status_to_string(WorkflowActivityStatus::Pending), "pending");
        assert_eq!(status_to_string(WorkflowActivityStatus::Running), "running");
        assert_eq!(
            status_to_string(WorkflowActivityStatus::Completed),
            "completed"
        );
        assert_eq!(status_to_string(WorkflowActivityStatus::Failed), "failed");
        assert_eq!(status_to_string(WorkflowActivityStatus::Skipped), "skipped");
    }

    #[test]
    fn test_is_workflow_complete_all_terminal() {
        let activities = vec![
            make_activity_state("a", WorkflowActivityStatus::Completed),
            make_activity_state("b", WorkflowActivityStatus::Failed),
            make_activity_state("c", WorkflowActivityStatus::Skipped),
        ];
        let state = make_workflow_state(activities);
        assert!(is_workflow_complete(&state));
    }

    #[test]
    fn test_is_workflow_complete_with_running() {
        let activities = vec![
            make_activity_state("a", WorkflowActivityStatus::Completed),
            make_activity_state("b", WorkflowActivityStatus::Running),
        ];
        let state = make_workflow_state(activities);
        assert!(!is_workflow_complete(&state));
    }

    #[test]
    fn test_is_workflow_complete_with_not_scheduled() {
        let activities = vec![
            make_activity_state("a", WorkflowActivityStatus::Completed),
            make_activity_state("b", WorkflowActivityStatus::NotScheduled),
        ];
        let state = make_workflow_state(activities);
        assert!(!is_workflow_complete(&state));
    }

    #[test]
    fn test_is_workflow_complete_with_pending() {
        let activities = vec![
            make_activity_state("a", WorkflowActivityStatus::Completed),
            make_activity_state("b", WorkflowActivityStatus::Pending),
        ];
        let state = make_workflow_state(activities);
        assert!(!is_workflow_complete(&state));
    }

    #[test]
    fn test_is_workflow_complete_with_waiting() {
        let activities = vec![
            make_activity_state("a", WorkflowActivityStatus::Completed),
            make_activity_state("b", WorkflowActivityStatus::Waiting),
        ];
        let state = make_workflow_state(activities);
        assert!(!is_workflow_complete(&state));
    }

    #[test]
    fn test_is_workflow_complete_empty() {
        let state = make_workflow_state(vec![]);
        assert!(is_workflow_complete(&state));
    }

    #[test]
    fn test_is_workflow_failed_with_failure() {
        let activities = vec![
            make_activity_state("a", WorkflowActivityStatus::Completed),
            make_activity_state("b", WorkflowActivityStatus::Failed),
        ];
        let state = make_workflow_state(activities);
        assert!(is_workflow_failed(&state));
    }

    #[test]
    fn test_is_workflow_failed_no_failure() {
        let activities = vec![
            make_activity_state("a", WorkflowActivityStatus::Completed),
            make_activity_state("b", WorkflowActivityStatus::Completed),
        ];
        let state = make_workflow_state(activities);
        assert!(!is_workflow_failed(&state));
    }

    #[test]
    fn test_is_workflow_failed_empty() {
        let state = make_workflow_state(vec![]);
        assert!(!is_workflow_failed(&state));
    }

    #[test]
    fn test_find_ready_root_activity() {
        let definition = WorkflowDefinition {
            name: "test".to_string(),
            settings: None,
            activities: vec![make_activity_def("root", None)],
        };
        let activities = vec![make_activity_state(
            "root",
            WorkflowActivityStatus::NotScheduled,
        )];
        let state = make_workflow_state(activities);

        let ready = find_ready_activities(&definition, &state).unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].key, "root");
    }

    #[test]
    fn test_find_ready_running_not_ready() {
        let definition = WorkflowDefinition {
            name: "test".to_string(),
            settings: None,
            activities: vec![make_activity_def("a", None)],
        };
        let activities = vec![make_activity_state("a", WorkflowActivityStatus::Running)];
        let state = make_workflow_state(activities);

        let ready = find_ready_activities(&definition, &state).unwrap();
        assert!(ready.is_empty());
    }

    #[test]
    fn test_find_ready_waiting_not_ready() {
        let definition = WorkflowDefinition {
            name: "test".to_string(),
            settings: None,
            activities: vec![make_activity_def("a", None)],
        };
        let activities = vec![make_activity_state("a", WorkflowActivityStatus::Waiting)];
        let state = make_workflow_state(activities);

        let ready = find_ready_activities(&definition, &state).unwrap();
        assert!(ready.is_empty());
    }

    #[test]
    fn test_evaluate_condition_number_truthy() {
        let state = make_workflow_state(vec![]);
        let context = build_condition_context(&state);
        // Non-zero number is truthy
        assert!(evaluate_condition("{{1}}", &context).unwrap());
        // Zero is falsy
        assert!(!evaluate_condition("{{0}}", &context).unwrap());
    }

    #[test]
    fn test_evaluate_condition_null_falsy() {
        let activities = vec![make_activity_state("a", WorkflowActivityStatus::Completed)];
        let state = make_workflow_state(activities);
        let context = build_condition_context(&state);
        // Access to non-existent property returns undefined/null
        assert!(!evaluate_condition("{{a.nonexistent}}", &context).unwrap());
    }

    #[test]
    fn test_build_condition_context_with_state_data() {
        let mut state = make_workflow_state(vec![make_activity_state(
            "step1",
            WorkflowActivityStatus::Completed,
        )]);
        state.state_data = serde_json::json!({"user_input": "hello"});

        let context = build_condition_context(&state);
        // state_data is available via inputs in the context
        // Verify context is built without error
        let result = evaluate_condition("{{step1.status == 'completed'}}", &context).unwrap();
        assert!(result);
    }

    #[test]
    fn test_get_dependencies_with_none() {
        let activity = make_activity_def("a", None);
        let deps = get_dependencies(&activity);
        assert!(deps.is_empty());
    }

    #[test]
    fn test_get_dependencies_with_some() {
        let activity = make_activity_def(
            "a",
            Some(vec![
                ActivityRelationship {
                    activity_key: "b".to_string(),
                    conditions: None,
                    is_back_edge: false,
                },
                ActivityRelationship {
                    activity_key: "c".to_string(),
                    conditions: None,
                    is_back_edge: false,
                },
            ]),
        );
        let deps = get_dependencies(&activity);
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].activity_key, "b");
        assert_eq!(deps[1].activity_key, "c");
    }

    #[test]
    fn test_find_skipped_root_activity_not_skipped() {
        // Root activities (no dependencies) should never be skipped
        let definition = WorkflowDefinition {
            name: "test".to_string(),
            settings: None,
            activities: vec![make_activity_def("root", None)],
        };
        let activities = vec![make_activity_state(
            "root",
            WorkflowActivityStatus::NotScheduled,
        )];
        let state = make_workflow_state(activities);

        let skipped = find_skipped_activities(&definition, &state).unwrap();
        assert!(skipped.is_empty());
    }

    #[test]
    fn test_find_skipped_already_completed_not_skipped() {
        // Completed activities should not appear in skipped list
        let definition = WorkflowDefinition {
            name: "test".to_string(),
            settings: None,
            activities: vec![
                make_activity_def("a", None),
                make_activity_def(
                    "b",
                    Some(vec![ActivityRelationship {
                        activity_key: "a".to_string(),
                        conditions: None,
                        is_back_edge: false,
                    }]),
                ),
            ],
        };
        let activities = vec![
            make_activity_state("a", WorkflowActivityStatus::Failed),
            make_activity_state("b", WorkflowActivityStatus::Completed),
        ];
        let state = make_workflow_state(activities);

        let skipped = find_skipped_activities(&definition, &state).unwrap();
        assert!(skipped.is_empty());
    }

    // ============================================================================
    // Loop members run again only on new input (#18)
    // ============================================================================

    fn rel(key: &str, is_back_edge: bool) -> ActivityRelationship {
        ActivityRelationship {
            activity_key: key.to_string(),
            conditions: None,
            is_back_edge,
        }
    }

    fn loop_def(key: &str, deps: Vec<ActivityRelationship>) -> ActivityDefinition {
        ActivityDefinition {
            is_loop_activity: true,
            ..make_activity_def(key, Some(deps))
        }
    }

    /// The 07a shape: start → head ⇄ tail (head holds the back-edge)
    fn two_step_loop() -> WorkflowDefinition {
        WorkflowDefinition {
            name: "loop".to_string(),
            settings: None,
            activities: vec![
                make_activity_def("start", None),
                loop_def("head", vec![rel("start", false), rel("tail", true)]),
                loop_def("tail", vec![rel("head", false)]),
            ],
        }
    }

    /// A completed activity with a completion sequence number and iteration
    fn completed(
        key: &str,
        seq: u64,
        iteration: u32,
    ) -> (String, super::super::workflow_state::ActivityState) {
        let (key, mut state) = make_activity_state(key, WorkflowActivityStatus::Completed);
        state.completion_seq = seq;
        state.iteration = iteration;
        (key, state)
    }

    fn ready_keys(definition: &WorkflowDefinition, state: &WorkflowState) -> Vec<String> {
        let mut keys: Vec<String> = find_ready_activities(definition, state)
            .unwrap()
            .into_iter()
            .map(|a| a.key.clone())
            .collect();
        keys.sort();
        keys
    }

    #[test]
    fn test_loop_tail_waits_for_the_head_to_complete_again() {
        let definition = two_step_loop();
        // start(1) → head(2) → tail(3): the tail just completed; the head's
        // completion (2) is older than the tail's (3), so the tail has no new
        // input. The head does: the tail completed after it.
        let state = make_workflow_state(vec![
            completed("start", 1, 0),
            completed("head", 2, 1),
            completed("tail", 3, 1),
        ]);
        assert_eq!(ready_keys(&definition, &state), vec!["head"]);
    }

    #[test]
    fn test_loop_head_waits_for_the_tail_to_complete_again() {
        let definition = two_step_loop();
        // The head just completed its second round (4); the tail's completion
        // (3) belongs to the previous round. Only the tail has new input.
        let state = make_workflow_state(vec![
            completed("start", 1, 0),
            completed("head", 4, 2),
            completed("tail", 3, 1),
        ]);
        assert_eq!(ready_keys(&definition, &state), vec!["tail"]);
    }

    #[test]
    fn test_loop_outside_dependency_is_not_new_input() {
        // `start` completed once, before the loop: it satisfies the head's
        // forward dependency but never counts as new input on its own
        let definition = two_step_loop();
        let state = make_workflow_state(vec![
            completed("start", 1, 0),
            completed("head", 2, 1),
            (
                "tail".to_string(),
                make_activity_state("tail", WorkflowActivityStatus::Running).1,
            ),
        ]);
        assert!(ready_keys(&definition, &state).is_empty());
    }

    #[test]
    fn test_self_loop_is_its_own_new_input() {
        let definition = WorkflowDefinition {
            name: "self".to_string(),
            settings: None,
            activities: vec![ActivityDefinition {
                iteration_limit: Some(5),
                ..loop_def("poll", vec![rel("poll", true)])
            }],
        };
        let state = make_workflow_state(vec![completed("poll", 7, 2)]);
        assert_eq!(ready_keys(&definition, &state), vec!["poll"]);
    }

    #[test]
    fn test_state_without_sequence_numbers_counts_as_new_input() {
        // All `completion_seq` 0: no ordering information, so every completed
        // loop member counts as having new input
        let definition = two_step_loop();
        let state = make_workflow_state(vec![
            completed("start", 0, 0),
            completed("head", 0, 1),
            completed("tail", 0, 1),
        ]);
        assert_eq!(ready_keys(&definition, &state), vec!["head", "tail"]);
    }

    #[test]
    fn test_has_new_input_needs_a_completed_dependency() {
        let definition = two_step_loop();
        let tail_def = &definition.activities[2];
        let (_, tail) = completed("tail", 3, 1);
        // The head is running again (a newer seq would come with its completion)
        let (_, mut head) = completed("head", 5, 2);
        head.status = WorkflowActivityStatus::Running;
        let state = make_workflow_state(vec![
            ("head".to_string(), head),
            ("tail".to_string(), tail.clone()),
        ]);
        assert!(!has_new_input(tail_def, &tail, &state));
    }

    // ============================================================================
    // Activities after a loop wait for the loop to end before being skipped (#27)
    // ============================================================================

    /// A condition that is false here (`check` is never skipped in these tests)
    fn rel_false(key: &str) -> ActivityRelationship {
        ActivityRelationship {
            activity_key: key.to_string(),
            conditions: Some(vec![format!("{{{{ {key}.status == 'skipped' }}}}")]),
            is_back_edge: false,
        }
    }

    /// run ⇄ check (run holds the back-edge), and `done` after the loop,
    /// conditional on `check`
    fn loop_with_exit_step() -> WorkflowDefinition {
        WorkflowDefinition {
            name: "loop_exit".to_string(),
            settings: None,
            activities: vec![
                loop_def("run", vec![rel("check", true)]),
                loop_def("check", vec![rel("run", false)]),
                make_activity_def("done", Some(vec![rel_false("check")])),
            ],
        }
    }

    fn with_status(
        key: &str,
        seq: u64,
        status: WorkflowActivityStatus,
    ) -> (String, super::super::workflow_state::ActivityState) {
        let (key, mut state) = completed(key, seq, 1);
        state.status = status;
        (key, state)
    }

    #[test]
    fn test_exit_step_is_not_skipped_while_the_loop_runs() {
        let definition = loop_with_exit_step();
        // check completed (condition false for done), run is going again
        let state = make_workflow_state(vec![
            with_status("run", 1, WorkflowActivityStatus::Pending),
            completed("check", 2, 1),
            make_activity_state("done", WorkflowActivityStatus::NotScheduled),
        ]);
        assert!(
            find_skipped_activities(&definition, &state)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn test_exit_step_is_skipped_once_the_loop_has_ended() {
        let definition = loop_with_exit_step();
        // Nothing in the loop is scheduled, running or waiting: it is over
        let state = make_workflow_state(vec![
            completed("run", 1, 1),
            completed("check", 2, 1),
            make_activity_state("done", WorkflowActivityStatus::NotScheduled),
        ]);
        let skipped: Vec<&str> = find_skipped_activities(&definition, &state)
            .unwrap()
            .iter()
            .map(|a| a.key.as_str())
            .collect();
        assert_eq!(skipped, vec!["done"]);
    }

    #[test]
    fn test_loop_member_is_skipped_while_its_loop_runs() {
        // Inside the loop nothing changes: a member whose condition is false is
        // skipped at once, so the steps after it in the same round can proceed
        let mut definition = loop_with_exit_step();
        definition.activities[2] = loop_def("done", vec![rel_false("check")]);
        let state = make_workflow_state(vec![
            with_status("run", 1, WorkflowActivityStatus::Running),
            completed("check", 2, 1),
            make_activity_state("done", WorkflowActivityStatus::NotScheduled),
        ]);
        let skipped: Vec<&str> = find_skipped_activities(&definition, &state)
            .unwrap()
            .iter()
            .map(|a| a.key.as_str())
            .collect();
        assert_eq!(skipped, vec!["done"]);
    }

    #[test]
    fn test_conditional_skip_without_a_loop_is_immediate() {
        // No loop: skipped at once, even while another activity still runs
        let definition = WorkflowDefinition {
            name: "branch".to_string(),
            settings: None,
            activities: vec![
                make_activity_def("check", None),
                make_activity_def("slow", Some(vec![rel("check", false)])),
                make_activity_def("done", Some(vec![rel_false("check")])),
            ],
        };
        let state = make_workflow_state(vec![
            completed("check", 1, 0),
            with_status("slow", 0, WorkflowActivityStatus::Running),
            make_activity_state("done", WorkflowActivityStatus::NotScheduled),
        ]);
        let skipped: Vec<&str> = find_skipped_activities(&definition, &state)
            .unwrap()
            .iter()
            .map(|a| a.key.as_str())
            .collect();
        assert_eq!(skipped, vec!["done"]);
    }
}
