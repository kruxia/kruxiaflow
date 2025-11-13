use super::{
    AdaptiveBackoff, OrchestratorConfig, Result,
    dependency_evaluator::{find_ready_activities, is_workflow_complete, is_workflow_failed},
    workflow_state::{
        WorkflowActivityStatus, apply_event_to_state, initialize_workflow_state,
        load_materialized_state, load_workflow_definition, save_materialized_state,
    },
};
use crate::events::{
    EventSource, NewWorkflowEvent, WorkflowEvent, WorkflowEventType, WorkflowStatus,
};
use crate::queue::{Activity, ActivityQueue};
use serde_json::json;
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
                tracing::error!("Failed to process event {}: {}", event.id, e);
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

            let activities_to_schedule: Vec<Activity> = ready_activities
                .iter()
                .map(|a| Activity {
                    key: a.key.clone(),
                    namespace: a.namespace.clone(),
                    name: a.name.clone(),
                    parameters: a.parameters.clone(),
                    settings: a.settings.clone(),
                    scheduled_for: None, // Schedule immediately (delayed scheduling deferred post-MVP)
                })
                .collect();

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
                        "namespace": activity.namespace,
                        "name": activity.name,
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
