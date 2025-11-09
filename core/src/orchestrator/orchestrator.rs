use super::{
    AdaptiveBackoff, OrchestratorConfig, Result,
    dependency_evaluator::{find_ready_activities, is_workflow_complete, is_workflow_failed},
    workflow_state::{
        apply_event_to_state, initialize_workflow_state, load_materialized_state,
        load_workflow_definition, save_materialized_state,
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

/// Run the orchestrator main loop
/// Polls for events, evaluates dependencies, schedules activities
///
/// # Arguments
/// * `event_source` - Event source for polling workflow events
/// * `activity_queue` - Activity queue for scheduling activities
/// * `config` - Orchestrator configuration
/// * `shutdown_token` - Optional cancellation token for graceful shutdown
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

    tracing::info!("Orchestrator starting with consumer_id={}", CONSUMER_ID);

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
            tokio::time::sleep(backoff.current()).await;
            continue;
        }

        tracing::debug!("Polled {} events", events.len());

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
    }
}

/// Process a single workflow event
pub async fn process_workflow_event(
    event: &WorkflowEvent,
    event_source: &Arc<dyn EventSource>,
    activity_queue: &Arc<dyn ActivityQueue>,
    config: &OrchestratorConfig,
) -> Result<()> {
    // Begin transaction with workflow-level advisory lock
    let mut tx = config.pool.begin().await?;

    // Acquire exclusive lock for this workflow (prevents concurrent evaluation)
    // Uses hash of workflow_id to get a 64-bit integer for pg_advisory_xact_lock
    sqlx::query!(
        "SELECT pg_advisory_xact_lock(hashtext($1::text))",
        event.workflow_id.to_string()
    )
    .execute(&mut *tx)
    .await?;

    tracing::debug!(
        "Processing event {} for workflow {}",
        event.event_type,
        event.workflow_id
    );

    // 1. Load workflow definition
    let definition = load_workflow_definition(&mut *tx, event.workflow_id).await?;

    // 2. Load materialized state from workflows table (O(1), not O(n))
    //    Special case: WorkflowCreated needs to initialize state first
    let mut state = if event.event_type == WorkflowEventType::WorkflowCreated {
        // Initialize new workflow state
        let initial_state_data = event.payload.get("state_data").cloned();
        initialize_workflow_state(&mut *tx, event.workflow_id, &definition, initial_state_data)
            .await?
    } else {
        // Load existing materialized state
        load_materialized_state(&mut *tx, event.workflow_id).await?
    };

    // 3. Apply THIS event to update state incrementally (just 1 event, not n events)
    apply_event_to_state(&mut state, event)?;

    // 4. Find ready activities (using updated state)
    let ready_activities = find_ready_activities(&definition, &state)?;

    // 5. Schedule ready activities to queue
    if !ready_activities.is_empty() {
        tracing::debug!(
            "Scheduling {} activities for workflow {}",
            ready_activities.len(),
            event.workflow_id
        );

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

        // Publish ActivityScheduled events
        for activity in &ready_activities {
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

    // 6. Check for workflow completion
    if is_workflow_complete(&state) {
        // tracing::info!("Workflow {} complete", event.workflow_id);

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
    }

    // 7. Save updated materialized state back to workflows table
    save_materialized_state(&mut *tx, event.workflow_id, &state).await?;

    // Commit transaction (releases advisory lock automatically)
    tx.commit().await?;

    Ok(())
}
