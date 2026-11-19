// ============================================================================
// Template errors fail the activity whose template is bad (#19 [template-hang])
//
// A template that cannot be resolved (here: a reference to a step that has
// not run, under strict undefined) used to be returned as an error from
// processing the *upstream* step's completion event. The event was retried
// five times, declared poison, and the upstream step (which had succeeded)
// was marked failed; the step with the bad template never ran or failed, a
// single-pass skip left later steps NotScheduled, and the workflow stayed
// running forever.
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

/// The issue's reproduction: first → second (references `later`, which has
/// not run) → later
#[sqlx::test(migrations = "../migrations")]
async fn test_bad_template_fails_its_own_activity_and_the_workflow(pool: PgPool) {
    let state = run_workflow(
        &pool,
        r#"
name: template_hang_repro
activities:
  - key: first
    worker: test
    activity_name: echo
  - key: second
    worker: test
    activity_name: echo
    parameters:
      value: "{{ later.result.value }}"
    depends_on: [first]
  - key: later
    worker: test
    activity_name: echo
    depends_on: [second]
"#,
    )
    .await;

    assert_eq!(
        status(&state, "first"),
        WorkflowActivityStatus::Completed,
        "the upstream step succeeded"
    );
    assert_eq!(status(&state, "second"), WorkflowActivityStatus::Failed);
    let error = state.activities["second"]
        .error
        .as_deref()
        .unwrap_or_default();
    assert!(error.starts_with("TEMPLATE_ERROR:"), "{error}");
    assert!(
        error.contains("second"),
        "the error names the activity: {error}"
    );
    assert!(
        error.contains("undefined"),
        "the error says what is wrong: {error}"
    );
    assert_eq!(status(&state, "later"), WorkflowActivityStatus::Skipped);
    assert_eq!(
        state.status,
        WorkflowStatus::Failed,
        "the workflow fails instead of hanging"
    );
}

/// Skips cascade down a chain in one go: before, only the first dependent was
/// skipped and the rest stayed NotScheduled, so the workflow never completed
#[sqlx::test(migrations = "../migrations")]
async fn test_skips_cascade_down_a_chain(pool: PgPool) {
    let state = run_workflow(
        &pool,
        r#"
name: skip_cascade
activities:
  - key: first
    worker: test
    activity_name: echo
  - key: bad
    worker: test
    activity_name: echo
    parameters:
      value: "{{ nowhere.result.value }}"
    depends_on: [first]
  - key: a
    worker: test
    activity_name: echo
    depends_on: [bad]
  - key: b
    worker: test
    activity_name: echo
    depends_on: [a]
  - key: c
    worker: test
    activity_name: echo
    depends_on: [b]
  - key: nowhere
    worker: test
    activity_name: echo
    depends_on: [c]
"#,
    )
    .await;

    assert_eq!(status(&state, "bad"), WorkflowActivityStatus::Failed);
    for key in ["a", "b", "c", "nowhere"] {
        assert_eq!(
            status(&state, key),
            WorkflowActivityStatus::Skipped,
            "{key}"
        );
    }
    assert_eq!(state.status, WorkflowStatus::Failed);
}

/// The failure is an ordinary activity failure, so a `status == 'failed'`
/// branch can handle it
#[sqlx::test(migrations = "../migrations")]
async fn test_failed_branch_runs_after_a_template_error(pool: PgPool) {
    let state = run_workflow(
        &pool,
        r#"
name: template_failure_branch
activities:
  - key: first
    worker: test
    activity_name: echo
  - key: second
    worker: test
    activity_name: echo
    parameters:
      value: "{{ missing.result.value }}"
    depends_on: [first]
  - key: on_success
    worker: test
    activity_name: echo
    depends_on:
      - activity_key: second
        conditions: ["{{ second.status == 'completed' }}"]
  - key: on_failure
    worker: test
    activity_name: echo
    depends_on:
      - activity_key: second
        conditions: ["{{ second.status == 'failed' }}"]
  - key: missing
    worker: test
    activity_name: echo
    depends_on: [on_success]
"#,
    )
    .await;

    assert_eq!(status(&state, "second"), WorkflowActivityStatus::Failed);
    assert_eq!(
        status(&state, "on_failure"),
        WorkflowActivityStatus::Completed
    );
    assert_eq!(
        status(&state, "on_success"),
        WorkflowActivityStatus::Skipped
    );
    assert_eq!(status(&state, "missing"), WorkflowActivityStatus::Skipped);
}

/// A bad `delay` template fails the activity the same way
#[sqlx::test(migrations = "../migrations")]
async fn test_bad_delay_template_fails_its_own_activity(pool: PgPool) {
    let state = run_workflow(
        &pool,
        r#"
name: delay_template_error
activities:
  - key: first
    worker: test
    activity_name: echo
  - key: wait
    worker: test
    activity_name: echo
    settings:
      delay: "{{ never.result.value }}"
    depends_on: [first]
  - key: never
    worker: test
    activity_name: echo
    depends_on: [wait]
"#,
    )
    .await;

    assert_eq!(status(&state, "first"), WorkflowActivityStatus::Completed);
    assert_eq!(status(&state, "wait"), WorkflowActivityStatus::Failed);
    assert!(
        state.activities["wait"]
            .error
            .as_deref()
            .unwrap_or_default()
            .starts_with("TEMPLATE_ERROR:")
    );
    assert_eq!(status(&state, "never"), WorkflowActivityStatus::Skipped);
    assert_eq!(state.status, WorkflowStatus::Failed);
}

/// Events recorded for one activity, by type (runtime query: no offline
/// query cache entry needed)
async fn event_count(
    pool: &PgPool,
    workflow_id: Uuid,
    activity_key: &str,
    event_type: &str,
) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM workflow_events \
         WHERE workflow_id = $1 AND activity_key = $2 AND event_type::text = $3",
    )
    .bind(workflow_id)
    .bind(activity_key)
    .bind(event_type)
    .fetch_one(pool)
    .await
    .expect("count events")
}

/// The failed activity was never queued, so no ActivityScheduled is recorded
/// for it: only its ActivityFailed
#[sqlx::test(migrations = "../migrations")]
async fn test_no_activity_scheduled_event_for_a_template_failure(pool: PgPool) {
    let (workflow_id, state) = run_workflow_with_id(
        &pool,
        r#"
name: template_failure_events
activities:
  - key: first
    worker: test
    activity_name: echo
  - key: second
    worker: test
    activity_name: echo
    parameters:
      value: "{{ later.result.value }}"
    depends_on: [first]
  - key: later
    worker: test
    activity_name: echo
    depends_on: [second]
"#,
    )
    .await;

    assert_eq!(status(&state, "second"), WorkflowActivityStatus::Failed);
    assert_eq!(
        event_count(&pool, workflow_id, "second", "ActivityScheduled").await,
        0
    );
    assert_eq!(
        event_count(&pool, workflow_id, "second", "ActivityFailed").await,
        1
    );
    assert_eq!(
        event_count(&pool, workflow_id, "first", "ActivityScheduled").await,
        1
    );
}

/// A dependency condition that cannot be evaluated fails the activity that
/// carries it, the same way as a bad parameter template, instead of failing
/// the upstream activity's completion event
#[sqlx::test(migrations = "../migrations")]
async fn test_bad_condition_template_fails_its_own_activity(pool: PgPool) {
    let state = run_workflow(
        &pool,
        r#"
name: condition_template_error
activities:
  - key: first
    worker: test
    activity_name: echo
  - key: gate
    worker: test
    activity_name: echo
    depends_on:
      - activity_key: first
        conditions: ["{{ later.result.value == 1 }}"]
  - key: later
    worker: test
    activity_name: echo
    depends_on: [gate]
"#,
    )
    .await;

    assert_eq!(
        status(&state, "first"),
        WorkflowActivityStatus::Completed,
        "the upstream step succeeded"
    );
    assert_eq!(status(&state, "gate"), WorkflowActivityStatus::Failed);
    let error = state.activities["gate"]
        .error
        .as_deref()
        .unwrap_or_default();
    assert!(error.starts_with("TEMPLATE_ERROR:"), "{error}");
    assert!(
        error.contains("condition"),
        "the error says it was a condition: {error}"
    );
    assert_eq!(status(&state, "later"), WorkflowActivityStatus::Skipped);
    assert_eq!(state.status, WorkflowStatus::Failed);
}

/// A condition that can only be evaluated after an earlier skip in the same
/// pass: `classify` is skipped (its own condition is false), which makes it
/// terminal, and `act`'s condition on it refers to its result, which a
/// skipped step does not have. The skip pass must fail `act` rather than
/// leave it NotScheduled with nothing left to wake it.
#[sqlx::test(migrations = "../migrations")]
async fn test_condition_unresolvable_after_a_skip_fails_its_activity(pool: PgPool) {
    let state = run_workflow(
        &pool,
        r#"
name: unresolvable_after_skip
activities:
  - key: first
    worker: test
    activity_name: echo
  - key: classify
    worker: test
    activity_name: echo
    depends_on:
      - activity_key: first
        conditions: ["{{ first.result.value == 2 }}"]
  - key: act
    worker: test
    activity_name: echo
    depends_on:
      - activity_key: classify
        conditions: ["{{ classify.result.value == 1 }}"]
"#,
    )
    .await;

    assert_eq!(status(&state, "first"), WorkflowActivityStatus::Completed);
    assert_eq!(status(&state, "classify"), WorkflowActivityStatus::Skipped);
    assert_eq!(
        status(&state, "act"),
        WorkflowActivityStatus::Failed,
        "not left NotScheduled"
    );
    assert!(
        state.activities["act"]
            .error
            .as_deref()
            .unwrap_or_default()
            .starts_with("TEMPLATE_ERROR:")
    );
    assert_eq!(state.status, WorkflowStatus::Failed);
}

/// A template error in a later round of a loop fails that round's activity:
/// `tail`'s template resolves in round 1 and refers to a missing value from
/// round 2 on. The failure event carries the round's iteration, so it is
/// not mistaken for a duplicate of an earlier event, and the workflow fails
/// instead of hanging.
#[sqlx::test(migrations = "../migrations")]
async fn test_template_error_in_a_later_loop_round_fails_that_round(pool: PgPool) {
    let state = run_workflow(
        &pool,
        r#"
name: loop_round_template_error
activities:
  - key: head
    worker: test
    activity_name: echo
    settings:
      iteration_limit: 3
    depends_on:
      - activity_key: tail
        conditions: ["{{ tail.result.value == 1 }}"]
  - key: tail
    worker: test
    activity_name: echo
    parameters:
      value: "{% if tail.status == 'completed' %}{{ head.result.missing }}{% else %}ok{% endif %}"
    depends_on: [head]
"#,
    )
    .await;

    assert_eq!(status(&state, "tail"), WorkflowActivityStatus::Failed);
    let error = state.activities["tail"]
        .error
        .as_deref()
        .unwrap_or_default();
    assert!(error.starts_with("TEMPLATE_ERROR:"), "{error}");
    assert_eq!(state.activities["head"].iteration, 2, "failed in round 2");
    assert_eq!(state.status, WorkflowStatus::Failed);
}
