use kruxiaflow_core::queue::{Activity, ActivityQueue, PostgresQueue, QueueConfig};
use serde_json::json;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

/// Helper to create test database pool
async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://kruxiaflow:kruxiaflow_dev@127.0.0.1:5433/kruxiaflow".to_string()
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

// ===========================================================================
// Worker-level claiming tests
// Tests for the worker-level filtering optimization
// ===========================================================================

#[tokio::test]
#[serial]
async fn test_claim_multiple_activities_single_worker() {
    let pool = setup_test_pool().await;
    let config = QueueConfig::default();
    let queue = PostgresQueue::new(pool.clone(), config);
    let workflow_id = Uuid::now_v7();

    // Schedule 5 activities of the same worker type
    let activities: Vec<Activity> = (0..5)
        .map(|i| Activity {
            key: format!("activity_{}", i),
            worker: "test".to_string(),
            activity_name: "test_task".to_string(),
            parameters: json!({"index": i}),
            settings: None,
            scheduled_for: None,
            output_definitions: None,
            iteration: None,
            signal_data: None,
        })
        .collect();

    queue
        .schedule(workflow_id, activities)
        .await
        .expect("Failed to schedule activities");

    // Claim up to 3 activities in a single call
    let claimed = queue
        .claim_next("worker_test_01", "test", 3)
        .await
        .expect("Failed to claim activities");

    assert_eq!(
        claimed.len(),
        3,
        "Should claim exactly 3 activities in single call"
    );

    // Verify all claimed activities are different
    let ids: Vec<_> = claimed.iter().map(|a| a.id).collect();
    assert_eq!(
        ids.len(),
        ids.iter().collect::<std::collections::HashSet<_>>().len(),
        "All claimed activities should be unique"
    );

    cleanup_queue(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_claim_multiple_activity_types_for_same_worker() {
    let pool = setup_test_pool().await;
    let config = QueueConfig::default();
    let queue = PostgresQueue::new(pool.clone(), config);
    let workflow_id = Uuid::now_v7();

    // Schedule activities of different types but same worker
    let activities = vec![
        Activity {
            key: "http_1".to_string(),
            worker: "std".to_string(),
            activity_name: "http_request".to_string(),
            parameters: json!({"url": "http://example.com/1"}),
            settings: None,
            scheduled_for: None,
            output_definitions: None,
            iteration: None,
            signal_data: None,
        },
        Activity {
            key: "http_2".to_string(),
            worker: "std".to_string(),
            activity_name: "http_request".to_string(),
            parameters: json!({"url": "http://example.com/2"}),
            settings: None,
            scheduled_for: None,
            output_definitions: None,
            iteration: None,
            signal_data: None,
        },
        Activity {
            key: "db_1".to_string(),
            worker: "std".to_string(),
            activity_name: "postgres_query".to_string(),
            parameters: json!({"query": "SELECT 1"}),
            settings: None,
            scheduled_for: None,
            output_definitions: None,
            iteration: None,
            signal_data: None,
        },
        Activity {
            key: "echo_1".to_string(),
            worker: "std".to_string(),
            activity_name: "echo".to_string(),
            parameters: json!({"message": "test"}),
            settings: None,
            scheduled_for: None,
            output_definitions: None,
            iteration: None,
            signal_data: None,
        },
    ];

    queue
        .schedule(workflow_id, activities)
        .await
        .expect("Failed to schedule activities");

    // Claim up to 4 activities - should get all different types
    let claimed = queue
        .claim_next("worker_test_01", "std", 4)
        .await
        .expect("Failed to claim activities");

    assert_eq!(
        claimed.len(),
        4,
        "Should claim all 4 activities from same worker"
    );

    // Verify we got activities from different activity types
    let activity_names: Vec<_> = claimed.iter().map(|a| a.activity_name.as_str()).collect();

    assert!(
        activity_names.contains(&"http_request"),
        "Should claim http_request activity"
    );
    assert!(
        activity_names.contains(&"postgres_query"),
        "Should claim postgres_query activity"
    );
    assert!(
        activity_names.contains(&"echo"),
        "Should claim echo activity"
    );

    cleanup_queue(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_claim_respects_max_activities_limit() {
    let pool = setup_test_pool().await;
    let config = QueueConfig::default();
    let queue = PostgresQueue::new(pool.clone(), config);
    let workflow_id = Uuid::now_v7();

    // Schedule 10 activities
    let activities: Vec<Activity> = (0..10)
        .map(|i| Activity {
            key: format!("activity_{}", i),
            worker: "test".to_string(),
            activity_name: "test_task".to_string(),
            parameters: json!({"index": i}),
            settings: None,
            scheduled_for: None,
            output_definitions: None,
            iteration: None,
            signal_data: None,
        })
        .collect();

    queue
        .schedule(workflow_id, activities)
        .await
        .expect("Failed to schedule activities");

    // Claim with max_activities=5
    let claimed = queue
        .claim_next("worker_test_01", "test", 5)
        .await
        .expect("Failed to claim activities");

    assert_eq!(claimed.len(), 5, "Should respect max_activities limit");

    // Verify remaining activities can still be claimed
    let remaining = queue
        .claim_next("worker_test_02", "test", 10)
        .await
        .expect("Failed to claim remaining activities");

    assert_eq!(remaining.len(), 5, "Should claim remaining 5 activities");

    cleanup_queue(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_claim_only_returns_matching_worker() {
    let pool = setup_test_pool().await;
    let config = QueueConfig::default();
    let queue = PostgresQueue::new(pool.clone(), config);
    let workflow_id = Uuid::now_v7();

    // Schedule activities of different worker types
    let activities = vec![
        Activity {
            key: "std_1".to_string(),
            worker: "std".to_string(),
            activity_name: "echo".to_string(),
            parameters: json!({}),
            settings: None,
            scheduled_for: None,
            output_definitions: None,
            iteration: None,
            signal_data: None,
        },
        Activity {
            key: "std_2".to_string(),
            worker: "std".to_string(),
            activity_name: "echo".to_string(),
            parameters: json!({}),
            settings: None,
            scheduled_for: None,
            output_definitions: None,
            iteration: None,
            signal_data: None,
        },
        Activity {
            key: "custom_1".to_string(),
            worker: "custom".to_string(),
            activity_name: "process".to_string(),
            parameters: json!({}),
            settings: None,
            scheduled_for: None,
            output_definitions: None,
            iteration: None,
            signal_data: None,
        },
    ];

    queue
        .schedule(workflow_id, activities)
        .await
        .expect("Failed to schedule activities");

    // Claim only std activities
    let claimed = queue
        .claim_next("worker_test_01", "std", 10)
        .await
        .expect("Failed to claim activities");

    assert_eq!(claimed.len(), 2, "Should only claim std activities");

    for activity in &claimed {
        assert_eq!(
            activity.worker, "std",
            "All claimed activities should be std"
        );
    }

    // Claim custom activities
    let claimed_custom = queue
        .claim_next("worker_test_02", "custom", 10)
        .await
        .expect("Failed to claim activities");

    assert_eq!(claimed_custom.len(), 1, "Should claim custom activity");
    assert_eq!(claimed_custom[0].worker, "custom");

    cleanup_queue(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_claim_when_fewer_available_than_requested() {
    let pool = setup_test_pool().await;
    let config = QueueConfig::default();
    let queue = PostgresQueue::new(pool.clone(), config);
    let workflow_id = Uuid::now_v7();

    // Schedule only 2 activities
    let activities: Vec<Activity> = (0..2)
        .map(|i| Activity {
            key: format!("activity_{}", i),
            worker: "test".to_string(),
            activity_name: "test_task".to_string(),
            parameters: json!({"index": i}),
            settings: None,
            scheduled_for: None,
            output_definitions: None,
            iteration: None,
            signal_data: None,
        })
        .collect();

    queue
        .schedule(workflow_id, activities)
        .await
        .expect("Failed to schedule activities");

    // Request 10 but only 2 are available
    let claimed = queue
        .claim_next("worker_test_01", "test", 10)
        .await
        .expect("Failed to claim activities");

    assert_eq!(
        claimed.len(),
        2,
        "Should claim all available activities even if less than max_activities"
    );

    cleanup_queue(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_worker_level_claim_concurrent_workers() {
    let pool = setup_test_pool().await;
    let config = QueueConfig::default();
    let workflow_id = Uuid::now_v7();

    // Schedule 9 activities
    let queue = PostgresQueue::new(pool.clone(), config.clone());
    let activities: Vec<Activity> = (0..9)
        .map(|i| Activity {
            key: format!("activity_{}", i),
            worker: "test".to_string(),
            activity_name: "test_task".to_string(),
            parameters: json!({"index": i}),
            settings: None,
            scheduled_for: None,
            output_definitions: None,
            iteration: None,
            signal_data: None,
        })
        .collect();

    queue
        .schedule(workflow_id, activities)
        .await
        .expect("Failed to schedule activities");

    // Simulate 3 workers claiming 3 activities each concurrently
    let worker1_id = "worker_test_01";
    let worker2_id = "worker_test_02";
    let worker3_id = "worker_test_03";

    let queue1 = PostgresQueue::new(pool.clone(), config.clone());
    let queue2 = PostgresQueue::new(pool.clone(), config.clone());
    let queue3 = PostgresQueue::new(pool.clone(), config);

    let (claimed1, claimed2, claimed3) = tokio::join!(
        queue1.claim_next(worker1_id, "test", 3),
        queue2.claim_next(worker2_id, "test", 3),
        queue3.claim_next(worker3_id, "test", 3),
    );

    let claimed1 = claimed1.expect("Worker 1 claim failed");
    let claimed2 = claimed2.expect("Worker 2 claim failed");
    let claimed3 = claimed3.expect("Worker 3 claim failed");

    // Each worker should claim 3 activities
    assert_eq!(claimed1.len(), 3, "Worker 1 should claim 3 activities");
    assert_eq!(claimed2.len(), 3, "Worker 2 should claim 3 activities");
    assert_eq!(claimed3.len(), 3, "Worker 3 should claim 3 activities");

    // All 9 activities should be unique across all workers
    let mut all_ids: Vec<_> = claimed1.iter().map(|a| a.id).collect();
    all_ids.extend(claimed2.iter().map(|a| a.id));
    all_ids.extend(claimed3.iter().map(|a| a.id));

    assert_eq!(
        all_ids.len(),
        all_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        "All activities should be unique (no double-claiming)"
    );
    assert_eq!(all_ids.len(), 9, "Should have claimed all 9 activities");

    cleanup_queue(&pool, workflow_id).await;
}
