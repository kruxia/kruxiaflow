mod common;

use kruxiaflow_core::events::{EventSource, NewWorkflowEvent, WorkflowEventType, WorkflowStatus};
use kruxiaflow_core::orchestrator::{
    OrchestratorConfig,
    workflow_state::{WorkflowActivityStatus, load_materialized_state},
};
use kruxiaflow_core::queue::{PostgresQueue, QueueConfig};
use kruxiaflow_core::workflow::{
    ActivityDefinition, ActivityRelationship, ActivitySettings, BudgetAction, BudgetSettings,
    WorkflowDefinition,
};
use kruxiaflow_core::{PostgresSubscriptionService, SubscriptionService};
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
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

/// Test simple loop workflow with iteration tracking
/// Pattern: process -> check -> loop back if not done
#[sqlx::test(migrations = "../migrations")]
async fn test_simple_loop_workflow(pool: PgPool) {
    let pool = common::unshared_pool(&pool).await;
    // Create a loop workflow definition (manually validated with metadata set)
    let definition = WorkflowDefinition {
        name: "simple_loop".to_string(),
        settings: None,
        activities: vec![
            ActivityDefinition {
                key: "process".to_string(),
                worker: "test".to_string(),
                activity_name: Some("process_step".to_string()),
                parameters: Some(Default::default()),
                settings: None,
                depends_on: Some(vec![ActivityRelationship {
                    activity_key: "check".to_string(),
                    conditions: Some(vec!["{{check.done | last == false}}".to_string()]),
                    is_back_edge: true, // Back-edge (loop)
                }]),
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: true,
                iteration_limit: Some(3),
                is_loop_activity: true, // Precomputed during validation
                streaming: Default::default(),
            },
            ActivityDefinition {
                key: "check".to_string(),
                worker: "test".to_string(),
                activity_name: Some("check_step".to_string()),
                parameters: Some(Default::default()),
                settings: None,
                depends_on: Some(vec![ActivityRelationship {
                    activity_key: "process".to_string(),
                    conditions: None,
                    is_back_edge: false,
                }]),
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: true,
                iteration_limit: None,
                is_loop_activity: true, // Part of loop
                streaming: Default::default(),
            },
        ],
    };

    // Insert workflow definition
    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();

    // Create event source and queue
    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let queue_config = QueueConfig {
        poll_interval: Duration::from_millis(10),
        batch_size: 10,
        default_timeout: Duration::from_secs(30),
        default_max_retries: 3,
        cleanup_interval: Duration::from_secs(60),
        vacuum_interval: Duration::from_secs(3600),
    };
    let activity_queue: Arc<dyn kruxiaflow_core::queue::ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), queue_config.clone()));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));

    // Publish WorkflowCreated event
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
        .expect("Failed to publish WorkflowCreated event");

    // Process event (should schedule first iteration of process)
    let config = OrchestratorConfig {
        pool: pool.clone(),
        poll_interval_min: Duration::from_millis(10),
        poll_interval_max: Duration::from_secs(1),
        wake_poll_interval_max: Duration::from_secs(1),
        partitions: 4,
        max_in_flight: 256,
        backoff_multiplier: 2.0,
        timeout_check_interval: Duration::from_secs(60),
        secrets: std::collections::HashMap::new(),
    };

    let events = event_source.poll("orchestrator").await.unwrap();
    assert!(!events.is_empty(), "Should have WorkflowCreated event");

    for event in events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await
        .expect("Failed to process event");
        event_source
            .update_position("orchestrator", event.cursor())
            .await
            .unwrap();
    }

    // Verify process activity was scheduled (iteration 0)
    let mut tx = pool.begin().await.unwrap();
    let state = load_materialized_state(&mut tx, workflow_id).await.unwrap();
    tx.commit().await.unwrap();

    let process_state = state.activities.get("process").unwrap();
    assert_eq!(process_state.status, WorkflowActivityStatus::Pending);
    assert_eq!(process_state.iteration, 0);
    assert!(process_state.iteration_outputs.is_some());

    // Simulate process completion (iteration 0)
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::ActivityCompleted,
            activity_key: Some("process".to_string()),
            payload: json!({
                "outputs": {
                    "result": "iteration_0_result"
                }
            }),
            iteration: Some(0),
        })
        .await
        .unwrap();

    // Process the completion event
    let events = event_source.poll("orchestrator").await.unwrap();
    for event in events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.cursor())
            .await
            .unwrap();
    }

    // Verify check activity was scheduled
    let mut tx = pool.begin().await.unwrap();
    let state = load_materialized_state(&mut tx, workflow_id).await.unwrap();
    tx.commit().await.unwrap();

    let check_state = state.activities.get("check").unwrap();
    assert_eq!(check_state.status, WorkflowActivityStatus::Pending);

    let process_state = state.activities.get("process").unwrap();
    assert_eq!(process_state.status, WorkflowActivityStatus::Completed);
    assert_eq!(process_state.iteration, 1); // Incremented after completion

    // Simulate check completion (done = false, should loop back)
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::ActivityCompleted,
            activity_key: Some("check".to_string()),
            payload: json!({
                "outputs": {
                    "done": false
                }
            }),
            iteration: Some(0),
        })
        .await
        .unwrap();

    // Process the check completion
    let events = event_source.poll("orchestrator").await.unwrap();
    for event in events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.cursor())
            .await
            .unwrap();
    }

    // Verify process activity was scheduled again (iteration 1 - loop back)
    let mut tx = pool.begin().await.unwrap();
    let state = load_materialized_state(&mut tx, workflow_id).await.unwrap();
    tx.commit().await.unwrap();

    let process_state = state.activities.get("process").unwrap();
    assert_eq!(process_state.status, WorkflowActivityStatus::Pending);
    assert_eq!(process_state.iteration, 1); // Should still be 1 (increments on completion)

    println!("✓ Simple loop workflow test passed");
}

/// Test iteration limit enforcement
#[sqlx::test(migrations = "../migrations")]
async fn test_loop_max_iterations_enforced(pool: PgPool) {
    let pool = common::unshared_pool(&pool).await;
    // Create a loop workflow with iteration_limit = 2
    let definition = WorkflowDefinition {
        name: "limited_loop".to_string(),
        settings: None,
        activities: vec![ActivityDefinition {
            key: "loop_task".to_string(),
            worker: "test".to_string(),
            activity_name: Some("task".to_string()),
            parameters: Some(Default::default()),
            settings: None,
            depends_on: Some(vec![ActivityRelationship {
                activity_key: "loop_task".to_string(),
                conditions: Some(vec!["{{true}}".to_string()]), // Always true
                is_back_edge: true,
            }]),
            dependency_of: None,
            output_definitions: None,
            iteration_scoped: false,
            iteration_limit: Some(2), // Max 2 iterations
            is_loop_activity: true,
            streaming: Default::default(),
        }],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();

    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let queue_config = QueueConfig {
        poll_interval: Duration::from_millis(10),
        batch_size: 10,
        default_timeout: Duration::from_secs(30),
        default_max_retries: 3,
        cleanup_interval: Duration::from_secs(60),
        vacuum_interval: Duration::from_secs(3600),
    };
    let activity_queue: Arc<dyn kruxiaflow_core::queue::ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), queue_config));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));

    // Create workflow
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

    let config = OrchestratorConfig {
        pool: pool.clone(),
        poll_interval_min: Duration::from_millis(10),
        poll_interval_max: Duration::from_secs(1),
        wake_poll_interval_max: Duration::from_secs(1),
        partitions: 4,
        max_in_flight: 256,
        backoff_multiplier: 2.0,
        timeout_check_interval: Duration::from_secs(60),
        secrets: std::collections::HashMap::new(),
    };

    // Process WorkflowCreated
    let events = event_source.poll("orchestrator").await.unwrap();
    for event in events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.cursor())
            .await
            .unwrap();
    }

    // Iteration 0: Complete the task
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::ActivityCompleted,
            activity_key: Some("loop_task".to_string()),
            payload: json!({
                "outputs": { "result": "iter0" }
            }),
            iteration: Some(0), // Must specify iteration for loop activities
        })
        .await
        .unwrap();

    let events = event_source.poll("orchestrator").await.unwrap();
    for event in events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.cursor())
            .await
            .unwrap();
    }

    // Verify iteration 1 was scheduled
    let mut tx = pool.begin().await.unwrap();
    let state = load_materialized_state(&mut tx, workflow_id).await.unwrap();
    tx.commit().await.unwrap();

    let task_state = state.activities.get("loop_task").unwrap();
    assert_eq!(task_state.status, WorkflowActivityStatus::Pending);
    assert_eq!(task_state.iteration, 1);

    // Iteration 1: Complete the task
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::ActivityCompleted,
            activity_key: Some("loop_task".to_string()),
            payload: json!({
                "outputs": { "result": "iter1" }
            }),
            iteration: Some(1), // Must specify iteration for loop activities
        })
        .await
        .unwrap();

    let events = event_source.poll("orchestrator").await.unwrap();
    for event in events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.cursor())
            .await
            .unwrap();
    }

    // Verify loop stopped (iteration limit reached)
    let mut tx = pool.begin().await.unwrap();
    let state = load_materialized_state(&mut tx, workflow_id).await.unwrap();
    tx.commit().await.unwrap();

    let task_state = state.activities.get("loop_task").unwrap();
    assert_eq!(task_state.status, WorkflowActivityStatus::Completed);
    assert_eq!(task_state.iteration, 2); // Stopped at iteration 2 (>= limit)

    println!("✓ Iteration limit enforcement test passed");
}

/// Test iteration counter for non-iteration_scoped loops
#[sqlx::test(migrations = "../migrations")]
async fn test_iteration_counter_without_scoping(pool: PgPool) {
    let pool = common::unshared_pool(&pool).await;
    // Create a loop workflow without iteration_scoped
    let definition = WorkflowDefinition {
        name: "non_scoped_loop".to_string(),
        settings: None,
        activities: vec![ActivityDefinition {
            key: "counter".to_string(),
            worker: "test".to_string(),
            activity_name: Some("count".to_string()),
            parameters: Some(Default::default()),
            settings: None,
            depends_on: Some(vec![ActivityRelationship {
                activity_key: "counter".to_string(),
                conditions: Some(vec!["{{true}}".to_string()]),
                is_back_edge: true,
            }]),
            dependency_of: None,
            output_definitions: None,
            iteration_scoped: false, // No iteration scoping
            iteration_limit: Some(3),
            is_loop_activity: true,
            streaming: Default::default(),
        }],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();

    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let queue_config = QueueConfig {
        poll_interval: Duration::from_millis(10),
        batch_size: 10,
        default_timeout: Duration::from_secs(30),
        default_max_retries: 3,
        cleanup_interval: Duration::from_secs(60),
        vacuum_interval: Duration::from_secs(3600),
    };
    let activity_queue: Arc<dyn kruxiaflow_core::queue::ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), queue_config));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));

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

    let config = OrchestratorConfig {
        pool: pool.clone(),
        poll_interval_min: Duration::from_millis(10),
        poll_interval_max: Duration::from_secs(1),
        wake_poll_interval_max: Duration::from_secs(1),
        partitions: 4,
        max_in_flight: 256,
        backoff_multiplier: 2.0,
        timeout_check_interval: Duration::from_secs(60),
        secrets: std::collections::HashMap::new(),
    };

    // Process WorkflowCreated
    let events = event_source.poll("orchestrator").await.unwrap();
    for event in events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.cursor())
            .await
            .unwrap();
    }

    // Complete iteration 0
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::ActivityCompleted,
            activity_key: Some("counter".to_string()),
            payload: json!({
                "outputs": { "count": 1 }
            }),
            iteration: Some(0),
        })
        .await
        .unwrap();

    let events = event_source.poll("orchestrator").await.unwrap();
    for event in events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.cursor())
            .await
            .unwrap();
    }

    // Verify iteration counter incremented but no iteration_outputs
    let mut tx = pool.begin().await.unwrap();
    let state = load_materialized_state(&mut tx, workflow_id).await.unwrap();
    tx.commit().await.unwrap();

    let counter_state = state.activities.get("counter").unwrap();
    assert_eq!(counter_state.iteration, 1);
    assert!(counter_state.iteration_outputs.is_none()); // No iteration scoping

    println!("✓ Iteration counter without scoping test passed");
}

/// Test budget accumulation across iterations
#[sqlx::test(migrations = "../migrations")]
async fn test_iteration_budget_accumulation(pool: PgPool) {
    let pool = common::unshared_pool(&pool).await;
    // Create a loop workflow with budget limit that spans multiple iterations
    let definition = WorkflowDefinition {
        name: "budget_loop".to_string(),
        settings: None,
        activities: vec![ActivityDefinition {
            key: "expensive_task".to_string(),
            worker: "llm".to_string(),
            activity_name: Some("llm_prompt".to_string()),
            parameters: Some({
                let mut params = std::collections::HashMap::new();
                params.insert(
                    "model".to_string(),
                    json!("anthropic/claude-3-haiku-20240307"),
                );
                params.insert("prompt".to_string(), json!("test"));
                params.insert("max_tokens".to_string(), json!(100));
                params
            }),
            settings: Some(ActivitySettings {
                timeout_seconds: None,
                retry: None,
                budget: Some(BudgetSettings {
                    limit: rust_decimal::Decimal::new(10, 0), // $10 limit
                    action: BudgetAction::Abort,
                }),
                cache: false,
                cache_ttl: None,
                iteration_limit: None,
                delay: None,
                scheduled_for: None,
                wait_for_signal: None,
                ..Default::default()
            }),
            depends_on: Some(vec![ActivityRelationship {
                activity_key: "expensive_task".to_string(),
                conditions: Some(vec!["{{true}}".to_string()]), // Always loop
                is_back_edge: true,
            }]),
            dependency_of: None,
            output_definitions: None,
            iteration_scoped: true,
            iteration_limit: Some(5),
            is_loop_activity: true,
            streaming: Default::default(),
        }],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();

    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let queue_config = QueueConfig {
        poll_interval: Duration::from_millis(10),
        batch_size: 10,
        default_timeout: Duration::from_secs(30),
        default_max_retries: 3,
        cleanup_interval: Duration::from_secs(60),
        vacuum_interval: Duration::from_secs(3600),
    };
    let activity_queue: Arc<dyn kruxiaflow_core::queue::ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), queue_config));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));

    // Create workflow
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

    let config = OrchestratorConfig {
        pool: pool.clone(),
        poll_interval_min: Duration::from_millis(10),
        poll_interval_max: Duration::from_secs(1),
        wake_poll_interval_max: Duration::from_secs(1),
        partitions: 4,
        max_in_flight: 256,
        backoff_multiplier: 2.0,
        timeout_check_interval: Duration::from_secs(60),
        secrets: std::collections::HashMap::new(),
    };

    // Process WorkflowCreated
    let events = event_source.poll("orchestrator").await.unwrap();
    for event in events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.cursor())
            .await
            .unwrap();
    }

    // Complete iterations with costs that accumulate
    // Iteration 0: $3.50
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::ActivityCompleted,
            activity_key: Some("expensive_task".to_string()),
            payload: json!({
                "outputs": { "result": "iter0" },
                "cost_usd": "3.50"
            }),
            iteration: Some(0),
        })
        .await
        .unwrap();

    let events = event_source.poll("orchestrator").await.unwrap();
    for event in events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.cursor())
            .await
            .unwrap();
    }

    // Verify accumulated cost after iteration 0
    let mut tx = pool.begin().await.unwrap();
    let state = load_materialized_state(&mut tx, workflow_id).await.unwrap();
    tx.commit().await.unwrap();

    let task_state = state.activities.get("expensive_task").unwrap();
    assert_eq!(
        task_state.accumulated_cost_usd,
        rust_decimal::Decimal::new(350, 2)
    ); // $3.50

    // Iteration 1: $3.50 more (total $7.00)
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::ActivityCompleted,
            activity_key: Some("expensive_task".to_string()),
            payload: json!({
                "outputs": { "result": "iter1" },
                "cost_usd": "3.50"
            }),
            iteration: Some(1),
        })
        .await
        .unwrap();

    let events = event_source.poll("orchestrator").await.unwrap();
    for event in events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.cursor())
            .await
            .unwrap();
    }

    // Verify accumulated cost after iteration 1
    let mut tx = pool.begin().await.unwrap();
    let state = load_materialized_state(&mut tx, workflow_id).await.unwrap();
    tx.commit().await.unwrap();

    let task_state = state.activities.get("expensive_task").unwrap();
    assert_eq!(
        task_state.accumulated_cost_usd,
        rust_decimal::Decimal::new(700, 2)
    ); // $7.00

    // Verify accumulated cost persists across iterations
    assert_eq!(task_state.iteration, 2); // After 2 completions, iteration counter is 2

    println!("✓ Budget accumulation across iterations test passed");
}

/// Test Pattern 1: Fixed iterations only (no condition)
#[sqlx::test(migrations = "../migrations")]
async fn test_loop_pattern_1_fixed_iterations(pool: PgPool) {
    let pool = common::unshared_pool(&pool).await;
    // Create a loop with only iteration_limit (no condition on back-edge)
    let definition = WorkflowDefinition {
        name: "fixed_iterations".to_string(),
        settings: None,
        activities: vec![ActivityDefinition {
            key: "newsletter".to_string(),
            worker: "test".to_string(),
            activity_name: Some("send_issue".to_string()),
            parameters: Some(Default::default()),
            settings: None,
            depends_on: Some(vec![ActivityRelationship {
                activity_key: "newsletter".to_string(),
                conditions: None, // No condition - relies on iteration_limit only
                is_back_edge: true,
            }]),
            dependency_of: None,
            output_definitions: None,
            iteration_scoped: true,
            iteration_limit: Some(3), // Exactly 3 issues
            is_loop_activity: true,
            streaming: Default::default(),
        }],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();

    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let queue_config = QueueConfig {
        poll_interval: Duration::from_millis(10),
        batch_size: 10,
        default_timeout: Duration::from_secs(30),
        default_max_retries: 3,
        cleanup_interval: Duration::from_secs(60),
        vacuum_interval: Duration::from_secs(3600),
    };
    let activity_queue: Arc<dyn kruxiaflow_core::queue::ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), queue_config));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));

    // Create and execute workflow
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

    let config = OrchestratorConfig {
        pool: pool.clone(),
        poll_interval_min: Duration::from_millis(10),
        poll_interval_max: Duration::from_secs(1),
        wake_poll_interval_max: Duration::from_secs(1),
        partitions: 4,
        max_in_flight: 256,
        backoff_multiplier: 2.0,
        timeout_check_interval: Duration::from_secs(60),
        secrets: std::collections::HashMap::new(),
    };

    // Process WorkflowCreated
    let events = event_source.poll("orchestrator").await.unwrap();
    for event in events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.cursor())
            .await
            .unwrap();
    }

    // Execute exactly 3 iterations
    for i in 0..3 {
        event_source
            .publish(NewWorkflowEvent {
                workflow_id,
                event_type: WorkflowEventType::ActivityCompleted,
                activity_key: Some("newsletter".to_string()),
                payload: json!({
                    "outputs": { "issue": format!("issue_{}", i) }
                }),
                iteration: Some(i),
            })
            .await
            .unwrap();

        let events = event_source.poll("orchestrator").await.unwrap();
        for event in events {
            kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
                &event,
                &event_source,
                &activity_queue,
                &subscription_service,
                &config,
            )
            .await
            .unwrap();
            event_source
                .update_position("orchestrator", event.cursor())
                .await
                .unwrap();
        }
    }

    // Verify exactly 3 iterations completed
    let mut tx = pool.begin().await.unwrap();
    let state = load_materialized_state(&mut tx, workflow_id).await.unwrap();
    tx.commit().await.unwrap();

    let newsletter_state = state.activities.get("newsletter").unwrap();
    assert_eq!(newsletter_state.status, WorkflowActivityStatus::Completed);
    assert_eq!(newsletter_state.iteration, 3);
    assert_eq!(
        newsletter_state
            .iteration_outputs
            .as_ref()
            .unwrap()
            .get("issue")
            .unwrap()
            .len(),
        3
    );

    println!("✓ Pattern 1 (fixed iterations) test passed");
}

/// Test Pattern 2: Condition only (with default limit)
#[sqlx::test(migrations = "../migrations")]
async fn test_loop_pattern_2_condition_only(pool: PgPool) {
    let pool = common::unshared_pool(&pool).await;
    // Create a loop with only condition (no explicit iteration_limit)
    // Should use DEFAULT_MAX_ITERATIONS = 100
    let definition = WorkflowDefinition {
        name: "condition_only".to_string(),
        settings: None,
        activities: vec![
            ActivityDefinition {
                key: "poll".to_string(),
                worker: "test".to_string(),
                activity_name: Some("poll_service".to_string()),
                parameters: Some(Default::default()),
                settings: None,
                depends_on: Some(vec![ActivityRelationship {
                    activity_key: "check".to_string(),
                    conditions: Some(vec!["{{check.ready == false}}".to_string()]),
                    is_back_edge: true,
                }]),
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false, // Only need latest value
                iteration_limit: None,   // No explicit limit (uses default 100)
                is_loop_activity: true,
                streaming: Default::default(),
            },
            ActivityDefinition {
                key: "check".to_string(),
                worker: "test".to_string(),
                activity_name: Some("check_status".to_string()),
                parameters: Some(Default::default()),
                settings: None,
                depends_on: Some(vec![ActivityRelationship {
                    activity_key: "poll".to_string(),
                    conditions: None,
                    is_back_edge: false,
                }]),
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: true,
                streaming: Default::default(),
            },
        ],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();

    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let queue_config = QueueConfig {
        poll_interval: Duration::from_millis(10),
        batch_size: 10,
        default_timeout: Duration::from_secs(30),
        default_max_retries: 3,
        cleanup_interval: Duration::from_secs(60),
        vacuum_interval: Duration::from_secs(3600),
    };
    let activity_queue: Arc<dyn kruxiaflow_core::queue::ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), queue_config));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));

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

    let config = OrchestratorConfig {
        pool: pool.clone(),
        poll_interval_min: Duration::from_millis(10),
        poll_interval_max: Duration::from_secs(1),
        wake_poll_interval_max: Duration::from_secs(1),
        partitions: 4,
        max_in_flight: 256,
        backoff_multiplier: 2.0,
        timeout_check_interval: Duration::from_secs(60),
        secrets: std::collections::HashMap::new(),
    };

    // Process WorkflowCreated
    let events = event_source.poll("orchestrator").await.unwrap();
    for event in events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.cursor())
            .await
            .unwrap();
    }

    // Loop 2 times with ready=false, then exit with ready=true
    for i in 0..2 {
        // Complete poll
        event_source
            .publish(NewWorkflowEvent {
                workflow_id,
                event_type: WorkflowEventType::ActivityCompleted,
                activity_key: Some("poll".to_string()),
                payload: json!({
                    "outputs": { "result": format!("poll_{}", i) }
                }),
                iteration: Some(i),
            })
            .await
            .unwrap();

        let events = event_source.poll("orchestrator").await.unwrap();
        for event in events {
            kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
                &event,
                &event_source,
                &activity_queue,
                &subscription_service,
                &config,
            )
            .await
            .unwrap();
            event_source
                .update_position("orchestrator", event.cursor())
                .await
                .unwrap();
        }

        // Complete check with ready=false (loop back)
        event_source
            .publish(NewWorkflowEvent {
                workflow_id,
                event_type: WorkflowEventType::ActivityCompleted,
                activity_key: Some("check".to_string()),
                payload: json!({
                    "outputs": { "ready": false }
                }),
                iteration: Some(i),
            })
            .await
            .unwrap();

        let events = event_source.poll("orchestrator").await.unwrap();
        for event in events {
            kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
                &event,
                &event_source,
                &activity_queue,
                &subscription_service,
                &config,
            )
            .await
            .unwrap();
            event_source
                .update_position("orchestrator", event.cursor())
                .await
                .unwrap();
        }
    }

    // Final iteration: ready=true (exit loop)
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::ActivityCompleted,
            activity_key: Some("poll".to_string()),
            payload: json!({
                "outputs": { "result": "poll_2" }
            }),
            iteration: Some(2),
        })
        .await
        .unwrap();

    let events = event_source.poll("orchestrator").await.unwrap();
    for event in events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.cursor())
            .await
            .unwrap();
    }

    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::ActivityCompleted,
            activity_key: Some("check".to_string()),
            payload: json!({
                "outputs": { "ready": true } // Exit condition
            }),
            iteration: Some(2),
        })
        .await
        .unwrap();

    let events = event_source.poll("orchestrator").await.unwrap();
    for event in events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.cursor())
            .await
            .unwrap();
    }

    // Verify loop exited due to condition (not limit)
    let mut tx = pool.begin().await.unwrap();
    let state = load_materialized_state(&mut tx, workflow_id).await.unwrap();
    tx.commit().await.unwrap();

    let poll_state = state.activities.get("poll").unwrap();
    assert_eq!(poll_state.status, WorkflowActivityStatus::Completed);
    assert_eq!(poll_state.iteration, 3); // 3 iterations (0, 1, 2)
    assert!(poll_state.iteration < 100); // Well under default limit

    println!("✓ Pattern 2 (condition only) test passed");
}

/// #18 [loop-tail-rerun]: in a two-step loop (the 07a shape: start → head ⇄
/// tail) each step runs once per round.
///
/// Drives the real orchestrator: completes whatever is pending, round after
/// round, until nothing is, and counts the runs.
#[sqlx::test(migrations = "../migrations")]
async fn test_two_step_loop_runs_each_step_once_per_round(pool: PgPool) {
    let definition = WorkflowDefinition::from_yaml(
        r#"
name: loop_tail_regression
activities:
  - key: start
    worker: test
    activity_name: echo
  - key: head
    worker: test
    activity_name: echo
    iteration_limit: 3
    depends_on:
      - start
      - activity_key: tail
        conditions: ["{{ tail.status == 'completed' }}"]
  - key: tail
    worker: test
    activity_name: echo
    depends_on: [head]
"#,
    )
    .expect("valid loop definition");
    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();

    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let activity_queue: Arc<dyn kruxiaflow_core::queue::ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));
    let config = OrchestratorConfig::new(pool.clone());

    let process_all = || async {
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
                .expect("process event");
                event_source
                    .update_position("orchestrator", event.cursor())
                    .await
                    .unwrap();
            }
        }
    };

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

    let mut runs: std::collections::BTreeMap<String, u32> = Default::default();
    for _ in 0..50 {
        process_all().await;
        let mut tx = pool.begin().await.unwrap();
        let state = load_materialized_state(&mut tx, workflow_id).await.unwrap();
        tx.commit().await.unwrap();

        let mut pending: Vec<(String, u32)> = state
            .activities
            .values()
            .filter(|a| a.status == WorkflowActivityStatus::Pending)
            .map(|a| (a.key.clone(), a.iteration))
            .collect();
        if pending.is_empty() {
            break;
        }
        pending.sort();
        // Each round must schedule only one step of the loop at a time
        assert_eq!(
            pending.len(),
            1,
            "only one step should be pending at a time, got {pending:?} (runs so far {runs:?})"
        );
        for (key, iteration) in pending {
            *runs.entry(key.clone()).or_default() += 1;
            event_source
                .publish(NewWorkflowEvent {
                    workflow_id,
                    event_type: WorkflowEventType::ActivityCompleted,
                    activity_key: Some(key),
                    payload: json!({"outputs": {"result": "ok"}}),
                    iteration: Some(iteration as i32),
                })
                .await
                .unwrap();
        }
    }
    process_all().await;

    assert_eq!(runs.get("start"), Some(&1), "{runs:?}");
    assert_eq!(
        runs.get("head"),
        Some(&3),
        "head runs its iteration_limit: {runs:?}"
    );
    assert_eq!(
        runs.get("tail"),
        Some(&3),
        "tail runs once per head round: {runs:?}"
    );
    let mut tx = pool.begin().await.unwrap();
    let state = load_materialized_state(&mut tx, workflow_id).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(state.status, WorkflowStatus::Completed, "{runs:?}");
}

/// #18: in a three-step loop (start → head → middle → tail → head) every step
/// runs once per round.
#[sqlx::test(migrations = "../migrations")]
async fn test_three_step_loop_runs_each_step_once_per_round(pool: PgPool) {
    let definition = WorkflowDefinition::from_yaml(
        r#"
name: three_step_loop_regression
activities:
  - key: start
    worker: test
    activity_name: echo
  - key: head
    worker: test
    activity_name: echo
    iteration_limit: 3
    depends_on:
      - start
      - activity_key: tail
        conditions: ["{{ tail.status == 'completed' }}"]
  - key: middle
    worker: test
    activity_name: echo
    depends_on: [head]
  - key: tail
    worker: test
    activity_name: echo
    depends_on: [middle]
"#,
    )
    .expect("valid loop definition");
    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();

    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let activity_queue: Arc<dyn kruxiaflow_core::queue::ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));
    let config = OrchestratorConfig::new(pool.clone());

    let process_all = || async {
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
                .expect("process event");
                event_source
                    .update_position("orchestrator", event.cursor())
                    .await
                    .unwrap();
            }
        }
    };

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

    let mut runs: std::collections::BTreeMap<String, u32> = Default::default();
    for _ in 0..50 {
        process_all().await;
        let mut tx = pool.begin().await.unwrap();
        let state = load_materialized_state(&mut tx, workflow_id).await.unwrap();
        tx.commit().await.unwrap();

        let mut pending: Vec<(String, u32)> = state
            .activities
            .values()
            .filter(|a| a.status == WorkflowActivityStatus::Pending)
            .map(|a| (a.key.clone(), a.iteration))
            .collect();
        if pending.is_empty() {
            break;
        }
        pending.sort();
        // Each round must schedule only one step of the loop at a time
        assert_eq!(
            pending.len(),
            1,
            "only one step should be pending at a time, got {pending:?} (runs so far {runs:?})"
        );
        for (key, iteration) in pending {
            *runs.entry(key.clone()).or_default() += 1;
            event_source
                .publish(NewWorkflowEvent {
                    workflow_id,
                    event_type: WorkflowEventType::ActivityCompleted,
                    activity_key: Some(key),
                    payload: json!({"outputs": {"result": "ok"}}),
                    iteration: Some(iteration as i32),
                })
                .await
                .unwrap();
        }
    }
    process_all().await;

    assert_eq!(runs.get("start"), Some(&1), "{runs:?}");
    assert_eq!(
        runs.get("head"),
        Some(&3),
        "head runs its iteration_limit: {runs:?}"
    );
    assert_eq!(
        runs.get("middle"),
        Some(&3),
        "middle runs every round: {runs:?}"
    );
    assert_eq!(
        runs.get("tail"),
        Some(&3),
        "tail runs once per head round: {runs:?}"
    );
    let mut tx = pool.begin().await.unwrap();
    let state = load_materialized_state(&mut tx, workflow_id).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(state.status, WorkflowStatus::Completed, "{runs:?}");
}

/// A stored definition that marks only the back-edge's two ends as loop
/// activities still runs every step once per round when the orchestrator
/// loads it.
#[sqlx::test(migrations = "../migrations")]
async fn test_three_step_loop_stored_with_endpoint_marking_runs_every_round(pool: PgPool) {
    let definition = WorkflowDefinition::from_yaml(
        r#"
name: three_step_loop_endpoint_marking
activities:
  - key: start
    worker: test
    activity_name: echo
  - key: head
    worker: test
    activity_name: echo
    iteration_limit: 3
    depends_on:
      - start
      - activity_key: tail
        conditions: ["{{ tail.status == 'completed' }}"]
  - key: middle
    worker: test
    activity_name: echo
    depends_on: [head]
  - key: tail
    worker: test
    activity_name: echo
    depends_on: [middle]
"#,
    )
    .expect("valid loop definition");
    // Only the back-edge's two ends marked
    let mut definition = definition;
    for activity in &mut definition.activities {
        if activity.key == "middle" {
            activity.is_loop_activity = false;
        }
    }
    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();

    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let activity_queue: Arc<dyn kruxiaflow_core::queue::ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));
    let config = OrchestratorConfig::new(pool.clone());

    let process_all = || async {
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
                .expect("process event");
                event_source
                    .update_position("orchestrator", event.cursor())
                    .await
                    .unwrap();
            }
        }
    };

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

    let mut runs: std::collections::BTreeMap<String, u32> = Default::default();
    for _ in 0..50 {
        process_all().await;
        let mut tx = pool.begin().await.unwrap();
        let state = load_materialized_state(&mut tx, workflow_id).await.unwrap();
        tx.commit().await.unwrap();

        let mut pending: Vec<(String, u32)> = state
            .activities
            .values()
            .filter(|a| a.status == WorkflowActivityStatus::Pending)
            .map(|a| (a.key.clone(), a.iteration))
            .collect();
        if pending.is_empty() {
            break;
        }
        pending.sort();
        // Each round must schedule only one step of the loop at a time
        assert_eq!(
            pending.len(),
            1,
            "only one step should be pending at a time, got {pending:?} (runs so far {runs:?})"
        );
        for (key, iteration) in pending {
            *runs.entry(key.clone()).or_default() += 1;
            event_source
                .publish(NewWorkflowEvent {
                    workflow_id,
                    event_type: WorkflowEventType::ActivityCompleted,
                    activity_key: Some(key),
                    payload: json!({"outputs": {"result": "ok"}}),
                    iteration: Some(iteration as i32),
                })
                .await
                .unwrap();
        }
    }
    process_all().await;

    assert_eq!(runs.get("start"), Some(&1), "{runs:?}");
    assert_eq!(
        runs.get("head"),
        Some(&3),
        "head runs its iteration_limit: {runs:?}"
    );
    assert_eq!(
        runs.get("middle"),
        Some(&3),
        "middle runs every round: {runs:?}"
    );
    assert_eq!(
        runs.get("tail"),
        Some(&3),
        "tail runs once per head round: {runs:?}"
    );
    let mut tx = pool.begin().await.unwrap();
    let state = load_materialized_state(&mut tx, workflow_id).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(state.status, WorkflowStatus::Completed, "{runs:?}");
}

/// #18: a re-delivered completion from an earlier loop round is ignored.
/// Delivery is at-least-once: after a restart, events whose effects were
/// already saved can be delivered again (the durable cursor trails the
/// processed events). Once the head is Pending for round 1 it passes the
/// status-based duplicate guard, so without the iteration check a re-delivered
/// round-0 completion would be applied as new: the head's iteration would
/// advance and the tail would get spurious new input. (Re-publishing would not
/// show this: the event table's dedup index drops an identical event, so the
/// test re-processes the original event, as a re-delivery does.)
#[sqlx::test(migrations = "../migrations")]
async fn test_redelivered_completion_from_an_earlier_round_is_ignored(pool: PgPool) {
    let definition = WorkflowDefinition::from_yaml(
        r#"
name: loop_replay_regression
activities:
  - key: start
    worker: test
    activity_name: echo
  - key: head
    worker: test
    activity_name: echo
    iteration_limit: 3
    depends_on:
      - start
      - activity_key: tail
        conditions: ["{{ tail.status == 'completed' }}"]
  - key: tail
    worker: test
    activity_name: echo
    depends_on: [head]
"#,
    )
    .expect("valid loop definition");
    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();

    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let activity_queue: Arc<dyn kruxiaflow_core::queue::ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));
    let config = OrchestratorConfig::new(pool.clone());

    let processed = std::sync::Mutex::new(Vec::new());
    let process_all = || async {
        loop {
            let events = event_source.poll("orchestrator").await.unwrap();
            if events.is_empty() {
                break;
            }
            for event in events {
                processed.lock().unwrap().push(event.clone());
                kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
                    &event,
                    &event_source,
                    &activity_queue,
                    &subscription_service,
                    &config,
                )
                .await
                .expect("process event");
                event_source
                    .update_position("orchestrator", event.cursor())
                    .await
                    .unwrap();
            }
        }
    };
    let complete = |key: &'static str, iteration: Option<i32>| {
        let event_source = event_source.clone();
        async move {
            event_source
                .publish(NewWorkflowEvent {
                    workflow_id,
                    event_type: WorkflowEventType::ActivityCompleted,
                    activity_key: Some(key.to_string()),
                    payload: json!({"outputs": {"result": key}}),
                    iteration,
                })
                .await
                .unwrap();
        }
    };
    let load = || async {
        let mut tx = pool.begin().await.unwrap();
        let state = load_materialized_state(&mut tx, workflow_id).await.unwrap();
        tx.commit().await.unwrap();
        state
    };

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
    process_all().await;
    complete("start", None).await;
    process_all().await;
    complete("head", Some(0)).await;
    process_all().await;
    complete("tail", Some(0)).await;
    process_all().await;

    // Round 1 has begun: head is Pending again, at iteration 1
    let before = load().await;
    assert_eq!(
        before.activities["head"].status,
        WorkflowActivityStatus::Pending
    );
    assert_eq!(before.activities["head"].iteration, 1);
    assert_eq!(
        before.activities["tail"].status,
        WorkflowActivityStatus::Completed
    );

    // Re-deliver head's round-0 completion: the same stored event, again
    let head_round_0 = processed
        .lock()
        .unwrap()
        .iter()
        .find(|e| {
            e.event_type == WorkflowEventType::ActivityCompleted
                && e.activity_key.as_deref() == Some("head")
                && e.iteration == Some(0)
        })
        .cloned()
        .expect("head's round-0 completion was processed");
    kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
        &head_round_0,
        &event_source,
        &activity_queue,
        &subscription_service,
        &config,
    )
    .await
    .expect("re-delivery processes cleanly");
    process_all().await;

    let after = load().await;
    assert_eq!(
        after.activities["head"].status,
        WorkflowActivityStatus::Pending,
        "the replay is ignored"
    );
    assert_eq!(
        after.activities["head"].iteration, 1,
        "the iteration does not advance"
    );
    assert_eq!(
        after.activities["tail"].status,
        WorkflowActivityStatus::Completed,
        "the tail gets no spurious new input"
    );
}

/// #26 [loop-backedge-by-name]: the repair loop `run → check → fix → run`
/// starts with `run`, the step written first, and runs each step once per
/// round.
#[sqlx::test(migrations = "../migrations")]
async fn test_repair_loop_starts_with_its_first_written_step(pool: PgPool) {
    let definition = WorkflowDefinition::from_yaml(
        r#"
name: repair_loop_regression
activities:
  - key: run
    worker: test
    activity_name: echo
    iteration_limit: 5
    depends_on:
      - activity_key: fix
        conditions: ["{{ fix.status == 'completed' }}"]
  - key: check
    worker: test
    activity_name: echo
    depends_on: [run]
  - key: fix
    worker: test
    activity_name: echo
    depends_on:
      - activity_key: check
        conditions: ["{{ check.result == 'fail' }}"]
  - key: done
    worker: test
    activity_name: echo
    depends_on:
      - activity_key: check
        conditions: ["{{ check.result == 'pass' }}"]
"#,
    )
    .expect("valid loop definition");
    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();

    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let activity_queue: Arc<dyn kruxiaflow_core::queue::ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));
    let config = OrchestratorConfig::new(pool.clone());

    let process_all = || async {
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
                .expect("process event");
                event_source
                    .update_position("orchestrator", event.cursor())
                    .await
                    .unwrap();
            }
        }
    };

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

    // The check fails twice, then passes
    let mut order: Vec<String> = Vec::new();
    let mut checks = 0;
    for _ in 0..50 {
        process_all().await;
        let mut tx = pool.begin().await.unwrap();
        let state = load_materialized_state(&mut tx, workflow_id).await.unwrap();
        tx.commit().await.unwrap();

        let pending: Vec<(String, u32)> = state
            .activities
            .values()
            .filter(|a| a.status == WorkflowActivityStatus::Pending)
            .map(|a| (a.key.clone(), a.iteration))
            .collect();
        if pending.is_empty() {
            break;
        }
        assert_eq!(
            pending.len(),
            1,
            "only one step should be pending at a time, got {pending:?} (order so far {order:?})"
        );
        let (key, iteration) = pending.into_iter().next().unwrap();
        let result = if key == "check" {
            checks += 1;
            if checks < 3 { "fail" } else { "pass" }
        } else {
            "ok"
        };
        order.push(key.clone());
        event_source
            .publish(NewWorkflowEvent {
                workflow_id,
                event_type: WorkflowEventType::ActivityCompleted,
                activity_key: Some(key),
                payload: json!({"outputs": {"result": result}}),
                iteration: Some(iteration as i32),
            })
            .await
            .unwrap();
    }
    process_all().await;

    // Only the loop's own steps: whether `done` runs after the loop is not
    // what this test is about
    let loop_steps: Vec<&str> = order
        .iter()
        .map(String::as_str)
        .filter(|key| *key != "done")
        .collect();
    assert_eq!(
        loop_steps,
        ["run", "check", "fix", "run", "check", "fix", "run", "check"],
        "each round runs run, check, then fix while the check fails"
    );
}
