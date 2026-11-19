// ============================================================================
// A step that becomes ready because another step was skipped runs (#25
// [skipped-branch-hang])
//
// Skips are applied after scheduling. A skip can make another activity ready
// (a `status == 'skipped'` branch, or a dependent whose other inputs are
// done), but skipping publishes no event, so nothing evaluated readiness
// again: the activity stayed not_scheduled and the workflow stayed running.
// ============================================================================

mod common;

use kruxiaflow_core::events::{EventSource, NewWorkflowEvent, WorkflowEventType, WorkflowStatus};
use kruxiaflow_core::orchestrator::{
    OrchestratorConfig,
    workflow_state::{WorkflowActivityStatus, WorkflowState, load_materialized_state},
};
use kruxiaflow_core::queue::{PostgresQueue, QueueConfig};
use kruxiaflow_core::workflow::WorkflowDefinition;
use kruxiaflow_core::{PostgresSubscriptionService, SubscriptionService};
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

async fn insert_workflow_definition(pool: &PgPool, definition: &WorkflowDefinition) -> Uuid {
    let activities_json =
        serde_json::to_value(&definition.activities).expect("Failed to serialize activities");
    let row = sqlx::query!(
        r#"INSERT INTO workflow_definitions (name, activities)
           VALUES ($1, $2)
           RETURNING id"#,
        definition.name,
        activities_json
    )
    .fetch_one(pool)
    .await
    .expect("Failed to insert workflow definition");
    row.id
}

/// Runs a workflow through the real orchestrator, completing whatever is
/// pending until nothing is. Every event must process without error: a
/// template error is not an event-processing failure.
async fn run_workflow(pool: &PgPool, yaml: &str) -> WorkflowState {
    run_workflow_with_id(pool, yaml).await.1
}

async fn run_workflow_with_id(pool: &PgPool, yaml: &str) -> (Uuid, WorkflowState) {
    let definition = WorkflowDefinition::from_yaml(yaml).expect("valid definition");
    let definition_id = insert_workflow_definition(pool, &definition).await;
    let workflow_id = Uuid::now_v7();

    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let activity_queue: Arc<dyn kruxiaflow_core::queue::ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));
    let config = OrchestratorConfig::new(pool.clone());

    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::WorkflowCreated,
            activity_key: None,
            payload: json!({
                "definition_name": definition.name,
                "workflow_definition_id": definition_id,
            }),
            iteration: None,
        })
        .await
        .unwrap();

    for _ in 0..20 {
        loop {
            let events = event_source.poll("orchestrator").await.unwrap();
            if events.is_empty() {
                break;
            }
            for event in events {
                kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
                    &event,
                    &event_source,
                    &activity_queue,
                    &subscription_service,
                    &config,
                )
                .await
                .unwrap_or_else(|e| {
                    panic!("{:?} must process cleanly, got: {e}", event.event_type)
                });
                event_source
                    .update_position("orchestrator", event.cursor())
                    .await
                    .unwrap();
            }
        }

        let mut tx = pool.begin().await.unwrap();
        let state = load_materialized_state(&mut tx, workflow_id).await.unwrap();
        tx.commit().await.unwrap();

        // Completions carry the iteration for loop activities, as a worker's
        // do; without it a later round's completion would be dropped as a
        // duplicate of the first
        let pending: Vec<(String, Option<i32>)> = state
            .activities
            .values()
            .filter(|a| a.status == WorkflowActivityStatus::Pending)
            .map(|a| {
                let is_loop = definition
                    .activities
                    .iter()
                    .any(|d| d.key == a.key && d.is_loop_activity);
                (a.key.clone(), is_loop.then_some(a.iteration as i32))
            })
            .collect();
        if pending.is_empty() {
            return (workflow_id, state);
        }
        for (key, iteration) in pending {
            event_source
                .publish(NewWorkflowEvent {
                    workflow_id,
                    event_type: WorkflowEventType::ActivityCompleted,
                    activity_key: Some(key),
                    payload: json!({"outputs": {"result": {"value": 1}}}),
                    iteration,
                })
                .await
                .unwrap();
        }
    }
    panic!("workflow did not settle");
}

fn status(state: &WorkflowState, key: &str) -> WorkflowActivityStatus {
    state.activities[key].status
}

/// The issue's reproduction: `fallback` runs because `maybe` was skipped
#[sqlx::test(migrations = "../migrations")]
async fn test_branch_on_a_skipped_step_runs(pool: PgPool) {
    let state = run_workflow(
        &pool,
        r#"
name: skipped_branch
activities:
  - key: first
    worker: test
    activity_name: echo
  - key: maybe
    worker: test
    activity_name: echo
    depends_on:
      - activity_key: first
        conditions: ["{{ first.result.value == 2 }}"]
  - key: fallback
    worker: test
    activity_name: echo
    depends_on:
      - activity_key: maybe
        conditions: ["{{ maybe.status == 'skipped' }}"]
"#,
    )
    .await;

    assert_eq!(status(&state, "first"), WorkflowActivityStatus::Completed);
    assert_eq!(status(&state, "maybe"), WorkflowActivityStatus::Skipped);
    assert_eq!(
        status(&state, "fallback"),
        WorkflowActivityStatus::Completed
    );
    assert_eq!(state.status, WorkflowStatus::Completed);
}

/// A skip that cascades two levels before the branch becomes ready
#[sqlx::test(migrations = "../migrations")]
async fn test_branch_after_a_cascade_of_skips_runs(pool: PgPool) {
    let state = run_workflow(
        &pool,
        r#"
name: skipped_branch_cascade
activities:
  - key: first
    worker: test
    activity_name: echo
  - key: maybe
    worker: test
    activity_name: echo
    depends_on:
      - activity_key: first
        conditions: ["{{ first.result.value == 2 }}"]
  - key: then
    worker: test
    activity_name: echo
    depends_on: [maybe]
  - key: fallback
    worker: test
    activity_name: echo
    depends_on:
      - activity_key: then
        conditions: ["{{ then.status == 'skipped' }}"]
"#,
    )
    .await;

    assert_eq!(status(&state, "maybe"), WorkflowActivityStatus::Skipped);
    assert_eq!(status(&state, "then"), WorkflowActivityStatus::Skipped);
    assert_eq!(
        status(&state, "fallback"),
        WorkflowActivityStatus::Completed
    );
    assert_eq!(state.status, WorkflowStatus::Completed);
}

/// Unchanged: a branch whose condition is false after the skip is skipped
/// too, and the workflow completes
#[sqlx::test(migrations = "../migrations")]
async fn test_branch_not_taken_after_a_skip_is_skipped(pool: PgPool) {
    let state = run_workflow(
        &pool,
        r#"
name: skipped_branch_not_taken
activities:
  - key: first
    worker: test
    activity_name: echo
  - key: maybe
    worker: test
    activity_name: echo
    depends_on:
      - activity_key: first
        conditions: ["{{ first.result.value == 2 }}"]
  - key: on_success
    worker: test
    activity_name: echo
    depends_on:
      - activity_key: maybe
        conditions: ["{{ maybe.status == 'completed' }}"]
"#,
    )
    .await;

    assert_eq!(status(&state, "maybe"), WorkflowActivityStatus::Skipped);
    assert_eq!(
        status(&state, "on_success"),
        WorkflowActivityStatus::Skipped
    );
    assert_eq!(state.status, WorkflowStatus::Completed);
}
