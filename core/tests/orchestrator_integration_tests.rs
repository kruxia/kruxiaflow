use serde_json::json;
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use streamflow_core::events::{
    ActivityDefinition, DependencyEdge, EventSource, NewWorkflowEvent, PostgresEventSource,
    WorkflowDefinition, WorkflowEventType,
};
use streamflow_core::orchestrator::OrchestratorConfig;
use streamflow_core::queue::{ActivityQueue, PostgresQueue, QueueConfig};
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
        r#"INSERT INTO workflow_definitions (name, version, activities)
           VALUES ($1, $2, $3)
           RETURNING id"#,
        definition.name,
        definition.version,
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
    workflow_type: &str,
    workflow_definition_id: Uuid,
) {
    sqlx::query!(
        r#"INSERT INTO workflows (id, workflow_type, workflow_definition_id, status, state_data)
           VALUES ($1, $2, $3, 'running', '{}'::jsonb)"#,
        workflow_id,
        workflow_type,
        workflow_definition_id
    )
    .execute(pool)
    .await
    .expect("Failed to insert workflow");
}

#[tokio::test]
#[serial]
async fn test_sequential_workflow_integration() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    // Create workflow definition
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "sequential_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "activity1".to_string(),
                namespace: "test".to_string(),
                name: "step1".to_string(),
                parameters: json!({}),
                settings: None,
                preceding: None,
                following: Some(vec![DependencyEdge {
                    activity_key: "activity2".to_string(),
                    conditions: None,
                }]),
            },
            ActivityDefinition {
                key: "activity2".to_string(),
                namespace: "test".to_string(),
                name: "step2".to_string(),
                parameters: json!({}),
                settings: None,
                preceding: None,
                following: Some(vec![DependencyEdge {
                    activity_key: "activity3".to_string(),
                    conditions: None,
                }]),
            },
            ActivityDefinition {
                key: "activity3".to_string(),
                namespace: "test".to_string(),
                name: "step3".to_string(),
                parameters: json!({}),
                settings: None,
                preceding: None,
                following: None,
            },
        ],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;

    let workflow_id = Uuid::now_v7();
    insert_workflow(&pool, workflow_id, &definition.name, definition_id).await;

    let event_source: Arc<dyn EventSource> = Arc::new(PostgresEventSource::new(pool.clone()));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));

    // Publish WorkflowCreated event
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::WorkflowCreated,
            activity_key: None,
            payload: json!({}),
        })
        .await
        .expect("Failed to publish WorkflowCreated");

    // Process event manually (simulating orchestrator)
    let config = OrchestratorConfig::new(pool.clone());
    let events = event_source
        .poll("test_orchestrator")
        .await
        .expect("Failed to poll");
    assert_eq!(events.len(), 1);

    // Process the WorkflowCreated event
    streamflow_core::orchestrator::orchestrator::process_workflow_event(
        &events[0],
        &event_source,
        &activity_queue,
        &config,
    )
    .await
    .expect("Failed to process event");

    // Verify activity1 was scheduled
    let queued = sqlx::query!(
        r#"SELECT activity_key FROM activity_queue WHERE workflow_id = $1"#,
        workflow_id
    )
    .fetch_all(&pool)
    .await
    .expect("Failed to query queue");

    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].activity_key, "activity1");

    // Simulate activity1 completion
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::ActivityCompleted,
            activity_key: Some("activity1".to_string()),
            payload: json!({"outputs": {"result": "success"}}),
        })
        .await
        .expect("Failed to publish ActivityCompleted");

    // Update position and poll again
    event_source
        .update_position("test_orchestrator", events[0].id)
        .await
        .expect("Failed to update position");

    let events = event_source
        .poll("test_orchestrator")
        .await
        .expect("Failed to poll");
    assert!(!events.is_empty());

    // Process ActivityCompleted event
    for event in &events {
        streamflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .expect("Failed to process event");
    }

    // Verify activity2 was scheduled
    let queued = sqlx::query!(
        r#"SELECT activity_key FROM activity_queue WHERE workflow_id = $1 ORDER BY created_at"#,
        workflow_id
    )
    .fetch_all(&pool)
    .await
    .expect("Failed to query queue");

    assert!(queued.iter().any(|a| a.activity_key == "activity2"));
}

#[tokio::test]
#[serial]
async fn test_parallel_workflow_integration() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    // Create workflow definition with fan-out/fan-in
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "parallel_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "root".to_string(),
                namespace: "test".to_string(),
                name: "root".to_string(),
                parameters: json!({}),
                settings: None,
                preceding: None,
                following: Some(vec![
                    DependencyEdge {
                        activity_key: "parallel1".to_string(),
                        conditions: None,
                    },
                    DependencyEdge {
                        activity_key: "parallel2".to_string(),
                        conditions: None,
                    },
                    DependencyEdge {
                        activity_key: "parallel3".to_string(),
                        conditions: None,
                    },
                ]),
            },
            ActivityDefinition {
                key: "parallel1".to_string(),
                namespace: "test".to_string(),
                name: "parallel".to_string(),
                parameters: json!({}),
                settings: None,
                preceding: None,
                following: None,
            },
            ActivityDefinition {
                key: "parallel2".to_string(),
                namespace: "test".to_string(),
                name: "parallel".to_string(),
                parameters: json!({}),
                settings: None,
                preceding: None,
                following: None,
            },
            ActivityDefinition {
                key: "parallel3".to_string(),
                namespace: "test".to_string(),
                name: "parallel".to_string(),
                parameters: json!({}),
                settings: None,
                preceding: None,
                following: None,
            },
            ActivityDefinition {
                key: "join".to_string(),
                namespace: "test".to_string(),
                name: "join".to_string(),
                parameters: json!({}),
                settings: None,
                preceding: Some(vec![
                    DependencyEdge {
                        activity_key: "parallel1".to_string(),
                        conditions: None,
                    },
                    DependencyEdge {
                        activity_key: "parallel2".to_string(),
                        conditions: None,
                    },
                    DependencyEdge {
                        activity_key: "parallel3".to_string(),
                        conditions: None,
                    },
                ]),
                following: None,
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

    // Publish WorkflowCreated and process
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::WorkflowCreated,
            activity_key: None,
            payload: json!({}),
        })
        .await
        .expect("Failed to publish WorkflowCreated");

    let events = event_source
        .poll("test_orchestrator")
        .await
        .expect("Failed to poll");

    for event in &events {
        streamflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .expect("Failed to process event");
    }

    // Verify root activity was scheduled
    let queued = sqlx::query!(
        r#"SELECT activity_key FROM activity_queue WHERE workflow_id = $1"#,
        workflow_id
    )
    .fetch_all(&pool)
    .await
    .expect("Failed to query queue");

    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].activity_key, "root");

    // Simulate root completion
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::ActivityCompleted,
            activity_key: Some("root".to_string()),
            payload: json!({"outputs": {"result": "success"}}),
        })
        .await
        .expect("Failed to publish ActivityCompleted");

    event_source
        .update_position("test_orchestrator", events[0].id)
        .await
        .expect("Failed to update position");

    let events = event_source
        .poll("test_orchestrator")
        .await
        .expect("Failed to poll");

    for event in &events {
        streamflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .expect("Failed to process event");
    }

    // Verify all 3 parallel activities were scheduled
    let queued = sqlx::query!(
        r#"SELECT activity_key FROM activity_queue WHERE workflow_id = $1"#,
        workflow_id
    )
    .fetch_all(&pool)
    .await
    .expect("Failed to query queue");

    assert_eq!(queued.len(), 4); // root + 3 parallel
    assert!(queued.iter().any(|a| a.activity_key == "parallel1"));
    assert!(queued.iter().any(|a| a.activity_key == "parallel2"));
    assert!(queued.iter().any(|a| a.activity_key == "parallel3"));

    // Complete parallel activities one by one
    for key in &["parallel1", "parallel2", "parallel3"] {
        event_source
            .publish(NewWorkflowEvent {
                workflow_id,
                event_type: WorkflowEventType::ActivityCompleted,
                activity_key: Some(key.to_string()),
                payload: json!({"outputs": {"result": "success"}}),
            })
            .await
            .expect("Failed to publish ActivityCompleted");
    }

    // Poll and process all completion events
    let last_event_id = events.last().unwrap().id;
    event_source
        .update_position("test_orchestrator", last_event_id)
        .await
        .expect("Failed to update position");

    let events = event_source
        .poll("test_orchestrator")
        .await
        .expect("Failed to poll");

    for event in &events {
        streamflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .expect("Failed to process event");
    }

    // Verify join activity was scheduled only after all parallel activities completed
    let queued = sqlx::query!(
        r#"SELECT activity_key FROM activity_queue WHERE workflow_id = $1"#,
        workflow_id
    )
    .fetch_all(&pool)
    .await
    .expect("Failed to query queue");

    assert!(queued.iter().any(|a| a.activity_key == "join"));
}

#[tokio::test]
#[serial]
async fn test_conditional_workflow_integration() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    // Create workflow with conditional branching
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "conditional_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "validate".to_string(),
                namespace: "test".to_string(),
                name: "validate".to_string(),
                parameters: json!({}),
                settings: None,
                preceding: None,
                following: Some(vec![
                    DependencyEdge {
                        activity_key: "approve".to_string(),
                        conditions: Some(vec!["{{validate.valid}} == true".to_string()]),
                    },
                    DependencyEdge {
                        activity_key: "reject".to_string(),
                        conditions: Some(vec!["{{validate.valid}} == false".to_string()]),
                    },
                ]),
            },
            ActivityDefinition {
                key: "approve".to_string(),
                namespace: "test".to_string(),
                name: "approve".to_string(),
                parameters: json!({}),
                settings: None,
                preceding: None,
                following: None,
            },
            ActivityDefinition {
                key: "reject".to_string(),
                namespace: "test".to_string(),
                name: "reject".to_string(),
                parameters: json!({}),
                settings: None,
                preceding: None,
                following: None,
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

    // Publish WorkflowCreated and process
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::WorkflowCreated,
            activity_key: None,
            payload: json!({}),
        })
        .await
        .expect("Failed to publish WorkflowCreated");

    let events = event_source
        .poll("test_orchestrator")
        .await
        .expect("Failed to poll");

    for event in &events {
        streamflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .expect("Failed to process event");
    }

    // Complete validate with valid=true
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::ActivityCompleted,
            activity_key: Some("validate".to_string()),
            payload: json!({"outputs": {"valid": true}}),
        })
        .await
        .expect("Failed to publish ActivityCompleted");

    event_source
        .update_position("test_orchestrator", events[0].id)
        .await
        .expect("Failed to update position");

    let events = event_source
        .poll("test_orchestrator")
        .await
        .expect("Failed to poll");

    for event in &events {
        streamflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .expect("Failed to process event");
    }

    // Verify only approve was scheduled (not reject)
    let queued = sqlx::query!(
        r#"SELECT activity_key FROM activity_queue WHERE workflow_id = $1"#,
        workflow_id
    )
    .fetch_all(&pool)
    .await
    .expect("Failed to query queue");

    assert!(queued.iter().any(|a| a.activity_key == "approve"));
    assert!(!queued.iter().any(|a| a.activity_key == "reject"));
}
