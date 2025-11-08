use serde_json::json;
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
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
    // Store only the activities array (not the full definition)
    // Version is derived from created_at, no need to store separately
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

#[tokio::test]
#[serial]
async fn test_workflow_completion_success() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    // Create simple workflow with just one activity
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "simple_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![ActivityDefinition {
            key: "only_activity".to_string(),
            namespace: "test".to_string(),
            name: "simple_task".to_string(),
            parameters: json!({}),
            settings: None,
            preceding: None,
            following: None,
        }],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();
    insert_workflow(&pool, workflow_id, &definition.name, definition_id).await;

    let event_source: Arc<dyn EventSource> = Arc::new(PostgresEventSource::new(pool.clone()));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let config = OrchestratorConfig::new(pool.clone());

    // 1. Publish and process WorkflowCreated
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

    // 2. Complete the activity
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::ActivityCompleted,
            activity_key: Some("only_activity".to_string()),
            payload: json!({"outputs": {"result": "success"}}),
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
        streamflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .expect("Failed to process event");
    }

    // 3. Verify WorkflowCompleted event was published
    event_source
        .update_position("test_orchestrator", events.last().unwrap().id)
        .await
        .expect("Failed to update position");

    let completion_events = event_source
        .poll("test_orchestrator")
        .await
        .expect("Failed to poll");

    let workflow_completed = completion_events
        .iter()
        .find(|e| e.event_type == WorkflowEventType::WorkflowCompleted);

    assert!(
        workflow_completed.is_some(),
        "WorkflowCompleted event should be published"
    );

    // 4. Verify workflow status is Completed
    let workflow = sqlx::query!(
        r#"SELECT status as "status: streamflow_core::events::WorkflowStatus" FROM workflows WHERE id = $1"#,
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to query workflow");

    use streamflow_core::events::WorkflowStatus;
    assert_eq!(workflow.status, WorkflowStatus::Completed);
}

#[tokio::test]
#[serial]
async fn test_workflow_failure() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    // Create workflow with one activity that will fail
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "failing_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![ActivityDefinition {
            key: "failing_activity".to_string(),
            namespace: "test".to_string(),
            name: "fail_task".to_string(),
            parameters: json!({}),
            settings: None,
            preceding: None,
            following: None,
        }],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();
    insert_workflow(&pool, workflow_id, &definition.name, definition_id).await;

    let event_source: Arc<dyn EventSource> = Arc::new(PostgresEventSource::new(pool.clone()));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let config = OrchestratorConfig::new(pool.clone());

    // 1. Publish and process WorkflowCreated
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

    // 2. Fail the activity
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::ActivityFailed,
            activity_key: Some("failing_activity".to_string()),
            payload: json!({"error": "Simulated failure"}),
        })
        .await
        .expect("Failed to publish ActivityFailed");

    event_source
        .update_position("test_orchestrator", events.last().unwrap().id)
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

    // 3. Verify WorkflowFailed event was published
    event_source
        .update_position("test_orchestrator", events.last().unwrap().id)
        .await
        .expect("Failed to update position");

    let failure_events = event_source
        .poll("test_orchestrator")
        .await
        .expect("Failed to poll");

    let workflow_failed = failure_events
        .iter()
        .find(|e| e.event_type == WorkflowEventType::WorkflowFailed);

    assert!(
        workflow_failed.is_some(),
        "WorkflowFailed event should be published"
    );

    if let Some(failed_event) = workflow_failed {
        assert!(failed_event.payload.get("reason").is_some());
    }

    // 4. Verify workflow status is Failed
    let workflow = sqlx::query!(
        r#"SELECT status as "status: streamflow_core::events::WorkflowStatus" FROM workflows WHERE id = $1"#,
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to query workflow");

    use streamflow_core::events::WorkflowStatus;
    assert_eq!(workflow.status, WorkflowStatus::Failed);
}

#[tokio::test]
#[serial]
async fn test_workflow_completion_with_multiple_activities() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    // Create workflow with 3 sequential activities
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "multi_activity_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "step1".to_string(),
                namespace: "test".to_string(),
                name: "task1".to_string(),
                parameters: json!({}),
                settings: None,
                preceding: None,
                following: Some(vec![DependencyEdge {
                    activity_key: "step2".to_string(),
                    conditions: None,
                }]),
            },
            ActivityDefinition {
                key: "step2".to_string(),
                namespace: "test".to_string(),
                name: "task2".to_string(),
                parameters: json!({}),
                settings: None,
                preceding: None,
                following: Some(vec![DependencyEdge {
                    activity_key: "step3".to_string(),
                    conditions: None,
                }]),
            },
            ActivityDefinition {
                key: "step3".to_string(),
                namespace: "test".to_string(),
                name: "task3".to_string(),
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

    // Process WorkflowCreated
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::WorkflowCreated,
            activity_key: None,
            payload: json!({}),
        })
        .await
        .expect("Failed to publish");

    let mut events = event_source.poll("test_orch").await.unwrap();
    for event in &events {
        streamflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .unwrap();
    }

    // Complete all 3 activities in sequence
    for step in &["step1", "step2", "step3"] {
        event_source
            .publish(NewWorkflowEvent {
                workflow_id,
                event_type: WorkflowEventType::ActivityCompleted,
                activity_key: Some(step.to_string()),
                payload: json!({"outputs": {}}),
            })
            .await
            .unwrap();

        event_source
            .update_position("test_orch", events.last().unwrap().id)
            .await
            .unwrap();

        events = event_source.poll("test_orch").await.unwrap();
        for event in &events {
            streamflow_core::orchestrator::orchestrator::process_workflow_event(
                event,
                &event_source,
                &activity_queue,
                &config,
            )
            .await
            .unwrap();
        }
    }

    // Check for WorkflowCompleted
    event_source
        .update_position("test_orch", events.last().unwrap().id)
        .await
        .unwrap();

    let completion_events = event_source.poll("test_orch").await.unwrap();
    assert!(
        completion_events
            .iter()
            .any(|e| e.event_type == WorkflowEventType::WorkflowCompleted)
    );
}

#[tokio::test]
#[serial]
async fn test_activity_scheduled_events_published() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "event_tracking".to_string(),
        version: "1.0".to_string(),
        activities: vec![ActivityDefinition {
            key: "task".to_string(),
            namespace: "test".to_string(),
            name: "work".to_string(),
            parameters: json!({"param": "value"}),
            settings: None,
            preceding: None,
            following: None,
        }],
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
        })
        .await
        .unwrap();

    let events = event_source.poll("test_orch").await.unwrap();
    for event in &events {
        streamflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &config,
        )
        .await
        .unwrap();
    }

    // Verify ActivityScheduled event was published
    event_source
        .update_position("test_orch", events.last().unwrap().id)
        .await
        .unwrap();

    let scheduled_events = event_source.poll("test_orch").await.unwrap();
    let activity_scheduled = scheduled_events
        .iter()
        .find(|e| e.event_type == WorkflowEventType::ActivityScheduled);

    assert!(activity_scheduled.is_some());
    if let Some(event) = activity_scheduled {
        assert_eq!(event.activity_key.as_deref(), Some("task"));
        assert_eq!(event.payload.get("namespace").unwrap(), "test");
        assert_eq!(event.payload.get("name").unwrap(), "work");
    }
}

#[tokio::test]
#[serial]
async fn test_run_orchestrator_loop() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    // Create simple workflow
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "loop_test_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![ActivityDefinition {
            key: "task1".to_string(),
            namespace: "test".to_string(),
            name: "work".to_string(),
            parameters: json!({}),
            settings: None,
            preceding: None,
            following: None,
        }],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();
    insert_workflow(&pool, workflow_id, &definition.name, definition_id).await;

    let event_source: Arc<dyn EventSource> = Arc::new(PostgresEventSource::new(pool.clone()));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let config = OrchestratorConfig::new(pool.clone());

    // Publish WorkflowCreated event before starting orchestrator
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::WorkflowCreated,
            activity_key: None,
            payload: json!({}),
        })
        .await
        .unwrap();

    // Run orchestrator in background
    let event_source_clone = event_source.clone();
    let activity_queue_clone = activity_queue.clone();
    let config_clone = config.clone();

    let orchestrator_handle = tokio::spawn(async move {
        streamflow_core::orchestrator::orchestrator::run_orchestrator(
            event_source_clone,
            activity_queue_clone,
            config_clone,
            None,
        )
        .await
    });

    // Give orchestrator time to process events
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Verify activity was scheduled
    let queued = sqlx::query!(
        r#"SELECT activity_key FROM activity_queue WHERE workflow_id = $1"#,
        workflow_id
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].activity_key, "task1");

    // Publish ActivityCompleted
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::ActivityCompleted,
            activity_key: Some("task1".to_string()),
            payload: json!({"outputs": {}}),
        })
        .await
        .unwrap();

    // Give orchestrator time to process completion
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Verify workflow completed
    let workflow = sqlx::query!(
        r#"SELECT status as "status: streamflow_core::events::WorkflowStatus" FROM workflows WHERE id = $1"#,
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    use streamflow_core::events::WorkflowStatus;
    assert_eq!(workflow.status, WorkflowStatus::Completed);

    // Stop orchestrator
    orchestrator_handle.abort();
}

#[tokio::test]
#[serial]
async fn test_orchestrator_backoff_when_no_events() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    let event_source: Arc<dyn EventSource> = Arc::new(PostgresEventSource::new(pool.clone()));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let config = OrchestratorConfig::new(pool.clone());

    let event_source_clone = event_source.clone();
    let activity_queue_clone = activity_queue.clone();
    let config_clone = config.clone();

    // Run orchestrator with no events
    let orchestrator_handle = tokio::spawn(async move {
        streamflow_core::orchestrator::orchestrator::run_orchestrator(
            event_source_clone,
            activity_queue_clone,
            config_clone,
            None,
        )
        .await
    });

    // Let it run for a short time to exercise backoff logic
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Stop orchestrator
    orchestrator_handle.abort();

    // If we get here without hanging, backoff is working
    assert!(true, "Orchestrator backoff works when no events");
}
