// core/tests/queue_model_tests.rs
//! Unit tests for queue models (serialization, deserialization, Display impls)

use chrono::Utc;
use rust_decimal::Decimal;
use serde_json::json;
use std::str::FromStr;
use streamflow_core::queue::models::*;
use uuid::Uuid;

// Helper to create a test UUID
fn test_uuid() -> Uuid {
    Uuid::now_v7()
}

// ============================================================================
// ActivityStatus Tests
// ============================================================================

#[test]
fn test_activity_status_display() {
    assert_eq!(ActivityStatus::Pending.to_string(), "pending");
    assert_eq!(ActivityStatus::Running.to_string(), "running");
    assert_eq!(ActivityStatus::Completed.to_string(), "completed");
    assert_eq!(ActivityStatus::Failed.to_string(), "failed");
}

#[test]
fn test_activity_status_serialization() {
    let status = ActivityStatus::Pending;
    let json = serde_json::to_string(&status).unwrap();
    assert_eq!(json, "\"pending\"");

    let status = ActivityStatus::Completed;
    let json = serde_json::to_string(&status).unwrap();
    assert_eq!(json, "\"completed\"");
}

#[test]
fn test_activity_status_deserialization() {
    let json = "\"pending\"";
    let status: ActivityStatus = serde_json::from_str(json).unwrap();
    assert_eq!(status, ActivityStatus::Pending);

    let json = "\"running\"";
    let status: ActivityStatus = serde_json::from_str(json).unwrap();
    assert_eq!(status, ActivityStatus::Running);
}

#[test]
fn test_activity_status_all_variants() {
    let variants = vec![
        ActivityStatus::Pending,
        ActivityStatus::Running,
        ActivityStatus::Completed,
        ActivityStatus::Failed,
    ];

    for variant in variants {
        // Test round-trip serialization
        let json = serde_json::to_string(&variant).unwrap();
        let deserialized: ActivityStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(variant, deserialized);
    }
}

// ============================================================================
// Activity Tests
// ============================================================================

#[test]
fn test_activity_serialization() {
    let activity = Activity {
        key: "act1".to_string(),
        worker: "builtin".to_string(),
        activity_name: "TestActivity".to_string(),
        parameters: json!({"input": "data"}),
        settings: None,
        scheduled_for: None,
        output_definitions: None,
        iteration: None,
    };

    let json = serde_json::to_string(&activity).unwrap();
    assert!(json.contains("act1"));
    assert!(json.contains("builtin"));
    assert!(json.contains("TestActivity"));
    assert!(json.contains("input"));
}

#[test]
fn test_activity_with_settings() {
    let settings = ActivitySettings {
        retry: Some(streamflow_core::workflow::RetryPolicy {
            max_attempts: 3,
            strategy: streamflow_core::workflow::BackoffStrategy::Fixed,
            base_seconds: 2,
            factor: 2.0,
            max_seconds: 300,
        }),
        timeout_seconds: None,
        budget: None,
        cache: false,
        cache_ttl: None,
        iteration_limit: None,
        delay: None,
        scheduled_for: None,
    };

    let activity = Activity {
        key: "act1".to_string(),
        worker: "builtin".to_string(),
        activity_name: "TestActivity".to_string(),
        parameters: json!({}),
        settings: Some(settings),
        scheduled_for: None,
        output_definitions: None,
        iteration: None,
    };

    let json = serde_json::to_string(&activity).unwrap();
    assert!(json.contains("retry"));
    assert!(json.contains("max_attempts"));
}

#[test]
fn test_activity_with_scheduled_for() {
    let scheduled = Utc::now();
    let activity = Activity {
        key: "act1".to_string(),
        worker: "builtin".to_string(),
        activity_name: "TestActivity".to_string(),
        parameters: json!({}),
        settings: None,
        scheduled_for: Some(scheduled),
        output_definitions: None,
        iteration: None,
    };

    let json = serde_json::to_string(&activity).unwrap();
    assert!(json.contains("scheduled_for"));
}

#[test]
fn test_activity_clone() {
    let activity1 = Activity {
        key: "act1".to_string(),
        worker: "ns".to_string(),
        activity_name: "Activity".to_string(),
        parameters: json!({"key": "value"}),
        settings: None,
        scheduled_for: None,
        output_definitions: None,
        iteration: None,
    };

    let activity2 = activity1.clone();
    assert_eq!(activity1.key, activity2.key);
    assert_eq!(activity1.worker, activity2.worker);
    assert_eq!(activity1.activity_name, activity2.activity_name);
}

// ============================================================================
// ActivitySettings Tests
// ============================================================================

#[test]
fn test_activity_settings_default_values() {
    let settings = ActivitySettings {
        retry: None,
        timeout_seconds: None,
        budget: None,
        cache: false,
        cache_ttl: None,
        iteration_limit: None,
        delay: None,
        scheduled_for: None,
    };

    let json = serde_json::to_string(&settings).unwrap();
    let deserialized: ActivitySettings = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.cache, false);
    assert_eq!(deserialized.cache_ttl, None);
}

#[test]
fn test_activity_settings_with_all_options() {
    let settings = ActivitySettings {
        retry: Some(streamflow_core::workflow::RetryPolicy {
            max_attempts: 5,
            strategy: streamflow_core::workflow::BackoffStrategy::Exponential,
            base_seconds: 2,
            factor: 2.0,
            max_seconds: 300,
        }),
        timeout_seconds: Some(30),
        budget: Some(streamflow_core::workflow::BudgetSettings {
            limit: Decimal::from_str("10.0").unwrap(),
            action: streamflow_core::workflow::BudgetAction::Abort,
        }),
        cache: true,
        cache_ttl: Some(3600),
        iteration_limit: None,
        delay: None,
        scheduled_for: None,
    };

    let json = serde_json::to_string(&settings).unwrap();
    assert!(json.contains("max_attempts"));
    assert!(json.contains("timeout_seconds"));
    assert!(json.contains("limit"));
    assert!(json.contains("cache_ttl"));
}

#[test]
fn test_activity_settings_serialization_skips_none() {
    let settings = ActivitySettings {
        retry: None,
        timeout_seconds: None,
        budget: None,
        cache: false,
        cache_ttl: None,
        iteration_limit: None,
        delay: None,
        scheduled_for: None,
    };

    let json = serde_json::to_string(&settings).unwrap();
    assert!(!json.contains("retry"));
    assert!(!json.contains("timeout_seconds"));
    assert!(!json.contains("budget"));
    assert!(!json.contains("cache_ttl"));
    // Note: cache is a bool with #[serde(default)], so it will be serialized even when false
    assert!(json.contains("\"cache\":false"));
}

#[test]
fn test_activity_settings_clone() {
    let settings1 = ActivitySettings {
        retry: Some(streamflow_core::workflow::RetryPolicy {
            max_attempts: 3,
            strategy: streamflow_core::workflow::BackoffStrategy::Fixed,
            base_seconds: 2,
            factor: 2.0,
            max_seconds: 300,
        }),
        timeout_seconds: None,
        budget: None,
        cache: false,
        cache_ttl: None,
        iteration_limit: None,
        delay: None,
        scheduled_for: None,
    };

    let settings2 = settings1.clone();
    assert_eq!(settings1.cache, settings2.cache);
    assert_eq!(
        settings1.retry.as_ref().unwrap().max_attempts,
        settings2.retry.as_ref().unwrap().max_attempts
    );
}

// ============================================================================
// RetryPolicy Tests
// ============================================================================

#[test]
fn test_retry_policy_with_exponential_backoff() {
    let policy = streamflow_core::workflow::RetryPolicy {
        max_attempts: 5,
        strategy: streamflow_core::workflow::BackoffStrategy::Exponential,
        base_seconds: 2,
        factor: 2.0,
        max_seconds: 300,
    };

    let json = serde_json::to_string(&policy).unwrap();
    assert!(json.contains("5"));
    assert!(json.contains("exponential"));
}

#[test]
fn test_retry_policy_with_fixed_backoff() {
    let policy = streamflow_core::workflow::RetryPolicy {
        max_attempts: 3,
        strategy: streamflow_core::workflow::BackoffStrategy::Fixed,
        base_seconds: 2,
        factor: 2.0,
        max_seconds: 300,
    };

    let json = serde_json::to_string(&policy).unwrap();
    assert!(json.contains("3"));
    assert!(json.contains("fixed"));
}

#[test]
fn test_retry_policy_clone() {
    let policy1 = streamflow_core::workflow::RetryPolicy {
        max_attempts: 3,
        strategy: streamflow_core::workflow::BackoffStrategy::Fixed,
        base_seconds: 2,
        factor: 2.0,
        max_seconds: 300,
    };

    let policy2 = policy1.clone();
    assert_eq!(policy1.max_attempts, policy2.max_attempts);
    assert_eq!(policy1.base_seconds, policy2.base_seconds);
}

// ============================================================================
// Timeout Tests (removed TimeoutConfig, now using timeout_seconds directly)
// ============================================================================

// Timeout config tests removed - timeout is now a simple u64 field (timeout_seconds)
// in ActivitySettings and is tested there.

// ============================================================================
// BudgetSettings Tests
// ============================================================================

#[test]
fn test_budget_settings_serialization() {
    let settings = streamflow_core::workflow::BudgetSettings {
        limit: Decimal::from_str("25.50").unwrap(),
        action: streamflow_core::workflow::BudgetAction::Abort,
    };

    let json = serde_json::to_string(&settings).unwrap();
    assert!(json.contains("25.5"));
    assert!(json.contains("abort"));
}

#[test]
fn test_budget_settings_deserialization() {
    let json = r#"{"limit": 10.0, "action": "abort"}"#;
    let settings: streamflow_core::workflow::BudgetSettings = serde_json::from_str(json).unwrap();
    assert_eq!(settings.limit, Decimal::from_str("10.0").unwrap());
}

#[test]
fn test_budget_settings_clone() {
    let settings1 = streamflow_core::workflow::BudgetSettings {
        limit: Decimal::from_str("15.0").unwrap(),
        action: streamflow_core::workflow::BudgetAction::Continue,
    };

    let settings2 = settings1.clone();
    assert_eq!(settings1.limit, settings2.limit);
}

// Cache config tests removed - caching is now handled directly in ActivitySettings
// with cache: bool and cache_ttl: Option<u64> fields.

// ============================================================================
// QueuedActivity Tests
// ============================================================================

#[test]
fn test_queued_activity_serialization() {
    let id = test_uuid();
    let workflow_id = test_uuid();
    let claimed_at = Utc::now();

    let activity = QueuedActivity {
        id,
        workflow_id,
        activity_key: "act1".to_string(),
        worker: "builtin".to_string(),
        activity_name: "TestActivity".to_string(),
        parameters: json!({"input": "data"}),
        settings: None,
        retry_count: 2,
        claimed_at,
        output_definitions: None,
        iteration: None,
    };

    let json = serde_json::to_string(&activity).unwrap();
    assert!(json.contains(&id.to_string()));
    assert!(json.contains(&workflow_id.to_string()));
    assert!(json.contains("act1"));
    assert!(json.contains("\"retry_count\":2"));
}

#[test]
fn test_queued_activity_with_settings() {
    let id = test_uuid();
    let workflow_id = test_uuid();

    let settings = ActivitySettings {
        retry: Some(streamflow_core::workflow::RetryPolicy {
            max_attempts: 3,
            strategy: streamflow_core::workflow::BackoffStrategy::Fixed,
            base_seconds: 2,
            factor: 2.0,
            max_seconds: 300,
        }),
        timeout_seconds: None,
        budget: None,
        cache: false,
        cache_ttl: None,
        iteration_limit: None,
        delay: None,
        scheduled_for: None,
    };

    let activity = QueuedActivity {
        id,
        workflow_id,
        activity_key: "act1".to_string(),
        worker: "builtin".to_string(),
        activity_name: "TestActivity".to_string(),
        parameters: json!({}),
        settings: Some(settings),
        retry_count: 0,
        claimed_at: Utc::now(),
        output_definitions: None,
        iteration: None,
    };

    let json = serde_json::to_string(&activity).unwrap();
    assert!(json.contains("retry"));
    assert!(json.contains("max_attempts"));
}

#[test]
fn test_queued_activity_clone() {
    let id = test_uuid();
    let workflow_id = test_uuid();

    let activity1 = QueuedActivity {
        id,
        workflow_id,
        activity_key: "act1".to_string(),
        worker: "ns".to_string(),
        activity_name: "Activity".to_string(),
        parameters: json!({}),
        settings: None,
        retry_count: 1,
        claimed_at: Utc::now(),
        output_definitions: None,
        iteration: None,
    };

    let activity2 = activity1.clone();
    assert_eq!(activity1.id, activity2.id);
    assert_eq!(activity1.retry_count, activity2.retry_count);
}

// ============================================================================
// ActivityResult Tests
// ============================================================================

#[test]
fn test_activity_result_success() {
    let result = ActivityResult {
        success: true,
        outputs: Some(vec![streamflow_core::workflow::ActivityOutput {
            name: "result".to_string(),
            output_type: streamflow_core::workflow::OutputType::Value,
            value: json!("ok"),
        }]),
        error: None,
        cost_usd: Some(Decimal::from_str("0.50").unwrap()),
        token_usage: Some(TokenUsage {
            prompt_tokens: 100,
            output_tokens: 50,
            total_tokens: 150,
        }),
    };

    let json = serde_json::to_string(&result).unwrap();
    assert!(json.contains("true"));
    assert!(json.contains("result"));
    assert!(json.contains("0.5"));
    assert!(json.contains("prompt_tokens"));
}

#[test]
fn test_activity_result_failure() {
    let result = ActivityResult {
        success: false,
        outputs: None,
        error: Some("Task failed".to_string()),
        cost_usd: None,
        token_usage: None,
    };

    let json = serde_json::to_string(&result).unwrap();
    assert!(json.contains("false"));
    assert!(json.contains("Task failed"));
    assert!(!json.contains("outputs"));
}

#[test]
fn test_activity_result_skips_none_fields() {
    let result = ActivityResult {
        success: true,
        outputs: None,
        error: None,
        cost_usd: None,
        token_usage: None,
    };

    let json = serde_json::to_string(&result).unwrap();
    assert!(!json.contains("outputs"));
    assert!(!json.contains("error"));
    assert!(!json.contains("cost_usd"));
    assert!(!json.contains("token_usage"));
}

#[test]
fn test_activity_result_clone() {
    let result1 = ActivityResult {
        success: true,
        outputs: Some(vec![streamflow_core::workflow::ActivityOutput {
            name: "key".to_string(),
            output_type: streamflow_core::workflow::OutputType::Value,
            value: json!("value"),
        }]),
        error: None,
        cost_usd: Some(Decimal::from_str("1.25").unwrap()),
        token_usage: None,
    };

    let result2 = result1.clone();
    assert_eq!(result1.success, result2.success);
    assert_eq!(result1.cost_usd, result2.cost_usd);
}

// ============================================================================
// TokenUsage Tests
// ============================================================================

#[test]
fn test_token_usage_serialization() {
    let usage = TokenUsage {
        prompt_tokens: 1000,
        output_tokens: 500,
        total_tokens: 1500,
    };

    let json = serde_json::to_string(&usage).unwrap();
    assert!(json.contains("1000"));
    assert!(json.contains("500"));
    assert!(json.contains("1500"));
}

#[test]
fn test_token_usage_deserialization() {
    let json = r#"{
        "prompt_tokens": 200,
        "output_tokens": 100,
        "total_tokens": 300
    }"#;

    let usage: TokenUsage = serde_json::from_str(json).unwrap();
    assert_eq!(usage.prompt_tokens, 200);
    assert_eq!(usage.output_tokens, 100);
    assert_eq!(usage.total_tokens, 300);
}

#[test]
fn test_token_usage_clone() {
    let usage1 = TokenUsage {
        prompt_tokens: 50,
        output_tokens: 25,
        total_tokens: 75,
    };

    let usage2 = usage1.clone();
    assert_eq!(usage1.prompt_tokens, usage2.prompt_tokens);
    assert_eq!(usage1.output_tokens, usage2.output_tokens);
    assert_eq!(usage1.total_tokens, usage2.total_tokens);
}

// ============================================================================
// ActivitySettings Deserialization with Default
// ============================================================================

#[test]
fn test_activity_settings_deserialization_defaults() {
    // When deserializing without cache field, should default to false
    let json = r#"{"retry": null, "timeout_seconds": null, "budget": null}"#;
    let settings: ActivitySettings = serde_json::from_str(json).unwrap();
    assert_eq!(settings.cache, false);
    assert_eq!(settings.cache_ttl, None);
}
