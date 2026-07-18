use kruxiaflow_core::events::{
    EventSource, NewWorkflowEvent, PostgresEventSource, WorkflowEventType,
};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test(migrations = "../migrations")]
async fn test_publish_event(pool: PgPool) {
    let event_source = PostgresEventSource::new(pool.clone());
    let workflow_id = Uuid::now_v7();

    let event = NewWorkflowEvent {
        workflow_id,
        event_type: WorkflowEventType::WorkflowCreated,
        activity_key: None,
        payload: json!({"state_data": {}}),
        iteration: None,
    };

    // Publish event
    event_source
        .publish(event.clone())
        .await
        .expect("Failed to publish event");

    // Verify event in database
    let stored_event = sqlx::query!(
        r#"SELECT id, workflow_id, event_type as "event_type: WorkflowEventType", activity_key, payload
           FROM workflow_events
           WHERE workflow_id = $1"#,
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .expect("Event not found");

    assert_eq!(stored_event.workflow_id, workflow_id);
    assert_eq!(stored_event.event_type, WorkflowEventType::WorkflowCreated);
    assert_eq!(stored_event.activity_key, None);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_publish_event_idempotency(pool: PgPool) {
    let event_source = PostgresEventSource::new(pool.clone());
    let workflow_id = Uuid::now_v7();

    let event = NewWorkflowEvent {
        workflow_id,
        event_type: WorkflowEventType::ActivityCompleted,
        activity_key: Some("test_activity".to_string()),
        payload: json!({"outputs": {"result": "success"}}),
        iteration: None,
    };

    // Publish event twice
    event_source
        .publish(event.clone())
        .await
        .expect("Failed to publish event");
    event_source
        .publish(event.clone())
        .await
        .expect("Failed to publish event second time");

    // Verify only one event exists
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM workflow_events WHERE workflow_id = $1 AND event_type = $2",
    )
    .bind(workflow_id)
    .bind(WorkflowEventType::ActivityCompleted)
    .fetch_one(&pool)
    .await
    .expect("Failed to count events");

    assert_eq!(count, 1, "Idempotency violated - duplicate event created");
}

#[sqlx::test(migrations = "../migrations")]
async fn test_poll_no_events(pool: PgPool) {
    let event_source = PostgresEventSource::new(pool.clone());

    // Poll with no events
    let events = event_source
        .poll("test_consumer")
        .await
        .expect("Failed to poll");

    assert!(events.is_empty());
}

#[sqlx::test(migrations = "../migrations")]
async fn test_poll_with_events(pool: PgPool) {
    let event_source = PostgresEventSource::new(pool.clone());
    let workflow_id = Uuid::now_v7();

    // Publish two events
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::WorkflowCreated,
            activity_key: None,
            payload: json!({}),
            iteration: None,
        })
        .await
        .expect("Failed to publish event 1");

    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::ActivityScheduled,
            activity_key: Some("activity1".to_string()),
            payload: json!({}),
            iteration: None,
        })
        .await
        .expect("Failed to publish event 2");

    // Poll for events
    let events = event_source
        .poll("test_consumer")
        .await
        .expect("Failed to poll");

    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_type, WorkflowEventType::WorkflowCreated);
    assert_eq!(events[1].event_type, WorkflowEventType::ActivityScheduled);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_position_tracking(pool: PgPool) {
    let event_source = PostgresEventSource::new(pool.clone());
    let workflow_id = Uuid::now_v7();

    // Publish three events
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::WorkflowCreated,
            activity_key: None,
            payload: json!({}),
            iteration: None,
        })
        .await
        .expect("Failed to publish event 1");

    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::ActivityScheduled,
            activity_key: Some("activity1".to_string()),
            payload: json!({}),
            iteration: None,
        })
        .await
        .expect("Failed to publish event 2");

    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::ActivityCompleted,
            activity_key: Some("activity1".to_string()),
            payload: json!({}),
            iteration: None,
        })
        .await
        .expect("Failed to publish event 3");

    // First poll - get all events
    let events = event_source
        .poll("test_consumer")
        .await
        .expect("Failed to poll");
    assert_eq!(events.len(), 3);

    // Update position to first event
    event_source
        .update_position("test_consumer", events[0].id)
        .await
        .expect("Failed to update position");

    // Second poll - should get remaining events
    let events = event_source
        .poll("test_consumer")
        .await
        .expect("Failed to poll");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_type, WorkflowEventType::ActivityScheduled);

    // Update position to second event
    event_source
        .update_position("test_consumer", events[0].id)
        .await
        .expect("Failed to update position");

    // Third poll - should get last event
    let events = event_source
        .poll("test_consumer")
        .await
        .expect("Failed to poll");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, WorkflowEventType::ActivityCompleted);

    // Update position to last event
    event_source
        .update_position("test_consumer", events[0].id)
        .await
        .expect("Failed to update position");

    // Fourth poll - should get no events
    let events = event_source
        .poll("test_consumer")
        .await
        .expect("Failed to poll");
    assert_eq!(events.len(), 0);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_multiple_consumers(pool: PgPool) {
    let event_source = PostgresEventSource::new(pool.clone());
    let workflow_id = Uuid::now_v7();

    // Publish events
    event_source
        .publish(NewWorkflowEvent {
            workflow_id,
            event_type: WorkflowEventType::WorkflowCreated,
            activity_key: None,
            payload: json!({}),
            iteration: None,
        })
        .await
        .expect("Failed to publish event");

    // Consumer 1 polls
    let events1 = event_source
        .poll("consumer1")
        .await
        .expect("Failed to poll");
    assert_eq!(events1.len(), 1);

    // Consumer 2 polls (should get same event)
    let events2 = event_source
        .poll("consumer2")
        .await
        .expect("Failed to poll");
    assert_eq!(events2.len(), 1);

    // Consumer 1 updates position
    event_source
        .update_position("consumer1", events1[0].id)
        .await
        .expect("Failed to update position");

    // Consumer 1 polls again (should get nothing)
    let events1 = event_source
        .poll("consumer1")
        .await
        .expect("Failed to poll");
    assert_eq!(events1.len(), 0);

    // Consumer 2 polls again (should still get event since it hasn't updated position)
    let events2 = event_source
        .poll("consumer2")
        .await
        .expect("Failed to poll");
    assert_eq!(events2.len(), 1);
}
