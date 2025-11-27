use serde_json::json;
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use streamflow_core::events::{
    EventSource, NewWorkflowEvent, PostgresEventSource, WorkflowEventType,
};
use streamflow_core::orchestrator::{
    OrchestratorConfig,
    workflow_state::{WorkflowActivityStatus, load_materialized_state},
};
use streamflow_core::queue::{PostgresQueue, QueueConfig};
use streamflow_core::workflow::{
    ActivityDefinition, ActivityRelationship, ActivitySettings, BudgetAction, BudgetSettings,
    WorkflowDefinition,
};
use uuid::Uuid;

async fn setup_test_db() -> PgPool {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://localhost/streamflow_test".to_string());

    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    // Run migrations
    sqlx::migrate!("../migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

async fn clean_test_data(pool: &PgPool) {
    sqlx::query!(
        "TRUNCATE workflow_events, workflow_event_consumers, workflows, workflow_definitions, activity_queue, activity_costs CASCADE"
    )
    .execute(pool)
    .await
    .expect("Failed to clean test data");
}

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
#[tokio::test]
#[serial]
async fn test_simple_loop_workflow() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    // Create a loop workflow definition (manually validated with metadata set)
    let definition = WorkflowDefinition {
        name: "simple_loop".to_string(),
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
    let event_source: Arc<dyn EventSource> = Arc::new(PostgresEventSource::new(pool.clone()));
    let queue_config = QueueConfig {
        poll_interval: Duration::from_millis(10),
        batch_size: 10,
        default_timeout: Duration::from_secs(30),
        default_max_retries: 3,
        cleanup_interval: Duration::from_secs(60),
        vacuum_interval: Duration::from_secs(3600),
    };
    let activity_queue: Arc<dyn streamflow_core::queue::ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), queue_config.clone()));

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
        backoff_multiplier: 2.0,
        workflow_timeout: Duration::from_secs(300),
        timeout_check_interval: Duration::from_secs(60),
    };

    let events = event_source.poll("orchestrator").await.unwrap();
    assert!(!events.is_empty(), "Should have WorkflowCreated event");

    for event in events {
        streamflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .expect("Failed to process event");
        event_source
            .update_position("orchestrator", event.id)
            .await
            .unwrap();
    }

    // Verify process activity was scheduled (iteration 0)
    let mut tx = pool.begin().await.unwrap();
    let state = load_materialized_state(&mut *tx, workflow_id)
        .await
        .unwrap();
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
        streamflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.id)
            .await
            .unwrap();
    }

    // Verify check activity was scheduled
    let mut tx = pool.begin().await.unwrap();
    let state = load_materialized_state(&mut *tx, workflow_id)
        .await
        .unwrap();
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
        streamflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.id)
            .await
            .unwrap();
    }

    // Verify process activity was scheduled again (iteration 1 - loop back)
    let mut tx = pool.begin().await.unwrap();
    let state = load_materialized_state(&mut *tx, workflow_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let process_state = state.activities.get("process").unwrap();
    assert_eq!(process_state.status, WorkflowActivityStatus::Pending);
    assert_eq!(process_state.iteration, 1); // Should still be 1 (increments on completion)

    println!("✓ Simple loop workflow test passed");
}

/// Test iteration limit enforcement
#[tokio::test]
#[serial]
async fn test_loop_max_iterations_enforced() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    // Create a loop workflow with iteration_limit = 2
    let definition = WorkflowDefinition {
        name: "limited_loop".to_string(),
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

    let event_source: Arc<dyn EventSource> = Arc::new(PostgresEventSource::new(pool.clone()));
    let queue_config = QueueConfig {
        poll_interval: Duration::from_millis(10),
        batch_size: 10,
        default_timeout: Duration::from_secs(30),
        default_max_retries: 3,
        cleanup_interval: Duration::from_secs(60),
        vacuum_interval: Duration::from_secs(3600),
    };
    let activity_queue: Arc<dyn streamflow_core::queue::ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), queue_config));

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
        backoff_multiplier: 2.0,
        workflow_timeout: Duration::from_secs(300),
        timeout_check_interval: Duration::from_secs(60),
    };

    // Process WorkflowCreated
    let events = event_source.poll("orchestrator").await.unwrap();
    for event in events {
        streamflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.id)
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
        streamflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.id)
            .await
            .unwrap();
    }

    // Verify iteration 1 was scheduled
    let mut tx = pool.begin().await.unwrap();
    let state = load_materialized_state(&mut *tx, workflow_id)
        .await
        .unwrap();
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
        streamflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.id)
            .await
            .unwrap();
    }

    // Verify loop stopped (iteration limit reached)
    let mut tx = pool.begin().await.unwrap();
    let state = load_materialized_state(&mut *tx, workflow_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let task_state = state.activities.get("loop_task").unwrap();
    assert_eq!(task_state.status, WorkflowActivityStatus::Completed);
    assert_eq!(task_state.iteration, 2); // Stopped at iteration 2 (>= limit)

    println!("✓ Iteration limit enforcement test passed");
}

/// Test iteration counter for non-iteration_scoped loops
#[tokio::test]
#[serial]
async fn test_iteration_counter_without_scoping() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    // Create a loop workflow without iteration_scoped
    let definition = WorkflowDefinition {
        name: "non_scoped_loop".to_string(),
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

    let event_source: Arc<dyn EventSource> = Arc::new(PostgresEventSource::new(pool.clone()));
    let queue_config = QueueConfig {
        poll_interval: Duration::from_millis(10),
        batch_size: 10,
        default_timeout: Duration::from_secs(30),
        default_max_retries: 3,
        cleanup_interval: Duration::from_secs(60),
        vacuum_interval: Duration::from_secs(3600),
    };
    let activity_queue: Arc<dyn streamflow_core::queue::ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), queue_config));

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
        backoff_multiplier: 2.0,
        workflow_timeout: Duration::from_secs(300),
        timeout_check_interval: Duration::from_secs(60),
    };

    // Process WorkflowCreated
    let events = event_source.poll("orchestrator").await.unwrap();
    for event in events {
        streamflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.id)
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
        streamflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.id)
            .await
            .unwrap();
    }

    // Verify iteration counter incremented but no iteration_outputs
    let mut tx = pool.begin().await.unwrap();
    let state = load_materialized_state(&mut *tx, workflow_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let counter_state = state.activities.get("counter").unwrap();
    assert_eq!(counter_state.iteration, 1);
    assert!(counter_state.iteration_outputs.is_none()); // No iteration scoping

    println!("✓ Iteration counter without scoping test passed");
}

/// Test budget accumulation across iterations
#[tokio::test]
#[serial]
async fn test_iteration_budget_accumulation() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    // Create a loop workflow with budget limit that spans multiple iterations
    let definition = WorkflowDefinition {
        name: "budget_loop".to_string(),
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

    let event_source: Arc<dyn EventSource> = Arc::new(PostgresEventSource::new(pool.clone()));
    let queue_config = QueueConfig {
        poll_interval: Duration::from_millis(10),
        batch_size: 10,
        default_timeout: Duration::from_secs(30),
        default_max_retries: 3,
        cleanup_interval: Duration::from_secs(60),
        vacuum_interval: Duration::from_secs(3600),
    };
    let activity_queue: Arc<dyn streamflow_core::queue::ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), queue_config));

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
        backoff_multiplier: 2.0,
        workflow_timeout: Duration::from_secs(300),
        timeout_check_interval: Duration::from_secs(60),
    };

    // Process WorkflowCreated
    let events = event_source.poll("orchestrator").await.unwrap();
    for event in events {
        streamflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.id)
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
        streamflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.id)
            .await
            .unwrap();
    }

    // Verify accumulated cost after iteration 0
    let mut tx = pool.begin().await.unwrap();
    let state = load_materialized_state(&mut *tx, workflow_id)
        .await
        .unwrap();
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
        streamflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.id)
            .await
            .unwrap();
    }

    // Verify accumulated cost after iteration 1
    let mut tx = pool.begin().await.unwrap();
    let state = load_materialized_state(&mut *tx, workflow_id)
        .await
        .unwrap();
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
#[tokio::test]
#[serial]
async fn test_loop_pattern_1_fixed_iterations() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    // Create a loop with only iteration_limit (no condition on back-edge)
    let definition = WorkflowDefinition {
        name: "fixed_iterations".to_string(),
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

    let event_source: Arc<dyn EventSource> = Arc::new(PostgresEventSource::new(pool.clone()));
    let queue_config = QueueConfig {
        poll_interval: Duration::from_millis(10),
        batch_size: 10,
        default_timeout: Duration::from_secs(30),
        default_max_retries: 3,
        cleanup_interval: Duration::from_secs(60),
        vacuum_interval: Duration::from_secs(3600),
    };
    let activity_queue: Arc<dyn streamflow_core::queue::ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), queue_config));

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
        backoff_multiplier: 2.0,
        workflow_timeout: Duration::from_secs(300),
        timeout_check_interval: Duration::from_secs(60),
    };

    // Process WorkflowCreated
    let events = event_source.poll("orchestrator").await.unwrap();
    for event in events {
        streamflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.id)
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
                iteration: Some(i as i32),
            })
            .await
            .unwrap();

        let events = event_source.poll("orchestrator").await.unwrap();
        for event in events {
            streamflow_core::orchestrator::orchestrator::process_workflow_event(
                &event,
                &event_source,
                &activity_queue,
                &config,
            )
            .await
            .unwrap();
            event_source
                .update_position("orchestrator", event.id)
                .await
                .unwrap();
        }
    }

    // Verify exactly 3 iterations completed
    let mut tx = pool.begin().await.unwrap();
    let state = load_materialized_state(&mut *tx, workflow_id)
        .await
        .unwrap();
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
#[tokio::test]
#[serial]
async fn test_loop_pattern_2_condition_only() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    // Create a loop with only condition (no explicit iteration_limit)
    // Should use DEFAULT_MAX_ITERATIONS = 100
    let definition = WorkflowDefinition {
        name: "condition_only".to_string(),
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

    let event_source: Arc<dyn EventSource> = Arc::new(PostgresEventSource::new(pool.clone()));
    let queue_config = QueueConfig {
        poll_interval: Duration::from_millis(10),
        batch_size: 10,
        default_timeout: Duration::from_secs(30),
        default_max_retries: 3,
        cleanup_interval: Duration::from_secs(60),
        vacuum_interval: Duration::from_secs(3600),
    };
    let activity_queue: Arc<dyn streamflow_core::queue::ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), queue_config));

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
        backoff_multiplier: 2.0,
        workflow_timeout: Duration::from_secs(300),
        timeout_check_interval: Duration::from_secs(60),
    };

    // Process WorkflowCreated
    let events = event_source.poll("orchestrator").await.unwrap();
    for event in events {
        streamflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.id)
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
                iteration: Some(i as i32),
            })
            .await
            .unwrap();

        let events = event_source.poll("orchestrator").await.unwrap();
        for event in events {
            streamflow_core::orchestrator::orchestrator::process_workflow_event(
                &event,
                &event_source,
                &activity_queue,
                &config,
            )
            .await
            .unwrap();
            event_source
                .update_position("orchestrator", event.id)
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
                iteration: Some(i as i32),
            })
            .await
            .unwrap();

        let events = event_source.poll("orchestrator").await.unwrap();
        for event in events {
            streamflow_core::orchestrator::orchestrator::process_workflow_event(
                &event,
                &event_source,
                &activity_queue,
                &config,
            )
            .await
            .unwrap();
            event_source
                .update_position("orchestrator", event.id)
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
        streamflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.id)
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
        streamflow_core::orchestrator::orchestrator::process_workflow_event(
            &event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .unwrap();
        event_source
            .update_position("orchestrator", event.id)
            .await
            .unwrap();
    }

    // Verify loop exited due to condition (not limit)
    let mut tx = pool.begin().await.unwrap();
    let state = load_materialized_state(&mut *tx, workflow_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let poll_state = state.activities.get("poll").unwrap();
    assert_eq!(poll_state.status, WorkflowActivityStatus::Completed);
    assert_eq!(poll_state.iteration, 3); // 3 iterations (0, 1, 2)
    assert!(poll_state.iteration < 100); // Well under default limit

    println!("✓ Pattern 2 (condition only) test passed");
}
