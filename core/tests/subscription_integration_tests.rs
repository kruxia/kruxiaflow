use kruxiaflow_core::subscription::{
    NewSubscription, PostgresSubscriptionService, SubscriptionError, SubscriptionService,
};
use kruxiaflow_core::workflow::OnTimeout;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

// Also need a workflow in the DB for foreign key constraints
async fn insert_test_workflow(pool: &PgPool) -> Uuid {
    // Insert a workflow definition first
    let def_row = sqlx::query(
        "INSERT INTO workflow_definitions (name, activities) VALUES ($1, $2) RETURNING id",
    )
    .bind("sub_test_workflow")
    .bind(serde_json::json!([]))
    .fetch_one(pool)
    .await
    .expect("Failed to insert workflow definition");

    let def_id: Uuid = sqlx::Row::get(&def_row, "id");

    let workflow_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO workflows (id, definition_name, workflow_definition_id, input, status, activities, state_data) \
         VALUES ($1, $2, $3, '{}'::jsonb, 'running', '{}'::jsonb, '{}'::jsonb)",
    )
    .bind(workflow_id)
    .bind("sub_test_workflow")
    .bind(def_id)
    .execute(pool)
    .await
    .expect("Failed to insert workflow");

    workflow_id
}

#[sqlx::test(migrations = "../migrations")]
async fn test_create_subscription(pool: PgPool) {
    let workflow_id = insert_test_workflow(&pool).await;

    let service = PostgresSubscriptionService::new(pool.clone());

    let sub = NewSubscription {
        workflow_id,
        activity_key: "wait_step".to_string(),
        event_name: "approval".to_string(),
        on_timeout: OnTimeout::Fail,
        timeout_seconds: 300,
        iteration: None,
    };

    let id = service.create_subscription(sub).await.unwrap();
    assert_ne!(id, Uuid::nil());
}

#[sqlx::test(migrations = "../migrations")]
async fn test_create_same_wait_again_returns_it(pool: PgPool) {
    let workflow_id = insert_test_workflow(&pool).await;

    let service = PostgresSubscriptionService::new(pool.clone());

    for iteration in [None, Some(2)] {
        let sub = NewSubscription {
            workflow_id,
            activity_key: format!("wait_step_{iteration:?}"),
            event_name: "approval".to_string(),
            on_timeout: OnTimeout::Fail,
            timeout_seconds: 300,
            iteration,
        };

        let first = service.create_subscription(sub.clone()).await.unwrap();
        let again = service.create_subscription(sub).await.unwrap();
        assert_eq!(again, first, "round {iteration:?}");
    }
}

#[sqlx::test(migrations = "../migrations")]
async fn test_create_wait_while_another_round_waits_fails(pool: PgPool) {
    let workflow_id = insert_test_workflow(&pool).await;

    let service = PostgresSubscriptionService::new(pool.clone());

    let sub = NewSubscription {
        workflow_id,
        activity_key: "wait_step".to_string(),
        event_name: "approval".to_string(),
        on_timeout: OnTimeout::Fail,
        timeout_seconds: 300,
        iteration: Some(0),
    };
    service.create_subscription(sub.clone()).await.unwrap();

    let result = service
        .create_subscription(NewSubscription {
            iteration: Some(1),
            ..sub
        })
        .await;
    match result.unwrap_err() {
        SubscriptionError::AlreadyExists(wf_id, key) => {
            assert_eq!(wf_id, workflow_id);
            assert_eq!(key, "wait_step");
        }
        other => panic!("Expected AlreadyExists, got {:?}", other),
    }
}

#[sqlx::test(migrations = "../migrations")]
async fn test_signal_activity_found(pool: PgPool) {
    let workflow_id = insert_test_workflow(&pool).await;

    let service = PostgresSubscriptionService::new(pool.clone());

    // Create subscription
    let sub = NewSubscription {
        workflow_id,
        activity_key: "wait_step".to_string(),
        event_name: "approval".to_string(),
        on_timeout: OnTimeout::Continue,
        timeout_seconds: 300,
        iteration: None,
    };
    service.create_subscription(sub).await.unwrap();

    // Signal the activity
    let signal = kruxiaflow_core::subscription::SignalRequest {
        workflow_id,
        activity_key: "wait_step".to_string(),
        event_name: "approval".to_string(),
        data: Some(json!({"approved": true})),
    };

    let result = service.signal_activity(signal).await.unwrap();
    assert!(result.is_some());

    let subscription = result.unwrap();
    assert_eq!(subscription.workflow_id, workflow_id);
    assert_eq!(subscription.activity_key, "wait_step");
    assert_eq!(subscription.event_name, "approval");
    assert_eq!(subscription.signal_data, Some(json!({"approved": true})));
    assert!(matches!(subscription.on_timeout, OnTimeout::Continue));
}

#[sqlx::test(migrations = "../migrations")]
async fn test_signal_activity_not_found(pool: PgPool) {
    let service = PostgresSubscriptionService::new(pool.clone());

    let signal = kruxiaflow_core::subscription::SignalRequest {
        workflow_id: Uuid::now_v7(),
        activity_key: "nonexistent".to_string(),
        event_name: "event".to_string(),
        data: None,
    };

    let result = service.signal_activity(signal).await.unwrap();
    assert!(result.is_none());
}

#[sqlx::test(migrations = "../migrations")]
async fn test_signal_activity_already_signaled(pool: PgPool) {
    let workflow_id = insert_test_workflow(&pool).await;

    let service = PostgresSubscriptionService::new(pool.clone());

    let sub = NewSubscription {
        workflow_id,
        activity_key: "wait_step".to_string(),
        event_name: "go".to_string(),
        on_timeout: OnTimeout::Fail,
        timeout_seconds: 300,
        iteration: None,
    };
    service.create_subscription(sub).await.unwrap();

    // First signal should succeed
    let signal = kruxiaflow_core::subscription::SignalRequest {
        workflow_id,
        activity_key: "wait_step".to_string(),
        event_name: "go".to_string(),
        data: Some(json!({"first": true})),
    };
    let result = service.signal_activity(signal).await.unwrap();
    assert!(result.is_some());

    // Second signal should return None (already signaled)
    let signal2 = kruxiaflow_core::subscription::SignalRequest {
        workflow_id,
        activity_key: "wait_step".to_string(),
        event_name: "go".to_string(),
        data: Some(json!({"second": true})),
    };
    let result2 = service.signal_activity(signal2).await.unwrap();
    assert!(result2.is_none());
}

#[sqlx::test(migrations = "../migrations")]
async fn test_get_signal_data(pool: PgPool) {
    let workflow_id = insert_test_workflow(&pool).await;

    let service = PostgresSubscriptionService::new(pool.clone());

    // Create and signal
    let sub = NewSubscription {
        workflow_id,
        activity_key: "wait_step".to_string(),
        event_name: "data_event".to_string(),
        on_timeout: OnTimeout::Skip,
        timeout_seconds: 300,
        iteration: None,
    };
    service.create_subscription(sub).await.unwrap();

    let signal = kruxiaflow_core::subscription::SignalRequest {
        workflow_id,
        activity_key: "wait_step".to_string(),
        event_name: "data_event".to_string(),
        data: Some(json!({"key": "value"})),
    };
    service.signal_activity(signal).await.unwrap();

    // Get signal data
    let data = service
        .get_signal_data(workflow_id, "wait_step")
        .await
        .unwrap();
    assert_eq!(data, Some(json!({"key": "value"})));
}

#[sqlx::test(migrations = "../migrations")]
async fn test_get_signal_data_not_found(pool: PgPool) {
    let service = PostgresSubscriptionService::new(pool.clone());

    let data = service
        .get_signal_data(Uuid::now_v7(), "nonexistent")
        .await
        .unwrap();
    assert!(data.is_none());
}

#[sqlx::test(migrations = "../migrations")]
async fn test_delete_subscription(pool: PgPool) {
    let workflow_id = insert_test_workflow(&pool).await;

    let service = PostgresSubscriptionService::new(pool.clone());

    let sub = NewSubscription {
        workflow_id,
        activity_key: "to_delete".to_string(),
        event_name: "event".to_string(),
        on_timeout: OnTimeout::Fail,
        timeout_seconds: 300,
        iteration: None,
    };
    service.create_subscription(sub).await.unwrap();

    // Delete
    service
        .delete_subscription(workflow_id, "to_delete")
        .await
        .unwrap();

    // Signal should not find it
    let signal = kruxiaflow_core::subscription::SignalRequest {
        workflow_id,
        activity_key: "to_delete".to_string(),
        event_name: "event".to_string(),
        data: None,
    };
    let result = service.signal_activity(signal).await.unwrap();
    assert!(result.is_none());
}

#[sqlx::test(migrations = "../migrations")]
async fn test_expire_subscriptions(pool: PgPool) {
    let workflow_id = insert_test_workflow(&pool).await;

    let service = PostgresSubscriptionService::new(pool.clone());

    // Create subscription with 1 second timeout
    let sub = NewSubscription {
        workflow_id,
        activity_key: "expiring_step".to_string(),
        event_name: "never_comes".to_string(),
        on_timeout: OnTimeout::Fail,
        timeout_seconds: 1,
        iteration: None,
    };
    service.create_subscription(sub).await.unwrap();

    // Wait for it to expire
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Expire subscriptions
    let expired = service.expire_subscriptions(100).await.unwrap();
    assert!(!expired.is_empty());
    assert_eq!(expired[0].workflow_id, workflow_id);
    assert_eq!(expired[0].activity_key, "expiring_step");
    assert!(matches!(expired[0].on_timeout, OnTimeout::Fail));
}

#[sqlx::test(migrations = "../migrations")]
async fn test_expire_subscriptions_none_expired(pool: PgPool) {
    let workflow_id = insert_test_workflow(&pool).await;

    let service = PostgresSubscriptionService::new(pool.clone());

    // Create subscription with long timeout
    let sub = NewSubscription {
        workflow_id,
        activity_key: "long_wait".to_string(),
        event_name: "event".to_string(),
        on_timeout: OnTimeout::Continue,
        timeout_seconds: 3600,
        iteration: None,
    };
    service.create_subscription(sub).await.unwrap();

    // Nothing should be expired
    let expired = service.expire_subscriptions(100).await.unwrap();
    assert!(expired.is_empty());
}

#[sqlx::test(migrations = "../migrations")]
async fn test_recover_expired(pool: PgPool) {
    let workflow_id = insert_test_workflow(&pool).await;

    let service = PostgresSubscriptionService::new(pool.clone());

    // Create and expire subscription
    let sub = NewSubscription {
        workflow_id,
        activity_key: "recover_step".to_string(),
        event_name: "event".to_string(),
        on_timeout: OnTimeout::Skip,
        timeout_seconds: 1,
        iteration: None,
    };
    service.create_subscription(sub).await.unwrap();

    // Wait for expiry
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Expire it first
    let expired = service.expire_subscriptions(100).await.unwrap();
    assert!(!expired.is_empty());

    // Wait another second for the recovery grace period
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Recover should find the expired subscription
    let recovered = service.recover_expired(100).await.unwrap();
    assert!(!recovered.is_empty());
    assert_eq!(recovered[0].activity_key, "recover_step");
    assert!(matches!(recovered[0].on_timeout, OnTimeout::Skip));
}

#[sqlx::test(migrations = "../migrations")]
async fn test_on_timeout_variants(pool: PgPool) {
    let workflow_id = insert_test_workflow(&pool).await;

    let service = PostgresSubscriptionService::new(pool.clone());

    // Test Continue variant
    let sub = NewSubscription {
        workflow_id,
        activity_key: "continue_step".to_string(),
        event_name: "event".to_string(),
        on_timeout: OnTimeout::Continue,
        timeout_seconds: 300,
        iteration: None,
    };
    service.create_subscription(sub).await.unwrap();

    let signal = kruxiaflow_core::subscription::SignalRequest {
        workflow_id,
        activity_key: "continue_step".to_string(),
        event_name: "event".to_string(),
        data: None,
    };
    let result = service.signal_activity(signal).await.unwrap().unwrap();
    assert!(matches!(result.on_timeout, OnTimeout::Continue));

    // Test Skip variant (need different activity_key due to unique constraint)
    let sub2 = NewSubscription {
        workflow_id,
        activity_key: "skip_step".to_string(),
        event_name: "event".to_string(),
        on_timeout: OnTimeout::Skip,
        timeout_seconds: 300,
        iteration: None,
    };
    service.create_subscription(sub2).await.unwrap();

    let signal2 = kruxiaflow_core::subscription::SignalRequest {
        workflow_id,
        activity_key: "skip_step".to_string(),
        event_name: "event".to_string(),
        data: None,
    };
    let result2 = service.signal_activity(signal2).await.unwrap().unwrap();
    assert!(matches!(result2.on_timeout, OnTimeout::Skip));
}

/// The loop round a subscription waits in comes back on the signal and on
/// expiry, so the events they produce carry it
#[sqlx::test(migrations = "../migrations")]
async fn test_subscription_iteration_round_trip(pool: PgPool) {
    use kruxiaflow_core::subscription::SignalRequest;

    let workflow_id = insert_test_workflow(&pool).await;
    let service = PostgresSubscriptionService::new(pool.clone());

    let new_sub = |key: &str, timeout_seconds| NewSubscription {
        workflow_id,
        activity_key: key.to_string(),
        event_name: "go".to_string(),
        on_timeout: OnTimeout::Continue,
        timeout_seconds,
        iteration: Some(2),
    };
    service
        .create_subscription(new_sub("signaled", 300))
        .await
        .unwrap();
    service
        .create_subscription(new_sub("expiring", 0))
        .await
        .unwrap();

    let signaled = service
        .signal_activity(SignalRequest {
            workflow_id,
            activity_key: "signaled".to_string(),
            event_name: "go".to_string(),
            data: Some(json!({"ok": true})),
        })
        .await
        .unwrap()
        .expect("a subscription to signal");
    assert_eq!(signaled.iteration, Some(2));

    let expired = service.expire_subscriptions(100).await.unwrap();
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].activity_key, "expiring");
    assert_eq!(expired[0].iteration, Some(2));

    // Past the recovery grace period, recovery returns it with its round too
    sqlx::query(
        "UPDATE activity_event_subscriptions SET expired_at = expired_at - INTERVAL '1 hour' \
         WHERE workflow_id = $1",
    )
    .bind(workflow_id)
    .execute(&pool)
    .await
    .unwrap();
    let recovered = service.recover_expired(100).await.unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].activity_key, "expiring");
    assert_eq!(recovered[0].iteration, Some(2));
}
