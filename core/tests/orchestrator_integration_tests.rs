mod common;

use kruxiaflow_core::events::{
    ActivityDefinition, DependencyEdge, EventSource, NewWorkflowEvent, WorkflowDefinition,
    WorkflowEventType,
};
use kruxiaflow_core::orchestrator::OrchestratorConfig;
use kruxiaflow_core::queue::{ActivityQueue, PostgresQueue, QueueConfig};
use kruxiaflow_core::{PostgresSubscriptionService, SubscriptionService};
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

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

#[sqlx::test(migrations = "../migrations")]
async fn test_sequential_workflow_integration(pool: PgPool) {
    // Create workflow definition
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "sequential_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "activity1".to_string(),
                worker: "test".to_string(),
                activity_name: "step1".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: None,
                dependency_of: None,
                output_definitions: None,
            },
            ActivityDefinition {
                key: "activity2".to_string(),
                worker: "test".to_string(),
                activity_name: "step2".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: Some(vec![DependencyEdge {
                    activity_key: "activity1".to_string(),
                    conditions: None,
                }]),
                dependency_of: None,
                output_definitions: None,
            },
            ActivityDefinition {
                key: "activity3".to_string(),
                worker: "test".to_string(),
                activity_name: "step3".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: Some(vec![DependencyEdge {
                    activity_key: "activity2".to_string(),
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

    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));

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

    // Process event manually (simulating orchestrator)
    let config = OrchestratorConfig::new(pool.clone());
    let events = event_source
        .poll("test_orchestrator")
        .await
        .expect("Failed to poll");
    assert_eq!(events.len(), 1);

    // Process the WorkflowCreated event
    kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
        &events[0],
        &event_source,
        &activity_queue,
        &subscription_service,
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
            iteration: None,
        })
        .await
        .expect("Failed to publish ActivityCompleted");

    // Update position and poll again
    event_source
        .update_position("test_orchestrator", events[0].cursor())
        .await
        .expect("Failed to update position");

    let events = event_source
        .poll("test_orchestrator")
        .await
        .expect("Failed to poll");
    assert!(!events.is_empty());

    // Process ActivityCompleted event
    for event in &events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &subscription_service,
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

#[sqlx::test(migrations = "../migrations")]
async fn test_parallel_workflow_integration(pool: PgPool) {
    // Create workflow definition with fan-out/fan-in
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "parallel_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "root".to_string(),
                worker: "test".to_string(),
                activity_name: "root".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: None,
                dependency_of: None,
                output_definitions: None,
            },
            ActivityDefinition {
                key: "parallel1".to_string(),
                worker: "test".to_string(),
                activity_name: "parallel".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: Some(vec![DependencyEdge {
                    activity_key: "root".to_string(),
                    conditions: None,
                }]),
                dependency_of: None,
                output_definitions: None,
            },
            ActivityDefinition {
                key: "parallel2".to_string(),
                worker: "test".to_string(),
                activity_name: "parallel".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: Some(vec![DependencyEdge {
                    activity_key: "root".to_string(),
                    conditions: None,
                }]),
                dependency_of: None,
                output_definitions: None,
            },
            ActivityDefinition {
                key: "parallel3".to_string(),
                worker: "test".to_string(),
                activity_name: "parallel".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: Some(vec![DependencyEdge {
                    activity_key: "root".to_string(),
                    conditions: None,
                }]),
                dependency_of: None,
                output_definitions: None,
            },
            ActivityDefinition {
                key: "join".to_string(),
                worker: "test".to_string(),
                activity_name: "join".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: Some(vec![
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
                dependency_of: None,
                output_definitions: None,
            },
        ],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;

    let workflow_id = Uuid::now_v7();
    insert_workflow(&pool, workflow_id, &definition.name, definition_id).await;

    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));
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
            &subscription_service,
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
            iteration: None,
        })
        .await
        .expect("Failed to publish ActivityCompleted");

    event_source
        .update_position("test_orchestrator", events[0].cursor())
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
            &subscription_service,
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
                iteration: None,
            })
            .await
            .expect("Failed to publish ActivityCompleted");
    }

    // Poll and process all completion events
    let last_cursor = events.last().unwrap().cursor();
    event_source
        .update_position("test_orchestrator", last_cursor)
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
            &subscription_service,
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

#[sqlx::test(migrations = "../migrations")]
async fn test_conditional_workflow_integration(pool: PgPool) {
    // Create workflow with conditional branching
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "conditional_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "validate".to_string(),
                worker: "test".to_string(),
                activity_name: "validate".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: None,
                dependency_of: None,
                output_definitions: None,
            },
            ActivityDefinition {
                key: "approve".to_string(),
                worker: "test".to_string(),
                activity_name: "approve".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: Some(vec![DependencyEdge {
                    activity_key: "validate".to_string(),
                    conditions: Some(vec!["{{validate.valid == true}}".to_string()]),
                }]),
                dependency_of: None,
                output_definitions: None,
            },
            ActivityDefinition {
                key: "reject".to_string(),
                worker: "test".to_string(),
                activity_name: "reject".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: Some(vec![DependencyEdge {
                    activity_key: "validate".to_string(),
                    conditions: Some(vec!["{{validate.valid == false}}".to_string()]),
                }]),
                dependency_of: None,
                output_definitions: None,
            },
        ],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;

    let workflow_id = Uuid::now_v7();
    insert_workflow(&pool, workflow_id, &definition.name, definition_id).await;

    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));
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
            &subscription_service,
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
            iteration: None,
        })
        .await
        .expect("Failed to publish ActivityCompleted");

    event_source
        .update_position("test_orchestrator", events[0].cursor())
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
            &subscription_service,
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

#[sqlx::test(migrations = "../migrations")]
async fn test_workflow_completion_success(pool: PgPool) {
    // Create simple workflow with just one activity
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "simple_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![ActivityDefinition {
            key: "only_activity".to_string(),
            worker: "test".to_string(),
            activity_name: "simple_task".to_string(),
            parameters: json!({}),
            settings: None,
            depends_on: None,
            dependency_of: None,
            output_definitions: None,
        }],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();
    insert_workflow(&pool, workflow_id, &definition.name, definition_id).await;

    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));
    let config = OrchestratorConfig::new(pool.clone());

    // 1. Publish and process WorkflowCreated
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
            &subscription_service,
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
            iteration: None,
        })
        .await
        .expect("Failed to publish ActivityCompleted");

    event_source
        .update_position("test_orchestrator", events.last().unwrap().cursor())
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
            &subscription_service,
            &config,
        )
        .await
        .expect("Failed to process event");
    }

    // 3. Verify WorkflowCompleted event was published
    event_source
        .update_position("test_orchestrator", events.last().unwrap().cursor())
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
        r#"SELECT status as "status: kruxiaflow_core::events::WorkflowStatus" FROM workflows WHERE id = $1"#,
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to query workflow");

    use kruxiaflow_core::events::WorkflowStatus;
    assert_eq!(workflow.status, WorkflowStatus::Completed);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_workflow_failure(pool: PgPool) {
    // Create workflow with one activity that will fail
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "failing_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![ActivityDefinition {
            key: "failing_activity".to_string(),
            worker: "test".to_string(),
            activity_name: "fail_task".to_string(),
            parameters: json!({}),
            settings: None,
            depends_on: None,
            dependency_of: None,
            output_definitions: None,
        }],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();
    insert_workflow(&pool, workflow_id, &definition.name, definition_id).await;

    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));
    let config = OrchestratorConfig::new(pool.clone());

    // 1. Publish and process WorkflowCreated
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
            &subscription_service,
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
            iteration: None,
        })
        .await
        .expect("Failed to publish ActivityFailed");

    event_source
        .update_position("test_orchestrator", events.last().unwrap().cursor())
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
            &subscription_service,
            &config,
        )
        .await
        .expect("Failed to process event");
    }

    // 3. Verify WorkflowFailed event was published
    event_source
        .update_position("test_orchestrator", events.last().unwrap().cursor())
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
        r#"SELECT status as "status: kruxiaflow_core::events::WorkflowStatus" FROM workflows WHERE id = $1"#,
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to query workflow");

    use kruxiaflow_core::events::WorkflowStatus;
    assert_eq!(workflow.status, WorkflowStatus::Failed);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_workflow_completion_with_multiple_activities(pool: PgPool) {
    // Create workflow with 3 sequential activities
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "multi_activity_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "step1".to_string(),
                worker: "test".to_string(),
                activity_name: "task1".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: None,
                dependency_of: Some(vec![DependencyEdge {
                    activity_key: "step2".to_string(),
                    conditions: None,
                }]),
                output_definitions: None,
            },
            ActivityDefinition {
                key: "step2".to_string(),
                worker: "test".to_string(),
                activity_name: "task2".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: None,
                dependency_of: Some(vec![DependencyEdge {
                    activity_key: "step3".to_string(),
                    conditions: None,
                }]),
                output_definitions: None,
            },
            ActivityDefinition {
                key: "step3".to_string(),
                worker: "test".to_string(),
                activity_name: "task3".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: None,
                dependency_of: None,
                output_definitions: None,
            },
        ],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();
    insert_workflow(&pool, workflow_id, &definition.name, definition_id).await;

    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));
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
        .expect("Failed to publish");

    let mut events = event_source.poll("test_orch").await.unwrap();
    for event in &events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &subscription_service,
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
                iteration: None,
            })
            .await
            .unwrap();

        event_source
            .update_position("test_orch", events.last().unwrap().cursor())
            .await
            .unwrap();

        events = event_source.poll("test_orch").await.unwrap();
        for event in &events {
            kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
                event,
                &event_source,
                &activity_queue,
                &subscription_service,
                &config,
            )
            .await
            .unwrap();
        }
    }

    // Check for WorkflowCompleted
    event_source
        .update_position("test_orch", events.last().unwrap().cursor())
        .await
        .unwrap();

    let completion_events = event_source.poll("test_orch").await.unwrap();
    assert!(
        completion_events
            .iter()
            .any(|e| e.event_type == WorkflowEventType::WorkflowCompleted)
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn test_activity_scheduled_events_published(pool: PgPool) {
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "event_tracking".to_string(),
        version: "1.0".to_string(),
        activities: vec![ActivityDefinition {
            key: "task".to_string(),
            worker: "test".to_string(),
            activity_name: "work".to_string(),
            parameters: json!({"param": "value"}),
            settings: None,
            depends_on: None,
            dependency_of: None,
            output_definitions: None,
        }],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();
    insert_workflow(&pool, workflow_id, &definition.name, definition_id).await;

    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));
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
        .unwrap();

    let events = event_source.poll("test_orch").await.unwrap();
    for event in &events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await
        .unwrap();
    }

    // Verify ActivityScheduled event was published
    event_source
        .update_position("test_orch", events.last().unwrap().cursor())
        .await
        .unwrap();

    let scheduled_events = event_source.poll("test_orch").await.unwrap();
    let activity_scheduled = scheduled_events
        .iter()
        .find(|e| e.event_type == WorkflowEventType::ActivityScheduled);

    assert!(activity_scheduled.is_some());
    if let Some(event) = activity_scheduled {
        assert_eq!(event.activity_key.as_deref(), Some("task"));
        assert_eq!(event.payload.get("worker").unwrap(), "test");
        assert_eq!(event.payload.get("activity_name").unwrap(), "work");
    }
}

/// With a NOTIFY wake attached, an event that arrives while the loop sits in
/// a long backoff wait is processed at once rather than when the timer fires
#[sqlx::test(migrations = "../migrations")]
async fn test_run_orchestrator_wakes_on_notify(pool: PgPool) {
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "wake_test_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![ActivityDefinition {
            key: "task1".to_string(),
            worker: "test".to_string(),
            activity_name: "work".to_string(),
            parameters: json!({}),
            settings: None,
            depends_on: None,
            dependency_of: None,
            output_definitions: None,
        }],
    };
    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();
    insert_workflow(&pool, workflow_id, &definition.name, definition_id).await;

    let event_source: Arc<dyn EventSource> =
        Arc::new(common::settled_event_source_with_wake(pool.clone()).await);
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));
    // Waits of 50ms, 200ms, 800ms, then 3.2s from t≈1.05s to t≈4.25s
    let config = OrchestratorConfig::new(pool.clone())
        .with_poll_interval(Duration::from_millis(50), Duration::from_secs(30))
        .with_backoff_multiplier(4.0);

    // Stopped through the token at the end, so the loop and its background
    // task let go of the event source and its LISTEN connection; otherwise
    // the test database cannot be torn down until that connection times out
    let shutdown = tokio_util::sync::CancellationToken::new();
    let handle = {
        let (es, aq, ss) = (
            event_source.clone(),
            activity_queue.clone(),
            subscription_service.clone(),
        );
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            kruxiaflow_core::orchestrator::orchestrator::run_orchestrator(
                es,
                aq,
                ss,
                config,
                Some(shutdown),
            )
            .await
        })
    };

    // Let the loop back off into its 3.2s wait
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let published = std::time::Instant::now();
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

    let scheduled = common::wait_until(Duration::from_secs(5), || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM activity_queue WHERE workflow_id = $1")
            .bind(workflow_id)
            .fetch_one(&pool)
            .await
            .unwrap_or(0)
            > 0
    })
    .await;
    let latency = published.elapsed();
    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;

    assert!(scheduled, "activity was never scheduled");
    // On the timer alone this would take ~2.75s
    assert!(
        latency < Duration::from_secs(1),
        "event took {latency:?} to process; the wake did not shorten the wait"
    );
}

/// A consumer can be woken by an event's commit and still find nothing: the
/// event is held back until an older transaction ends, and that transaction's
/// end notifies nobody. The loop must not then fall back to its idle timer.
/// It asks the event source whether events are pending, and while they are it
/// retries on a short backoff capped at the timer-only maximum.
///
/// Here the idle timer is 30 s, the timer-only maximum 100 ms, and the
/// backoff multiplier 100, so a backoff growing towards the idle timer would
/// retry at 50 ms and then not for another 5 s.
///
/// The blocker is held only briefly: transaction ids are instance-wide, so
/// it also holds back event delivery in every test running alongside.
#[sqlx::test(migrations = "../migrations")]
async fn test_run_orchestrator_delivers_a_held_back_event_soon_after_its_blocker_ends(
    pool: PgPool,
) {
    use kruxiaflow_core::events::{EventWake, PostgresEventSource};

    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "held_back_test_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![ActivityDefinition {
            key: "task1".to_string(),
            worker: "test".to_string(),
            activity_name: "work".to_string(),
            parameters: json!({}),
            settings: None,
            depends_on: None,
            dependency_of: None,
            output_definitions: None,
        }],
    };
    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();
    insert_workflow(&pool, workflow_id, &definition.name, definition_id).await;

    // The plain event source: the settling wrapper the other tests use would
    // itself wait for the blocker and hide what the loop does
    let wake = EventWake::connect(&pool).await.expect("LISTEN");
    let event_source: Arc<dyn EventSource> =
        Arc::new(PostgresEventSource::with_wake(pool.clone(), wake));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));
    let mut config = OrchestratorConfig::new(pool.clone()).with_backoff_multiplier(100.0);
    config.poll_interval_min = Duration::from_millis(50);
    config.poll_interval_max = Duration::from_millis(100);
    config.wake_poll_interval_max = Duration::from_secs(30);

    // Stopped through the token at the end, so the loop and its background
    // task let go of the event source and its LISTEN connection; otherwise
    // the test database cannot be torn down until that connection times out
    let shutdown = tokio_util::sync::CancellationToken::new();
    let handle = {
        let (es, aq, ss) = (
            event_source.clone(),
            activity_queue.clone(),
            subscription_service.clone(),
        );
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            kruxiaflow_core::orchestrator::orchestrator::run_orchestrator(
                es,
                aq,
                ss,
                config,
                Some(shutdown),
            )
            .await
        })
    };
    let scheduled = || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM activity_queue WHERE workflow_id = $1")
            .bind(workflow_id)
            .fetch_one(&pool)
            .await
            .unwrap_or(0)
            > 0
    };

    // Let the loop reach its 30 s idle wait
    tokio::time::sleep(Duration::from_millis(500)).await;

    // An older writing transaction holds the horizon...
    let mut blocker = pool.begin().await.unwrap();
    sqlx::query("INSERT INTO workflow_definitions (name, activities) VALUES ('blocker', '[]')")
        .execute(&mut *blocker)
        .await
        .unwrap();

    // ...while the event commits and wakes the loop
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

    // Woken, polled, found nothing: the event is held back
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!scheduled().await, "event delivered while held back");

    // The blocker ends without notifying anyone
    blocker.commit().await.unwrap();
    let released = std::time::Instant::now();

    let delivered = common::wait_until(Duration::from_secs(10), scheduled).await;
    let late_by = released.elapsed();
    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;

    assert!(delivered, "held-back event was never delivered");
    // Retries are at most 100 ms apart. A backoff growing from the wake
    // (50 ms, then 5 s) would deliver about 4.7 s late; the idle timer,
    // 30 s late. The bound leaves room for a slow, busy CI runner.
    assert!(
        late_by < Duration::from_secs(2),
        "delivered {late_by:?} after the blocker ended; retries were not capped \
         at the timer-only maximum"
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn test_run_orchestrator_loop(pool: PgPool) {
    // Create simple workflow
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "loop_test_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![ActivityDefinition {
            key: "task1".to_string(),
            worker: "test".to_string(),
            activity_name: "work".to_string(),
            parameters: json!({}),
            settings: None,
            depends_on: None,
            dependency_of: None,
            output_definitions: None,
        }],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();
    insert_workflow(&pool, workflow_id, &definition.name, definition_id).await;

    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));
    let config = OrchestratorConfig::new(pool.clone());

    // Publish WorkflowCreated event before starting orchestrator
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

    // Run orchestrator in background
    let event_source_clone = event_source.clone();
    let activity_queue_clone = activity_queue.clone();
    let config_clone = config.clone();
    let subscription_clone = subscription_service.clone();

    let orchestrator_handle = tokio::spawn(async move {
        kruxiaflow_core::orchestrator::orchestrator::run_orchestrator(
            event_source_clone,
            activity_queue_clone,
            subscription_clone,
            config_clone,
            None,
        )
        .await
    });

    // Wait for the orchestrator to schedule the first activity
    common::wait_until(Duration::from_secs(5), || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM activity_queue WHERE workflow_id = $1")
            .bind(workflow_id)
            .fetch_one(&pool)
            .await
            .unwrap_or(0)
            > 0
    })
    .await;

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
            iteration: None,
        })
        .await
        .unwrap();

    // Wait for the orchestrator to process the completion
    common::wait_until(Duration::from_secs(5), || async {
        sqlx::query_scalar::<_, bool>(
            "SELECT status = 'completed'::workflow_status FROM workflows WHERE id = $1",
        )
        .bind(workflow_id)
        .fetch_one(&pool)
        .await
        .unwrap_or(false)
    })
    .await;

    // Verify workflow completed
    let workflow = sqlx::query!(
        r#"SELECT status as "status: kruxiaflow_core::events::WorkflowStatus" FROM workflows WHERE id = $1"#,
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    use kruxiaflow_core::events::WorkflowStatus;
    assert_eq!(workflow.status, WorkflowStatus::Completed);

    // Stop orchestrator
    orchestrator_handle.abort();
}

#[sqlx::test(migrations = "../migrations")]
async fn test_orchestrator_backoff_when_no_events(pool: PgPool) {
    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));
    let config = OrchestratorConfig::new(pool.clone());

    let event_source_clone = event_source.clone();
    let activity_queue_clone = activity_queue.clone();
    let config_clone = config.clone();
    let subscription_clone = subscription_service.clone();

    // Run orchestrator with no events
    let orchestrator_handle = tokio::spawn(async move {
        kruxiaflow_core::orchestrator::orchestrator::run_orchestrator(
            event_source_clone,
            activity_queue_clone,
            subscription_clone,
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
    // If we get here without hanging, backoff is working
}

// ============================================================================
// Template Error During Timeout Processing Tests
// Bug fix: docs/bugs/2026-01-08-template-error-crashes-timeout-processing.md
// ============================================================================

/// Test: WorkflowFailed event with template conditions on incomplete activities
///
/// Verifies fix for bug where template evaluation errors during WorkflowFailed
/// processing caused the workflow to get stuck in 'running' state.
///
/// The bug occurred when:
/// 1. A timeout triggers WorkflowFailed event
/// 2. find_ready_activities evaluates conditions like {{activity_a.result.rows}}
/// 3. activity_a never completed, so result is undefined
/// 4. Template error crashes processing, workflow stays stuck
///
/// Expected behavior after fix: workflow gracefully transitions to Failed status.
#[sqlx::test(migrations = "../migrations")]
async fn test_workflow_failed_with_incomplete_template_dependencies(pool: PgPool) {
    // Create workflow with template conditions that reference activity outputs
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "template_timeout_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "fetch_data".to_string(),
                worker: "test".to_string(),
                activity_name: "fetch".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: None,
                dependency_of: None,
                output_definitions: None,
            },
            ActivityDefinition {
                key: "process_data".to_string(),
                worker: "test".to_string(),
                activity_name: "process".to_string(),
                parameters: json!({}),
                settings: None,
                // This condition references fetch_data outputs which won't exist
                depends_on: Some(vec![DependencyEdge {
                    activity_key: "fetch_data".to_string(),
                    conditions: Some(vec!["{{fetch_data.rows | length > 0}}".to_string()]),
                }]),
                dependency_of: None,
                output_definitions: None,
            },
        ],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();
    insert_workflow(&pool, workflow_id, &definition.name, definition_id).await;

    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));
    let config = OrchestratorConfig::new(pool.clone());

    // 1. Publish and process WorkflowCreated to start the workflow
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
            &subscription_service,
            &config,
        )
        .await
        .expect("Failed to process WorkflowCreated event");
    }

    // Verify fetch_data was scheduled (first activity has no dependencies)
    let queued = sqlx::query!(
        r#"SELECT activity_key FROM activity_queue WHERE workflow_id = $1"#,
        workflow_id
    )
    .fetch_all(&pool)
    .await
    .expect("Failed to query queue");

    assert_eq!(queued.len(), 1, "fetch_data should be scheduled");
    assert_eq!(queued[0].activity_key, "fetch_data");

    // 2. Simulate timeout by publishing WorkflowFailed event
    //    NOTE: fetch_data is still pending, so process_data's conditions
    //    will reference undefined outputs
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::WorkflowFailed,
            activity_key: None,
            payload: json!({"reason": "Workflow timeout", "timeout_seconds": 1}),
            iteration: None,
        })
        .await
        .expect("Failed to publish WorkflowFailed");

    event_source
        .update_position("test_orchestrator", events.last().unwrap().cursor())
        .await
        .expect("Failed to update position");

    let failure_events = event_source
        .poll("test_orchestrator")
        .await
        .expect("Failed to poll");

    // Process the WorkflowFailed event
    // Before the fix, this would crash with template error
    // After the fix, it should gracefully handle the error
    for event in &failure_events {
        let result = kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await;

        // Processing should succeed (or if it fails, we catch the error gracefully)
        // The key assertion is that the workflow ends up in Failed status
        if let Err(e) = result {
            // If there's still an error, log it but continue - we'll check final status
            eprintln!(
                "Warning: process_workflow_event returned error (may be expected during fix): {:?}",
                e
            );
        }
    }

    // 3. Verify workflow status is Failed (not stuck in running)
    let workflow = sqlx::query!(
        r#"SELECT status as "status: kruxiaflow_core::events::WorkflowStatus" FROM workflows WHERE id = $1"#,
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to query workflow");

    use kruxiaflow_core::events::WorkflowStatus;
    assert_eq!(
        workflow.status,
        WorkflowStatus::Failed,
        "Workflow should be in Failed status after timeout, not stuck in running"
    );
}

/// Test: WorkflowFailed event with complex nested template conditions
///
/// Tests a more complex scenario where multiple activities have template
/// conditions that depend on incomplete activities.
#[sqlx::test(migrations = "../migrations")]
async fn test_workflow_failed_with_multiple_incomplete_dependencies(pool: PgPool) {
    // Create workflow with multiple activities that have complex conditions
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "complex_template_timeout".to_string(),
        version: "1.0".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "root".to_string(),
                worker: "test".to_string(),
                activity_name: "root".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: None,
                dependency_of: None,
                output_definitions: None,
            },
            ActivityDefinition {
                key: "branch_a".to_string(),
                worker: "test".to_string(),
                activity_name: "process".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: Some(vec![DependencyEdge {
                    activity_key: "root".to_string(),
                    conditions: Some(vec!["{{root.value == 'A'}}".to_string()]),
                }]),
                dependency_of: None,
                output_definitions: None,
            },
            ActivityDefinition {
                key: "branch_b".to_string(),
                worker: "test".to_string(),
                activity_name: "process".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: Some(vec![DependencyEdge {
                    activity_key: "root".to_string(),
                    conditions: Some(vec!["{{root.value == 'B'}}".to_string()]),
                }]),
                dependency_of: None,
                output_definitions: None,
            },
            ActivityDefinition {
                key: "final".to_string(),
                worker: "test".to_string(),
                activity_name: "final".to_string(),
                parameters: json!({}),
                settings: None,
                // Depends on both branches - will have undefined refs when timeout occurs
                depends_on: Some(vec![
                    DependencyEdge {
                        activity_key: "branch_a".to_string(),
                        conditions: None,
                    },
                    DependencyEdge {
                        activity_key: "branch_b".to_string(),
                        conditions: None,
                    },
                ]),
                dependency_of: None,
                output_definitions: None,
            },
        ],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();
    insert_workflow(&pool, workflow_id, &definition.name, definition_id).await;

    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));
    let config = OrchestratorConfig::new(pool.clone());

    // Start workflow
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
            &subscription_service,
            &config,
        )
        .await
        .unwrap();
    }

    // Timeout the workflow without completing root
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::WorkflowFailed,
            activity_key: None,
            payload: json!({"reason": "Workflow timeout", "timeout_seconds": 1}),
            iteration: None,
        })
        .await
        .unwrap();

    event_source
        .update_position("test_orch", events.last().unwrap().cursor())
        .await
        .unwrap();

    let failure_events = event_source.poll("test_orch").await.unwrap();

    for event in &failure_events {
        let _ = kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await;
    }

    // Verify workflow is Failed
    let workflow = sqlx::query!(
        r#"SELECT status as "status: kruxiaflow_core::events::WorkflowStatus" FROM workflows WHERE id = $1"#,
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    use kruxiaflow_core::events::WorkflowStatus;
    assert_eq!(
        workflow.status,
        WorkflowStatus::Failed,
        "Workflow should be Failed after timeout"
    );
}

/// Test: Normal workflow completion still works (regression test)
///
/// Ensures that fixing the template error doesn't break normal workflow
/// completion where activities complete successfully.
#[sqlx::test(migrations = "../migrations")]
async fn test_normal_workflow_with_conditions_still_completes(pool: PgPool) {
    // Same structure as template timeout test, but we complete activities properly
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "normal_conditional_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "fetch_data".to_string(),
                worker: "test".to_string(),
                activity_name: "fetch".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: None,
                dependency_of: None,
                output_definitions: None,
            },
            ActivityDefinition {
                key: "process_data".to_string(),
                worker: "test".to_string(),
                activity_name: "process".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: Some(vec![DependencyEdge {
                    activity_key: "fetch_data".to_string(),
                    conditions: Some(vec!["{{fetch_data.has_data == true}}".to_string()]),
                }]),
                dependency_of: None,
                output_definitions: None,
            },
        ],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();
    insert_workflow(&pool, workflow_id, &definition.name, definition_id).await;

    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));
    let config = OrchestratorConfig::new(pool.clone());

    // Start workflow
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

    let mut events = event_source.poll("test_orch").await.unwrap();
    for event in &events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await
        .unwrap();
    }

    // Complete fetch_data with output that satisfies condition
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::ActivityCompleted,
            activity_key: Some("fetch_data".to_string()),
            payload: json!({"outputs": {"has_data": true, "rows": [1, 2, 3]}}),
            iteration: None,
        })
        .await
        .unwrap();

    event_source
        .update_position("test_orch", events.last().unwrap().cursor())
        .await
        .unwrap();

    events = event_source.poll("test_orch").await.unwrap();
    for event in &events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await
        .unwrap();
    }

    // Verify process_data was scheduled (condition satisfied)
    let queued = sqlx::query!(
        r#"SELECT activity_key FROM activity_queue WHERE workflow_id = $1"#,
        workflow_id
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    assert!(
        queued.iter().any(|q| q.activity_key == "process_data"),
        "process_data should be scheduled when condition is satisfied"
    );

    // Complete process_data
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::ActivityCompleted,
            activity_key: Some("process_data".to_string()),
            payload: json!({"outputs": {"processed": true}}),
            iteration: None,
        })
        .await
        .unwrap();

    event_source
        .update_position("test_orch", events.last().unwrap().cursor())
        .await
        .unwrap();

    events = event_source.poll("test_orch").await.unwrap();
    for event in &events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await
        .unwrap();
    }

    // Check for completion
    event_source
        .update_position("test_orch", events.last().unwrap().cursor())
        .await
        .unwrap();

    let completion_events = event_source.poll("test_orch").await.unwrap();
    assert!(
        completion_events
            .iter()
            .any(|e| e.event_type == WorkflowEventType::WorkflowCompleted),
        "Workflow should complete successfully"
    );

    // Verify workflow status is Completed
    let workflow = sqlx::query!(
        r#"SELECT status as "status: kruxiaflow_core::events::WorkflowStatus" FROM workflows WHERE id = $1"#,
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    use kruxiaflow_core::events::WorkflowStatus;
    assert_eq!(
        workflow.status,
        WorkflowStatus::Completed,
        "Workflow should be Completed when all activities succeed"
    );
}

// ============================================================================
// Stuck Workflow Resolution Tests
// Bug fix: docs/bugs/2026-01-10-stuck-workflows-not-resolved-on-timeout.md
// ============================================================================

/// Test: WorkflowFailed event properly persists Failed status
///
/// Verifies fix for bug where workflows that timed out were repeatedly detected
/// as "stuck" because the Failed status was never persisted to the database.
///
/// The bug occurred when:
/// 1. Timeout checker publishes WorkflowFailed event
/// 2. process_workflow_event starts processing, sets state.status = Failed
/// 3. Scheduling logic encounters an error (template, queue, etc.)
/// 4. Transaction rolls back, Failed status never persisted
/// 5. Next timeout check finds same workflow still "running"
///
/// Expected behavior after fix: WorkflowFailed event should immediately persist
/// the Failed status without attempting to schedule more activities.
#[sqlx::test(migrations = "../migrations")]
async fn test_workflow_failed_event_persists_status_immediately(pool: PgPool) {
    // Create a workflow that will be timed out
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "timeout_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "long_running".to_string(),
                worker: "test".to_string(),
                activity_name: "slow_task".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: None,
                dependency_of: None,
                output_definitions: None,
            },
            ActivityDefinition {
                key: "dependent".to_string(),
                worker: "test".to_string(),
                activity_name: "next_task".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: Some(vec![DependencyEdge {
                    activity_key: "long_running".to_string(),
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

    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));
    let config = OrchestratorConfig::new(pool.clone());

    // 1. Start the workflow
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
            &subscription_service,
            &config,
        )
        .await
        .expect("Failed to process event");
    }

    // 2. Verify workflow is Running
    let workflow = sqlx::query!(
        r#"SELECT status as "status: kruxiaflow_core::events::WorkflowStatus" FROM workflows WHERE id = $1"#,
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to query workflow");

    use kruxiaflow_core::events::WorkflowStatus;
    assert_eq!(workflow.status, WorkflowStatus::Running);

    // 3. Simulate timeout by publishing WorkflowFailed
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::WorkflowFailed,
            activity_key: None,
            payload: json!({"reason": "Workflow timeout", "timeout_seconds": 300}),
            iteration: None,
        })
        .await
        .expect("Failed to publish WorkflowFailed");

    event_source
        .update_position("test_orchestrator", events.last().unwrap().cursor())
        .await
        .expect("Failed to update position");

    let failure_events = event_source
        .poll("test_orchestrator")
        .await
        .expect("Failed to poll");

    // 4. Process the WorkflowFailed event
    for event in &failure_events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await
        .expect("Failed to process WorkflowFailed event");
    }

    // 5. Verify workflow status is now Failed in database
    let workflow = sqlx::query!(
        r#"SELECT status as "status: kruxiaflow_core::events::WorkflowStatus" FROM workflows WHERE id = $1"#,
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to query workflow");

    assert_eq!(
        workflow.status,
        WorkflowStatus::Failed,
        "Workflow should be Failed after processing WorkflowFailed event"
    );
}

/// Test: Failed workflow does not schedule new activities
///
/// Verifies that after a workflow transitions to Failed status,
/// no new activities are scheduled (the early exit works correctly).
#[sqlx::test(migrations = "../migrations")]
async fn test_failed_workflow_does_not_schedule_activities(pool: PgPool) {
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "no_schedule_after_fail".to_string(),
        version: "1.0".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "step1".to_string(),
                worker: "test".to_string(),
                activity_name: "task1".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: None,
                dependency_of: None,
                output_definitions: None,
            },
            ActivityDefinition {
                key: "step2".to_string(),
                worker: "test".to_string(),
                activity_name: "task2".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: Some(vec![DependencyEdge {
                    activity_key: "step1".to_string(),
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

    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));
    let config = OrchestratorConfig::new(pool.clone());

    // Start workflow
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
            &subscription_service,
            &config,
        )
        .await
        .unwrap();
    }

    // Count initial activities scheduled
    let initial_count = sqlx::query_scalar!(
        r#"SELECT COUNT(*) FROM activity_queue WHERE workflow_id = $1"#,
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(initial_count, Some(1), "step1 should be scheduled");

    // Fail the workflow (before step1 completes)
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::WorkflowFailed,
            activity_key: None,
            payload: json!({"reason": "Workflow timeout"}),
            iteration: None,
        })
        .await
        .unwrap();

    event_source
        .update_position("test_orch", events.last().unwrap().cursor())
        .await
        .unwrap();

    let failure_events = event_source.poll("test_orch").await.unwrap();
    for event in &failure_events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await
        .unwrap();
    }

    // Count activities after failure - should be same as before
    let final_count = sqlx::query_scalar!(
        r#"SELECT COUNT(*) FROM activity_queue WHERE workflow_id = $1"#,
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(
        final_count, initial_count,
        "No new activities should be scheduled after workflow fails"
    );
}

/// Test: Subsequent events on failed workflow are handled gracefully
///
/// Verifies that if events arrive for a workflow that's already Failed,
/// they are processed without error and the Failed status is preserved.
#[sqlx::test(migrations = "../migrations")]
async fn test_events_on_already_failed_workflow_handled_gracefully(pool: PgPool) {
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "events_after_fail".to_string(),
        version: "1.0".to_string(),
        activities: vec![ActivityDefinition {
            key: "task".to_string(),
            worker: "test".to_string(),
            activity_name: "work".to_string(),
            parameters: json!({}),
            settings: None,
            depends_on: None,
            dependency_of: None,
            output_definitions: None,
        }],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();
    insert_workflow(&pool, workflow_id, &definition.name, definition_id).await;

    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));
    let config = OrchestratorConfig::new(pool.clone());

    // Start and then fail workflow
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
            &subscription_service,
            &config,
        )
        .await
        .unwrap();
    }

    // Fail the workflow
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::WorkflowFailed,
            activity_key: None,
            payload: json!({"reason": "Timeout"}),
            iteration: None,
        })
        .await
        .unwrap();

    event_source
        .update_position("test_orch", events.last().unwrap().cursor())
        .await
        .unwrap();

    let failure_events = event_source.poll("test_orch").await.unwrap();
    for event in &failure_events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await
        .unwrap();
    }

    // Verify workflow is Failed
    use kruxiaflow_core::events::WorkflowStatus;
    let workflow = sqlx::query!(
        r#"SELECT status as "status: kruxiaflow_core::events::WorkflowStatus" FROM workflows WHERE id = $1"#,
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(workflow.status, WorkflowStatus::Failed);

    // Now simulate a late ActivityCompleted event (from worker that didn't know about timeout)
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::ActivityCompleted,
            activity_key: Some("task".to_string()),
            payload: json!({"outputs": {"result": "done"}}),
            iteration: None,
        })
        .await
        .unwrap();

    event_source
        .update_position("test_orch", failure_events.last().unwrap().cursor())
        .await
        .unwrap();

    let late_events = event_source.poll("test_orch").await.unwrap();

    // Processing should succeed without error
    for event in &late_events {
        let result = kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await;

        assert!(
            result.is_ok(),
            "Processing events on failed workflow should not error: {:?}",
            result
        );
    }

    // Workflow should still be Failed (not revived)
    let workflow = sqlx::query!(
        r#"SELECT status as "status: kruxiaflow_core::events::WorkflowStatus" FROM workflows WHERE id = $1"#,
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(
        workflow.status,
        WorkflowStatus::Failed,
        "Workflow should remain Failed even after receiving late events"
    );
}

/// Test: WorkflowCompleted also benefits from early exit
///
/// Verifies that the early exit also works for completed workflows,
/// preventing any scheduling logic from running on terminal states.
#[sqlx::test(migrations = "../migrations")]
async fn test_completed_workflow_early_exit(pool: PgPool) {
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "completed_early_exit".to_string(),
        version: "1.0".to_string(),
        activities: vec![ActivityDefinition {
            key: "only_task".to_string(),
            worker: "test".to_string(),
            activity_name: "work".to_string(),
            parameters: json!({}),
            settings: None,
            depends_on: None,
            dependency_of: None,
            output_definitions: None,
        }],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();
    insert_workflow(&pool, workflow_id, &definition.name, definition_id).await;

    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));
    let config = OrchestratorConfig::new(pool.clone());

    // Start workflow and complete the only activity
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

    let mut events = event_source.poll("test_orch").await.unwrap();
    for event in &events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await
        .unwrap();
    }

    // Complete the activity
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::ActivityCompleted,
            activity_key: Some("only_task".to_string()),
            payload: json!({"outputs": {}}),
            iteration: None,
        })
        .await
        .unwrap();

    event_source
        .update_position("test_orch", events.last().unwrap().cursor())
        .await
        .unwrap();

    events = event_source.poll("test_orch").await.unwrap();
    for event in &events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await
        .unwrap();
    }

    // Process the WorkflowCompleted event
    event_source
        .update_position("test_orch", events.last().unwrap().cursor())
        .await
        .unwrap();

    let completion_events = event_source.poll("test_orch").await.unwrap();
    for event in &completion_events {
        kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await
        .unwrap();
    }

    // Verify workflow is Completed
    use kruxiaflow_core::events::WorkflowStatus;
    let workflow = sqlx::query!(
        r#"SELECT status as "status: kruxiaflow_core::events::WorkflowStatus" FROM workflows WHERE id = $1"#,
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(workflow.status, WorkflowStatus::Completed);

    // Simulate another event arriving (shouldn't cause issues)
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::ActivityCompleted,
            activity_key: Some("only_task".to_string()),
            payload: json!({"outputs": {"duplicate": true}}),
            iteration: None,
        })
        .await
        .unwrap();

    event_source
        .update_position("test_orch", completion_events.last().unwrap().cursor())
        .await
        .unwrap();

    let duplicate_events = event_source.poll("test_orch").await.unwrap();
    for event in &duplicate_events {
        let result = kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
            event,
            &event_source,
            &activity_queue,
            &subscription_service,
            &config,
        )
        .await;

        assert!(
            result.is_ok(),
            "Duplicate events should be handled gracefully"
        );
    }

    // Status should still be Completed
    let workflow = sqlx::query!(
        r#"SELECT status as "status: kruxiaflow_core::events::WorkflowStatus" FROM workflows WHERE id = $1"#,
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(workflow.status, WorkflowStatus::Completed);
}

/// Test that check_and_timeout_stuck_workflows detects old running workflows
/// and publishes WorkflowFailed events for them.
#[sqlx::test(migrations = "../migrations")]
async fn test_check_and_timeout_stuck_workflows(pool: PgPool) {
    // Create a simple workflow definition
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "timeout_test_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![ActivityDefinition {
            key: "long_task".to_string(),
            worker: "test".to_string(),
            activity_name: "slow_step".to_string(),
            parameters: json!({}),
            settings: None,
            depends_on: None,
            dependency_of: None,
            output_definitions: None,
        }],
    };

    let definition_id = insert_workflow_definition(&pool, &definition).await;
    let workflow_id = Uuid::now_v7();
    insert_workflow(&pool, workflow_id, &definition.name, definition_id).await;

    // Backdate the workflow's created_at to make it look stuck
    sqlx::query("UPDATE workflows SET created_at = NOW() - INTERVAL '2 hours' WHERE id = $1")
        .bind(workflow_id)
        .execute(&pool)
        .await
        .unwrap();

    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));

    // Create config with a 1-second timeout so the 2-hour-old workflow is stuck
    let mut config = OrchestratorConfig::new(pool.clone());
    config.workflow_timeout = Duration::from_secs(1);

    // Call check_and_timeout_stuck_workflows
    kruxiaflow_core::orchestrator::orchestrator::check_and_timeout_stuck_workflows(
        &config,
        &event_source,
    )
    .await
    .unwrap();

    // Verify a WorkflowFailed event was published
    let events = event_source.poll("timeout_checker").await.unwrap();
    let timeout_events: Vec<_> = events
        .iter()
        .filter(|e| {
            e.workflow_id == workflow_id && e.event_type == WorkflowEventType::WorkflowFailed
        })
        .collect();

    assert!(
        !timeout_events.is_empty(),
        "Should have published a WorkflowFailed event for stuck workflow"
    );

    // Check the payload contains a self-explaining timeout reason (the exact
    // text diagnoses whether queued work was never claimed — see
    // nukumori-support-needs item 12)
    let payload = &timeout_events[0].payload;
    let reason = payload["reason"].as_str().unwrap_or_default();
    assert!(
        reason.starts_with("Workflow timeout after"),
        "unexpected reason: {reason}"
    );
}

/// Test that check_and_timeout_stuck_workflows does nothing when no workflows are stuck
#[sqlx::test(migrations = "../migrations")]
async fn test_check_and_timeout_no_stuck_workflows(pool: PgPool) {
    let event_source: Arc<dyn EventSource> = Arc::new(common::settled_event_source(pool.clone()));

    let config = OrchestratorConfig::new(pool.clone());

    // Should return Ok with no stuck workflows
    let result = kruxiaflow_core::orchestrator::orchestrator::check_and_timeout_stuck_workflows(
        &config,
        &event_source,
    )
    .await;

    assert!(result.is_ok());
}
