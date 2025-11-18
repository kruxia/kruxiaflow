// core/tests/queue_model_tests.rs
//! Unit tests for queue models (serialization, deserialization, Display impls)

use chrono::Utc;
use serde_json::json;
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
        retry: Some(RetryConfig {
            max_attempts: 3,
            backoff: Some(1000),
        }),
        timeout: None,
        budget: None,
        cache: None,
        deterministic: true,
    };

    let activity = Activity {
        key: "act1".to_string(),
        worker: "builtin".to_string(),
        activity_name: "TestActivity".to_string(),
        parameters: json!({}),
        settings: Some(settings),
        scheduled_for: None,
        output_definitions: None,
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
fn test_activity_settings_default_deterministic() {
    let settings = ActivitySettings {
        retry: None,
        timeout: None,
        budget: None,
        cache: None,
        deterministic: true,
    };

    let json = serde_json::to_string(&settings).unwrap();
    let deserialized: ActivitySettings = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.deterministic, true);
}

#[test]
fn test_activity_settings_with_all_options() {
    let settings = ActivitySettings {
        retry: Some(RetryConfig {
            max_attempts: 5,
            backoff: Some(2000),
        }),
        timeout: Some(TimeoutConfig {
            timeout: 30000,
            heartbeat: Some(5000),
        }),
        budget: Some(BudgetConfig { max_cost_usd: 10.0 }),
        cache: Some(CacheConfig { ttl: 3600 }),
        deterministic: false,
    };

    let json = serde_json::to_string(&settings).unwrap();
    assert!(json.contains("max_attempts"));
    assert!(json.contains("timeout"));
    assert!(json.contains("max_cost_usd"));
    assert!(json.contains("ttl"));
    assert!(json.contains("deterministic"));
}

#[test]
fn test_activity_settings_serialization_skips_none() {
    let settings = ActivitySettings {
        retry: None,
        timeout: None,
        budget: None,
        cache: None,
        deterministic: true,
    };

    let json = serde_json::to_string(&settings).unwrap();
    assert!(!json.contains("retry"));
    assert!(!json.contains("timeout"));
    assert!(!json.contains("budget"));
    assert!(!json.contains("cache"));
}

#[test]
fn test_activity_settings_clone() {
    let settings1 = ActivitySettings {
        retry: Some(RetryConfig {
            max_attempts: 3,
            backoff: Some(1000),
        }),
        timeout: None,
        budget: None,
        cache: None,
        deterministic: true,
    };

    let settings2 = settings1.clone();
    assert_eq!(settings1.deterministic, settings2.deterministic);
    assert_eq!(
        settings1.retry.as_ref().unwrap().max_attempts,
        settings2.retry.as_ref().unwrap().max_attempts
    );
}

// ============================================================================
// RetryConfig Tests
// ============================================================================

#[test]
fn test_retry_config_with_backoff() {
    let config = RetryConfig {
        max_attempts: 5,
        backoff: Some(1000),
    };

    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("5"));
    assert!(json.contains("1000"));
}

#[test]
fn test_retry_config_without_backoff() {
    let config = RetryConfig {
        max_attempts: 3,
        backoff: None,
    };

    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("3"));
    assert!(!json.contains("backoff"));
}

#[test]
fn test_retry_config_clone() {
    let config1 = RetryConfig {
        max_attempts: 3,
        backoff: Some(500),
    };

    let config2 = config1.clone();
    assert_eq!(config1.max_attempts, config2.max_attempts);
    assert_eq!(config1.backoff, config2.backoff);
}

// ============================================================================
// TimeoutConfig Tests
// ============================================================================

#[test]
fn test_timeout_config_with_heartbeat() {
    let config = TimeoutConfig {
        timeout: 30000,
        heartbeat: Some(5000),
    };

    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("30000"));
    assert!(json.contains("5000"));
}

#[test]
fn test_timeout_config_without_heartbeat() {
    let config = TimeoutConfig {
        timeout: 10000,
        heartbeat: None,
    };

    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("10000"));
    assert!(!json.contains("heartbeat"));
}

#[test]
fn test_timeout_config_clone() {
    let config1 = TimeoutConfig {
        timeout: 15000,
        heartbeat: Some(3000),
    };

    let config2 = config1.clone();
    assert_eq!(config1.timeout, config2.timeout);
    assert_eq!(config1.heartbeat, config2.heartbeat);
}

// ============================================================================
// BudgetConfig Tests
// ============================================================================

#[test]
fn test_budget_config_serialization() {
    let config = BudgetConfig {
        max_cost_usd: 25.50,
    };

    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("25.5"));
}

#[test]
fn test_budget_config_deserialization() {
    let json = r#"{"max_cost_usd": 10.0}"#;
    let config: BudgetConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.max_cost_usd, 10.0);
}

#[test]
fn test_budget_config_clone() {
    let config1 = BudgetConfig { max_cost_usd: 15.0 };

    let config2 = config1.clone();
    assert_eq!(config1.max_cost_usd, config2.max_cost_usd);
}

// ============================================================================
// CacheConfig Tests
// ============================================================================

#[test]
fn test_cache_config_serialization() {
    let config = CacheConfig { ttl: 7200 };

    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("7200"));
}

#[test]
fn test_cache_config_deserialization() {
    let json = r#"{"ttl": 3600}"#;
    let config: CacheConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.ttl, 3600);
}

#[test]
fn test_cache_config_clone() {
    let config1 = CacheConfig { ttl: 1800 };

    let config2 = config1.clone();
    assert_eq!(config1.ttl, config2.ttl);
}

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
        output_definitions: None,
        retry_count: 2,
        claimed_at,
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
        retry: Some(RetryConfig {
            max_attempts: 3,
            backoff: None,
        }),
        timeout: None,
        budget: None,
        cache: None,
        deterministic: true,
    };

    let activity = QueuedActivity {
        id,
        workflow_id,
        activity_key: "act1".to_string(),
        worker: "builtin".to_string(),
        activity_name: "TestActivity".to_string(),
        parameters: json!({}),
        settings: Some(settings),
        output_definitions: None,
        retry_count: 0,
        claimed_at: Utc::now(),
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
        output_definitions: None,
        retry_count: 1,
        claimed_at: Utc::now(),
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
        cost_usd: Some(0.50),
        token_usage: Some(TokenUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
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
        cost_usd: Some(1.25),
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
        completion_tokens: 500,
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
        "completion_tokens": 100,
        "total_tokens": 300
    }"#;

    let usage: TokenUsage = serde_json::from_str(json).unwrap();
    assert_eq!(usage.prompt_tokens, 200);
    assert_eq!(usage.completion_tokens, 100);
    assert_eq!(usage.total_tokens, 300);
}

#[test]
fn test_token_usage_clone() {
    let usage1 = TokenUsage {
        prompt_tokens: 50,
        completion_tokens: 25,
        total_tokens: 75,
    };

    let usage2 = usage1.clone();
    assert_eq!(usage1.prompt_tokens, usage2.prompt_tokens);
    assert_eq!(usage1.completion_tokens, usage2.completion_tokens);
    assert_eq!(usage1.total_tokens, usage2.total_tokens);
}

// ============================================================================
// ActivitySettings Deserialization with Default
// ============================================================================

#[test]
fn test_activity_settings_deserialization_defaults() {
    // When deserializing without deterministic field, should default to true
    let json = r#"{"retry": null, "timeout": null, "budget": null, "cache": null}"#;
    let settings: ActivitySettings = serde_json::from_str(json).unwrap();
    assert_eq!(settings.deterministic, true);
}
