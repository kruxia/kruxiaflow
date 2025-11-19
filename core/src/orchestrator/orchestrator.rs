use super::{
    AdaptiveBackoff, OrchestratorConfig, Result,
    dependency_evaluator::{
        find_ready_activities, find_skipped_activities, is_workflow_complete, is_workflow_failed,
    },
    workflow_state::{
        WorkflowActivityStatus, apply_event_to_state, initialize_workflow_state,
        load_materialized_state, load_workflow_definition, save_materialized_state,
    },
};
use crate::cost::{ActivityCostRecord, CostCalculator, CostTracker};
use crate::events::{
    EventSource, NewWorkflowEvent, WorkflowEvent, WorkflowEventType, WorkflowStatus,
};
use crate::queue::{Activity, ActivityQueue};
use crate::workflow::BudgetAction;
use crate::workflow::template::{TemplateContext, resolve_template_value};
use rust_decimal::Decimal;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

const CONSUMER_ID: &str = "orchestrator";

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
#[tracing::instrument(skip(event_source, activity_queue, config, shutdown_token))]
pub async fn run_orchestrator(
    event_source: Arc<dyn EventSource>,
    activity_queue: Arc<dyn ActivityQueue>,
    config: OrchestratorConfig,
    shutdown_token: Option<CancellationToken>,
) -> Result<()> {
    let mut backoff = AdaptiveBackoff::new(
        config.poll_interval_min,
        config.poll_interval_max,
        config.backoff_multiplier,
    );

    tracing::info!(
        "Orchestrator starting with consumer_id={}, workflow_timeout={}s",
        CONSUMER_ID,
        config.workflow_timeout.as_secs()
    );

    // Spawn background task to check for stuck workflows
    let timeout_config = config.clone();
    let timeout_event_source = event_source.clone();
    let timeout_shutdown = shutdown_token.clone();
    tokio::spawn(async move {
        timeout_checker_task(timeout_config, timeout_event_source, timeout_shutdown).await;
    });

    loop {
        // Check if shutdown has been requested
        if let Some(ref token) = shutdown_token {
            if token.is_cancelled() {
                tracing::info!("Shutdown requested, orchestrator stopping gracefully");
                return Ok(());
            }
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
            if let Some(ref token) = shutdown_token {
                if token.is_cancelled() {
                    tracing::info!("Shutdown requested during event processing, stopping");
                    return Ok(());
                }
            }

            if let Err(e) =
                process_workflow_event(event, &event_source, &activity_queue, &config).await
            {
                // Log error but continue processing
                tracing::error!(
                    event_id = %event.id,
                    workflow_id = %event.workflow_id,
                    event_type = ?event.event_type,
                    error = %e,
                    "Failed to process event"
                );
                // Note: Event position is NOT updated on error, will be reprocessed
                continue;
            }

            // Update consumer position after successful processing (durable checkpoint)
            event_source.update_position(CONSUMER_ID, event.id).await?;
        }

        // Got events - reset backoff
        backoff.reset();

        // Always sleep for at least minimum interval to avoid spinning
        // This caps polling rate at ~1000/sec (with 1ms min) even under heavy load
        tokio::time::sleep(backoff.current()).await;
    }
}

/// Build TemplateContext from WorkflowState for resolving template expressions
fn build_template_context(
    state: &super::workflow_state::WorkflowState,
    workflow_id: uuid::Uuid,
) -> TemplateContext {
    let mut context = TemplateContext::new();

    // Add workflow inputs from state_data
    if let serde_json::Value::Object(state_obj) = &state.state_data {
        let inputs: HashMap<String, serde_json::Value> = state_obj
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        context = context.with_inputs(inputs);
    }

    // Add activity outputs
    for (activity_key, activity_state) in &state.activities {
        if let Some(outputs) = &activity_state.outputs {
            context.add_activity_output(activity_key.clone(), outputs.clone());
        }
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
    definition: &crate::events::WorkflowDefinition,
    event: &WorkflowEvent,
    event_source: &Arc<dyn EventSource>,
    activity_queue: &Arc<dyn ActivityQueue>,
    pool: &sqlx::PgPool,
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
    let template_context = build_template_context(state, state.workflow_id);

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
    let resolved_params = match resolve_template_value(&activity_def.parameters, &template_context)
    {
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
    let (params_w_budget, should_schedule) = enrich_llm_activity_params_w_budget(
        &activity_def.activity_name,
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
        activity_name: activity_def.activity_name.clone(),
        parameters: params_w_budget,
        settings: activity_def.settings.clone(),
        scheduled_for,
        output_definitions: activity_def.output_definitions.clone(),
    };

    activity_queue
        .schedule(state.workflow_id, vec![activity_to_schedule])
        .await?;

    // Publish ActivityScheduled event for observability
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
    };
    event_source.publish(scheduled_event).await?;

    Ok(true) // Retry scheduled
}

/// Process a single workflow event
#[tracing::instrument(
    skip(event, event_source, activity_queue, config),
    fields(
        workflow_id = %event.workflow_id,
        event_type = ?event.event_type
    )
)]
pub async fn process_workflow_event(
    event: &WorkflowEvent,
    event_source: &Arc<dyn EventSource>,
    activity_queue: &Arc<dyn ActivityQueue>,
    config: &OrchestratorConfig,
) -> Result<()> {
    // Skip ActivityScheduled events - they are for observability only, not orchestration
    // Processing them causes duplicate scheduling and performance issues
    if event.event_type == WorkflowEventType::ActivityScheduled {
        tracing::trace!(
            "Skipping ActivityScheduled event for workflow {} (observability only)",
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
        load_workflow_definition(&mut *tx, event.workflow_id).await?
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
            initialize_workflow_state(&mut *tx, event.workflow_id, &definition, initial_state_data)
                .await?
        } else {
            // Load existing materialized state
            load_materialized_state(&mut *tx, event.workflow_id).await?
        }
    };
    tracing::trace!(
        "Workflow state loaded in {:?} for workflow {}",
        state_start.elapsed(),
        event.workflow_id
    );

    // 3. Handle ActivityFailed event with retry logic BEFORE applying to state
    if event.event_type == WorkflowEventType::ActivityFailed {
        let retry_start = std::time::Instant::now();
        let retry_handled = handle_activity_failed(
            &mut state,
            &definition,
            event,
            event_source,
            activity_queue,
            &config.pool,
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
        if let Some(activity_key) = &event.activity_key {
            // Get activity definition to check if it's an LLM activity
            if let Some(activity_def) = definition
                .activities
                .iter()
                .find(|a| &a.key == activity_key)
            {
                // Check if this is an LLM activity (llm_prompt or embedding)
                if activity_def.activity_name == "llm_prompt"
                    || activity_def.activity_name == "embedding"
                {
                    if let Err(e) =
                        record_llm_activity_cost(&mut tx, &event, &state, &activity_def, config)
                            .await
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
            }
        }
        tracing::trace!(
            "Cost recording completed in {:?} for workflow {}",
            cost_start.elapsed(),
            event.workflow_id
        );
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
                build_template_context(&state, event.workflow_id)
            };

            // Resolve templates in activity parameters
            let mut activities_to_schedule = Vec::new();
            for a in &ready_activities {
                // Resolve template expressions in parameters
                let resolved_params = match resolve_template_value(&a.parameters, &template_context)
                {
                    Ok(resolved) => resolved,
                    Err(e) => {
                        tracing::error!("Template resolution failed for activity {}: {}", a.key, e);
                        return Err(super::OrchestratorError::TemplateFailed(format!(
                            "Failed to resolve templates in activity {}: {}",
                            a.key, e
                        )));
                    }
                };

                // Enrich LLM activity parameters with budget information
                let (params_w_budget, should_schedule) = enrich_llm_activity_params_w_budget(
                    &a.activity_name,
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

                activities_to_schedule.push(Activity {
                    key: a.key.clone(),
                    worker: a.worker.clone(),
                    activity_name: a.activity_name.clone(),
                    parameters: params_w_budget,
                    settings: a.settings.clone(),
                    scheduled_for: None, // Schedule immediately (delayed scheduling deferred post-MVP)
                    output_definitions: a.output_definitions.clone(),
                });
            }

            activity_queue
                .schedule(event.workflow_id, activities_to_schedule)
                .await?;

            // Update state immediately to mark activities as Pending
            // This prevents race condition where activity completes before ActivityScheduled event is processed
            for activity in &ready_activities {
                if let Some(activity_state) = state.activities.get_mut(&activity.key) {
                    activity_state.status = WorkflowActivityStatus::Pending;
                    activity_state.started_at = Some(chrono::Utc::now());
                }
            }

            // Publish ActivityScheduled events (for external observers)
            // Note: Orchestrator skips these events to avoid duplicate processing
            for activity in &ready_activities {
                tracing::debug!(
                    "Publishing ActivityScheduled event for {} in workflow {}",
                    activity.key,
                    event.workflow_id
                );

                let scheduled_event = NewWorkflowEvent {
                    workflow_id: event.workflow_id,
                    event_type: WorkflowEventType::ActivityScheduled,
                    activity_key: Some(activity.key.clone()),
                    payload: json!({
                        "worker": activity.worker,
                        "activity_name": activity.activity_name,
                    }),
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
            }
        } else {
            NewWorkflowEvent {
                workflow_id: event.workflow_id,
                event_type: WorkflowEventType::WorkflowCompleted,
                activity_key: None,
                payload: json!({}),
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
        tracing::debug!(
            event = if is_workflow_failed(&state) { "WorkflowFailed" } else { "WorkflowCompleted" },
            workflow_id = %event.workflow_id,
        );
    }

    // 7. Save updated materialized state back to workflows table
    let save_start = std::time::Instant::now();
    let save_span = profile_span!("save_workflow_state");
    {
        let _enter = save_span.enter();
        save_materialized_state(&mut *tx, event.workflow_id, &state).await?;
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
    activity_def: &crate::events::ActivityDefinition,
    _workflow_def: &crate::events::WorkflowDefinition,
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
        for model_key in &model_list {
            if let Some(pricing) = model_pricing.get(model_key) {
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

            if let Some(limit) = effective_limit {
                if cumulative_cost + estimate > limit {
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
    activity_def: &crate::events::ActivityDefinition,
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

/// Background task to check for stuck workflows and timeout them
async fn timeout_checker_task(
    config: OrchestratorConfig,
    event_source: Arc<dyn EventSource>,
    shutdown_token: Option<CancellationToken>,
) {
    tracing::info!(
        "Timeout checker starting (check_interval={}s, timeout={}s)",
        config.timeout_check_interval.as_secs(),
        config.workflow_timeout.as_secs()
    );

    loop {
        // Check if shutdown has been requested
        if let Some(ref token) = shutdown_token {
            if token.is_cancelled() {
                tracing::info!("Shutdown requested, timeout checker stopping");
                return;
            }
        }

        // Sleep for check interval
        tokio::time::sleep(config.timeout_check_interval).await;

        // Check for stuck workflows
        if let Err(e) = check_and_timeout_stuck_workflows(&config, &event_source).await {
            tracing::error!("Failed to check for stuck workflows: {}", e);
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
