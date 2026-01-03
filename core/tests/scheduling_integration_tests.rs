use chrono::{Duration as ChronoDuration, Utc};
use serde_json::json;
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use kruxiaflow_core::events::{
    ActivityDefinition, DependencyEdge, EventSource, NewWorkflowEvent, PostgresEventSource,
    WorkflowDefinition, WorkflowEventType,
};
use kruxiaflow_core::orchestrator::OrchestratorConfig;
use kruxiaflow_core::queue::{ActivityQueue, PostgresQueue, QueueConfig};
use kruxiaflow_core::workflow::ActivitySettings;
use uuid::Uuid;

async fn setup_test_db() -> PgPool {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://localhost/kruxiaflow_test".to_string());

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
        "TRUNCATE workflow_events, workflow_event_consumers, workflows, workflow_definitions, activity_queue CASCADE"
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

async fn insert_workflow(
    pool: &PgPool,
    workflow_id: Uuid,
    definition_name: &str,
    workflow_definition_id: Uuid,
) {
    sqlx::query!(
        r#"INSERT INTO workflows (id, definition_name, workflow_definition_id, input, status, activities, state_data)
           VALUES ($1, $2, $3, '{}'::jsonb, 'running', '{}'::jsonb, '{}'::jsonb)"#,
        workflow_id,
        definition_name,
        workflow_definition_id
    )
    .execute(pool)
    .await
    .expect("Failed to insert workflow");
}

#[tokio::test]
#[serial]
async fn test_delayed_activity_execution() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    // Create workflow with a 2-second delay
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "delayed_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![ActivityDefinition {
            key: "delayed_task".to_string(),
            worker: "test".to_string(),
            activity_name: "delayed_work".to_string(),
            parameters: json!({}),
            settings: Some(ActivitySettings {
                timeout_seconds: None,
                retry: None,
                budget: None,
                cache: false,
                cache_ttl: None,
                iteration_limit: None,
                delay: Some("2s".to_string()),
                scheduled_for: None,
            }),
            depends_on: None,
            dependency_of: None,
            output_definitions: None,
        }],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();
    insert_workflow(&pool, workflow_id, &definition.name, definition_id).await;

    let event_source: Arc<dyn EventSource> = Arc::new(PostgresEventSource::new(pool.clone()));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let config = OrchestratorConfig::new(pool.clone());

    // Publish WorkflowCreated event
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::WorkflowCreated,
            activity_key: None,
            payload: json!({}),
            iteration: None,
        })
        .await
        .expect("Failed to publish WorkflowCreated");

    let events = event_source
        .poll("test_orchestrator")
        .await
        .expect("Failed to poll");

    for event in &events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .expect("Failed to process event");
    }

    // Verify activity was scheduled with future scheduled_for
    let queued = sqlx::query!(
        r#"SELECT activity_key, scheduled_for FROM activity_queue WHERE workflow_id = $1"#,
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to query queue");

    assert_eq!(queued.activity_key, "delayed_task");

    let scheduled_for = queued.scheduled_for;
    let now = Utc::now();

    // Verify scheduled_for is approximately 2 seconds in the future
    let diff = (scheduled_for - now).num_milliseconds();
    assert!(
        diff > 1500 && diff < 2500,
        "Expected delay ~2000ms, got {}ms",
        diff
    );
}

#[tokio::test]
#[serial]
async fn test_scheduled_activity_execution() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    // Schedule activity for 3 seconds in the future
    let future_time = Utc::now() + ChronoDuration::seconds(3);
    let scheduled_time_str = future_time.to_rfc3339();

    // Create workflow definition with input placeholder
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "scheduled_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![ActivityDefinition {
            key: "scheduled_task".to_string(),
            worker: "test".to_string(),
            activity_name: "scheduled_work".to_string(),
            parameters: json!({}),
            settings: Some(ActivitySettings {
                timeout_seconds: None,
                retry: None,
                budget: None,
                cache: false,
                cache_ttl: None,
                iteration_limit: None,
                delay: None,
                scheduled_for: Some("{{INPUT.scheduled_time}}".to_string()),
            }),
            depends_on: None,
            dependency_of: None,
            output_definitions: None,
        }],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();

    // Insert workflow with input containing the scheduled time
    sqlx::query!(
        r#"INSERT INTO workflows (id, definition_name, workflow_definition_id, input, status, activities, state_data)
           VALUES ($1, $2, $3, $4, 'running', '{}'::jsonb, '{}'::jsonb)"#,
        workflow_id,
        definition.name,
        definition_id,
        json!({"scheduled_time": scheduled_time_str})
    )
    .execute(&pool)
    .await
    .expect("Failed to insert workflow");

    let event_source: Arc<dyn EventSource> = Arc::new(PostgresEventSource::new(pool.clone()));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let config = OrchestratorConfig::new(pool.clone());

    // Publish WorkflowCreated event
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::WorkflowCreated,
            activity_key: None,
            payload: json!({
                "input": {"scheduled_time": scheduled_time_str}
            }),
            iteration: None,
        })
        .await
        .expect("Failed to publish WorkflowCreated");

    let events = event_source
        .poll("test_orchestrator")
        .await
        .expect("Failed to poll");

    for event in &events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .expect("Failed to process event");
    }

    // Verify activity was scheduled with correct scheduled_for
    let queued = sqlx::query!(
        r#"SELECT activity_key, scheduled_for FROM activity_queue WHERE workflow_id = $1"#,
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to query queue");

    assert_eq!(queued.activity_key, "scheduled_task");

    let scheduled_for = queued.scheduled_for;

    // Verify scheduled_for matches the input (within 1 second tolerance for processing time)
    let diff = (scheduled_for - future_time).num_milliseconds().abs();
    assert!(
        diff < 1000,
        "Expected scheduled time to match input, diff: {}ms",
        diff
    );
}

#[tokio::test]
#[serial]
async fn test_immediate_activity_unaffected() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    // Create workflow without scheduling fields
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "immediate_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![ActivityDefinition {
            key: "immediate_task".to_string(),
            worker: "test".to_string(),
            activity_name: "immediate_work".to_string(),
            parameters: json!({}),
            settings: None, // No settings = immediate execution
            depends_on: None,
            dependency_of: None,
            output_definitions: None,
        }],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();
    insert_workflow(&pool, workflow_id, &definition.name, definition_id).await;

    let event_source: Arc<dyn EventSource> = Arc::new(PostgresEventSource::new(pool.clone()));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let config = OrchestratorConfig::new(pool.clone());

    // Publish WorkflowCreated event
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::WorkflowCreated,
            activity_key: None,
            payload: json!({}),
            iteration: None,
        })
        .await
        .expect("Failed to publish WorkflowCreated");

    let events = event_source
        .poll("test_orchestrator")
        .await
        .expect("Failed to poll");

    for event in &events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .expect("Failed to process event");
    }

    // Verify activity was scheduled with NO scheduled_for (immediate)
    let queued = sqlx::query!(
        r#"SELECT activity_key, scheduled_for FROM activity_queue WHERE workflow_id = $1"#,
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to query queue");

    assert_eq!(queued.activity_key, "immediate_task");

    // Immediate activities should have scheduled_for set to NOW (not in the future)
    let scheduled_for = queued.scheduled_for;
    let now = Utc::now();
    let diff = (scheduled_for - now).num_milliseconds().abs();
    assert!(
        diff < 1000,
        "Immediate activities should have scheduled_for approximately NOW, diff: {}ms",
        diff
    );
}

#[tokio::test]
#[serial]
async fn test_worker_respects_scheduled_for() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    // Create workflow with 5-second delay
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "future_scheduled".to_string(),
        version: "1.0".to_string(),
        activities: vec![ActivityDefinition {
            key: "future_task".to_string(),
            worker: "test".to_string(),
            activity_name: "future_work".to_string(),
            parameters: json!({}),
            settings: Some(ActivitySettings {
                timeout_seconds: None,
                retry: None,
                budget: None,
                cache: false,
                cache_ttl: None,
                iteration_limit: None,
                delay: Some("5s".to_string()),
                scheduled_for: None,
            }),
            depends_on: None,
            dependency_of: None,
            output_definitions: None,
        }],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();
    insert_workflow(&pool, workflow_id, &definition.name, definition_id).await;

    let event_source: Arc<dyn EventSource> = Arc::new(PostgresEventSource::new(pool.clone()));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let config = OrchestratorConfig::new(pool.clone());

    // Publish WorkflowCreated and process
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::WorkflowCreated,
            activity_key: None,
            payload: json!({}),
            iteration: None,
        })
        .await
        .expect("Failed to publish WorkflowCreated");

    let events = event_source
        .poll("test_orchestrator")
        .await
        .expect("Failed to poll");

    for event in &events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .expect("Failed to process event");
    }

    // Try to claim the activity immediately - should return None
    let claimed = activity_queue
        .claim_next("test_worker_id", "test", "future_work")
        .await
        .expect("Failed to claim");

    assert!(
        claimed.is_none(),
        "Worker should not be able to claim future-scheduled activity"
    );

    // Verify activity is in queue but not claimable
    let queued = sqlx::query!(
        r#"SELECT activity_key, scheduled_for, status::text FROM activity_queue WHERE workflow_id = $1"#,
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to query queue");

    assert_eq!(queued.activity_key, "future_task");

    // Verify scheduled_for is in the future
    let scheduled_for = queued.scheduled_for;
    let now = Utc::now();
    let diff = (scheduled_for - now).num_milliseconds();
    assert!(
        diff > 4000,
        "Activity should be scheduled in the future, diff: {}ms",
        diff
    );

    assert_eq!(queued.status.as_ref().unwrap(), "pending");
}

#[tokio::test]
#[serial]
async fn test_multiple_delayed_activities() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    // Create workflow with chain of delays (rate limiting pattern)
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "rate_limited_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "call_1".to_string(),
                worker: "test".to_string(),
                activity_name: "api_call".to_string(),
                parameters: json!({}),
                settings: None, // Immediate
                depends_on: None,
                dependency_of: None,
                output_definitions: None,
            },
            ActivityDefinition {
                key: "call_2".to_string(),
                worker: "test".to_string(),
                activity_name: "api_call".to_string(),
                parameters: json!({}),
                settings: Some(ActivitySettings {
                    timeout_seconds: None,
                    retry: None,
                    budget: None,
                    cache: false,
                    cache_ttl: None,
                    iteration_limit: None,
                    delay: Some("2s".to_string()),
                    scheduled_for: None,
                }),
                depends_on: Some(vec![DependencyEdge {
                    activity_key: "call_1".to_string(),
                    conditions: None,
                }]),
                dependency_of: None,
                output_definitions: None,
            },
            ActivityDefinition {
                key: "call_3".to_string(),
                worker: "test".to_string(),
                activity_name: "api_call".to_string(),
                parameters: json!({}),
                settings: Some(ActivitySettings {
                    timeout_seconds: None,
                    retry: None,
                    budget: None,
                    cache: false,
                    cache_ttl: None,
                    iteration_limit: None,
                    delay: Some("2s".to_string()),
                    scheduled_for: None,
                }),
                depends_on: Some(vec![DependencyEdge {
                    activity_key: "call_2".to_string(),
                    conditions: None,
                }]),
                dependency_of: None,
                output_definitions: None,
            },
        ],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();
    insert_workflow(&pool, workflow_id, &definition.name, definition_id).await;

    let event_source: Arc<dyn EventSource> = Arc::new(PostgresEventSource::new(pool.clone()));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let config = OrchestratorConfig::new(pool.clone());

    // Process WorkflowCreated
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::WorkflowCreated,
            activity_key: None,
            payload: json!({}),
            iteration: None,
        })
        .await
        .expect("Failed to publish WorkflowCreated");

    let events = event_source
        .poll("test_orchestrator")
        .await
        .expect("Failed to poll");

    for event in &events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .expect("Failed to process event");
    }

    // Verify only call_1 is scheduled (no delay)
    let queued = sqlx::query!(
        r#"SELECT activity_key, scheduled_for FROM activity_queue WHERE workflow_id = $1"#,
        workflow_id
    )
    .fetch_all(&pool)
    .await
    .expect("Failed to query queue");

    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].activity_key, "call_1");

    // Verify call_1 has scheduled_for approximately NOW (immediate execution)
    let scheduled_for = queued[0].scheduled_for;
    let now = Utc::now();
    let diff = (scheduled_for - now).num_milliseconds().abs();
    assert!(
        diff < 1000,
        "call_1 should be scheduled immediately, diff: {}ms",
        diff
    );

    // Complete call_1
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::ActivityCompleted,
            activity_key: Some("call_1".to_string()),
            payload: json!({"outputs": {}}),
            iteration: None,
        })
        .await
        .expect("Failed to publish ActivityCompleted");

    event_source
        .update_position("test_orchestrator", events.last().unwrap().id)
        .await
        .expect("Failed to update position");

    let events = event_source
        .poll("test_orchestrator")
        .await
        .expect("Failed to poll");

    for event in &events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .expect("Failed to process event");
    }

    // Verify call_2 is scheduled with delay
    let queued = sqlx::query!(
        r#"SELECT activity_key, scheduled_for FROM activity_queue WHERE workflow_id = $1 ORDER BY created_at"#,
        workflow_id
    )
    .fetch_all(&pool)
    .await
    .expect("Failed to query queue");

    assert!(queued.iter().any(|a| a.activity_key == "call_2"));
    let call_2 = queued.iter().find(|a| a.activity_key == "call_2").unwrap();

    // Verify delay is approximately 2 seconds
    let scheduled_for = call_2.scheduled_for;
    let now = Utc::now();
    let diff = (scheduled_for - now).num_milliseconds();
    assert!(
        diff > 1500 && diff < 2500,
        "Expected delay ~2000ms for call_2, got {}ms",
        diff
    );
}

#[tokio::test]
#[serial]
async fn test_delay_with_all_duration_units() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    // Test milliseconds
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "ms_delay".to_string(),
        version: "1.0".to_string(),
        activities: vec![ActivityDefinition {
            key: "ms_task".to_string(),
            worker: "test".to_string(),
            activity_name: "work".to_string(),
            parameters: json!({}),
            settings: Some(ActivitySettings {
                timeout_seconds: None,
                retry: None,
                budget: None,
                cache: false,
                cache_ttl: None,
                iteration_limit: None,
                delay: Some("500ms".to_string()),
                scheduled_for: None,
            }),
            depends_on: None,
            dependency_of: None,
            output_definitions: None,
        }],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();
    insert_workflow(&pool, workflow_id, &definition.name, definition_id).await;

    let event_source: Arc<dyn EventSource> = Arc::new(PostgresEventSource::new(pool.clone()));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let config = OrchestratorConfig::new(pool.clone());

    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::WorkflowCreated,
            activity_key: None,
            payload: json!({}),
            iteration: None,
        })
        .await
        .unwrap();

    let events = event_source.poll("test_orch").await.unwrap();
    for event in &events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .unwrap();
    }

    // Verify millisecond precision
    let queued = sqlx::query!(
        r#"SELECT scheduled_for FROM activity_queue WHERE workflow_id = $1"#,
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let scheduled_for = queued.scheduled_for;
    let now = Utc::now();
    let diff = (scheduled_for - now).num_milliseconds();

    // Should be approximately 500ms in the future
    assert!(
        diff >= 400 && diff <= 600,
        "Expected ~500ms delay, got {}ms",
        diff
    );
}
