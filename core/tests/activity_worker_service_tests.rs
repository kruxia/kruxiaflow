use kruxiaflow_core::activity::ActivityWorkerService;
use kruxiaflow_core::events::{EventSource, PostgresEventSource};
use kruxiaflow_core::queue::{Activity, ActivityQueue, PostgresQueue, QueueConfig};
use rust_decimal::Decimal;
use serde_json::json;
use sqlx::PgPool;
use std::str::FromStr;
use std::sync::Arc;
use uuid::Uuid;

/// Helper to schedule a test activity
async fn schedule_test_activity(
    pool: &PgPool,
    workflow_id: Uuid,
    activity_key: &str,
    worker: &str,
    activity_name: &str,
) {
    let queue = PostgresQueue::new(pool.clone(), QueueConfig::default());
    let activity = Activity {
        key: activity_key.to_string(),
        worker: worker.to_string(),
        activity_name: activity_name.to_string(),
        parameters: json!({"test": "data"}),
        settings: None,
        scheduled_for: None,
        output_definitions: None,
        iteration: None,
        signal_data: None,
    };

    queue
        .schedule(workflow_id, vec![activity])
        .await
        .expect("Failed to schedule test activity");
}

#[sqlx::test(migrations = "../migrations")]
async fn test_poll_activities_success(pool: PgPool) {
    let queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()))
        as Arc<dyn ActivityQueue>;
    let event_source = Arc::new(PostgresEventSource::new(pool.clone())) as Arc<dyn EventSource>;
    let service = ActivityWorkerService::new(queue, event_source);
    let workflow_id = Uuid::now_v7();

    // Schedule test activities
    schedule_test_activity(&pool, workflow_id, "activity_1", "payments", "authorize").await;
    schedule_test_activity(&pool, workflow_id, "activity_2", "payments", "capture").await;

    // Poll for activities
    let result = service.poll_activities("payments", "worker_01", 10).await;

    assert!(result.is_ok());
    let activities = result.unwrap();
    assert_eq!(activities.len(), 2);
    assert_eq!(activities[0].worker, "payments");
}

#[sqlx::test(migrations = "../migrations")]
async fn test_poll_activities_empty(pool: PgPool) {
    let queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()))
        as Arc<dyn ActivityQueue>;
    let event_source = Arc::new(PostgresEventSource::new(pool.clone())) as Arc<dyn EventSource>;
    let service = ActivityWorkerService::new(queue, event_source);

    // Poll for non-existent worker type
    let result = service
        .poll_activities("nonexistent", "worker_01", 10)
        .await;

    assert!(result.is_ok());
    let activities = result.unwrap();
    assert_eq!(activities.len(), 0);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_poll_activities_concurrent_workers(pool: PgPool) {
    let queue1 = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()))
        as Arc<dyn ActivityQueue>;
    let event_source1 = Arc::new(PostgresEventSource::new(pool.clone())) as Arc<dyn EventSource>;
    let service1 = ActivityWorkerService::new(queue1, event_source1);
    let queue2 = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()))
        as Arc<dyn ActivityQueue>;
    let event_source2 = Arc::new(PostgresEventSource::new(pool.clone())) as Arc<dyn EventSource>;
    let service2 = ActivityWorkerService::new(queue2, event_source2);
    let workflow_id = Uuid::now_v7();

    // Schedule 10 activities
    for i in 0..10 {
        schedule_test_activity(
            &pool,
            workflow_id,
            &format!("activity_{}", i),
            "payments",
            "authorize",
        )
        .await;
    }

    // Two workers poll concurrently
    let (result1, result2) = tokio::join!(
        service1.poll_activities("payments", "worker_01", 10),
        service2.poll_activities("payments", "worker_02", 10)
    );

    assert!(result1.is_ok());
    assert!(result2.is_ok());

    let activities1 = result1.unwrap();
    let activities2 = result2.unwrap();
    let total = activities1.len() + activities2.len();
    assert_eq!(total, 10);

    // Verify no duplicate IDs (FOR UPDATE SKIP LOCKED working)
    let mut ids = activities1.iter().map(|a| a.id).collect::<Vec<_>>();
    ids.extend(activities2.iter().map(|a| a.id));
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 10);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_poll_activities_max_limit(pool: PgPool) {
    let queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()))
        as Arc<dyn ActivityQueue>;
    let event_source = Arc::new(PostgresEventSource::new(pool.clone())) as Arc<dyn EventSource>;
    let service = ActivityWorkerService::new(queue, event_source);
    let workflow_id = Uuid::now_v7();

    // Schedule 10 activities
    for i in 0..10 {
        schedule_test_activity(
            &pool,
            workflow_id,
            &format!("activity_{}", i),
            "payments",
            "authorize",
        )
        .await;
    }

    // Poll with max_activities = 5
    let result = service.poll_activities("payments", "worker_01", 5).await;

    assert!(result.is_ok());
    let activities = result.unwrap();
    assert_eq!(activities.len(), 5);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_heartbeat_activity_success(pool: PgPool) {
    let queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()))
        as Arc<dyn ActivityQueue>;
    let event_source = Arc::new(PostgresEventSource::new(pool.clone())) as Arc<dyn EventSource>;
    let service = ActivityWorkerService::new(queue, event_source);
    let workflow_id = Uuid::now_v7();

    // Schedule and claim an activity
    schedule_test_activity(&pool, workflow_id, "activity_1", "payments", "authorize").await;

    let activities = service
        .poll_activities("payments", "worker_01", 1)
        .await
        .unwrap();

    let activity_id = activities[0].id;

    // Send heartbeat
    let result = service
        .heartbeat_activity(activity_id, "worker_01".to_string())
        .await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 30);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_heartbeat_wrong_worker(pool: PgPool) {
    let queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()))
        as Arc<dyn ActivityQueue>;
    let event_source = Arc::new(PostgresEventSource::new(pool.clone())) as Arc<dyn EventSource>;
    let service = ActivityWorkerService::new(queue, event_source);
    let workflow_id = Uuid::now_v7();

    // Schedule and claim with worker_01
    schedule_test_activity(&pool, workflow_id, "activity_1", "payments", "authorize").await;

    let activities = service
        .poll_activities("payments", "worker_01", 1)
        .await
        .unwrap();

    let activity_id = activities[0].id;

    // Try to send heartbeat from worker_02
    let result = service
        .heartbeat_activity(activity_id, "worker_02".to_string())
        .await;

    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        kruxiaflow_core::activity::ActivityWorkerError::WrongWorker
    ));
}

#[sqlx::test(migrations = "../migrations")]
async fn test_heartbeat_activity_not_found(pool: PgPool) {
    let queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()))
        as Arc<dyn ActivityQueue>;
    let event_source = Arc::new(PostgresEventSource::new(pool.clone())) as Arc<dyn EventSource>;
    let service = ActivityWorkerService::new(queue, event_source);

    let result = service
        .heartbeat_activity(Uuid::now_v7(), "worker_01".to_string())
        .await;

    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        kruxiaflow_core::activity::ActivityWorkerError::ActivityNotFound(_)
    ));
}

#[sqlx::test(migrations = "../migrations")]
async fn test_complete_activity_success(pool: PgPool) {
    let queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()))
        as Arc<dyn ActivityQueue>;
    let event_source = Arc::new(PostgresEventSource::new(pool.clone())) as Arc<dyn EventSource>;
    let service = ActivityWorkerService::new(queue, event_source);
    let workflow_id = Uuid::now_v7();

    // Schedule and claim an activity
    schedule_test_activity(&pool, workflow_id, "activity_1", "payments", "authorize").await;

    let activities = service
        .poll_activities("payments", "worker_01", 1)
        .await
        .unwrap();

    let activity_id = activities[0].id;

    // Complete the activity
    let output = json!({"result": "success", "transaction_id": "txn_123"});
    let result = service
        .complete_activity(
            activity_id,
            "worker_01".to_string(),
            output.clone(),
            Some(Decimal::from_str("0.05").unwrap()),
            None,
        )
        .await;

    assert!(result.is_ok());

    // Verify activity is marked as completed (soft-delete)
    let status = sqlx::query_scalar!(
        r#"SELECT status::text FROM activity_queue WHERE id = $1"#,
        activity_id
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(status, Some("completed".to_string()));

    // Verify event was published
    let event = sqlx::query!(
        r#"
        SELECT event_type AS "event_type: String", payload
        FROM workflow_events
        WHERE workflow_id = $1
        ORDER BY timestamp DESC
        LIMIT 1
        "#,
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(event.event_type, "ActivityCompleted");
    assert_eq!(event.payload["outputs"], output);
    assert_eq!(event.payload["cost_usd"], "0.05");
}

#[sqlx::test(migrations = "../migrations")]
async fn test_complete_activity_idempotency(pool: PgPool) {
    let queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()))
        as Arc<dyn ActivityQueue>;
    let event_source = Arc::new(PostgresEventSource::new(pool.clone())) as Arc<dyn EventSource>;
    let service = ActivityWorkerService::new(queue, event_source);
    let workflow_id = Uuid::now_v7();

    // Schedule and claim an activity
    schedule_test_activity(&pool, workflow_id, "activity_1", "payments", "authorize").await;

    let activities = service
        .poll_activities("payments", "worker_01", 1)
        .await
        .unwrap();

    let activity_id = activities[0].id;
    let output = json!({"result": "success"});

    // Complete the activity first time
    service
        .complete_activity(activity_id, "worker_01".to_string(), output.clone(), None, None)
        .await
        .unwrap();

    // Try to complete again (should succeed idempotently)
    let result = service
        .complete_activity(activity_id, "worker_01".to_string(), output, None, None)
        .await;

    assert!(result.is_ok());
}

#[sqlx::test(migrations = "../migrations")]
async fn test_complete_activity_wrong_worker(pool: PgPool) {
    let queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()))
        as Arc<dyn ActivityQueue>;
    let event_source = Arc::new(PostgresEventSource::new(pool.clone())) as Arc<dyn EventSource>;
    let service = ActivityWorkerService::new(queue, event_source);
    let workflow_id = Uuid::now_v7();

    // Schedule and claim with worker_01
    schedule_test_activity(&pool, workflow_id, "activity_1", "payments", "authorize").await;

    let activities = service
        .poll_activities("payments", "worker_01", 1)
        .await
        .unwrap();

    let activity_id = activities[0].id;

    // Try to complete from worker_02
    let output = json!({"result": "success"});
    let result = service
        .complete_activity(activity_id, "worker_02".to_string(), output, None, None)
        .await;

    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        kruxiaflow_core::activity::ActivityWorkerError::WrongWorker
    ));
}

#[sqlx::test(migrations = "../migrations")]
async fn test_fail_activity_success(pool: PgPool) {
    let queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()))
        as Arc<dyn ActivityQueue>;
    let event_source = Arc::new(PostgresEventSource::new(pool.clone())) as Arc<dyn EventSource>;
    let service = ActivityWorkerService::new(queue, event_source);
    let workflow_id = Uuid::now_v7();

    // Schedule and claim an activity
    schedule_test_activity(&pool, workflow_id, "activity_1", "payments", "authorize").await;

    let activities = service
        .poll_activities("payments", "worker_01", 1)
        .await
        .unwrap();

    let activity_id = activities[0].id;

    // Fail the activity
    let result = service
        .fail_activity(
            activity_id,
            "worker_01".to_string(),
            "PAYMENT_DECLINED".to_string(),
            "Card was declined by the bank".to_string(),
            false,
            None,
            None,
        )
        .await;

    assert!(result.is_ok());
    assert!(!result.unwrap()); // will_retry = false

    // Verify activity is marked as failed (soft-delete)
    let status = sqlx::query_scalar!(
        r#"SELECT status::text FROM activity_queue WHERE id = $1"#,
        activity_id
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(status, Some("failed".to_string()));

    // Verify event was published
    let event = sqlx::query!(
        r#"
        SELECT event_type AS "event_type: String", payload
        FROM workflow_events
        WHERE workflow_id = $1
        ORDER BY timestamp DESC
        LIMIT 1
        "#,
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(event.event_type, "ActivityFailed");
    assert_eq!(event.payload["error_code"], "PAYMENT_DECLINED");
    assert_eq!(event.payload["retryable"], false);
    assert_eq!(event.payload["will_retry"], false);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_fail_activity_wrong_worker(pool: PgPool) {
    let queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()))
        as Arc<dyn ActivityQueue>;
    let event_source = Arc::new(PostgresEventSource::new(pool.clone())) as Arc<dyn EventSource>;
    let service = ActivityWorkerService::new(queue, event_source);
    let workflow_id = Uuid::now_v7();

    // Schedule and claim with worker_01
    schedule_test_activity(&pool, workflow_id, "activity_1", "payments", "authorize").await;

    let activities = service
        .poll_activities("payments", "worker_01", 1)
        .await
        .unwrap();

    let activity_id = activities[0].id;

    // Try to fail from worker_02
    let result = service
        .fail_activity(
            activity_id,
            "worker_02".to_string(),
            "ERROR".to_string(),
            "Error message".to_string(),
            false,
            None,
            None,
        )
        .await;

    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        kruxiaflow_core::activity::ActivityWorkerError::WrongWorker
    ));
}

#[sqlx::test(migrations = "../migrations")]
async fn test_fail_activity_not_found(pool: PgPool) {
    let queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()))
        as Arc<dyn ActivityQueue>;
    let event_source = Arc::new(PostgresEventSource::new(pool.clone())) as Arc<dyn EventSource>;
    let service = ActivityWorkerService::new(queue, event_source);

    let result = service
        .fail_activity(
            Uuid::now_v7(),
            "worker_01".to_string(),
            "ERROR".to_string(),
            "Error message".to_string(),
            false,
            None,
            None,
        )
        .await;

    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        kruxiaflow_core::activity::ActivityWorkerError::ActivityNotFound(_)
    ));
}
