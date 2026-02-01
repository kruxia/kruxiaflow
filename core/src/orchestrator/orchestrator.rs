use super::{
    AdaptiveBackoff, OrchestratorConfig, Result,
    dependency_evaluator::{
        find_ready_activities, find_skipped_activities, is_workflow_complete, is_workflow_failed,
        status_to_string,
    },
    workflow_state::{
        WorkflowActivityStatus, apply_event_to_state, initialize_workflow_state,
        load_materialized_state, load_workflow_definition, load_workflow_definition_by_id,
        save_materialized_state,
    },
};
use crate::cost::{ActivityCostRecord, CostCalculator, CostTracker};
use crate::events::{
    EventSource, NewWorkflowEvent, WorkflowEvent, WorkflowEventType, WorkflowStatus,
};
use crate::queue::{Activity, ActivityQueue, StaleActivityAction};
use crate::subscription::{ExpiredSubscription, NewSubscription, SubscriptionService};
use crate::workflow::template::{TemplateContext, resolve_template_value};
use crate::workflow::{BudgetAction, OnTimeout, apply_duration, parse_scheduled_for};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const CONSUMER_ID: &str = "orchestrator";
const MAX_EVENT_PROCESSING_RETRIES: u32 = 5;

// Conditional span creation macro - only creates spans when profiling feature is enabled
#[cfg(feature = "profiling")]
macro_rules! profile_span {
    ($($tt:tt)*) => {
        tracing::debug_span!($($tt)*)
    };
}

#[cfg(not(feature = "profiling"))]
macro_rules! profile_span {
    ($($tt:tt)*) => {
        tracing::Span::none()
    };
}

/// Run the orchestrator main loop
/// Polls for events, evaluates dependencies, schedules activities
///
/// # Arguments
/// * `event_source` - Event source for polling workflow events
/// * `activity_queue` - Activity queue for scheduling activities
/// * `config` - Orchestrator configuration
/// * `shutdown_token` - Optional cancellation token for graceful shutdown
#[tracing::instrument(skip(
    event_source,
    activity_queue,
    subscription_service,
    config,
    shutdown_token
))]
pub async fn run_orchestrator(
    event_source: Arc<dyn EventSource>,
    activity_queue: Arc<dyn ActivityQueue>,
    subscription_service: Arc<dyn SubscriptionService>,
    config: OrchestratorConfig,
    shutdown_token: Option<CancellationToken>,
) -> Result<()> {
    let mut backoff = AdaptiveBackoff::new(
        config.poll_interval_min,
        config.poll_interval_max,
        config.backoff_multiplier,
    );

    // Track event processing failures to detect poison messages
    let mut event_failures: HashMap<Uuid, u32> = HashMap::new();

    tracing::info!(
        "Orchestrator starting with consumer_id={}, workflow_timeout={}s, max_event_retries={}",
        CONSUMER_ID,
        config.workflow_timeout.as_secs(),
        MAX_EVENT_PROCESSING_RETRIES
    );

    // Spawn background task to check for stuck workflows and stale activities
    let timeout_config = config.clone();
    let timeout_event_source = event_source.clone();
    let timeout_activity_queue = activity_queue.clone();
    let timeout_subscription_service = subscription_service.clone();
    let timeout_shutdown = shutdown_token.clone();
    tokio::spawn(async move {
        timeout_checker_task(
            timeout_config,
            timeout_event_source,
            timeout_activity_queue,
            timeout_subscription_service,
            timeout_shutdown,
        )
        .await;
    });

    loop {
        // Check if shutdown has been requested
        if let Some(ref token) = shutdown_token
            && token.is_cancelled()
        {
            tracing::info!("Shutdown requested, orchestrator stopping gracefully");
            return Ok(());
        }

        // Poll for new events (durable position tracking)
        let events = event_source.poll(CONSUMER_ID).await?;

        if events.is_empty() {
            // No events - increase backoff
            backoff.increase();
            let interval = backoff.current();
            tracing::debug!("No events found, backoff interval: {:?}", interval);
            tokio::time::sleep(interval).await;
            continue;
        }

        tracing::debug!("Polled {} events, resetting backoff", events.len());

        // Process each event
        for event in &events {
            // Check shutdown again before processing each event
            if let Some(ref token) = shutdown_token
                && token.is_cancelled()
            {
                tracing::info!("Shutdown requested during event processing, stopping");
                return Ok(());
            }

            if let Err(e) = process_workflow_event(
                event,
                &event_source,
                &activity_queue,
                &subscription_service,
                &config,
            )
            .await
            {
                // Track failure count for this event
                let failure_count = event_failures.entry(event.id).or_insert(0);
                *failure_count += 1;

                tracing::error!(
                    event_id = %event.id,
                    workflow_id = %event.workflow_id,
                    event_type = ?event.event_type,
                    failure_count = *failure_count,
                    max_retries = MAX_EVENT_PROCESSING_RETRIES,
                    error = %e,
                    "Failed to process event"
                );

                // If we've exceeded max retries, treat as poison message
                if *failure_count >= MAX_EVENT_PROCESSING_RETRIES {
                    tracing::error!(
                        event_id = %event.id,
                        workflow_id = %event.workflow_id,
                        event_type = ?event.event_type,
                        failure_count = *failure_count,
                        "Poison message detected - publishing failure event after {} failed attempts",
                        MAX_EVENT_PROCESSING_RETRIES
                    );

                    // Publish a failure event for this poison message
                    let _ = publish_failure_for_poison_event(event, &event_source, &e.to_string())
                        .await;

                    // Update position to skip this poison event
                    // The failure event will be processed normally by the orchestrator
                    event_source.update_position(CONSUMER_ID, event.id).await?;
                    event_failures.remove(&event.id);
                } else {
                    // Will be retried on next poll
                    continue;
                }
            } else {
                // Success - clear failure count and update position
                event_failures.remove(&event.id);
                event_source.update_position(CONSUMER_ID, event.id).await?;
            }
        }

        // Got events - reset backoff
        backoff.reset();

        // Always sleep for at least minimum interval to avoid spinning
        // This caps polling rate at ~1000/sec (with 1ms min) even under heavy load
        tokio::time::sleep(backoff.current()).await;
    }
}

/// Compute scheduled_for timestamp based on activity settings
///
/// Handles both relative delays (e.g., "5s", "30m") and absolute timestamps (ISO 8601).
/// Templates are resolved using the provided context.
///
/// User-specified scheduling only applies to initial attempts (iteration 0).
/// Retry attempts use the backoff logic instead.
///
/// Returns:
/// - Some(DateTime) if activity should be delayed
/// - None if activity should execute immediately
fn compute_scheduled_for(
    activity_def: &crate::workflow::ActivityDefinition,
    template_context: &TemplateContext,
    iteration: Option<i32>,
) -> Result<Option<DateTime<Utc>>> {
    // User-specified scheduling only applies to initial attempt (iteration = 0 or None)
    // Retries (iteration > 0) use backoff logic, which is handled separately by the queue
    let is_initial_attempt = iteration.unwrap_or(0) == 0;

    if !is_initial_attempt {
        // For retries, scheduled_for is handled by the retry backoff logic
        return Ok(None);
    }

    let settings = match &activity_def.settings {
        Some(s) => s,
        None => return Ok(None), // No settings = immediate execution
    };

    // Case 1: delay (relative with flexible units)
    if let Some(delay_str) = &settings.delay {
        // Resolve template (in case of "{{INPUT.delay}}m")
        let delay_value = serde_json::Value::String(delay_str.clone());
        let resolved_value =
            resolve_template_value(&delay_value, template_context).map_err(|e| {
                super::OrchestratorError::TemplateFailed(format!(
                    "Failed to resolve delay template for activity {}: {}",
                    activity_def.key, e
                ))
            })?;

        let resolved_delay = resolved_value.as_str().ok_or_else(|| {
            super::OrchestratorError::TemplateFailed(format!(
                "Resolved delay template for activity {} is not a string: {}",
                activity_def.key, resolved_value
            ))
        })?;

        // Apply duration to current time
        let scheduled_time = apply_duration(Utc::now(), resolved_delay).map_err(|e| {
            super::OrchestratorError::TemplateFailed(format!(
                "Failed to parse duration '{}' for activity {}: {}",
                resolved_delay, activity_def.key, e
            ))
        })?;

        tracing::debug!(
            "Activity {} scheduled with delay {} (resolved: {}) -> {}",
            activity_def.key,
            delay_str,
            resolved_delay,
            scheduled_time
        );

        return Ok(Some(scheduled_time));
    }

    // Case 2: scheduled_for (absolute)
    if let Some(template) = &settings.scheduled_for {
        // Resolve template to get ISO 8601 string
        let scheduled_value = serde_json::Value::String(template.clone());
        let resolved_value =
            resolve_template_value(&scheduled_value, template_context).map_err(|e| {
                super::OrchestratorError::TemplateFailed(format!(
                    "Failed to resolve scheduled_for template for activity {}: {}",
                    activity_def.key, e
                ))
            })?;

        let resolved_timestamp = resolved_value.as_str().ok_or_else(|| {
            super::OrchestratorError::TemplateFailed(format!(
                "Resolved scheduled_for template for activity {} is not a string: {}",
                activity_def.key, resolved_value
            ))
        })?;

        // Parse ISO 8601 to DateTime
        let dt = parse_scheduled_for(resolved_timestamp).map_err(|e| {
            super::OrchestratorError::TemplateFailed(format!(
                "Failed to parse timestamp '{}' for activity {}: {}",
                resolved_timestamp, activity_def.key, e
            ))
        })?;

        // Validate not in the past (warning, not error)
        if dt < Utc::now() {
            tracing::warn!(
                "Activity {} scheduled in the past: {} (will execute immediately)",
                activity_def.key,
                dt
            );
        }

        tracing::debug!(
            "Activity {} scheduled for absolute time {} (resolved: {})",
            activity_def.key,
            template,
            dt
        );

        return Ok(Some(dt));
    }

    // Case 3: No scheduling (immediate execution)
    Ok(None)
}

/// Build TemplateContext from WorkflowState for resolving template expressions
fn build_template_context(
    state: &super::workflow_state::WorkflowState,
    workflow_id: uuid::Uuid,
    secrets: &HashMap<String, String>,
) -> TemplateContext {
    let mut context = TemplateContext::new();

    // Add workflow inputs from input field
    if let serde_json::Value::Object(input_obj) = &state.input {
        let inputs: HashMap<String, serde_json::Value> = input_obj
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        context = context.with_inputs(inputs);
    }

    // Add secrets (from environment variables KRUXIAFLOW_SECRET_*)
    context = context.with_secrets(secrets.clone());

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

    // Add workflow-level variables
    context.workflow.insert(
        "id".to_string(),
        serde_json::Value::String(workflow_id.to_string()),
    );
    context.workflow.insert(
        "status".to_string(),
        serde_json::Value::String(state.status.to_string()),
    );

    context
}

/// Handle ActivityFailed event with retry logic.
/// Returns true if activity will be retried, false if it should fail permanently
async fn handle_activity_failed(
    state: &mut super::workflow_state::WorkflowState,
    definition: &crate::workflow::WorkflowDefinition,
    event: &WorkflowEvent,
    event_source: &Arc<dyn EventSource>,
    activity_queue: &Arc<dyn ActivityQueue>,
    pool: &sqlx::PgPool,
    secrets: &HashMap<String, String>,
) -> Result<bool> {
    let activity_key = event
        .activity_key
        .as_ref()
        .ok_or_else(|| super::OrchestratorError::MissingActivityKey)?;

    // Get activity definition
    let activity_def = definition
        .activities
        .iter()
        .find(|a| &a.key == activity_key)
        .ok_or_else(|| {
            super::OrchestratorError::ActivityNotFound(format!(
                "Activity definition not found: {}",
                activity_key
            ))
        })?;

    // Get error message from event payload
    let error_message = event
        .payload
        .get("error")
        .and_then(|e| e.as_str())
        .unwrap_or("Unknown error");

    // Get activity settings (use default if not specified)
    let settings = activity_def.settings.as_ref();

    // Get current attempt number (before mutable borrow)
    let current_attempt = state
        .activities
        .get(activity_key)
        .map(|a| a.attempt)
        .unwrap_or(1);

    // Check if should retry
    let should_retry = if let Some(settings) = settings {
        settings.should_retry(current_attempt)
    } else {
        false
    };

    if !should_retry {
        tracing::error!(
            workflow_id = %state.workflow_id,
            activity_key = %activity_key,
            attempt = current_attempt,
            error = %error_message,
            "Activity failed permanently (no retry configured or max attempts reached)"
        );
        return Ok(false); // Don't retry - let it fail permanently
    }

    // Calculate backoff delay
    let backoff_seconds = settings
        .unwrap()
        .calculate_backoff(current_attempt)
        .unwrap_or(0);

    tracing::info!(
        workflow_id = %state.workflow_id,
        activity_key = %activity_key,
        attempt = current_attempt,
        backoff_seconds = backoff_seconds,
        max_attempts = settings.and_then(|s| s.retry.as_ref().map(|r| r.max_attempts)),
        "Retrying activity after failure"
    );

    // Build template context for resolving parameters (before mutating state)
    let template_context = build_template_context(state, state.workflow_id, secrets);

    // Now update activity state (mutable borrow)
    let activity_state = state.activities.get_mut(activity_key).ok_or_else(|| {
        super::OrchestratorError::ActivityNotFound(format!(
            "Activity state not found: {}",
            activity_key
        ))
    })?;

    // Update error state
    activity_state.set_error(error_message.to_string());

    // Increment attempt count
    activity_state.increment_attempt();

    // Reset status to Pending for retry
    activity_state.status = WorkflowActivityStatus::Pending;

    // Resolve template expressions in parameters
    let params_value = serde_json::to_value(
        activity_def
            .parameters
            .as_ref()
            .unwrap_or(&Default::default()),
    )
    .map_err(|e| {
        super::OrchestratorError::TemplateFailed(format!("Failed to serialize parameters: {}", e))
    })?;

    let resolved_params = match resolve_template_value(&params_value, &template_context) {
        Ok(resolved) => resolved,
        Err(e) => {
            tracing::error!(
                "Template resolution failed for activity retry {}: {}",
                activity_key,
                e
            );
            return Err(super::OrchestratorError::TemplateFailed(format!(
                "Failed to resolve templates in activity {}: {}",
                activity_key, e
            )));
        }
    };

    // Enrich LLM activity parameters with budget information
    let activity_name_str = activity_def.activity_name.as_deref().unwrap_or("");
    let (params_w_budget, should_schedule) = enrich_llm_activity_params_w_budget(
        activity_name_str,
        resolved_params,
        activity_def,
        definition,
        activity_key,
        state.workflow_id,
        pool,
    )
    .await?;

    // Don't retry if budget check failed
    if !should_schedule {
        tracing::warn!(
            "Skipping LLM activity retry {} due to budget constraint",
            activity_key
        );
        return Ok(false); // Don't retry, let it fail permanently
    }

    // Schedule retry with delay
    let scheduled_for = if backoff_seconds > 0 {
        Some(chrono::Utc::now() + chrono::Duration::seconds(backoff_seconds as i64))
    } else {
        None
    };

    let activity_to_schedule = Activity {
        key: activity_key.clone(),
        worker: activity_def.worker.clone(),
        activity_name: activity_def.activity_name.clone().unwrap_or_default(),
        parameters: params_w_budget,
        settings: activity_def.settings.clone(),
        scheduled_for,
        output_definitions: activity_def.output_definitions.clone(),
        iteration: if activity_def.is_loop_activity {
            Some(activity_state.iteration as i32)
        } else {
            None
        },
        signal_data: activity_state.signal_data.clone(),
    };

    // Log info when retry activity is started
    tracing::info!(
        workflow_id = %state.workflow_id,
        activity_key = %activity_to_schedule.key,
        worker = %activity_to_schedule.worker,
        activity_name = %activity_to_schedule.activity_name,
        iteration = ?activity_to_schedule.iteration,
        attempt = current_attempt + 1,
        scheduled_for = ?scheduled_for,
        "Activity started (retry)"
    );

    activity_queue
        .schedule(state.workflow_id, vec![activity_to_schedule])
        .await?;

    // Publish ActivityScheduled event for observability
    // For looping activities, include iteration to avoid unique constraint violation
    let iteration = if activity_def.is_loop_activity {
        Some(activity_state.iteration as i32)
    } else {
        None
    };

    let scheduled_event = NewWorkflowEvent {
        workflow_id: state.workflow_id,
        event_type: WorkflowEventType::ActivityScheduled,
        activity_key: Some(activity_key.clone()),
        payload: json!({
            "worker": activity_def.worker,
            "activity_name": activity_def.activity_name,
            "attempt": activity_state.attempt,
            "scheduled_for": scheduled_for,
        }),
        iteration,
    };
    event_source.publish(scheduled_event).await?;

    Ok(true) // Retry scheduled
}

/// Process a workflow event and advance workflow execution.
///
/// # Methodology
///
/// This function implements the **event-driven orchestration loop** through a multi-phase process
/// that updates workflow state incrementally and schedules ready activities.
///
/// ## Architecture Overview
///
/// **Event-Driven**: Workflow progresses one event at a time (not polling/scanning)
/// **Transactional**: All state changes within a single ACID transaction **Concurrent-Safe**:
/// PostgreSQL advisory locks prevent race conditions **Incremental**: Applies one event to existing
/// state (not replaying all events)
///
/// ## Processing Phases
///
/// ### Phase 0: Event Filter
/// - Skip `ActivityScheduled` events (observability-only, not orchestration)
/// - Prevents duplicate scheduling and performance degradation
///
/// ### Phase 1: Transaction & Locking
/// **Purpose**: Ensure exclusive access to workflow state during evaluation
///
/// 1. Begin PostgreSQL transaction
/// 2. Acquire advisory lock: `pg_advisory_xact_lock(hash(workflow_id))`
///    - Serializes concurrent events for same workflow
///    - Automatically released on transaction commit/rollback
///    - Different workflows can process in parallel
///
/// ### Phase 2: Load Workflow Definition
/// **Purpose**: Get immutable workflow structure with precomputed metadata
///
/// - **Standard events**: Load via JOIN with workflows table
/// - **WorkflowCreated**: Load by definition_id from event payload (workflow row may not exist yet)
/// - Definition contains: activities, dependencies, loop metadata (`is_loop_activity`,
///   `is_back_edge`)
/// - **Performance**: Metadata precomputed at registration (O(1) lookups, not O(V+E) graph
///   traversal)
///
/// ### Phase 3: Load/Initialize Workflow State
/// **Purpose**: Get current execution state (activity statuses, outputs, iteration counters)
///
/// - **WorkflowCreated**: Initialize new state from definition + event payload
/// - **Other events**: Load materialized state from workflows table (O(1), not O(n) event replay)
/// - State includes: activity statuses, outputs, iteration counters, accumulated costs
///
/// ### Phase 4: Retry Logic (ActivityFailed Events Only)
/// **Purpose**: Handle activity failures with exponential backoff retry
///
/// 1. Check if activity has retry settings configured
/// 2. Check if max retry attempts exceeded
/// 3. If retriable:
///    - Increment attempt counter
///    - Calculate exponential backoff delay
///    - Publish `ActivityScheduled` event for retry
///    - Save state and **exit early** (don't mark as failed yet)
/// 4. If not retriable:
///    - Fall through to mark as permanently failed
///
/// ### Phase 5: Iteration Management (ActivityCompleted Events Only)
/// **Purpose**: Handle loop iteration for activities marked with `is_loop_activity = true`
///
/// **BEFORE applying event to state**:
/// 1. Check if activity is a loop activity (precomputed flag)
/// 2. If `iteration_scoped = true`:
///    - Extract outputs from event payload
///    - Archive to `iteration_outputs` map (grouped by name)
/// 3. Increment iteration counter for ALL loop activities (regardless of `iteration_scoped`)
/// 4. Iteration counter and outputs preserved across loop iterations
///
/// **Why before event application?**: Allows proper sequencing of archive → increment → apply
///
/// ### Phase 6: Apply Event to State
/// **Purpose**: Update workflow state based on event type
///
/// - `WorkflowCreated`: Set initial status
/// - `ActivityCompleted`: Update activity status, outputs, completion time, cost
/// - `ActivityFailed`: Update activity status, error message
/// - `ActivityScheduled`: Update activity status to Pending
/// - State mutations are incremental (not full replay)
///
/// ### Phase 7: Record LLM Costs (ActivityCompleted Events Only)
/// **Purpose**: Track token usage and costs for LLM activities
///
/// 1. Check if activity is LLM-based (`llm_prompt` or `embedding`)
/// 2. Extract usage metrics from event payload (prompt_tokens, completion_tokens, cost_usd)
/// 3. Insert into `llm_activity_costs` table for observability
/// 4. **Non-blocking**: Failures logged but don't fail workflow
///
/// ### Phase 8: Dependency Evaluation
/// **Purpose**: Find activities that are now ready to execute
///
/// 1. **For each activity** in definition:
///    - Check status gate (NotScheduled or Completed+loop)
///    - Check iteration limit (for loop activities)
///    - Evaluate dependencies:
///      - **Root activities** (no dependencies): Always ready
///      - **Forward dependencies**: Check completion + conditions
///      - **Back-edge dependencies**: Check loop conditions (iteration 0 auto-satisfied)
///    - Require ALL applicable dependencies satisfied (AND semantics)
/// 2. Return list of ready activities
///
/// **Performance**: O(D × C) per activity, no graph traversal (uses precomputed metadata)
///
/// ### Phase 9: Activity Scheduling
/// **Purpose**: Enqueue ready activities for worker execution
///
/// #### 9a. Template Resolution
/// For each ready activity:
/// 1. Build template context:
///    - Workflow inputs (`INPUT.*`)
///    - Activity outputs (iteration-scoped as arrays, non-scoped as single values)
///    - Current activity info (`ACTIVITY.iteration`, `ACTIVITY.accumulated_cost_usd`)
/// 2. Resolve parameter templates via MiniJinja:
///    - `{{INPUT.topic}}` → actual input value
///    - `{{search.results | last}}` → latest iteration result
///    - `{{ACTIVITY.iteration}}` → current iteration number
/// 3. Handle template errors → fail workflow
///
/// #### 9b. Budget Enrichment (LLM Activities Only)
/// For `llm_prompt` and `embedding` activities:
/// 1. Check accumulated cost vs budget limit
/// 2. If budget exceeded:
///    - Mark activity as Failed with "Budget exceeded" error
///    - Skip scheduling (don't send to queue)
/// 3. If budget OK:
///    - Enrich parameters with budget info
///    - Continue scheduling
///
/// #### 9c. Queue Scheduling
/// 1. Construct `Activity` objects with resolved parameters
/// 2. Include iteration number for loop activities
/// 3. Send to activity queue for worker consumption
/// 4. **Immediately** update state to `Pending` (prevents race if activity completes before event
///    processed)
/// 5. Initialize `iteration_outputs` map for first-time `iteration_scoped` activities
///
/// #### 9d. Event Publishing
/// For each scheduled activity:
/// 1. Publish `ActivityScheduled` event (observability)
/// 2. Include iteration number for loop activities (prevents unique constraint violations)
/// 3. **Note**: Orchestrator skips these events (Phase 0 filter)
///
/// ### Phase 10: Mark Skipped Activities
/// **Purpose**: Identify activities that can never execute due to unsatisfied conditions
///
/// 1. Find `NotScheduled` activities where:
///    - All dependencies are terminal (Completed/Failed/Skipped)
///    - No applicable dependencies (all conditions false)
/// 2. Mark as `Skipped` (terminal state)
/// 3. Allows workflow completion when conditional branches not taken
///
/// ### Phase 11: Workflow Completion Check
/// **Purpose**: Detect when workflow has reached terminal state
///
/// 1. Check if **all activities** are terminal (Completed/Failed/Skipped)
/// 2. If complete and **not already terminal**:
///    - Publish `WorkflowCompleted` or `WorkflowFailed` event
///    - Update workflow status in state
///    - Prevent duplicate completion events
///
/// ### Phase 12: State Persistence
/// **Purpose**: Atomically persist all state changes
///
/// 1. Save updated state to workflows table (materialized state column)
/// 2. Includes: activity statuses, outputs, iteration counters, accumulated costs
/// 3. **O(1) write**: Single UPDATE, not O(n) event inserts
///
/// ### Phase 13: Transaction Commit
/// **Purpose**: Make all changes visible and release lock
///
/// 1. Commit PostgreSQL transaction
/// 2. Advisory lock automatically released
/// 3. All state changes become visible to concurrent workflow events
///
/// ## Concurrency & Safety
///
/// **Advisory Locks**:
/// - Serialize events for same workflow (prevents race conditions)
/// - Different workflows process in parallel (no global lock)
/// - Transaction-scoped (automatic cleanup on error)
///
/// **Immediate State Updates**:
/// - Activities marked `Pending` immediately after scheduling
/// - Prevents race: activity completes before `ActivityScheduled` event processed
/// - Ensures state consistency
///
/// **Idempotency**:
/// - Events can be processed multiple times safely
/// - Status gates prevent duplicate scheduling
/// - Completion events only published once (terminal state check)
///
/// ## Loop Handling
///
/// **First Iteration** (iteration 0):
/// - Back-edge dependencies automatically satisfied
/// - Activity scheduled when forward dependencies met
/// - Iteration counter = 0
///
/// **Subsequent Iterations** (iteration 1+):
/// - ActivityCompleted → archive outputs → increment counter → apply event
/// - Back-edge conditions evaluated
/// - If conditions pass → re-schedule (loop back)
/// - If conditions fail or limit exceeded → don't schedule (exit loop)
///
/// **Iteration-Scoped Storage**:
/// - `iteration_scoped = true`: Outputs stored as arrays (one per iteration)
/// - `iteration_scoped = false`: Only latest output stored
/// - Template access: `{{activity.output}}` returns array or single value accordingly
///
/// ## Performance Characteristics
///
/// **Per Event**:
/// - Transaction overhead: ~1-5ms (network + lock)
/// - State load: O(1) (single SELECT)
/// - Dependency evaluation: O(A × D × C) where A=activities, D=dependencies, C=conditions
///   - No graph traversal (uses precomputed metadata)
///   - Typically <1ms for workflows with <100 activities
/// - Scheduling: O(R) where R=ready activities
/// - State save: O(1) (single UPDATE)
/// - Total: **~5-20ms** for typical workflows
///
/// **Scalability**:
/// - Different workflows process in parallel (advisory lock per workflow)
/// - No global bottlenecks
/// - Throughput: **1,000+ workflows/sec** (limited by database, not orchestration logic)
///
/// ## Error Handling
///
/// **Transaction Rollback**:
/// - Any error → transaction rolled back
/// - State changes discarded
/// - Advisory lock released
/// - Event can be retried (idempotent)
///
/// **Template Errors**:
/// - Invalid template syntax → workflow fails
/// - Error message includes activity key and template expression
///
/// **Budget Exceeded**:
/// - Activity marked as Failed
/// - Workflow can continue if other paths exist
///
/// **Cost Recording Failures**:
/// - Logged but don't fail workflow (non-critical)
#[tracing::instrument(
    skip(event, event_source, activity_queue, subscription_service, config),
    fields(
        workflow_id = %event.workflow_id,
        event_type = ?event.event_type
    )
)]
pub async fn process_workflow_event(
    event: &WorkflowEvent,
    event_source: &Arc<dyn EventSource>,
    activity_queue: &Arc<dyn ActivityQueue>,
    subscription_service: &Arc<dyn SubscriptionService>,
    config: &OrchestratorConfig,
) -> Result<()> {
    // Skip ActivityScheduled and ActivityWaiting events - they are for observability only, not orchestration
    // Processing them causes duplicate scheduling and performance issues
    if event.event_type == WorkflowEventType::ActivityScheduled
        || event.event_type == WorkflowEventType::ActivityWaiting
    {
        tracing::trace!(
            "Skipping {:?} event for workflow {}",
            event.event_type,
            event.workflow_id
        );
        return Ok(());
    }

    let event_start = std::time::Instant::now();

    // Begin transaction with workflow-level advisory lock
    let tx_start = std::time::Instant::now();
    let tx_span = profile_span!("begin_transaction");
    let mut tx = {
        let _enter = tx_span.enter();
        config.pool.begin().await?
    };
    tracing::trace!(
        "Transaction started in {:?} for workflow {}",
        tx_start.elapsed(),
        event.workflow_id
    );

    // Acquire exclusive lock for this workflow (prevents concurrent evaluation)
    // Uses hash of workflow_id to get a 64-bit integer for pg_advisory_xact_lock
    let lock_start = std::time::Instant::now();
    let lock_span = profile_span!("acquire_advisory_lock");
    {
        let _enter = lock_span.enter();
        sqlx::query!(
            "SELECT pg_advisory_xact_lock(hashtext($1::text))",
            event.workflow_id.to_string()
        )
        .execute(&mut *tx)
        .await?;
    }
    tracing::trace!(
        "Advisory lock acquired in {:?} for workflow {}",
        lock_start.elapsed(),
        event.workflow_id
    );

    tracing::debug!(
        "Processing event {} for workflow {}",
        event.event_type,
        event.workflow_id
    );

    // 1. Load workflow definition
    let def_start = std::time::Instant::now();
    let definition_span = profile_span!("load_workflow_definition");
    let definition = {
        let _enter = definition_span.enter();
        // For WorkflowCreated events, load definition by ID from event payload
        // (workflow row might not exist yet in test scenarios)
        if event.event_type == WorkflowEventType::WorkflowCreated {
            if let Some(definition_id) = event.payload.get("workflow_definition_id") {
                if let Some(definition_id_str) = definition_id.as_str() {
                    let def_uuid = uuid::Uuid::parse_str(definition_id_str).map_err(|e| {
                        super::OrchestratorError::WorkflowDefinitionNotFound(format!(
                            "Invalid workflow_definition_id: {}",
                            e
                        ))
                    })?;
                    load_workflow_definition_by_id(&mut tx, def_uuid).await?
                } else {
                    load_workflow_definition(&mut tx, event.workflow_id).await?
                }
            } else {
                // Fallback to normal join-based loading (production path)
                load_workflow_definition(&mut tx, event.workflow_id).await?
            }
        } else {
            load_workflow_definition(&mut tx, event.workflow_id).await?
        }
    };
    tracing::trace!(
        "Workflow definition loaded in {:?} for workflow {}",
        def_start.elapsed(),
        event.workflow_id
    );

    // 2. Load materialized state from workflows table (O(1), not O(n))
    //    Special case: WorkflowCreated needs to initialize state first
    let state_start = std::time::Instant::now();
    let state_span = profile_span!("load_workflow_state");
    let mut state = {
        let _enter = state_span.enter();
        if event.event_type == WorkflowEventType::WorkflowCreated {
            // Initialize new workflow state
            let initial_state_data = event.payload.get("state_data").cloned();

            // Extract workflow_definition_id and input from payload
            let workflow_definition_id = event
                .payload
                .get("workflow_definition_id")
                .and_then(|v| v.as_str())
                .and_then(|s| uuid::Uuid::parse_str(s).ok());

            let input = event.payload.get("input").cloned();

            initialize_workflow_state(
                &mut tx,
                event.workflow_id,
                &definition,
                initial_state_data,
                workflow_definition_id,
                input,
            )
            .await?
        } else {
            // Load existing materialized state
            load_materialized_state(&mut tx, event.workflow_id).await?
        }
    };
    tracing::trace!(
        "Workflow state loaded in {:?} for workflow {}",
        state_start.elapsed(),
        event.workflow_id
    );

    // 3. Handle ActivityFailed event with retry logic BEFORE applying to state
    if event.event_type == WorkflowEventType::ActivityFailed {
        // Only process if the activity is in a state where failure makes sense.
        // A duplicate event (e.g., from event replay) should not trigger another
        // retry or re-fail an activity that has already moved on.
        let activity_key = event
            .activity_key
            .as_ref()
            .ok_or_else(|| super::OrchestratorError::MissingActivityKey)?;
        let current_status = state.activities.get(activity_key).map(|a| a.status);

        match current_status {
            Some(
                WorkflowActivityStatus::Running
                | WorkflowActivityStatus::Waiting
                | WorkflowActivityStatus::Pending,
            ) => {
                // Expected states for a failure event — proceed with retry logic
            }
            Some(status) => {
                tracing::debug!(
                    activity_key = %activity_key,
                    status = ?status,
                    "Ignoring duplicate ActivityFailed event, activity is not running or waiting"
                );
                tx.commit().await?;
                return Ok(());
            }
            None => {
                // Activity not in state — shouldn't happen, but let it fall through
                // to apply_event_to_state which will handle the missing key
            }
        }

        let retry_start = std::time::Instant::now();
        let retry_handled = handle_activity_failed(
            &mut state,
            &definition,
            event,
            event_source,
            activity_queue,
            &config.pool,
            &config.secrets,
        )
        .await?;

        if retry_handled {
            // Activity will be retried - save state and exit early
            tracing::trace!(
                "Activity retry scheduled in {:?} for workflow {}",
                retry_start.elapsed(),
                event.workflow_id
            );

            save_materialized_state(&mut tx, event.workflow_id, &state).await?;
            tx.commit().await?;
            return Ok(());
        }
        // Otherwise, fall through to mark as permanently failed
    }

    // 3. Apply THIS event to update state incrementally (just 1 event, not n events)
    let apply_start = std::time::Instant::now();
    let apply_span = profile_span!("apply_event_to_state");

    // Handle iteration management BEFORE applying event for ActivityCompleted
    // This allows us to archive outputs and increment iteration counter
    if event.event_type == WorkflowEventType::ActivityCompleted
        && let Some(activity_key) = &event.activity_key
        && let Some(activity_def) = definition
            .activities
            .iter()
            .find(|a| &a.key == activity_key)
        && let Some(activity_state) = state.activities.get_mut(activity_key)
    {
        // Check if this is a loop activity (precomputed during validation)
        if activity_def.is_loop_activity {
            tracing::debug!(
                "Activity {} is a loop activity (iteration={})",
                activity_key,
                activity_state.iteration
            );

            // Archive outputs BEFORE incrementing iteration
            // Only for iteration_scoped activities
            if activity_def.iteration_scoped
                // Get outputs from event payload and convert to Vec<ActivityOutput>
                && let Some(outputs) = event.payload.get("outputs") 
                && let serde_json::Value::Object(outputs_map) = outputs
            {
                let current_outputs: Vec<crate::workflow::ActivityOutput> = outputs_map
                    .iter()
                    .map(|(name, value)| {
                        crate::workflow::ActivityOutput::value(name.clone(), value.clone())
                    })
                    .collect();

                activity_state.archive_iteration_outputs(current_outputs);

                tracing::debug!(
                    "Archived iteration {} outputs for {}",
                    activity_state.iteration,
                    activity_key
                );
            }

            // Increment iteration counter for ALL looping activities
            activity_state.increment_iteration();

            tracing::debug!(
                "Incremented iteration counter for {} to {}",
                activity_key,
                activity_state.iteration
            );
        }
    }

    {
        let _enter = apply_span.enter();
        apply_event_to_state(&mut state, event)?;
    }
    tracing::trace!(
        "Event applied to state in {:?} for workflow {}",
        apply_start.elapsed(),
        event.workflow_id
    );

    // 3.5. Record costs for completed LLM activities
    if event.event_type == WorkflowEventType::ActivityCompleted {
        let cost_start = std::time::Instant::now();
        if let Some(activity_key) = &event.activity_key
            // Get activity definition to check if it's an LLM activity
            && let Some(activity_def) = definition
                .activities
                .iter()
                .find(|a| &a.key == activity_key)
        {
            // Check if this is an LLM activity (llm_prompt or embedding)
            if (activity_def.activity_name.as_deref() == Some("llm_prompt")
                || activity_def.activity_name.as_deref() == Some("embedding"))
                && let Err(e) =
                    record_llm_activity_cost(&mut tx, event, &state, activity_def, config).await
            {
                tracing::error!(
                    workflow_id = %event.workflow_id,
                    activity_key = %activity_key,
                    error = %e,
                    "Failed to record LLM activity cost"
                );
                // Don't fail the workflow, just log the error
            }
        }

        tracing::trace!(
            "Cost recording completed in {:?} for workflow {}",
            cost_start.elapsed(),
            event.workflow_id
        );
    }

    // 3.6. Early exit for terminal workflow states
    // If workflow is already Completed or Failed, skip all scheduling logic
    // This prevents errors in scheduling from blocking workflow state persistence
    // and avoids scheduling activities for workflows that should not run anymore
    let is_terminal = matches!(
        state.status,
        WorkflowStatus::Completed | WorkflowStatus::Failed
    );

    if is_terminal {
        tracing::info!(
            workflow_id = %event.workflow_id,
            workflow_name = %state.definition_name,
            status = %state.status,
            "Workflow in terminal state, skipping activity scheduling"
        );

        // Save the terminal state and return early
        save_materialized_state(&mut tx, event.workflow_id, &state).await?;
        tx.commit().await?;
        return Ok(());
    }

    // 4. Find ready activities (using updated state)
    let eval_start = std::time::Instant::now();
    let eval_span = profile_span!(
        "evaluate_dependencies",
        num_activities = state.activities.len()
    );
    let ready_activities = {
        let _enter = eval_span.enter();

        // Log activity state distribution for observability
        let mut state_counts = std::collections::HashMap::new();
        for activity in state.activities.values() {
            *state_counts.entry(&activity.status).or_insert(0) += 1;
        }
        tracing::debug!(
            "Activity state distribution: not_scheduled={}, pending={}, completed={}, failed={}",
            state_counts
                .get(&WorkflowActivityStatus::NotScheduled)
                .unwrap_or(&0),
            state_counts
                .get(&WorkflowActivityStatus::Pending)
                .unwrap_or(&0),
            state_counts
                .get(&WorkflowActivityStatus::Completed)
                .unwrap_or(&0),
            state_counts
                .get(&WorkflowActivityStatus::Failed)
                .unwrap_or(&0)
        );

        find_ready_activities(&definition, &state)?
    };
    tracing::trace!(
        "Dependencies evaluated in {:?} for workflow {} ({} ready)",
        eval_start.elapsed(),
        event.workflow_id,
        ready_activities.len()
    );

    tracing::debug!(
        "Found {} ready activities for workflow {}",
        ready_activities.len(),
        event.workflow_id
    );

    // 5. Schedule ready activities to queue
    if !ready_activities.is_empty() {
        let schedule_start = std::time::Instant::now();
        tracing::debug!(
            "Scheduling {} activities for workflow {}: [{}]",
            ready_activities.len(),
            event.workflow_id,
            ready_activities
                .iter()
                .map(|a| a.key.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );

        let schedule_span = profile_span!("schedule_activities", count = ready_activities.len());
        {
            let _enter = schedule_span.enter();

            // Build template context for resolving parameters
            let context_span = profile_span!("build_template_context");
            let template_context = {
                let _enter = context_span.enter();
                build_template_context(&state, event.workflow_id, &config.secrets)
            };

            // Resolve templates in activity parameters
            let mut activities_to_schedule = Vec::new();
            for a in &ready_activities {
                // Resolve template expressions in parameters
                let params_value =
                    serde_json::to_value(a.parameters.as_ref().unwrap_or(&Default::default()))
                        .map_err(|e| {
                            super::OrchestratorError::TemplateFailed(format!(
                                "Failed to serialize parameters: {}",
                                e
                            ))
                        })?;

                // Debug: Log input parameters before resolution
                if a.key == "store_passages" {
                    if let Some(obj) = params_value.as_object() {
                        tracing::info!(
                            activity = %a.key,
                            param_keys = ?obj.keys().collect::<Vec<_>>(),
                            has_embeddings_file_key = obj.contains_key("embeddings_file"),
                            embeddings_file_template = ?obj.get("embeddings_file"),
                            "Input parameters BEFORE template resolution"
                        );
                    }
                }

                // Debug: Log dependency activity outputs for template resolution
                for dep in a.depends_on.iter().flatten() {
                    if let Some(dep_state) = state.activities.get(&dep.activity_key) {
                        if let Some(outputs) = &dep_state.outputs {
                            let output_names: Vec<_> = outputs.iter().map(|o| &o.name).collect();
                            tracing::info!(
                                activity = %a.key,
                                dependency = %dep.activity_key,
                                output_names = ?output_names,
                                "Template context: dependency outputs"
                            );
                            // Log the actual output values for debugging
                            for output in outputs {
                                tracing::info!(
                                    activity = %a.key,
                                    dependency = %dep.activity_key,
                                    output_name = %output.name,
                                    output_value_type = ?output.value.as_object().map(|o| o.keys().collect::<Vec<_>>()).unwrap_or_default(),
                                    output_value_preview = %format!("{:.200}", output.value),
                                    "Template context: output detail"
                                );
                            }
                        }
                    }
                }

                let resolved_params = match resolve_template_value(&params_value, &template_context)
                {
                    Ok(resolved) => {
                        // Debug: Log resolved embeddings-related fields
                        if a.key == "store_passages" {
                            if let Some(obj) = resolved.as_object() {
                                tracing::info!(
                                    activity = %a.key,
                                    embeddings_type = ?obj.get("embeddings").map(|v| match v {
                                        serde_json::Value::Null => "null",
                                        serde_json::Value::Array(_) => "array",
                                        _ => "other",
                                    }),
                                    embeddings_file = ?obj.get("embeddings_file"),
                                    "Resolved store_passages parameters"
                                );
                            }
                        }
                        resolved
                    }
                    Err(e) => {
                        tracing::error!("Template resolution failed for activity {}: {}", a.key, e);
                        return Err(super::OrchestratorError::TemplateFailed(format!(
                            "Failed to resolve templates in activity {}: {}",
                            a.key, e
                        )));
                    }
                };

                // Enrich LLM activity parameters with budget information
                let activity_name_str = a.activity_name.as_deref().unwrap_or("");
                let (params_w_budget, should_schedule) = enrich_llm_activity_params_w_budget(
                    activity_name_str,
                    resolved_params,
                    a,
                    &definition,
                    &a.key,
                    event.workflow_id,
                    &config.pool,
                )
                .await?;

                // Only schedule if budget check passed
                if !should_schedule {
                    tracing::warn!("Skipping LLM activity {} due to budget constraint", a.key);
                    // Mark activity as failed due to budget
                    if let Some(activity_state) = state.activities.get_mut(&a.key) {
                        activity_state.status = WorkflowActivityStatus::Failed;
                        activity_state.set_error("Budget exceeded before execution".to_string());
                    }
                    continue;
                }

                // Compute iteration for loop activities
                let iteration = if a.is_loop_activity {
                    state.activities.get(&a.key).map(|s| s.iteration as i32)
                } else {
                    None
                };

                // Check if activity has wait_for_signal setting
                if let Some(ref wait_settings) =
                    a.settings.as_ref().and_then(|s| s.wait_for_signal.as_ref())
                {
                    // Activity needs to wait for an external signal before executing
                    // Create a subscription instead of scheduling to queue
                    let new_sub = NewSubscription {
                        workflow_id: event.workflow_id,
                        activity_key: a.key.clone(),
                        event_name: wait_settings.event_name.clone(),
                        on_timeout: wait_settings.on_timeout.clone(),
                        timeout_seconds: wait_settings.timeout_seconds,
                    };

                    match subscription_service.create_subscription(new_sub).await {
                        Ok(sub_id) => {
                            tracing::info!(
                                workflow_id = %event.workflow_id,
                                activity_key = %a.key,
                                event_name = %wait_settings.event_name,
                                timeout_seconds = wait_settings.timeout_seconds,
                                subscription_id = %sub_id,
                                "Activity now waiting for signal"
                            );

                            // Mark activity as Waiting in state
                            if let Some(activity_state) = state.activities.get_mut(&a.key) {
                                activity_state.status = WorkflowActivityStatus::Waiting;
                                activity_state.started_at = Some(chrono::Utc::now());
                            }

                            // Publish ActivityWaiting event
                            let waiting_event = NewWorkflowEvent {
                                workflow_id: event.workflow_id,
                                event_type: WorkflowEventType::ActivityWaiting,
                                activity_key: Some(a.key.clone()),
                                payload: json!({
                                    "event_name": wait_settings.event_name,
                                    "timeout_seconds": wait_settings.timeout_seconds,
                                    "on_timeout": format!("{:?}", wait_settings.on_timeout),
                                }),
                                iteration,
                            };
                            event_source.publish(waiting_event).await?;
                        }
                        Err(e) => {
                            tracing::error!(
                                workflow_id = %event.workflow_id,
                                activity_key = %a.key,
                                "Failed to create subscription: {}",
                                e
                            );
                            // Mark activity as failed
                            if let Some(activity_state) = state.activities.get_mut(&a.key) {
                                activity_state.status = WorkflowActivityStatus::Failed;
                                activity_state.set_error(format!(
                                    "Failed to create signal subscription: {}",
                                    e
                                ));
                            }
                        }
                    }
                    continue;
                }

                // Compute scheduled_for based on activity settings
                let scheduled_for = compute_scheduled_for(a, &template_context, iteration)?;

                // Get signal_data from activity state if present
                let signal_data = state
                    .activities
                    .get(&a.key)
                    .and_then(|s| s.signal_data.clone());

                activities_to_schedule.push(Activity {
                    key: a.key.clone(),
                    worker: a.worker.clone(),
                    activity_name: a.activity_name.clone().unwrap_or_default(),
                    parameters: params_w_budget,
                    settings: a.settings.clone(),
                    scheduled_for,
                    output_definitions: a.output_definitions.clone(),
                    iteration,
                    signal_data,
                });
            }

            // Log info when activities are started
            for activity in &activities_to_schedule {
                tracing::info!(
                    workflow_id = %event.workflow_id,
                    activity_key = %activity.key,
                    worker = %activity.worker,
                    activity_name = %activity.activity_name,
                    iteration = ?activity.iteration,
                    "Activity started"
                );
            }

            activity_queue
                .schedule(event.workflow_id, activities_to_schedule)
                .await?;

            // Update state immediately to mark activities as Pending
            // This prevents race condition where activity completes before ActivityScheduled event is processed
            // (Only for activities that were actually scheduled, not those entering Waiting state)
            for activity in &ready_activities {
                if let Some(activity_state) = state.activities.get_mut(&activity.key) {
                    // Skip activities that entered Waiting state (already handled above)
                    if activity_state.status == WorkflowActivityStatus::Waiting {
                        continue;
                    }
                    // Skip activities that failed (e.g., budget or subscription error)
                    if activity_state.status == WorkflowActivityStatus::Failed {
                        continue;
                    }

                    // Check if this is a loop-back (re-execution)
                    let is_loop_back = activity_state.status == WorkflowActivityStatus::Completed;

                    if is_loop_back {
                        tracing::debug!(
                            "Activity {} is looping back (iteration={})",
                            activity.key,
                            activity_state.iteration
                        );

                        // Reset status for next iteration
                        // Note: iteration counter and iteration_outputs are preserved
                        activity_state.status = WorkflowActivityStatus::Pending;
                        activity_state.started_at = Some(chrono::Utc::now());
                        activity_state.completed_at = None;
                        // Don't reset outputs - they're either in iteration_outputs (iteration_scoped)
                        // or will be overwritten on next completion (non-iteration_scoped)
                    } else {
                        // First execution
                        activity_state.status = WorkflowActivityStatus::Pending;
                        activity_state.started_at = Some(chrono::Utc::now());

                        // Initialize iteration_outputs if iteration_scoped
                        if activity.iteration_scoped && activity_state.iteration_outputs.is_none() {
                            activity_state.iteration_outputs =
                                Some(std::collections::HashMap::new());
                            tracing::debug!(
                                "Initialized iteration_outputs for iteration_scoped activity {}",
                                activity.key
                            );
                        }
                    }
                }
            }

            // Publish ActivityScheduled events (for external observers)
            // Note: Orchestrator skips these events to avoid duplicate processing
            for activity in &ready_activities {
                // Skip activities that entered Waiting state (they got ActivityWaiting events instead)
                if let Some(activity_state) = state.activities.get(&activity.key) {
                    if activity_state.status == WorkflowActivityStatus::Waiting {
                        continue;
                    }
                    if activity_state.status == WorkflowActivityStatus::Failed {
                        continue;
                    }
                }

                tracing::debug!(
                    "Publishing ActivityScheduled event for {} in workflow {}",
                    activity.key,
                    event.workflow_id
                );

                // For looping activities, include iteration to avoid unique constraint violation
                let iteration = if activity.is_loop_activity {
                    state
                        .activities
                        .get(&activity.key)
                        .map(|s| s.iteration as i32)
                } else {
                    None
                };

                let scheduled_event = NewWorkflowEvent {
                    workflow_id: event.workflow_id,
                    event_type: WorkflowEventType::ActivityScheduled,
                    activity_key: Some(activity.key.clone()),
                    payload: json!({
                        "worker": activity.worker,
                        "activity_name": activity.activity_name,
                    }),
                    iteration,
                };
                event_source.publish(scheduled_event).await?;
            }
        }
        tracing::trace!(
            "Activities scheduled and events published in {:?} for workflow {}",
            schedule_start.elapsed(),
            event.workflow_id
        );
    } else {
        tracing::debug!(
            "No activities ready to schedule for workflow {} (event: {:?})",
            event.workflow_id,
            event.event_type
        );
    }

    // 5.5. Mark activities as Skipped if they can never be scheduled
    let skipped_activities = find_skipped_activities(&definition, &state)?;
    if !skipped_activities.is_empty() {
        tracing::debug!(
            "Marking {} activities as skipped for workflow {}: [{}]",
            skipped_activities.len(),
            event.workflow_id,
            skipped_activities
                .iter()
                .map(|a| a.key.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );

        for activity in skipped_activities {
            if let Some(activity_state) = state.activities.get_mut(&activity.key) {
                activity_state.status = WorkflowActivityStatus::Skipped;
            }
        }
    }

    // 6. Check for workflow completion
    // Only publish completion event if workflow is not already in a terminal state
    // This prevents publishing WorkflowCompleted/WorkflowFailed multiple times
    let is_terminal_state = matches!(
        state.status,
        WorkflowStatus::Completed | WorkflowStatus::Failed
    );

    if is_workflow_complete(&state) && !is_terminal_state {
        tracing::debug!(
            "Workflow {} completing with final status",
            event.workflow_id
        );

        let completion_event = if is_workflow_failed(&state) {
            NewWorkflowEvent {
                workflow_id: event.workflow_id,
                event_type: WorkflowEventType::WorkflowFailed,
                activity_key: None,
                payload: json!({
                    "reason": "One or more activities failed",
                }),
                iteration: None,
            }
        } else {
            NewWorkflowEvent {
                workflow_id: event.workflow_id,
                event_type: WorkflowEventType::WorkflowCompleted,
                activity_key: None,
                payload: json!({}),
                iteration: None,
            }
        };

        event_source.publish(completion_event).await?;

        // Update workflow status in memory (will be persisted by save_materialized_state)
        let new_status = if is_workflow_failed(&state) {
            WorkflowStatus::Failed
        } else {
            WorkflowStatus::Completed
        };

        state.status = new_status;

        // Log workflow completion at info level
        tracing::info!(
            workflow_id = %event.workflow_id,
            workflow_name = %state.definition_name,
            status = %new_status,
            "Workflow completed"
        );
    }

    // 7. Save updated materialized state back to workflows table
    let save_start = std::time::Instant::now();
    let save_span = profile_span!("save_workflow_state");
    {
        let _enter = save_span.enter();
        save_materialized_state(&mut tx, event.workflow_id, &state).await?;
    }
    tracing::trace!(
        "Workflow state saved in {:?} for workflow {}",
        save_start.elapsed(),
        event.workflow_id
    );

    // Commit transaction (releases advisory lock automatically)
    let commit_start = std::time::Instant::now();
    let commit_span = profile_span!("commit_transaction");
    {
        let _enter = commit_span.enter();
        tx.commit().await?;
    }
    tracing::trace!(
        "Transaction committed in {:?} for workflow {}",
        commit_start.elapsed(),
        event.workflow_id
    );

    tracing::trace!(
        "Total event processing time: {:?} for workflow {} (event: {:?})",
        event_start.elapsed(),
        event.workflow_id,
        event.event_type
    );

    Ok(())
}

/// Enrich LLM activity parameters with budget information before scheduling
/// Returns enriched parameters and whether to proceed with scheduling
async fn enrich_llm_activity_params_w_budget(
    activity_name: &str,
    mut params: serde_json::Value,
    activity_def: &crate::workflow::ActivityDefinition,
    _workflow_def: &crate::workflow::WorkflowDefinition,
    activity_key: &str,
    workflow_id: uuid::Uuid,
    pool: &sqlx::PgPool,
) -> Result<(serde_json::Value, bool)> {
    // Only process LLM activities
    if activity_name != "llm_prompt" && activity_name != "embedding" {
        return Ok((params, true)); // Not an LLM activity, proceed without enrichment
    }

    // Extract model list from parameters and parse into (provider, model) tuples
    let model_strings: Vec<String> = match params.get("model") {
        Some(serde_json::Value::String(single_model)) => vec![single_model.clone()],
        Some(serde_json::Value::Array(model_array)) => model_array
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => {
            // No model specified, cannot enrich
            return Ok((params, true));
        }
    };

    if model_strings.is_empty() {
        return Ok((params, true));
    }

    // Parse model strings into (provider, model) tuples
    let model_list: Vec<(String, String)> = model_strings
        .iter()
        .filter_map(|model_str| {
            let parts: Vec<&str> = model_str.split('/').collect();
            if parts.len() == 2 {
                Some((parts[0].to_string(), parts[1].to_string()))
            } else {
                tracing::warn!("Invalid model format: {}", model_str);
                None
            }
        })
        .collect();

    if model_list.is_empty() {
        return Ok((params, true));
    }

    tracing::debug!(
        "Enriching LLM activity {} with budget info for models: {:?}",
        activity_key,
        model_strings
    );

    // Extract budget limits from activity settings
    // Note: Workflow-level budget settings not yet implemented (WorkflowDefinition has no settings field)
    let activity_budget_limit = activity_def
        .settings
        .as_ref()
        .and_then(|s| s.budget.as_ref())
        .map(|b| b.limit);

    let workflow_budget_limit: Option<Decimal> = None; // TODO: Add workflow-level budget support

    // Query pricing for all models in batch
    let cost_calculator = CostCalculator::new(pool.clone());
    let model_pricing = cost_calculator
        .batch_get_pricing(&model_list)
        .await
        .map_err(|e| {
            super::OrchestratorError::CostTrackingFailed(format!("Failed to get pricing: {}", e))
        })?;

    // Get cumulative cost for this activity (across retry attempts)
    let cost_tracker = CostTracker::new(pool.clone());
    let budget_status = cost_tracker
        .get_budget_status(
            workflow_id,
            activity_key,
            activity_budget_limit,
            workflow_budget_limit,
        )
        .await
        .map_err(|e| {
            super::OrchestratorError::CostTrackingFailed(format!(
                "Failed to get budget status: {}",
                e
            ))
        })?;
    let cumulative_cost = budget_status.activity_cost;

    // Pre-execution abort check (only if budget enforcement action is Abort)
    let budget_action = activity_def
        .settings
        .as_ref()
        .and_then(|s| s.budget.as_ref())
        .map(|b| b.action.clone());

    if budget_action == Some(BudgetAction::Abort) {
        // Check if we should abort before scheduling
        // Find the cheapest model in the chain
        let mut cheapest_estimate = None;
        for (provider, model) in &model_list {
            let model_key = format!("{}/{}", provider, model);
            if let Some(pricing) = model_pricing.get(&model_key) {
                // Conservative estimate: assume 1000 input tokens and max_tokens output
                let estimated_input_tokens = 1000;
                let estimated_output_tokens = params
                    .get("max_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(4096);

                let input_cost = Decimal::from(estimated_input_tokens)
                    * pricing.input_price_per_million
                    / Decimal::from(1_000_000);
                let output_cost = Decimal::from(estimated_output_tokens)
                    * pricing.output_price_per_million
                    / Decimal::from(1_000_000);
                let estimate = input_cost + output_cost;

                if cheapest_estimate.is_none() || estimate < cheapest_estimate.unwrap() {
                    cheapest_estimate = Some(estimate);
                }
            }
        }

        // Check against budget limits
        if let Some(estimate) = cheapest_estimate {
            let effective_limit = match (activity_budget_limit, workflow_budget_limit) {
                (Some(a), Some(w)) => Some(if a < w { a } else { w }),
                (Some(a), None) => Some(a),
                (None, Some(w)) => Some(w),
                (None, None) => None,
            };

            if let Some(limit) = effective_limit
                && cumulative_cost + estimate > limit
            {
                tracing::warn!(
                    "Aborting LLM activity {}: cheapest model estimate ${:.6} + cumulative ${:.6} exceeds budget ${:.6}",
                    activity_key,
                    estimate,
                    cumulative_cost,
                    limit
                );
                return Ok((params, false)); // Don't schedule
            }
        }
    }

    // Enrich parameters with budget information
    if let Some(obj) = params.as_object_mut() {
        obj.insert(
            "model_pricing".to_string(),
            serde_json::to_value(&model_pricing)?,
        );

        if let Some(limit) = activity_budget_limit {
            obj.insert(
                "activity_budget_limit_usd".to_string(),
                serde_json::to_value(limit)?,
            );
        }

        if let Some(limit) = workflow_budget_limit {
            obj.insert(
                "workflow_budget_limit_usd".to_string(),
                serde_json::to_value(limit)?,
            );
        }

        obj.insert(
            "cumulative_activity_cost_usd".to_string(),
            serde_json::to_value(cumulative_cost)?,
        );
    }

    Ok((params, true))
}

/// Record LLM activity cost after completion
async fn record_llm_activity_cost(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event: &WorkflowEvent,
    state: &super::workflow_state::WorkflowState,
    activity_def: &crate::workflow::ActivityDefinition,
    config: &OrchestratorConfig,
) -> Result<()> {
    let activity_key = event
        .activity_key
        .as_ref()
        .ok_or_else(|| super::OrchestratorError::MissingActivityKey)?;

    // Extract usage information from activity outputs
    let usage = event
        .payload
        .get("outputs")
        .and_then(|o| o.get("result"))
        .and_then(|r| r.get("usage"));

    if usage.is_none() {
        tracing::debug!(
            "No usage information found for LLM activity {}",
            activity_key
        );
        return Ok(());
    }

    let usage = usage.unwrap();

    // Extract token counts
    let prompt_tokens = usage
        .get("prompt_tokens")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);
    let output_tokens = usage
        .get("output_tokens")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);
    let cached_tokens = usage
        .get("cached_tokens")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    // Extract provider and model
    let provider = event
        .payload
        .get("outputs")
        .and_then(|o| o.get("result"))
        .and_then(|r| r.get("provider"))
        .and_then(|p| p.as_str())
        .ok_or_else(|| {
            super::OrchestratorError::InvalidEvent(
                "Missing provider in activity output".to_string(),
            )
        })?;

    let model = event
        .payload
        .get("outputs")
        .and_then(|o| o.get("result"))
        .and_then(|r| r.get("model"))
        .and_then(|m| m.as_str())
        .ok_or_else(|| {
            super::OrchestratorError::InvalidEvent("Missing model in activity output".to_string())
        })?;

    // Calculate cost using CostCalculator
    let cost_calculator = CostCalculator::new(config.pool.clone());
    let cost = cost_calculator
        .calculate_llm_cost(
            provider,
            model,
            prompt_tokens.unwrap_or(0),
            output_tokens.unwrap_or(0),
            cached_tokens,
        )
        .await
        .map_err(|e| super::OrchestratorError::CostTrackingFailed(e.to_string()))?;

    // Get activity attempt number
    let attempt = state
        .activities
        .get(activity_key)
        .map(|a| a.attempt)
        .unwrap_or(1);

    // Get budget limits from activity settings and workflow
    let activity_limit = activity_def
        .settings
        .as_ref()
        .and_then(|s| s.budget.as_ref())
        .map(|b| b.limit);

    // Get workflow budget limit from database
    let workflow_limit = sqlx::query_scalar!(
        "SELECT budget_limit_usd FROM workflows WHERE id = $1",
        event.workflow_id
    )
    .fetch_one(&mut **tx)
    .await?;

    // Get budget action from activity settings
    let budget_action = activity_def
        .settings
        .as_ref()
        .and_then(|s| s.budget.as_ref())
        .map(|b| match b.action {
            BudgetAction::Abort => "abort",
            BudgetAction::Continue => "continue",
        });

    // Record the cost
    let cost_tracker = CostTracker::new(config.pool.clone());
    let record = ActivityCostRecord {
        workflow_id: event.workflow_id,
        activity_key: activity_key.clone(),
        attempt,
        cost_usd: cost,
        estimated_cost_usd: None, // We don't have estimated cost at completion time
        prompt_tokens,
        output_tokens,
        total_tokens: Some(prompt_tokens.unwrap_or(0) + output_tokens.unwrap_or(0)),
        cached_tokens,
        provider: provider.to_string(),
        model: model.to_string(),
        activity_budget_limit_usd: activity_limit,
        workflow_budget_limit_usd: workflow_limit,
        budget_exceeded: false, // Always false for completed activities
        budget_action: budget_action.map(String::from),
    };

    cost_tracker
        .record_cost(record)
        .await
        .map_err(|e| super::OrchestratorError::CostTrackingFailed(e.to_string()))?;

    tracing::info!(
        workflow_id = %event.workflow_id,
        activity_key = %activity_key,
        cost_usd = %cost,
        provider = %provider,
        model = %model,
        "Recorded LLM activity cost"
    );

    Ok(())
}

/// Publish a failure event when a poison message is detected
///
/// When the orchestrator cannot process an event (typically ActivityCompleted)
/// after multiple retries, this publishes an ActivityFailed event so the workflow
/// can continue processing with the activity marked as failed.
///
/// This is better than failing the entire workflow - other branches may still succeed.
async fn publish_failure_for_poison_event(
    event: &WorkflowEvent,
    event_source: &Arc<dyn EventSource>,
    error: &str,
) -> Result<()> {
    // Only handle events that relate to specific activities
    let activity_key = match &event.activity_key {
        Some(key) => key.clone(),
        None => {
            tracing::warn!(
                workflow_id = %event.workflow_id,
                event_id = %event.id,
                event_type = ?event.event_type,
                "Cannot publish ActivityFailed for poison event - no activity_key (event type: {:?})",
                event.event_type
            );
            return Ok(());
        }
    };

    tracing::warn!(
        workflow_id = %event.workflow_id,
        event_id = %event.id,
        event_type = ?event.event_type,
        activity_key = %activity_key,
        "Publishing ActivityFailed event for poison message"
    );

    // Publish ActivityFailed event with orchestrator error
    let failed_event = NewWorkflowEvent {
        workflow_id: event.workflow_id,
        event_type: WorkflowEventType::ActivityFailed,
        activity_key: Some(activity_key.clone()),
        payload: json!({
            "error": format!("ORCHESTRATOR_ERROR: Failed to process {} event after {} attempts: {}",
                event.event_type, MAX_EVENT_PROCESSING_RETRIES, error),
            "error_code": "ORCHESTRATOR_PROCESSING_ERROR",
            "poison_event_id": event.id,
            "original_event_type": format!("{:?}", event.event_type),
        }),
        iteration: event.iteration,
    };

    event_source.publish(failed_event).await?;

    tracing::info!(
        workflow_id = %event.workflow_id,
        activity_key = %activity_key,
        "Published ActivityFailed event for poison message - workflow will process failure normally"
    );

    Ok(())
}

/// Background task to check for stuck workflows and stale activities
async fn timeout_checker_task(
    config: OrchestratorConfig,
    event_source: Arc<dyn EventSource>,
    activity_queue: Arc<dyn ActivityQueue>,
    subscription_service: Arc<dyn SubscriptionService>,
    shutdown_token: Option<CancellationToken>,
) {
    tracing::info!(
        "Timeout checker starting (check_interval={}s, workflow_timeout={}s)",
        config.timeout_check_interval.as_secs(),
        config.workflow_timeout.as_secs()
    );

    loop {
        // Check if shutdown has been requested
        if let Some(ref token) = shutdown_token
            && token.is_cancelled()
        {
            tracing::info!("Shutdown requested, timeout checker stopping");
            return;
        }

        // Sleep for check interval
        tokio::time::sleep(config.timeout_check_interval).await;

        // Check for stuck workflows (workflow-level timeout)
        if let Err(e) = check_and_timeout_stuck_workflows(&config, &event_source).await {
            tracing::error!("Failed to check for stuck workflows: {}", e);
        }

        // Check for stale activities (activity-level timeout)
        if let Err(e) = check_and_reclaim_stale_activities(&activity_queue, &event_source).await {
            tracing::error!("Failed to check for stale activities: {}", e);
        }

        // Check for expired signal subscriptions
        if let Err(e) =
            check_and_handle_expired_subscriptions(&subscription_service, &event_source).await
        {
            tracing::error!("Failed to check for expired subscriptions: {}", e);
        }
    }
}

/// Check for workflows that have been running longer than the timeout and mark them as failed
async fn check_and_timeout_stuck_workflows(
    config: &OrchestratorConfig,
    event_source: &Arc<dyn EventSource>,
) -> Result<()> {
    let timeout_secs = config.workflow_timeout.as_secs() as f64;

    // Query for workflows that are stuck (running for longer than timeout)
    let stuck_workflows = sqlx::query!(
        r#"
        SELECT id, definition_name
        FROM workflows
        WHERE status = 'running'
          AND created_at < NOW() - make_interval(secs => $1)
        LIMIT 100
        "#,
        timeout_secs
    )
    .fetch_all(&config.pool)
    .await?;

    if stuck_workflows.is_empty() {
        return Ok(());
    }

    tracing::warn!(
        "Found {} stuck workflows (running > {}s), timing out",
        stuck_workflows.len(),
        timeout_secs
    );

    // Publish timeout events for each stuck workflow
    for workflow in stuck_workflows {
        tracing::warn!(
            "Timing out workflow {} ({})",
            workflow.id,
            workflow.definition_name
        );

        // Publish WorkflowFailed event with timeout reason
        let timeout_event = NewWorkflowEvent {
            workflow_id: workflow.id,
            event_type: WorkflowEventType::WorkflowFailed,
            activity_key: None,
            payload: json!({
                "reason": "Workflow timeout",
                "timeout_seconds": timeout_secs,
            }),
            iteration: None,
        };

        if let Err(e) = event_source.publish(timeout_event).await {
            tracing::error!(
                "Failed to publish timeout event for workflow {}: {}",
                workflow.id,
                e
            );
        }
    }

    Ok(())
}

/// Check for stale running activities and reclaim them
///
/// Activities that have been running longer than their timeout are either:
/// - Reset to pending (if retries remain) - workers will pick them up again
/// - Marked as failed (if retries exhausted) - we emit ActivityFailed event
async fn check_and_reclaim_stale_activities(
    activity_queue: &Arc<dyn ActivityQueue>,
    event_source: &Arc<dyn EventSource>,
) -> Result<()> {
    // Reclaim up to 100 stale activities per check
    let reclaimed = activity_queue.reclaim_stale_activities(100).await?;

    if reclaimed.is_empty() {
        return Ok(());
    }

    tracing::info!("Reclaimed {} stale activities", reclaimed.len());

    // For activities that were marked as failed, emit ActivityFailed events
    // so the orchestrator can update workflow state
    for activity in reclaimed {
        match activity.action {
            StaleActivityAction::ResetToPending => {
                // Activity will be picked up by a worker - no event needed
                // The worker will emit ActivityCompleted or ActivityFailed when done
                tracing::debug!(
                    workflow_id = %activity.workflow_id,
                    activity_key = %activity.activity_key,
                    retry_count = activity.retry_count,
                    "Stale activity reset to pending for retry"
                );
            }
            StaleActivityAction::MarkedFailed => {
                // Emit ActivityFailed event so orchestrator updates workflow state
                let failed_event = NewWorkflowEvent {
                    workflow_id: activity.workflow_id,
                    event_type: WorkflowEventType::ActivityFailed,
                    activity_key: Some(activity.activity_key.clone()),
                    payload: json!({
                        "error": format!(
                            "Activity timed out after {} retries (max_retries={})",
                            activity.retry_count,
                            activity.max_retries
                        ),
                        "error_code": "ACTIVITY_TIMEOUT",
                        "retry_count": activity.retry_count,
                        "max_retries": activity.max_retries,
                    }),
                    iteration: activity.iteration,
                };

                if let Err(e) = event_source.publish(failed_event).await {
                    tracing::error!(
                        workflow_id = %activity.workflow_id,
                        activity_key = %activity.activity_key,
                        error = %e,
                        "Failed to publish ActivityFailed event for timed out activity"
                    );
                } else {
                    tracing::warn!(
                        workflow_id = %activity.workflow_id,
                        activity_key = %activity.activity_key,
                        "Published ActivityFailed event for timed out activity"
                    );
                }
            }
        }
    }

    Ok(())
}

/// Check for expired signal subscriptions and handle them based on their on_timeout action
///
/// When an activity is waiting for a signal and the timeout expires:
/// - Continue: Publish ActivitySignaled event with null data (activity proceeds without signal)
/// - Skip: Publish ActivitySignaled event with on_timeout=skip (activity is skipped)
/// - Fail: Publish ActivityFailed event (activity fails)
///
/// Also recovers any subscriptions that were marked expired but not fully processed
/// (e.g., due to a server crash between marking expired and publishing events).
async fn check_and_handle_expired_subscriptions(
    subscription_service: &Arc<dyn SubscriptionService>,
    event_source: &Arc<dyn EventSource>,
) -> Result<()> {
    // First, recover any previously-expired-but-unprocessed subscriptions (crash recovery)
    let recovered = subscription_service
        .recover_expired(100)
        .await
        .map_err(|e| {
            super::OrchestratorError::InternalError(format!(
                "Failed to recover expired subscriptions: {}",
                e
            ))
        })?;

    if !recovered.is_empty() {
        tracing::warn!(
            "Recovering {} expired-but-unprocessed signal subscriptions",
            recovered.len()
        );
        process_expired_subscriptions(&recovered, subscription_service, event_source).await;
    }

    // Then, mark newly expired subscriptions
    let expired = subscription_service
        .expire_subscriptions(100)
        .await
        .map_err(|e| {
            super::OrchestratorError::InternalError(format!(
                "Failed to expire subscriptions: {}",
                e
            ))
        })?;

    if !expired.is_empty() {
        tracing::info!("Found {} expired signal subscriptions", expired.len());
        process_expired_subscriptions(&expired, subscription_service, event_source).await;
    }

    Ok(())
}

/// Publish timeout events for expired subscriptions and delete them on success.
async fn process_expired_subscriptions(
    expired: &[ExpiredSubscription],
    subscription_service: &Arc<dyn SubscriptionService>,
    event_source: &Arc<dyn EventSource>,
) {
    for sub in expired {
        let event = match sub.on_timeout {
            OnTimeout::Continue => {
                tracing::info!(
                    workflow_id = %sub.workflow_id,
                    activity_key = %sub.activity_key,
                    event_name = %sub.event_name,
                    "Signal subscription timed out, continuing without signal data"
                );
                NewWorkflowEvent {
                    workflow_id: sub.workflow_id,
                    event_type: WorkflowEventType::ActivitySignaled,
                    activity_key: Some(sub.activity_key.clone()),
                    payload: json!({
                        "event_name": sub.event_name,
                        "reason": "timeout",
                        "on_timeout": "continue",
                    }),
                    iteration: None,
                }
            }
            OnTimeout::Skip => {
                tracing::info!(
                    workflow_id = %sub.workflow_id,
                    activity_key = %sub.activity_key,
                    event_name = %sub.event_name,
                    "Signal subscription timed out, skipping activity"
                );
                NewWorkflowEvent {
                    workflow_id: sub.workflow_id,
                    event_type: WorkflowEventType::ActivitySignaled,
                    activity_key: Some(sub.activity_key.clone()),
                    payload: json!({
                        "event_name": sub.event_name,
                        "reason": "timeout",
                        "on_timeout": "skip",
                    }),
                    iteration: None,
                }
            }
            OnTimeout::Fail => {
                tracing::warn!(
                    workflow_id = %sub.workflow_id,
                    activity_key = %sub.activity_key,
                    event_name = %sub.event_name,
                    "Signal subscription timed out, failing activity"
                );
                NewWorkflowEvent {
                    workflow_id: sub.workflow_id,
                    event_type: WorkflowEventType::ActivityFailed,
                    activity_key: Some(sub.activity_key.clone()),
                    payload: json!({
                        "error": format!("Signal '{}' not received before timeout", sub.event_name),
                        "error_code": "SIGNAL_TIMEOUT",
                        "event_name": sub.event_name,
                        "on_timeout": "fail",
                    }),
                    iteration: None,
                }
            }
        };

        // Only delete the subscription after the event is successfully published.
        // If publish fails, the subscription stays marked as expired and will be
        // retried on the next cycle via recover_expired.
        match event_source.publish(event).await {
            Ok(_) => {
                let _ = subscription_service
                    .delete_subscription(sub.workflow_id, &sub.activity_key)
                    .await;
            }
            Err(e) => {
                tracing::error!(
                    workflow_id = %sub.workflow_id,
                    activity_key = %sub.activity_key,
                    "Failed to publish timeout event, will retry on next cycle: {}",
                    e
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::WorkflowStatus;
    use crate::orchestrator::workflow_state::{
        ActivityState, WorkflowActivityStatus, WorkflowState,
    };
    use crate::workflow::{ActivityDefinition, ActivityOutput, ActivitySettings, OutputType};
    use rust_decimal::Decimal;
    use serde_json::json;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn create_activity_def(key: &str, settings: Option<ActivitySettings>) -> ActivityDefinition {
        ActivityDefinition {
            key: key.to_string(),
            worker: "test".to_string(),
            activity_name: Some("test_activity".to_string()),
            parameters: Some(HashMap::new()),
            settings,
            depends_on: None,
            dependency_of: None,
            output_definitions: None,
            iteration_scoped: false,
            iteration_limit: None,
            is_loop_activity: false,
            streaming: Default::default(),
        }
    }

    fn create_test_workflow_state(
        activities: Vec<(&str, WorkflowActivityStatus, Option<Vec<ActivityOutput>>)>,
        input: serde_json::Value,
    ) -> WorkflowState {
        let activities_map = activities
            .into_iter()
            .map(|(key, status, outputs)| {
                (
                    key.to_string(),
                    ActivityState {
                        key: key.to_string(),
                        status,
                        outputs,
                        error: None,
                        started_at: None,
                        completed_at: None,
                        attempt: 0,
                        last_error: None,
                        accumulated_cost_usd: Decimal::ZERO,
                        iteration: 0,
                        iteration_outputs: None,
                        signal_data: None,
                    },
                )
            })
            .collect();

        WorkflowState {
            workflow_id: Uuid::now_v7(),
            definition_name: "test_workflow".to_string(),
            status: WorkflowStatus::Running,
            activities: activities_map,
            state_data: json!({}),
            input,
        }
    }

    // =========================================================================
    // compute_scheduled_for tests
    // =========================================================================

    #[test]
    fn test_compute_scheduled_for_no_settings() {
        let activity_def = create_activity_def("test", None);
        let context = TemplateContext::new();

        let result = compute_scheduled_for(&activity_def, &context, None).unwrap();
        assert!(
            result.is_none(),
            "No settings should result in immediate execution"
        );
    }

    #[test]
    fn test_compute_scheduled_for_empty_settings() {
        let settings = ActivitySettings {
            timeout_seconds: Some(30),
            retry: None,
            budget: None,
            cache: false,
            cache_ttl: None,
            iteration_limit: None,
            delay: None,
            scheduled_for: None,
            wait_for_signal: None,
        };
        let activity_def = create_activity_def("test", Some(settings));
        let context = TemplateContext::new();

        let result = compute_scheduled_for(&activity_def, &context, None).unwrap();
        assert!(
            result.is_none(),
            "Settings without delay/scheduled_for should result in immediate execution"
        );
    }

    #[test]
    fn test_compute_scheduled_for_with_delay_seconds() {
        let settings = ActivitySettings {
            timeout_seconds: None,
            retry: None,
            budget: None,
            cache: false,
            cache_ttl: None,
            iteration_limit: None,
            delay: Some("10s".to_string()),
            scheduled_for: None,
            wait_for_signal: None,
        };
        let activity_def = create_activity_def("test", Some(settings));
        let context = TemplateContext::new();

        let before = Utc::now();
        let result = compute_scheduled_for(&activity_def, &context, None)
            .unwrap()
            .unwrap();
        let after = Utc::now();

        // Result should be ~10 seconds in the future
        let expected_min = before + chrono::Duration::seconds(9);
        let expected_max = after + chrono::Duration::seconds(11);

        assert!(
            result >= expected_min && result <= expected_max,
            "Scheduled time {:?} should be ~10 seconds after {:?}",
            result,
            before
        );
    }

    #[test]
    fn test_compute_scheduled_for_with_delay_minutes() {
        let settings = ActivitySettings {
            timeout_seconds: None,
            retry: None,
            budget: None,
            cache: false,
            cache_ttl: None,
            iteration_limit: None,
            delay: Some("5m".to_string()),
            scheduled_for: None,
            wait_for_signal: None,
        };
        let activity_def = create_activity_def("test", Some(settings));
        let context = TemplateContext::new();

        let before = Utc::now();
        let result = compute_scheduled_for(&activity_def, &context, None)
            .unwrap()
            .unwrap();

        // Result should be ~5 minutes in the future
        let expected_min = before + chrono::Duration::minutes(4);
        let expected_max = before + chrono::Duration::minutes(6);

        assert!(
            result >= expected_min && result <= expected_max,
            "Scheduled time {:?} should be ~5 minutes after {:?}",
            result,
            before
        );
    }

    #[test]
    fn test_compute_scheduled_for_retry_ignores_delay() {
        // On retry (iteration > 0), delay should be ignored
        let settings = ActivitySettings {
            timeout_seconds: None,
            retry: None,
            budget: None,
            cache: false,
            cache_ttl: None,
            iteration_limit: None,
            delay: Some("1h".to_string()), // 1 hour delay
            scheduled_for: None,
            wait_for_signal: None,
        };
        let activity_def = create_activity_def("test", Some(settings));
        let context = TemplateContext::new();

        // Iteration 1 = retry, should ignore delay
        let result = compute_scheduled_for(&activity_def, &context, Some(1)).unwrap();
        assert!(
            result.is_none(),
            "Retry attempts should ignore delay and execute immediately"
        );
    }

    #[test]
    fn test_compute_scheduled_for_with_absolute_time() {
        let future_time = Utc::now() + chrono::Duration::hours(1);
        let iso_time = future_time.to_rfc3339();

        let settings = ActivitySettings {
            timeout_seconds: None,
            retry: None,
            budget: None,
            cache: false,
            cache_ttl: None,
            iteration_limit: None,
            delay: None,
            scheduled_for: Some(iso_time.clone()),
            wait_for_signal: None,
        };
        let activity_def = create_activity_def("test", Some(settings));
        let context = TemplateContext::new();

        let result = compute_scheduled_for(&activity_def, &context, None)
            .unwrap()
            .unwrap();

        // Result should be within a second of the specified time
        let diff = (result - future_time).num_seconds().abs();
        assert!(
            diff <= 1,
            "Scheduled time should match specified absolute time (diff: {}s)",
            diff
        );
    }

    #[test]
    fn test_compute_scheduled_for_with_template_delay() {
        let settings = ActivitySettings {
            timeout_seconds: None,
            retry: None,
            budget: None,
            cache: false,
            cache_ttl: None,
            iteration_limit: None,
            delay: Some("{{INPUT.delay_value}}".to_string()),
            scheduled_for: None,
            wait_for_signal: None,
        };
        let activity_def = create_activity_def("test", Some(settings));

        let mut inputs = HashMap::new();
        inputs.insert("delay_value".to_string(), json!("30s"));
        let context = TemplateContext::new().with_inputs(inputs);

        let before = Utc::now();
        let result = compute_scheduled_for(&activity_def, &context, None)
            .unwrap()
            .unwrap();

        // Result should be ~30 seconds in the future
        let expected_min = before + chrono::Duration::seconds(29);
        let expected_max = before + chrono::Duration::seconds(31);

        assert!(
            result >= expected_min && result <= expected_max,
            "Template-resolved delay should work correctly"
        );
    }

    // =========================================================================
    // build_template_context tests
    // =========================================================================

    #[test]
    fn test_build_template_context_empty_state() {
        let state = create_test_workflow_state(vec![], json!({}));
        let workflow_id = state.workflow_id;
        let secrets = HashMap::new();

        let context = build_template_context(&state, workflow_id, &secrets);

        assert!(context.inputs.is_empty());
        assert!(context.activity_states.is_empty());
        assert_eq!(
            context.workflow.get("id").unwrap().as_str().unwrap(),
            workflow_id.to_string()
        );
    }

    #[test]
    fn test_build_template_context_with_inputs() {
        let input = json!({
            "topic": "rust programming",
            "max_results": 10
        });
        let state = create_test_workflow_state(vec![], input);
        let workflow_id = state.workflow_id;
        let secrets = HashMap::new();

        let context = build_template_context(&state, workflow_id, &secrets);

        assert_eq!(
            context.inputs.get("topic").unwrap().as_str().unwrap(),
            "rust programming"
        );
        assert_eq!(
            context.inputs.get("max_results").unwrap().as_i64().unwrap(),
            10
        );
    }

    #[test]
    fn test_build_template_context_with_activity_outputs() {
        let state = create_test_workflow_state(
            vec![(
                "search",
                WorkflowActivityStatus::Completed,
                Some(vec![ActivityOutput {
                    name: "results".to_string(),
                    output_type: OutputType::Value,
                    value: json!(["result1", "result2"]),
                }]),
            )],
            json!({}),
        );
        let workflow_id = state.workflow_id;
        let secrets = HashMap::new();

        let context = build_template_context(&state, workflow_id, &secrets);

        // Verify activity was added to context
        assert!(context.activity_states.contains_key("search"));
    }

    #[test]
    fn test_build_template_context_workflow_status() {
        let state = create_test_workflow_state(vec![], json!({}));
        let workflow_id = state.workflow_id;
        let secrets = HashMap::new();

        let context = build_template_context(&state, workflow_id, &secrets);

        assert_eq!(
            context.workflow.get("status").unwrap().as_str().unwrap(),
            "running"
        );
    }

    #[test]
    fn test_build_template_context_with_multiple_activities() {
        let state = create_test_workflow_state(
            vec![
                (
                    "activity1",
                    WorkflowActivityStatus::Completed,
                    Some(vec![ActivityOutput {
                        name: "output1".to_string(),
                        output_type: OutputType::Value,
                        value: json!("value1"),
                    }]),
                ),
                (
                    "activity2",
                    WorkflowActivityStatus::Completed,
                    Some(vec![ActivityOutput {
                        name: "output2".to_string(),
                        output_type: OutputType::Value,
                        value: json!("value2"),
                    }]),
                ),
                ("activity3", WorkflowActivityStatus::Pending, None),
            ],
            json!({}),
        );
        let workflow_id = state.workflow_id;
        let secrets = HashMap::new();

        let context = build_template_context(&state, workflow_id, &secrets);

        // All activities should be in context, even pending ones
        assert!(context.activity_states.contains_key("activity1"));
        assert!(context.activity_states.contains_key("activity2"));
        assert!(context.activity_states.contains_key("activity3"));
    }

    #[test]
    fn test_build_template_context_with_iteration_state() {
        let mut activities_map = HashMap::new();
        activities_map.insert(
            "loop_activity".to_string(),
            ActivityState {
                key: "loop_activity".to_string(),
                status: WorkflowActivityStatus::Completed,
                outputs: Some(vec![ActivityOutput {
                    name: "result".to_string(),
                    output_type: OutputType::Value,
                    value: json!("iteration_2_result"),
                }]),
                error: None,
                started_at: None,
                completed_at: None,
                attempt: 0,
                last_error: None,
                accumulated_cost_usd: Decimal::new(150, 2), // $1.50
                iteration: 2,
                iteration_outputs: Some({
                    let mut map = HashMap::new();
                    map.insert(
                        "result".to_string(),
                        vec![json!("iter0"), json!("iter1"), json!("iteration_2_result")],
                    );
                    map
                }),
                signal_data: None,
            },
        );

        let state = WorkflowState {
            workflow_id: Uuid::now_v7(),
            definition_name: "test".to_string(),
            status: WorkflowStatus::Running,
            activities: activities_map,
            state_data: json!({}),
            input: json!({}),
        };
        let workflow_id = state.workflow_id;
        let secrets = HashMap::new();

        let context = build_template_context(&state, workflow_id, &secrets);

        // Verify iteration context is captured
        assert!(context.activity_states.contains_key("loop_activity"));
    }

    #[test]
    fn test_build_template_context_with_secrets() {
        let state = create_test_workflow_state(vec![], json!({}));
        let workflow_id = state.workflow_id;
        let mut secrets = HashMap::new();
        secrets.insert(
            "db_url".to_string(),
            "postgres://secret:pass@localhost/db".to_string(),
        );
        secrets.insert("api_key".to_string(), "sk-12345".to_string());

        let context = build_template_context(&state, workflow_id, &secrets);

        // Verify secrets are in context
        assert_eq!(
            context.secrets.get("db_url").unwrap(),
            "postgres://secret:pass@localhost/db"
        );
        assert_eq!(context.secrets.get("api_key").unwrap(), "sk-12345");
    }
}
