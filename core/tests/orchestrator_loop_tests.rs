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
/// tail) each step runs once per round. The tail used to re-run after every
/// one of its own completions, on the head's stale output, until the default
/// cap of 100 iterations, and the head re-ran on the tail's stale output.
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
}

/// #18: in a three-step loop (start → head → middle → tail → head) every step
/// runs once per round. The middle step used to be left out of the loop (only
/// a back-edge's endpoints were marked as loop members), so it never ran again.
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
        runs.get("tail"),
        Some(&3),
        "tail runs once per head round: {runs:?}"
    );
}

/// A definition stored before the fix marks only the back-edge's two ends as
/// loop activities. Loaded by the orchestrator, it must still run every step
/// once per round (it used to stop after the head's second run), without a
/// redeploy.
#[sqlx::test(migrations = "../migrations")]
async fn test_three_step_loop_stored_with_the_old_marking_runs_every_round(pool: PgPool) {
    let definition = WorkflowDefinition::from_yaml(
        r#"
name: three_step_loop_old_marking
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
    // Stored as before the fix: only the back-edge's two ends are marked
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
        runs.get("tail"),
        Some(&3),
        "tail runs once per head round: {runs:?}"
    );
}

/// #18: a re-delivered completion from an earlier loop round is ignored.
/// Delivery is at-least-once: after a restart, events whose effects were
/// already saved can be delivered again (the durable cursor trails the
/// processed events). Once the head is Pending for round 1 it passes the
/// status-based duplicate guard, so before the iteration check a re-delivered
/// round-0 completion was applied as new: the head's iteration advanced and the
/// tail got spurious new input. (Re-publishing would not show this: the event
/// table's dedup index drops an identical event, so the test re-processes the
/// original event, as a re-delivery does.)
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

/// Drive a workflow through the real orchestrator to the end: complete every
/// pending activity (with the outputs `outputs(key, run)` gives for its
/// `run`-th execution, counting from 1), round after round, until nothing is
/// pending. Completions carry the activity's iteration, as the worker does.
/// Returns how often each activity ran and the final state.
async fn run_to_end(
    pool: &PgPool,
    yaml: &str,
    outputs: impl Fn(&str, u32) -> serde_json::Value,
) -> (
    std::collections::BTreeMap<String, u32>,
    kruxiaflow_core::orchestrator::WorkflowState,
) {
    let definition = WorkflowDefinition::from_yaml(yaml).expect("valid definition");
    let definition_id = insert_workflow_definition(pool, &definition).await;
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

    let mut runs: std::collections::BTreeMap<String, u32> = Default::default();
    for _ in 0..100 {
        process_all().await;
        let state = load().await;
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
        for (key, iteration) in pending {
            let run = runs.entry(key.clone()).or_default();
            *run += 1;
            let payload = json!({ "outputs": outputs(&key, *run) });
            event_source
                .publish(NewWorkflowEvent {
                    workflow_id,
                    event_type: WorkflowEventType::ActivityCompleted,
                    activity_key: Some(key),
                    payload,
                    iteration: Some(iteration as i32),
                })
                .await
                .unwrap();
        }
    }
    process_all().await;
    (runs, load().await)
}

/// The repair loop of #27: run, check, fix while the check fails, and finish
/// when it passes. Keys sort in written order (see #26).
const REPAIR_LOOP: &str = r#"
name: repair
activities:
  - key: a_run
    worker: test
    activity_name: echo
    iteration_limit: 5
    depends_on:
      - activity_key: c_fix
        conditions: ["{{ c_fix.status == 'completed' }}"]
  - key: b_check
    worker: test
    activity_name: echo
    depends_on: [a_run]
  - key: c_fix
    worker: test
    activity_name: echo
    depends_on:
      - activity_key: b_check
        conditions: ["{{ b_check.result == 'fail' }}"]
  - key: d_done
    worker: test
    activity_name: echo
    depends_on:
      - activity_key: b_check
        conditions: ["{{ b_check.result == 'pass' }}"]
  - key: e_report
    worker: test
    activity_name: echo
    depends_on: [d_done]
  - key: f_give_up
    worker: test
    activity_name: echo
    depends_on:
      - activity_key: d_done
        conditions: ["{{ d_done.status == 'skipped' }}"]
"#;

/// `b_check` results: `fail` for its first `fails` runs, then `pass`
fn check_results(fails: u32) -> impl Fn(&str, u32) -> serde_json::Value {
    move |key, run| {
        if key == "b_check" {
            json!({ "result": if run <= fails { "fail" } else { "pass" } })
        } else {
            json!({ "result": key })
        }
    }
}

fn status_of(
    state: &kruxiaflow_core::orchestrator::WorkflowState,
    key: &str,
) -> WorkflowActivityStatus {
    state.activities[key].status
}

/// #27 [loop-exit-skipped]: the loop's exit step runs when a later round
/// exits. It used to be skipped after the first check failed, for good, and
/// the workflow completed without it.
#[sqlx::test(migrations = "../migrations")]
async fn test_loop_exit_step_runs_when_a_later_round_exits(pool: PgPool) {
    let (runs, state) = run_to_end(&pool, REPAIR_LOOP, check_results(2)).await;

    assert_eq!(runs.get("a_run"), Some(&3), "{runs:?}");
    assert_eq!(runs.get("b_check"), Some(&3), "{runs:?}");
    assert_eq!(runs.get("c_fix"), Some(&2), "{runs:?}");
    assert_eq!(
        runs.get("d_done"),
        Some(&1),
        "exit step runs once: {runs:?}"
    );
    assert_eq!(runs.get("e_report"), Some(&1), "{runs:?}");
    assert_eq!(
        status_of(&state, "d_done"),
        WorkflowActivityStatus::Completed
    );
    assert_eq!(
        status_of(&state, "f_give_up"),
        WorkflowActivityStatus::Skipped
    );
    assert_eq!(state.status, WorkflowStatus::Completed);
}

/// #27: when the loop exits in its first round, the exit step runs then
#[sqlx::test(migrations = "../migrations")]
async fn test_loop_exit_step_runs_when_the_first_round_exits(pool: PgPool) {
    let (runs, state) = run_to_end(&pool, REPAIR_LOOP, check_results(0)).await;

    assert_eq!(runs.get("a_run"), Some(&1), "{runs:?}");
    assert_eq!(runs.get("b_check"), Some(&1), "{runs:?}");
    assert_eq!(runs.get("c_fix"), None, "{runs:?}");
    assert_eq!(runs.get("d_done"), Some(&1), "{runs:?}");
    assert_eq!(status_of(&state, "c_fix"), WorkflowActivityStatus::Skipped);
    assert_eq!(state.status, WorkflowStatus::Completed);
}

/// #27: when the loop reaches its iteration limit without passing, the exit
/// step is skipped once the loop has ended, what depends on it is decided in
/// the same pass (`e_report` skipped, `f_give_up` run), and the workflow
/// completes instead of waiting for it.
#[sqlx::test(migrations = "../migrations")]
async fn test_loop_exit_step_is_skipped_when_the_iteration_limit_is_reached(pool: PgPool) {
    let (runs, state) = run_to_end(&pool, REPAIR_LOOP, check_results(u32::MAX)).await;

    assert_eq!(runs.get("a_run"), Some(&5), "{runs:?}");
    assert_eq!(runs.get("b_check"), Some(&5), "{runs:?}");
    assert_eq!(runs.get("c_fix"), Some(&5), "{runs:?}");
    assert_eq!(runs.get("d_done"), None, "{runs:?}");
    assert_eq!(status_of(&state, "d_done"), WorkflowActivityStatus::Skipped);
    assert_eq!(
        status_of(&state, "e_report"),
        WorkflowActivityStatus::Skipped
    );
    assert_eq!(runs.get("f_give_up"), Some(&1), "{runs:?}");
    assert_eq!(state.status, WorkflowStatus::Completed);
}

/// #27: a conditional step outside any loop is still skipped as soon as its
/// condition is false, even while other activities are still running
#[sqlx::test(migrations = "../migrations")]
async fn test_conditional_skip_outside_a_loop_is_unchanged(pool: PgPool) {
    let yaml = r#"
name: branch
activities:
  - key: a_start
    worker: test
    activity_name: echo
  - key: b_slow
    worker: test
    activity_name: echo
    depends_on: [a_start]
  - key: c_never
    worker: test
    activity_name: echo
    depends_on:
      - activity_key: a_start
        conditions: ["{{ a_start.result == 'other' }}"]
"#;
    let definition = WorkflowDefinition::from_yaml(yaml).expect("valid definition");
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
    for (event_type, key) in [
        (WorkflowEventType::WorkflowCreated, None),
        (WorkflowEventType::ActivityCompleted, Some("a_start")),
    ] {
        event_source
            .publish(NewWorkflowEvent {
                workflow_id,
                event_type,
                activity_key: key.map(str::to_string),
                payload: if key.is_some() {
                    json!({"outputs": {"result": "ok"}})
                } else {
                    json!({
                        "definition_name": definition.name,
                        "workflow_definition_id": definition_id,
                    })
                },
                iteration: None,
            })
            .await
            .unwrap();
        process_all().await;
    }

    let mut tx = pool.begin().await.unwrap();
    let state = load_materialized_state(&mut tx, workflow_id).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(status_of(&state, "b_slow"), WorkflowActivityStatus::Pending);
    assert_eq!(
        status_of(&state, "c_never"),
        WorkflowActivityStatus::Skipped,
        "skipped right away, not when the workflow ends"
    );
}

/// #27: skipping one activity decides what depends on it in the same pass.
/// A loop's exit step is now skipped in the loop's last pass, after which no
/// event may come; everything after it must be decided then, or the workflow
/// never completes. (This also hung without a loop: `c_after` and `d_else`
/// stayed unscheduled and the workflow stayed running.)
#[sqlx::test(migrations = "../migrations")]
async fn test_skips_are_followed_through_in_the_same_pass(pool: PgPool) {
    let yaml = r#"
name: chain
activities:
  - key: a_start
    worker: test
    activity_name: echo
  - key: b_maybe
    worker: test
    activity_name: echo
    depends_on:
      - activity_key: a_start
        conditions: ["{{ a_start.result == 'other' }}"]
  - key: c_after
    worker: test
    activity_name: echo
    depends_on: [b_maybe]
  - key: d_else
    worker: test
    activity_name: echo
    depends_on:
      - activity_key: b_maybe
        conditions: ["{{ b_maybe.status == 'skipped' }}"]
"#;
    let (runs, state) = run_to_end(&pool, yaml, |key, _| json!({ "result": key })).await;

    assert_eq!(
        status_of(&state, "b_maybe"),
        WorkflowActivityStatus::Skipped
    );
    assert_eq!(
        status_of(&state, "c_after"),
        WorkflowActivityStatus::Skipped
    );
    assert_eq!(runs.get("d_else"), Some(&1), "{runs:?}");
    assert_eq!(state.status, WorkflowStatus::Completed);
}

/// What [`drive_loop_workflow`] saw: how often each activity ran, the set of
/// pending activities at every step, and the workflow's final status.
struct LoopRun {
    workflow_id: Uuid,
    runs: std::collections::BTreeMap<String, u32>,
    pending_history: Vec<Vec<String>>,
    status: kruxiaflow_core::events::models::WorkflowStatus,
}

/// Drive a workflow through the real orchestrator, acting as the worker:
/// at every step, complete the pending activities listed in `first` if any
/// are pending, otherwise every pending activity. Completions carry the loop
/// activity's iteration, and output it as `round`. Stops when nothing is pending (done, or hung).
async fn drive_loop_workflow(pool: &PgPool, yaml: &str, first: &[&str]) -> LoopRun {
    let definition = WorkflowDefinition::from_yaml(yaml).expect("valid loop definition");
    let definition_id = insert_workflow_definition(pool, &definition).await;
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

    let mut runs: std::collections::BTreeMap<String, u32> = Default::default();
    let mut pending_history = Vec::new();
    for _ in 0..100 {
        process_all().await;
        let state = load().await;
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
        pending_history.push(pending.iter().map(|(k, _)| k.clone()).collect());
        let preferred: Vec<(String, u32)> = pending
            .iter()
            .filter(|(k, _)| first.contains(&k.as_str()))
            .cloned()
            .collect();
        let to_complete = if preferred.is_empty() {
            pending
        } else {
            preferred
        };
        for (key, iteration) in to_complete {
            *runs.entry(key.clone()).or_default() += 1;
            event_source
                .publish(NewWorkflowEvent {
                    workflow_id,
                    event_type: WorkflowEventType::ActivityCompleted,
                    activity_key: Some(key),
                    // `round` lets conditions tell rounds apart (0-based)
                    payload: json!({"outputs": {"result": "ok", "round": iteration}}),
                    iteration: Some(iteration as i32),
                })
                .await
                .unwrap();
        }
    }
    process_all().await;
    let status = load().await.status;
    LoopRun {
        workflow_id,
        runs,
        pending_history,
        status,
    }
}

/// #28 [loop-join-stale-input]: a join of parallel branches of different
/// lengths inside a loop runs once per round, after both branches finished
/// that round. It used to run as soon as the short branch finished, on the
/// long branch's output from the previous round, and then again.
#[sqlx::test(migrations = "../migrations")]
async fn test_loop_join_waits_for_every_branch_of_the_round(pool: PgPool) {
    let run = drive_loop_workflow(
        &pool,
        r#"
name: diamond_loop
activities:
  - key: start
    worker: test
    activity_name: echo
  - key: a_head
    worker: test
    activity_name: echo
    iteration_limit: 3
    depends_on:
      - start
      - activity_key: e_join
        conditions: ["{{ e_join.status == 'completed' }}"]
  - key: b_long1
    worker: test
    activity_name: echo
    depends_on: [a_head]
  - key: c_long2
    worker: test
    activity_name: echo
    depends_on: [b_long1]
  - key: d_short
    worker: test
    activity_name: echo
    depends_on: [a_head]
  - key: e_join
    worker: test
    activity_name: echo
    depends_on: [c_long2, d_short]
"#,
        &["d_short"],
    )
    .await;

    for pending in &run.pending_history {
        let long_branch_busy = pending.iter().any(|k| k == "b_long1" || k == "c_long2");
        assert!(
            !(pending.iter().any(|k| k == "e_join") && long_branch_busy),
            "e_join scheduled while the long branch is still running its round: {pending:?} \
             (history {:?})",
            run.pending_history
        );
    }
    for key in ["a_head", "b_long1", "c_long2", "d_short", "e_join"] {
        assert_eq!(
            run.runs.get(key),
            Some(&3),
            "{key} runs once per round: {:?}",
            run.runs
        );
    }
    assert_eq!(
        run.status,
        kruxiaflow_core::events::models::WorkflowStatus::Completed,
        "{:?}",
        run.runs
    );
}

/// #28: a branch that runs in round 1 but whose condition is false from
/// round 2 on stays `Completed` from round 1. The join must not wait for it
/// (it will not run again), so the join still runs every round and the
/// workflow completes instead of hanging.
#[sqlx::test(migrations = "../migrations")]
async fn test_loop_join_does_not_wait_for_a_branch_that_will_not_run(pool: PgPool) {
    let run = drive_loop_workflow(
        &pool,
        r#"
name: diamond_loop_conditional_branch
activities:
  - key: start
    worker: test
    activity_name: echo
  - key: a_head
    worker: test
    activity_name: echo
    iteration_limit: 3
    depends_on:
      - start
      - activity_key: e_join
        conditions: ["{{ e_join.status == 'completed' }}"]
  - key: b_long1
    worker: test
    activity_name: echo
    depends_on: [a_head]
  - key: c_long2
    worker: test
    activity_name: echo
    depends_on: [b_long1]
  - key: d_cond
    worker: test
    activity_name: echo
    depends_on:
      - activity_key: a_head
        conditions: ["{{ a_head.round == 0 }}"]
  - key: e_join
    worker: test
    activity_name: echo
    depends_on: [c_long2, d_cond]
"#,
        &["d_cond"],
    )
    .await;

    assert_eq!(
        run.runs.get("d_cond"),
        Some(&1),
        "d_cond runs in round 1 only: {:?}",
        run.runs
    );
    for key in ["a_head", "b_long1", "c_long2", "e_join"] {
        assert_eq!(
            run.runs.get(key),
            Some(&3),
            "{key} runs once per round: {:?} (history {:?})",
            run.runs,
            run.pending_history
        );
    }
    assert_eq!(
        run.status,
        kruxiaflow_core::events::models::WorkflowStatus::Completed,
        "the workflow completes, it doesn't hang: {:?} (history {:?})",
        run.runs,
        run.pending_history
    );
}

/// #28: two loops in sequence (loop 2's head depends on loop 1's tail). Each
/// loop's steps belong to that loop only: loop 1's steps are upstream of
/// loop 2's head but never hold its later rounds back (#28's hold), and both
/// loops complete. Loop 2 starts once loop 1 has ended (#40): its head is a
/// step after loop 1.
#[sqlx::test(migrations = "../migrations")]
async fn test_two_sequential_loops_both_complete(pool: PgPool) {
    let run = drive_loop_workflow(
        &pool,
        r#"
name: sequential_loops
activities:
  - key: start
    worker: test
    activity_name: echo
  - key: a_head
    worker: test
    activity_name: echo
    iteration_limit: 3
    depends_on:
      - start
      - activity_key: b_tail
        conditions: ["{{ b_tail.status == 'completed' }}"]
  - key: b_tail
    worker: test
    activity_name: echo
    depends_on: [a_head]
  - key: c_head
    worker: test
    activity_name: echo
    iteration_limit: 2
    depends_on:
      - b_tail
      - activity_key: d_tail
        conditions: ["{{ d_tail.status == 'completed' }}"]
  - key: d_tail
    worker: test
    activity_name: echo
    depends_on: [c_head]
"#,
        &[],
    )
    .await;

    assert_eq!(run.runs.get("start"), Some(&1), "{:?}", run.runs);
    for (key, limit) in [("a_head", 3), ("b_tail", 3), ("c_head", 2), ("d_tail", 2)] {
        assert_eq!(
            run.runs.get(key),
            Some(&limit),
            "{key} runs its loop's iteration_limit: {:?} (history {:?})",
            run.runs,
            run.pending_history
        );
    }
    // Loop 2 starts after loop 1's last round, not alongside its first ones
    // (#40); its second round then follows its own first, with nothing of
    // loop 1 left to hold it back
    let overlapping = run
        .pending_history
        .iter()
        .filter(|p| {
            p.iter().any(|k| k == "c_head" || k == "d_tail")
                && p.iter().any(|k| k == "a_head" || k == "b_tail")
        })
        .count();
    assert_eq!(
        overlapping, 0,
        "loop 2 starts once loop 1 has ended: {:?}",
        run.pending_history
    );
    assert_eq!(
        run.status,
        kruxiaflow_core::events::models::WorkflowStatus::Completed,
        "{:?}",
        run.runs
    );
}

// ============================================================================
// A loop whose way in is closed is skipped [loop-entry-not-skipped]
//
// Entering a loop for the first time, a step's back-edge points at a loop step
// that hasn't run (for a self-loop, the step itself). It used to count as
// satisfied, so the loop ran although its entry condition was false, and it
// kept the skip pass waiting for that step to finish, so a loop behind a
// skipped step stayed not_scheduled and the workflow hung.
// ============================================================================

/// `gate` echoes `open` from `open_gate`; everything else echoes its key
fn gate_outputs(open_gate: bool) -> impl Fn(&str, u32) -> serde_json::Value {
    move |key, run| match key {
        "gate" => json!({ "open": open_gate }),
        _ => json!({ "step": key, "run": run }),
    }
}

/// A self-loop behind a conditional entry, and a step after it
const GATED_SELF_LOOP: &str = r#"
name: gated_self_loop
activities:
  - key: gate
    worker: test
    activity_name: echo
  - key: porter
    worker: test
    activity_name: echo
    iteration_limit: 3
    depends_on:
      - activity_key: gate
        conditions: ["{{ gate.open }}"]
      - porter
  - key: lobby
    worker: test
    activity_name: echo
    depends_on: [porter]
"#;

/// The entry condition is false: the self-loop is skipped, and so is the step
/// after it; the workflow completes. It used to run three times.
#[sqlx::test(migrations = "../migrations")]
async fn test_self_loop_with_a_false_entry_condition_is_skipped(pool: PgPool) {
    let (runs, state) = run_to_end(&pool, GATED_SELF_LOOP, gate_outputs(false)).await;

    assert_eq!(runs.get("gate"), Some(&1), "{runs:?}");
    assert_eq!(runs.get("porter"), None, "the loop never starts: {runs:?}");
    assert_eq!(runs.get("lobby"), None, "{runs:?}");
    assert_eq!(status_of(&state, "porter"), WorkflowActivityStatus::Skipped);
    assert_eq!(status_of(&state, "lobby"), WorkflowActivityStatus::Skipped);
    assert_eq!(state.status, WorkflowStatus::Completed);
}

/// The entry condition is true: the loop runs to its iteration_limit, as before
#[sqlx::test(migrations = "../migrations")]
async fn test_self_loop_with_a_true_entry_condition_runs(pool: PgPool) {
    let (runs, state) = run_to_end(&pool, GATED_SELF_LOOP, gate_outputs(true)).await;

    assert_eq!(runs.get("porter"), Some(&3), "{runs:?}");
    assert_eq!(runs.get("lobby"), Some(&1), "{runs:?}");
    assert_eq!(state.status, WorkflowStatus::Completed);
}

/// The entry is a skipped step: the self-loop is skipped too, and the skip
/// cascades past it; the workflow completes. The loop used to stay
/// not_scheduled, and the workflow running, for ever.
#[sqlx::test(migrations = "../migrations")]
async fn test_self_loop_behind_a_skipped_step_is_skipped(pool: PgPool) {
    let yaml = r#"
name: skipped_entry_self_loop
activities:
  - key: gate
    worker: test
    activity_name: echo
  - key: bell
    worker: test
    activity_name: echo
    depends_on:
      - activity_key: gate
        conditions: ["{{ gate.open }}"]
  - key: ringer
    worker: test
    activity_name: echo
    iteration_limit: 3
    depends_on: [bell, ringer]
  - key: lobby
    worker: test
    activity_name: echo
    depends_on: [ringer]
"#;
    let (runs, state) = run_to_end(&pool, yaml, gate_outputs(false)).await;

    assert_eq!(runs.get("ringer"), None, "{runs:?}");
    assert_eq!(status_of(&state, "bell"), WorkflowActivityStatus::Skipped);
    assert_eq!(status_of(&state, "ringer"), WorkflowActivityStatus::Skipped);
    assert_eq!(status_of(&state, "lobby"), WorkflowActivityStatus::Skipped);
    assert_eq!(state.status, WorkflowStatus::Completed);
}

/// A two-step loop (head and tail) behind a conditional entry, and a step
/// after it that runs only when the loop was skipped
const GATED_TWO_STEP_LOOP: &str = r#"
name: gated_two_step_loop
activities:
  - key: gate
    worker: test
    activity_name: echo
  - key: a_head
    worker: test
    activity_name: echo
    iteration_limit: 3
    depends_on:
      - activity_key: gate
        conditions: ["{{ gate.open }}"]
      - activity_key: b_tail
        conditions: ["{{ b_tail.status == 'completed' }}"]
  - key: b_tail
    worker: test
    activity_name: echo
    depends_on: [a_head]
  - key: c_closed
    worker: test
    activity_name: echo
    depends_on:
      - activity_key: b_tail
        conditions: ["{{ b_tail.status == 'skipped' }}"]
"#;

/// The head's entry condition is false: head and tail are skipped, and the
/// step that handles a skipped loop runs. The loop used to run three rounds.
#[sqlx::test(migrations = "../migrations")]
async fn test_two_step_loop_with_a_false_entry_condition_is_skipped(pool: PgPool) {
    let (runs, state) = run_to_end(&pool, GATED_TWO_STEP_LOOP, gate_outputs(false)).await;

    assert_eq!(runs.get("a_head"), None, "{runs:?}");
    assert_eq!(runs.get("b_tail"), None, "{runs:?}");
    assert_eq!(status_of(&state, "a_head"), WorkflowActivityStatus::Skipped);
    assert_eq!(status_of(&state, "b_tail"), WorkflowActivityStatus::Skipped);
    assert_eq!(runs.get("c_closed"), Some(&1), "{runs:?}");
    assert_eq!(state.status, WorkflowStatus::Completed);
}

/// The head's entry condition is true: three rounds, and the skipped-loop
/// step is skipped
#[sqlx::test(migrations = "../migrations")]
async fn test_two_step_loop_with_a_true_entry_condition_runs(pool: PgPool) {
    let (runs, state) = run_to_end(&pool, GATED_TWO_STEP_LOOP, gate_outputs(true)).await;

    assert_eq!(runs.get("a_head"), Some(&3), "{runs:?}");
    assert_eq!(runs.get("b_tail"), Some(&3), "{runs:?}");
    assert_eq!(
        status_of(&state, "c_closed"),
        WorkflowActivityStatus::Skipped
    );
    assert_eq!(state.status, WorkflowStatus::Completed);
}

/// A two-step loop whose head depends on a skipped step: head and tail are
/// skipped and the workflow completes. It used to hang with both
/// not_scheduled.
#[sqlx::test(migrations = "../migrations")]
async fn test_two_step_loop_behind_a_skipped_step_is_skipped(pool: PgPool) {
    let yaml = r#"
name: skipped_entry_two_step_loop
activities:
  - key: gate
    worker: test
    activity_name: echo
  - key: bell
    worker: test
    activity_name: echo
    depends_on:
      - activity_key: gate
        conditions: ["{{ gate.open }}"]
  - key: c_head
    worker: test
    activity_name: echo
    iteration_limit: 3
    depends_on:
      - bell
      - activity_key: d_tail
        conditions: ["{{ d_tail.status == 'completed' }}"]
  - key: d_tail
    worker: test
    activity_name: echo
    depends_on: [c_head]
"#;
    let (runs, state) = run_to_end(&pool, yaml, gate_outputs(false)).await;

    assert_eq!(runs.get("c_head"), None, "{runs:?}");
    assert_eq!(status_of(&state, "c_head"), WorkflowActivityStatus::Skipped);
    assert_eq!(status_of(&state, "d_tail"), WorkflowActivityStatus::Skipped);
    assert_eq!(state.status, WorkflowStatus::Completed);
}

/// Two entry dependencies decide like any step's: the loop runs when one of
/// them applies, and is skipped when neither does
#[sqlx::test(migrations = "../migrations")]
async fn test_loop_with_two_entry_dependencies(pool: PgPool) {
    let yaml = |second_open: bool| {
        format!(
            r#"
name: two_entries_{second_open}
activities:
  - key: gate
    worker: test
    activity_name: echo
  - key: side_gate
    worker: test
    activity_name: echo
  - key: porter
    worker: test
    activity_name: echo
    iteration_limit: 2
    depends_on:
      - activity_key: gate
        conditions: ["{{{{ gate.open }}}}"]
      - activity_key: side_gate
        conditions: ["{{{{ {second_open} }}}}"]
      - porter
"#
        )
    };

    let (runs, state) = run_to_end(&pool, &yaml(true), gate_outputs(false)).await;
    assert_eq!(runs.get("porter"), Some(&2), "one way in is open: {runs:?}");
    assert_eq!(state.status, WorkflowStatus::Completed);

    let (runs, state) = run_to_end(&pool, &yaml(false), gate_outputs(false)).await;
    assert_eq!(runs.get("porter"), None, "both ways in are shut: {runs:?}");
    assert_eq!(status_of(&state, "porter"), WorkflowActivityStatus::Skipped);
    assert_eq!(state.status, WorkflowStatus::Completed);
}

/// The entry written with `dependency_of` on the gate decides the same way
#[sqlx::test(migrations = "../migrations")]
async fn test_loop_entry_written_with_dependency_of_is_skipped(pool: PgPool) {
    let yaml = r#"
name: dependency_of_entry
activities:
  - key: gate
    worker: test
    activity_name: echo
    dependency_of:
      - activity_key: porter
        conditions: ["{{ gate.open }}"]
  - key: porter
    worker: test
    activity_name: echo
    iteration_limit: 3
    depends_on: [porter]
"#;
    let (runs, state) = run_to_end(&pool, yaml, gate_outputs(false)).await;

    assert_eq!(runs.get("porter"), None, "{runs:?}");
    assert_eq!(status_of(&state, "porter"), WorkflowActivityStatus::Skipped);
    assert_eq!(state.status, WorkflowStatus::Completed);
}

/// A loop with no dependency outside it starts with the workflow, as before:
/// its back-edge neither lets it in nor keeps it out
#[sqlx::test(migrations = "../migrations")]
async fn test_loop_entered_from_the_workflow_start_runs(pool: PgPool) {
    let yaml = r#"
name: loop_from_start
activities:
  - key: a_step_in
    worker: test
    activity_name: echo
    iteration_limit: 3
    depends_on:
      - activity_key: b_alight
        conditions: ["{{ b_alight.status == 'completed' }}"]
  - key: b_alight
    worker: test
    activity_name: echo
    depends_on: [a_step_in]
  - key: spinner
    worker: test
    activity_name: echo
    iteration_limit: 2
    depends_on: [spinner]
"#;
    let (runs, state) = run_to_end(&pool, yaml, gate_outputs(true)).await;

    assert_eq!(runs.get("a_step_in"), Some(&3), "{runs:?}");
    assert_eq!(runs.get("b_alight"), Some(&3), "{runs:?}");
    assert_eq!(runs.get("spinner"), Some(&2), "{runs:?}");
    assert_eq!(state.status, WorkflowStatus::Completed);
}

/// A loop whose entry condition reads an earlier loop's step waits for that
/// loop to end before it is skipped, like a step after a loop (#27): the
/// condition is false after the counter's first rounds and true after its
/// last, so the second loop starts then, not before. It used to start after
/// the counter's first round, condition or not.
#[sqlx::test(migrations = "../migrations")]
async fn test_loop_entry_on_an_earlier_loop_waits_for_it(pool: PgPool) {
    let run = drive_loop_workflow(
        &pool,
        r#"
name: loop_after_loop
activities:
  - key: counter
    worker: test
    activity_name: echo
    iteration_limit: 3
    depends_on: [counter]
  - key: spender
    worker: test
    activity_name: echo
    iteration_limit: 2
    depends_on:
      - activity_key: counter
        conditions: ["{{ counter.round >= 2 }}"]
      - spender
"#,
        &[],
    )
    .await;

    assert_eq!(run.runs.get("counter"), Some(&3), "{:?}", run.runs);
    assert_eq!(
        run.runs.get("spender"),
        Some(&2),
        "starts once the counter's last round is done: {:?} (history {:?})",
        run.runs,
        run.pending_history
    );
    assert!(
        !run.pending_history
            .iter()
            .any(|p| p.iter().any(|k| k == "counter") && p.iter().any(|k| k == "spender")),
        "spender never runs alongside counter: {:?}",
        run.pending_history
    );
    assert_eq!(
        run.status,
        kruxiaflow_core::events::models::WorkflowStatus::Completed,
        "{:?}",
        run.runs
    );
}

// ============================================================================
// A step after a loop runs after the loop's last round [loop-dependent-runs-early]
//
// A step that depends on a loop step without being part of the loop is how
// the loop hands off its result. With no condition on that dependency, it ran
// as soon as the loop step first completed, after round 1, on round 1's
// output, and never again: the loop's later rounds were lost to it.
// ============================================================================

/// The parameters the orchestrator queued `key` with, in `workflow_id`
async fn queued_parameters(pool: &PgPool, workflow_id: Uuid, key: &str) -> serde_json::Value {
    sqlx::query_scalar::<_, serde_json::Value>(
        "SELECT parameters FROM activity_queue WHERE workflow_id = $1 AND activity_key = $2",
    )
    .bind(workflow_id)
    .bind(key)
    .fetch_one(pool)
    .await
    .expect("queued activity")
}

/// Whether `key` was ever pending alongside one of `others`
fn ran_alongside(run: &LoopRun, key: &str, others: &[&str]) -> bool {
    run.pending_history
        .iter()
        .any(|p| p.iter().any(|k| k == key) && p.iter().any(|k| others.contains(&k.as_str())))
}

/// A two-step loop (lift ⇄ drop, three rounds) and `survey`, which depends on
/// `drop` with no condition: it runs once, after the third drop, on its
/// output. It used to run after the first drop, on round 0's output.
#[sqlx::test(migrations = "../migrations")]
async fn test_step_after_a_two_step_loop_runs_after_its_last_round(pool: PgPool) {
    let run = drive_loop_workflow(
        &pool,
        r#"
name: step_after_two_step_loop
activities:
  - key: crane
    worker: test
    activity_name: echo
  - key: lift
    worker: test
    activity_name: echo
    iteration_limit: 3
    depends_on:
      - crane
      - activity_key: drop
        conditions: ["{{ drop.status == 'completed' }}"]
  - key: drop
    worker: test
    activity_name: echo
    depends_on: [lift]
  - key: survey
    worker: test
    activity_name: echo
    parameters:
      depth: "{{ drop.round }}"
    depends_on: [drop]
"#,
        &[],
    )
    .await;

    for (key, expected) in [("crane", 1), ("lift", 3), ("drop", 3), ("survey", 1)] {
        assert_eq!(
            run.runs.get(key),
            Some(&expected),
            "{key}: {:?} (history {:?})",
            run.runs,
            run.pending_history
        );
    }
    assert!(
        !ran_alongside(&run, "survey", &["lift", "drop"]),
        "survey waits for the loop to end: {:?}",
        run.pending_history
    );
    assert_eq!(
        queued_parameters(&pool, run.workflow_id, "survey").await,
        json!({ "depth": 2 }),
        "survey takes the last round's output (round 2, 0-based)"
    );
    assert_eq!(run.status, WorkflowStatus::Completed, "{:?}", run.runs);
}

/// A self-loop (three rounds) and a step after it with no condition: the
/// step runs once, after the last round, on its output
#[sqlx::test(migrations = "../migrations")]
async fn test_step_after_a_self_loop_runs_after_its_last_round(pool: PgPool) {
    let run = drive_loop_workflow(
        &pool,
        r#"
name: step_after_self_loop
activities:
  - key: tamper
    worker: test
    activity_name: echo
    iteration_limit: 3
    depends_on: [tamper]
  - key: sign_off
    worker: test
    activity_name: echo
    parameters:
      tamps: "{{ tamper.round }}"
    depends_on: [tamper]
"#,
        &[],
    )
    .await;

    assert_eq!(run.runs.get("tamper"), Some(&3), "{:?}", run.runs);
    assert_eq!(run.runs.get("sign_off"), Some(&1), "{:?}", run.runs);
    assert!(
        !ran_alongside(&run, "sign_off", &["tamper"]),
        "sign_off waits for the loop to end: {:?}",
        run.pending_history
    );
    assert_eq!(
        queued_parameters(&pool, run.workflow_id, "sign_off").await,
        json!({ "tamps": 2 })
    );
    assert_eq!(run.status, WorkflowStatus::Completed, "{:?}", run.runs);
}

/// A step after one loop doesn't wait for another loop that runs alongside
/// it: `survey` runs once `lift ⇄ drop` has ended, while `poller` (a
/// separate self-loop with more rounds) is still going
#[sqlx::test(migrations = "../migrations")]
async fn test_step_after_a_loop_does_not_wait_for_another_loop(pool: PgPool) {
    let run = drive_loop_workflow(
        &pool,
        r#"
name: step_after_loop_beside_another
activities:
  - key: lift
    worker: test
    activity_name: echo
    iteration_limit: 2
    depends_on:
      - activity_key: drop
        conditions: ["{{ drop.status == 'completed' }}"]
  - key: drop
    worker: test
    activity_name: echo
    depends_on: [lift]
  - key: survey
    worker: test
    activity_name: echo
    depends_on: [drop]
  - key: poller
    worker: test
    activity_name: echo
    iteration_limit: 8
    depends_on: [poller]
"#,
        &[],
    )
    .await;

    for (key, expected) in [("lift", 2), ("drop", 2), ("survey", 1), ("poller", 8)] {
        assert_eq!(
            run.runs.get(key),
            Some(&expected),
            "{key}: {:?} (history {:?})",
            run.runs,
            run.pending_history
        );
    }
    assert!(
        !ran_alongside(&run, "survey", &["lift", "drop"]),
        "{:?}",
        run.pending_history
    );
    assert!(
        ran_alongside(&run, "survey", &["poller"]),
        "survey runs while the other loop still goes: {:?}",
        run.pending_history
    );
    assert_eq!(run.status, WorkflowStatus::Completed, "{:?}", run.runs);
}

/// The exit step of the #27 repair loop still runs once, after the check
/// that passes, and is never scheduled alongside the loop
#[sqlx::test(migrations = "../migrations")]
async fn test_conditional_exit_step_still_runs_after_the_round_that_exits(pool: PgPool) {
    let (runs, state) = run_to_end(&pool, REPAIR_LOOP, check_results(2)).await;

    assert_eq!(runs.get("b_check"), Some(&3), "{runs:?}");
    assert_eq!(runs.get("d_done"), Some(&1), "{runs:?}");
    assert_eq!(runs.get("e_report"), Some(&1), "{runs:?}");
    assert_eq!(
        status_of(&state, "d_done"),
        WorkflowActivityStatus::Completed
    );
    assert_eq!(state.status, WorkflowStatus::Completed);
}
