use serde_json::json;
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use streamflow_core::queue::{
    Activity, ActivityQueue, ActivityResult, ActivitySettings, PostgresQueue, QueueConfig,
    QueueMonitor,
};
use streamflow_core::workflow::{BackoffStrategy, RetryPolicy};
use tokio::time::{Duration, sleep};
use uuid::Uuid;

/// Helper to create test database pool
async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://streamflow:streamflow_dev@127.0.0.1:5433/streamflow".to_string()
    });

    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    // Run migrations from workspace root
    sqlx::migrate!("../migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    // Truncate activity_queue table to ensure clean state for tests
    // Safe because tests run serially with #[serial]
    sqlx::query!("TRUNCATE TABLE activity_queue")
        .execute(&pool)
        .await
        .expect("Failed to truncate activity_queue");

    pool
}

/// Helper to clean up test data
async fn cleanup_queue(pool: &PgPool, workflow_id: Uuid) {
    sqlx::query!(
        "DELETE FROM activity_queue WHERE workflow_id = $1",
        workflow_id
    )
    .execute(pool)
    .await
    .expect("Failed to cleanup test data");
}

#[tokio::test]
#[serial]
async fn test_idempotent_scheduling() {
    let pool = setup_test_pool().await;
    let config = QueueConfig::default();
    let queue = PostgresQueue::new(pool.clone(), config);
    let workflow_id = Uuid::now_v7();

    let activity = Activity {
        key: "test_activity".to_string(),
        worker: "test".to_string(),
        activity_name: "test_task".to_string(),
        parameters: json!({"key": "value"}),
        settings: None,
        scheduled_for: None,
        output_definitions: None,
    };

    // Schedule activity first time
    queue
        .schedule(workflow_id, vec![activity.clone()])
        .await
        .expect("First schedule should succeed");

    // Schedule same activity again
    queue
        .schedule(workflow_id, vec![activity.clone()])
        .await
        .expect("Second schedule should succeed (idempotent)");

    // Verify only one row in database
    let count = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM activity_queue WHERE workflow_id = $1",
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to count rows");

    assert_eq!(count, Some(1), "Should have exactly one activity in queue");

    cleanup_queue(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_concurrent_claiming() {
    let pool = setup_test_pool().await;
    let config = QueueConfig::default();
    let workflow_id = Uuid::now_v7();

    // Schedule 3 activities
    let queue = PostgresQueue::new(pool.clone(), config.clone());
    let activities: Vec<Activity> = (0..3)
        .map(|i| Activity {
            key: format!("activity_{}", i),
            worker: "test".to_string(),
            activity_name: "test_task".to_string(),
            parameters: json!({"index": i}),
            settings: None,
            scheduled_for: None,
            output_definitions: None,
        })
        .collect();

    queue
        .schedule(workflow_id, activities)
        .await
        .expect("Failed to schedule activities");

    // Simulate 3 workers claiming concurrently
    let worker1_id = "worker_test_01";
    let worker2_id = "worker_test_02";
    let worker3_id = "worker_test_03";

    let queue1 = PostgresQueue::new(pool.clone(), config.clone());
    let queue2 = PostgresQueue::new(pool.clone(), config.clone());
    let queue3 = PostgresQueue::new(pool.clone(), config);

    let (claimed1, claimed2, claimed3) = tokio::join!(
        queue1.claim_next(worker1_id, "test", "test_task"),
        queue2.claim_next(worker2_id, "test", "test_task"),
        queue3.claim_next(worker3_id, "test", "test_task"),
    );

    let claimed1 = claimed1.expect("Worker 1 claim failed");
    let claimed2 = claimed2.expect("Worker 2 claim failed");
    let claimed3 = claimed3.expect("Worker 3 claim failed");

    // All should succeed
    assert!(claimed1.is_some(), "Worker 1 should claim an activity");
    assert!(claimed2.is_some(), "Worker 2 should claim an activity");
    assert!(claimed3.is_some(), "Worker 3 should claim an activity");

    // All should be different activities
    let id1 = claimed1.unwrap().id;
    let id2 = claimed2.unwrap().id;
    let id3 = claimed3.unwrap().id;

    assert_ne!(id1, id2, "Workers should claim different activities");
    assert_ne!(id1, id3, "Workers should claim different activities");
    assert_ne!(id2, id3, "Workers should claim different activities");

    cleanup_queue(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_stale_activity_recovery() {
    let pool = setup_test_pool().await;

    // Clean all test data before starting
    sqlx::query!("TRUNCATE activity_queue CASCADE")
        .execute(&pool)
        .await
        .expect("Failed to clean test data");

    let mut config = QueueConfig::default();
    config.default_timeout = Duration::from_secs(1); // Very short timeout for testing

    let queue = PostgresQueue::new(pool.clone(), config);
    let workflow_id = Uuid::now_v7();
    let worker1_id = "worker_test_01";
    let worker2_id = "worker_test_02";

    // Schedule activity with short timeout
    let activity = Activity {
        key: "test_activity".to_string(),
        worker: "test".to_string(),
        activity_name: "test_task".to_string(),
        parameters: json!({"key": "value"}),
        settings: Some(ActivitySettings {
            timeout_seconds: Some(1), // 1 second timeout
            retry: Some(RetryPolicy {
                max_attempts: 3,
                strategy: BackoffStrategy::Fixed,
                base_seconds: 2,
                factor: 2.0,
                max_seconds: 300,
            }),
            budget: None,
            cache: false,
            cache_ttl: None,
        }),
        scheduled_for: None,
        output_definitions: None,
    };

    queue
        .schedule(workflow_id, vec![activity])
        .await
        .expect("Failed to schedule activity");

    // Worker 1 claims activity
    let claimed1 = queue
        .claim_next(worker1_id, "test", "test_task")
        .await
        .expect("Failed to claim activity")
        .expect("Should have claimed activity");

    assert_eq!(
        claimed1.retry_count, 0,
        "First claim should have retry_count = 0"
    );

    // Wait for timeout to expire (1 second + buffer)
    sleep(Duration::from_millis(1200)).await;

    // Worker 2 claims activity (should reclaim stale activity)
    let claimed2 = queue
        .claim_next(worker2_id, "test", "test_task")
        .await
        .expect("Failed to reclaim activity")
        .expect("Should have reclaimed stale activity");

    assert_eq!(claimed1.id, claimed2.id, "Should reclaim same activity");
    assert_eq!(
        claimed2.retry_count, 1,
        "Reclaimed activity should have retry_count = 1"
    );

    cleanup_queue(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_heartbeat_conflict_detection() {
    let pool = setup_test_pool().await;
    let mut config = QueueConfig::default();
    config.default_timeout = Duration::from_secs(1);

    let queue = PostgresQueue::new(pool.clone(), config);
    let workflow_id = Uuid::now_v7();
    let worker1_id = "worker_test_01";
    let worker2_id = "worker_test_02";

    // Schedule activity
    let activity = Activity {
        key: "test_activity".to_string(),
        worker: "test".to_string(),
        activity_name: "test_task".to_string(),
        parameters: json!({"key": "value"}),
        settings: Some(ActivitySettings {
            timeout_seconds: Some(1),
            retry: Some(RetryPolicy {
                max_attempts: 3,
                strategy: BackoffStrategy::Fixed,
                base_seconds: 2,
                factor: 2.0,
                max_seconds: 300,
            }),
            budget: None,
            cache: false,
            cache_ttl: None,
        }),
        scheduled_for: None,
        output_definitions: None,
    };

    queue
        .schedule(workflow_id, vec![activity])
        .await
        .expect("Failed to schedule activity");

    // Worker 1 claims activity
    let claimed = queue
        .claim_next(worker1_id, "test", "test_task")
        .await
        .expect("Failed to claim activity")
        .expect("Should have claimed activity");

    // Wait for timeout
    sleep(Duration::from_millis(1200)).await;

    // Worker 2 reclaims stale activity
    let reclaimed = queue
        .claim_next(worker2_id, "test", "test_task")
        .await
        .expect("Failed to reclaim activity")
        .expect("Should have reclaimed activity");

    assert_eq!(claimed.id, reclaimed.id);

    // Worker 1 tries to send heartbeat - should get conflict
    let heartbeat_result = queue.heartbeat(claimed.id, worker1_id).await;

    assert!(
        heartbeat_result.is_err(),
        "Heartbeat from original worker should fail"
    );

    match heartbeat_result {
        Err(streamflow_core::queue::QueueError::ActivityReclaimed) => {
            // Expected error
        }
        _ => panic!("Expected ActivityReclaimed error"),
    }

    cleanup_queue(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_max_retries_exhaustion() {
    let pool = setup_test_pool().await;
    let mut config = QueueConfig::default();
    config.default_timeout = Duration::from_secs(1);

    let queue = PostgresQueue::new(pool.clone(), config.clone());
    let workflow_id = Uuid::now_v7();

    // Schedule activity with max_attempts = 2
    // This allows: initial claim (retry_count 0) + 1 retry (retry_count 1) = 2 total claims
    // Then retry_count becomes 2, and 2 < 2 is false, so no more claims
    let activity = Activity {
        key: "test_activity".to_string(),
        worker: "test".to_string(),
        activity_name: "test_task".to_string(),
        parameters: json!({"key": "value"}),
        settings: Some(ActivitySettings {
            timeout_seconds: Some(1), // Very short timeout
            retry: Some(RetryPolicy {
                max_attempts: 2, // 2 allows: initial + 1 retry = 2 claims, then retry_count=2 blocks further claims
                strategy: BackoffStrategy::Fixed,
                base_seconds: 2,
                factor: 2.0,
                max_seconds: 300,
            }),
            budget: None,
            cache: false,
            cache_ttl: None,
        }),
        scheduled_for: None,
        output_definitions: None,
    };

    queue
        .schedule(workflow_id, vec![activity])
        .await
        .expect("Failed to schedule activity");

    // Claim and timeout: initial claim + max_retries attempts
    // With max_retries=2, we should be able to claim 3 times total (initial + 2 retries)
    for i in 0..3 {
        let worker_id = format!("worker_test_{:02}", i);
        let claimed = queue
            .claim_next(&worker_id, "test", "test_task")
            .await
            .expect("Failed to claim activity")
            .expect("Should have claimed activity");

        // First claim has retry_count=0, subsequent reclaims increment it
        let expected_retry_count = if i == 0 { 0 } else { i };
        assert_eq!(
            claimed.retry_count, expected_retry_count,
            "Retry count should match: iteration={}, expected={}",
            i, expected_retry_count
        );

        // Wait for timeout (timeout_seconds is 1, so wait longer)
        sleep(Duration::from_millis(1200)).await;
    }

    // Try to claim again - should not return activity (retry_count >= max_retries)
    let worker_id = "worker_test_final";
    let no_claim = queue
        .claim_next(worker_id, "test", "test_task")
        .await
        .expect("Claim call should succeed");

    assert!(
        no_claim.is_none(),
        "Should not claim activity after max retries exhausted"
    );

    // Verify activity still exists in queue (cleanup hasn't run yet)
    let count = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM activity_queue WHERE workflow_id = $1",
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to count");

    assert_eq!(
        count,
        Some(1),
        "Activity should still be in queue before cleanup"
    );

    // Cleanup will be handled by the monitor background thread in production
    // For testing, we verify the activity is in the correct state
    let _monitor = Arc::new(QueueMonitor::new(pool.clone(), config));

    // Access cleanup via a test-only method would be ideal, but for now we'll just verify
    // the activity is in a failed state by checking retry_count >= max_retries
    let failed_activity = sqlx::query!(
        r#"
        SELECT id, retry_count, max_retries
        FROM activity_queue
        WHERE workflow_id = $1
        "#,
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to fetch activity");

    assert!(
        failed_activity.retry_count >= failed_activity.max_retries,
        "Activity should have exhausted retries"
    );

    cleanup_queue(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_completion_idempotency() {
    let pool = setup_test_pool().await;
    let config = QueueConfig::default();
    let queue = PostgresQueue::new(pool.clone(), config);
    let workflow_id = Uuid::now_v7();
    let worker_id = "worker_test_01";

    // Schedule activity
    let activity = Activity {
        key: "test_activity".to_string(),
        worker: "test".to_string(),
        activity_name: "test_task".to_string(),
        parameters: json!({"key": "value"}),
        settings: None,
        scheduled_for: None,
        output_definitions: None,
    };

    queue
        .schedule(workflow_id, vec![activity])
        .await
        .expect("Failed to schedule activity");

    // Claim activity
    let claimed = queue
        .claim_next(worker_id, "test", "test_task")
        .await
        .expect("Failed to claim activity")
        .expect("Should have claimed activity");

    let result = ActivityResult {
        success: true,
        outputs: Some(vec![streamflow_core::workflow::ActivityOutput {
            name: "result".to_string(),
            output_type: streamflow_core::workflow::OutputType::Value,
            value: json!("success"),
        }]),
        error: None,
        cost_usd: None,
        token_usage: None,
    };

    // Complete activity first time
    queue
        .complete(claimed.id, worker_id, result.clone())
        .await
        .expect("First completion should succeed");

    // Complete activity second time (should succeed idempotently)
    let second_result = queue.complete(claimed.id, worker_id, result).await;

    assert!(
        second_result.is_ok(),
        "Second completion should succeed idempotently"
    );

    // Verify activity is marked as completed (soft-delete)
    let status = sqlx::query_scalar!(
        r#"SELECT status::text FROM activity_queue WHERE id = $1"#,
        claimed.id
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to get status");

    assert_eq!(
        status,
        Some("completed".to_string()),
        "Activity should be marked as completed"
    );

    cleanup_queue(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_sequential_ordering() {
    let pool = setup_test_pool().await;
    let config = QueueConfig::default();
    let queue = PostgresQueue::new(pool.clone(), config);
    let workflow_id = Uuid::now_v7();
    let worker_id = "worker_test_01";

    // Schedule first activity
    let activity1 = Activity {
        key: "step1".to_string(),
        worker: "test".to_string(),
        activity_name: "test_task".to_string(),
        parameters: json!({"step": 1}),
        settings: None,
        scheduled_for: None,
        output_definitions: None,
    };

    queue
        .schedule(workflow_id, vec![activity1])
        .await
        .expect("Failed to schedule activity 1");

    // Claim and complete step 1
    let claimed1 = queue
        .claim_next(worker_id, "test", "test_task")
        .await
        .expect("Failed to claim")
        .expect("Should claim step1");

    assert_eq!(claimed1.activity_key, "step1");

    queue
        .complete(
            claimed1.id,
            worker_id,
            ActivityResult {
                success: true,
                outputs: None,
                error: None,
                cost_usd: None,
                token_usage: None,
            },
        )
        .await
        .expect("Failed to complete step1");

    // Now schedule step 2 (simulating orchestrator behavior)
    let activity2 = Activity {
        key: "step2".to_string(),
        worker: "test".to_string(),
        activity_name: "test_task".to_string(),
        parameters: json!({"step": 2}),
        settings: None,
        scheduled_for: None,
        output_definitions: None,
    };

    queue
        .schedule(workflow_id, vec![activity2])
        .await
        .expect("Failed to schedule activity 2");

    // Claim step 2
    let claimed2 = queue
        .claim_next(worker_id, "test", "test_task")
        .await
        .expect("Failed to claim")
        .expect("Should claim step2");

    assert_eq!(claimed2.activity_key, "step2");

    cleanup_queue(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_parallel_execution() {
    let pool = setup_test_pool().await;
    let config = QueueConfig::default();
    let queue = PostgresQueue::new(pool.clone(), config);
    let workflow_id = Uuid::now_v7();

    // Schedule 3 activities simultaneously (simulating orchestrator fan-out)
    let activities: Vec<Activity> = (1..=3)
        .map(|i| Activity {
            key: format!("parallel_{}", i),
            worker: "test".to_string(),
            activity_name: "test_task".to_string(),
            parameters: json!({"index": i}),
            settings: None,
            scheduled_for: None,
            output_definitions: None,
        })
        .collect();

    queue
        .schedule(workflow_id, activities)
        .await
        .expect("Failed to schedule parallel activities");

    // Verify all 3 are in queue
    let count = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM activity_queue WHERE workflow_id = $1",
        workflow_id
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to count");

    assert_eq!(count, Some(3), "All 3 activities should be in queue");

    // Claim all 3 in parallel with different workers
    let worker1_id = "worker_test_01";
    let worker2_id = "worker_test_02";
    let worker3_id = "worker_test_03";

    let queue1 = PostgresQueue::new(pool.clone(), QueueConfig::default());
    let queue2 = PostgresQueue::new(pool.clone(), QueueConfig::default());
    let queue3 = PostgresQueue::new(pool.clone(), QueueConfig::default());

    let (claimed1, claimed2, claimed3) = tokio::join!(
        queue1.claim_next(worker1_id, "test", "test_task"),
        queue2.claim_next(worker2_id, "test", "test_task"),
        queue3.claim_next(worker3_id, "test", "test_task"),
    );

    assert!(
        claimed1.unwrap().is_some(),
        "Worker 1 should claim activity"
    );
    assert!(
        claimed2.unwrap().is_some(),
        "Worker 2 should claim activity"
    );
    assert!(
        claimed3.unwrap().is_some(),
        "Worker 3 should claim activity"
    );

    cleanup_queue(&pool, workflow_id).await;
}
