//! Integration tests for semantic caching functionality
//!
//! These tests verify cache hit/miss behavior, cache key determinism,
//! TTL expiration, and cache invalidation.

use anyhow::Result;
use async_trait::async_trait;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
#[cfg(feature = "redis-cache")]
use streamflow_core::cache::RedisCache;
use streamflow_core::cache::{CacheService, NoOpCache};
use streamflow_core::workflow::ActivitySettings;
use streamflow_worker::{ActivityImpl, ActivityRegistry, ActivityResult};

/// Test activity that returns a predictable result with cost
struct CostlyActivity {
    cost: Decimal,
    call_count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl CostlyActivity {
    fn new(cost: Decimal) -> Self {
        Self {
            cost,
            call_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    fn get_call_count(&self) -> usize {
        self.call_count.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait]
impl ActivityImpl for CostlyActivity {
    async fn execute(&self, parameters: Value) -> Result<ActivityResult> {
        // Increment call count
        self.call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        // Simulate some work
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Return result with cost
        let mut result = ActivityResult::value("result", parameters);
        result.cost_usd = Some(self.cost);
        Ok(result)
    }

    fn name(&self) -> &str {
        "costly_operation"
    }

    fn worker(&self) -> &str {
        "test"
    }
}

/// Helper to check if Redis is available
#[cfg(feature = "redis-cache")]
async fn redis_available() -> bool {
    match RedisCache::new("redis://localhost:6379", Some("test:cache:".to_string())) {
        Ok(cache) => cache.ping().await.is_ok(),
        Err(_) => false,
    }
}

/// Helper to create Redis cache for testing with unique prefix
#[cfg(feature = "redis-cache")]
async fn create_redis_cache_with_prefix(prefix: &str) -> Option<Arc<dyn CacheService>> {
    if !redis_available().await {
        eprintln!("Skipping test: Redis not available at localhost:6379");
        eprintln!("Run: docker-compose up -d redis");
        return None;
    }

    let full_prefix = format!("test:{}:", prefix);
    let cache = RedisCache::new("redis://localhost:6379", Some(full_prefix))
        .expect("Failed to create Redis cache");

    // Clear any existing keys with this prefix
    let _ = cache.invalidate_pattern("*").await;

    Some(Arc::new(cache) as Arc<dyn CacheService>)
}

#[tokio::test]
#[serial]
#[cfg(feature = "redis-cache")]
async fn test_cache_hit_returns_zero_cost() {
    let cache = match create_redis_cache_with_prefix("hit_zero_cost").await {
        Some(c) => c,
        None => return,
    };

    let activity = Arc::new(CostlyActivity::new(Decimal::new(123, 6))); // $0.000123
    let mut registry = ActivityRegistry::new(cache);
    registry.register(activity.clone());

    let params = json!({
        "input": "test data",
        "value": 42
    });

    let settings = Some(ActivitySettings {
        cache: true,
        cache_ttl: Some(60),
        ..Default::default()
    });

    // First execution: cache miss
    let result1 = registry
        .execute(
            "test",
            "costly_operation",
            params.clone(),
            settings.clone(),
            Duration::from_secs(5),
        )
        .await
        .expect("First execution failed");

    // Verify first call actually executed the activity
    assert_eq!(activity.get_call_count(), 1);
    assert_eq!(result1.cost_usd, Some(Decimal::new(123, 6)));

    // Verify cache_key is present in metadata
    let metadata1 = result1
        .metadata
        .as_ref()
        .expect("First result should have metadata");
    let cache_key1 = metadata1
        .get("cache_key")
        .expect("First result should have cache_key")
        .as_str()
        .expect("cache_key should be string");
    assert!(!cache_key1.is_empty());
    assert_eq!(metadata1.get("cached"), Some(&json!(false)));

    // Second execution: cache hit
    let result2 = registry
        .execute(
            "test",
            "costly_operation",
            params.clone(),
            settings,
            Duration::from_secs(5),
        )
        .await
        .expect("Second execution failed");

    // Verify second call did NOT execute the activity (still 1 call)
    assert_eq!(activity.get_call_count(), 1);

    // Verify cache hit returns zero cost
    assert_eq!(result2.cost_usd, Some(Decimal::ZERO));

    // Verify cache_key is present and matches first call
    let metadata2 = result2
        .metadata
        .as_ref()
        .expect("Second result should have metadata");
    let cache_key2 = metadata2
        .get("cache_key")
        .expect("Second result should have cache_key")
        .as_str()
        .expect("cache_key should be string");
    assert_eq!(cache_key1, cache_key2, "cache_key should be deterministic");
    assert_eq!(metadata2.get("cached"), Some(&json!(true)));

    // Verify outputs match
    assert_eq!(result1.outputs, result2.outputs);
}

#[tokio::test]
#[serial]
#[cfg(feature = "redis-cache")]
async fn test_cache_key_deterministic() {
    let cache = match create_redis_cache_with_prefix("key_determ").await {
        Some(c) => c,
        None => return,
    };

    let activity = Arc::new(CostlyActivity::new(Decimal::new(100, 6)));
    let mut registry = ActivityRegistry::new(cache);
    registry.register(activity);

    // Same parameters in different order
    let params1 = json!({
        "a": 1,
        "b": 2,
        "c": 3
    });

    let params2 = json!({
        "c": 3,
        "a": 1,
        "b": 2
    });

    let settings = Some(ActivitySettings {
        cache: true,
        cache_ttl: Some(60),
        ..Default::default()
    });

    // Execute with params1
    let result1 = registry
        .execute(
            "test",
            "costly_operation",
            params1,
            settings.clone(),
            Duration::from_secs(5),
        )
        .await
        .expect("First execution failed");

    let cache_key1 = result1
        .metadata
        .as_ref()
        .and_then(|m| m.get("cache_key"))
        .and_then(|k| k.as_str())
        .expect("First result should have cache_key");

    // Execute with params2 (different order, same values)
    let result2 = registry
        .execute(
            "test",
            "costly_operation",
            params2,
            settings,
            Duration::from_secs(5),
        )
        .await
        .expect("Second execution failed");

    let cache_key2 = result2
        .metadata
        .as_ref()
        .and_then(|m| m.get("cache_key"))
        .and_then(|k| k.as_str())
        .expect("Second result should have cache_key");

    // Cache keys should be identical (parameter order doesn't matter)
    assert_eq!(cache_key1, cache_key2);

    // Second call should be cache hit
    assert_eq!(
        result2.metadata.as_ref().unwrap().get("cached"),
        Some(&json!(true))
    );
}

#[tokio::test]
#[serial]
#[cfg(feature = "redis-cache")]
async fn test_cache_ttl_expiration() {
    let cache = match create_redis_cache_with_prefix("ttl_expire").await {
        Some(c) => c,
        None => return,
    };

    let activity = Arc::new(CostlyActivity::new(Decimal::new(100, 6)));
    let mut registry = ActivityRegistry::new(cache);
    registry.register(activity.clone());

    let params = json!({"test": "ttl"});

    let settings = Some(ActivitySettings {
        cache: true,
        cache_ttl: Some(1), // 1 second TTL
        ..Default::default()
    });

    // First execution: cache miss
    let result1 = registry
        .execute(
            "test",
            "costly_operation",
            params.clone(),
            settings.clone(),
            Duration::from_secs(5),
        )
        .await
        .expect("First execution failed");

    assert_eq!(activity.get_call_count(), 1);
    assert_eq!(
        result1.metadata.as_ref().unwrap().get("cached"),
        Some(&json!(false))
    );

    // Immediate second execution: cache hit
    let result2 = registry
        .execute(
            "test",
            "costly_operation",
            params.clone(),
            settings.clone(),
            Duration::from_secs(5),
        )
        .await
        .expect("Second execution failed");

    assert_eq!(activity.get_call_count(), 1);
    assert_eq!(
        result2.metadata.as_ref().unwrap().get("cached"),
        Some(&json!(true))
    );

    // Wait for TTL expiration
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Third execution after TTL: cache miss
    let result3 = registry
        .execute(
            "test",
            "costly_operation",
            params,
            settings,
            Duration::from_secs(5),
        )
        .await
        .expect("Third execution failed");

    assert_eq!(activity.get_call_count(), 2); // Should execute again
    assert_eq!(
        result3.metadata.as_ref().unwrap().get("cached"),
        Some(&json!(false))
    );
}

#[tokio::test]
#[serial]
#[cfg(feature = "redis-cache")]
async fn test_cache_invalidation_by_key() {
    let cache = match create_redis_cache_with_prefix("inval_key").await {
        Some(c) => c,
        None => return,
    };

    let activity = Arc::new(CostlyActivity::new(Decimal::new(100, 6)));
    let mut registry = ActivityRegistry::new(Arc::clone(&cache) as Arc<dyn CacheService>);
    registry.register(activity.clone());

    let params = json!({"test": "invalidation"});

    let settings = Some(ActivitySettings {
        cache: true,
        cache_ttl: Some(3600),
        ..Default::default()
    });

    // First execution: cache miss
    let result1 = registry
        .execute(
            "test",
            "costly_operation",
            params.clone(),
            settings.clone(),
            Duration::from_secs(5),
        )
        .await
        .expect("First execution failed");

    let cache_key = result1
        .metadata
        .as_ref()
        .and_then(|m| m.get("cache_key"))
        .and_then(|k| k.as_str())
        .expect("Result should have cache_key")
        .to_string();

    assert_eq!(activity.get_call_count(), 1);

    // Second execution: cache hit
    let result2 = registry
        .execute(
            "test",
            "costly_operation",
            params.clone(),
            settings.clone(),
            Duration::from_secs(5),
        )
        .await
        .expect("Second execution failed");

    assert_eq!(activity.get_call_count(), 1);
    assert_eq!(
        result2.metadata.as_ref().unwrap().get("cached"),
        Some(&json!(true))
    );

    // Invalidate cache by key
    cache
        .invalidate(&cache_key)
        .await
        .expect("Cache invalidation failed");

    // Third execution: cache miss (after invalidation)
    let result3 = registry
        .execute(
            "test",
            "costly_operation",
            params,
            settings,
            Duration::from_secs(5),
        )
        .await
        .expect("Third execution failed");

    assert_eq!(activity.get_call_count(), 2); // Should execute again
    assert_eq!(
        result3.metadata.as_ref().unwrap().get("cached"),
        Some(&json!(false))
    );
}

#[tokio::test]
#[serial]
#[cfg(feature = "redis-cache")]
async fn test_cache_invalidation_by_pattern() {
    let cache = match create_redis_cache_with_prefix("inval_pattern").await {
        Some(c) => c,
        None => return,
    };

    let activity = Arc::new(CostlyActivity::new(Decimal::new(100, 6)));
    let mut registry = ActivityRegistry::new(Arc::clone(&cache) as Arc<dyn CacheService>);
    registry.register(activity.clone());

    let settings = Some(ActivitySettings {
        cache: true,
        cache_ttl: Some(3600),
        ..Default::default()
    });

    // Execute multiple times with different parameters
    let params1 = json!({"test": "pattern1"});
    let params2 = json!({"test": "pattern2"});

    registry
        .execute(
            "test",
            "costly_operation",
            params1.clone(),
            settings.clone(),
            Duration::from_secs(5),
        )
        .await
        .expect("First execution failed");

    registry
        .execute(
            "test",
            "costly_operation",
            params2.clone(),
            settings.clone(),
            Duration::from_secs(5),
        )
        .await
        .expect("Second execution failed");

    assert_eq!(activity.get_call_count(), 2);

    // Verify both are cached
    let result1 = registry
        .execute(
            "test",
            "costly_operation",
            params1.clone(),
            settings.clone(),
            Duration::from_secs(5),
        )
        .await
        .expect("Cached execution 1 failed");
    let result2 = registry
        .execute(
            "test",
            "costly_operation",
            params2.clone(),
            settings.clone(),
            Duration::from_secs(5),
        )
        .await
        .expect("Cached execution 2 failed");

    assert_eq!(activity.get_call_count(), 2); // Still 2 (cache hits)
    assert_eq!(
        result1.metadata.as_ref().unwrap().get("cached"),
        Some(&json!(true))
    );
    assert_eq!(
        result2.metadata.as_ref().unwrap().get("cached"),
        Some(&json!(true))
    );

    // Invalidate all cache entries using wildcard pattern
    // Note: Cache keys are SHA256 hashes, so we use "*" to match all keys under the prefix
    let count = cache
        .invalidate_pattern("*")
        .await
        .expect("Pattern invalidation failed");

    // Should invalidate at least the 2 entries we just created
    // (may be more if other tests left keys)
    assert!(
        count >= 2,
        "Should invalidate at least 2 entries, got {}",
        count
    );

    // Execute again: both should be cache misses
    registry
        .execute(
            "test",
            "costly_operation",
            params1,
            settings.clone(),
            Duration::from_secs(5),
        )
        .await
        .expect("After invalidation execution 1 failed");

    registry
        .execute(
            "test",
            "costly_operation",
            params2,
            settings,
            Duration::from_secs(5),
        )
        .await
        .expect("After invalidation execution 2 failed");

    assert_eq!(activity.get_call_count(), 4); // Should execute both again
}

#[tokio::test]
async fn test_noop_cache_graceful_fallback() {
    // Use NoOpCache (no Redis required)
    let cache = Arc::new(NoOpCache::new()) as Arc<dyn CacheService>;

    let activity = Arc::new(CostlyActivity::new(Decimal::new(100, 6)));
    let mut registry = ActivityRegistry::new(cache);
    registry.register(activity.clone());

    let params = json!({"test": "noop"});

    let settings = Some(ActivitySettings {
        cache: true,
        cache_ttl: Some(60),
        ..Default::default()
    });

    // First execution
    let result1 = registry
        .execute(
            "test",
            "costly_operation",
            params.clone(),
            settings.clone(),
            Duration::from_secs(5),
        )
        .await
        .expect("First execution failed");

    assert_eq!(activity.get_call_count(), 1);
    assert!(result1.cost_usd.is_some());

    // Second execution: should NOT be cached (NoOp cache)
    let result2 = registry
        .execute(
            "test",
            "costly_operation",
            params,
            settings,
            Duration::from_secs(5),
        )
        .await
        .expect("Second execution failed");

    // Should execute again (no caching)
    assert_eq!(activity.get_call_count(), 2);
    assert!(result2.cost_usd.is_some());

    // Both should succeed without errors
    assert_eq!(result1.outputs, result2.outputs);
}

#[tokio::test]
async fn test_cache_disabled_when_setting_false() {
    let cache = Arc::new(NoOpCache::new()) as Arc<dyn CacheService>;

    let activity = Arc::new(CostlyActivity::new(Decimal::new(100, 6)));
    let mut registry = ActivityRegistry::new(cache);
    registry.register(activity.clone());

    let params = json!({"test": "disabled"});

    // Cache explicitly disabled
    let settings = Some(ActivitySettings {
        cache: false,
        cache_ttl: Some(60),
        ..Default::default()
    });

    // First execution
    registry
        .execute(
            "test",
            "costly_operation",
            params.clone(),
            settings.clone(),
            Duration::from_secs(5),
        )
        .await
        .expect("First execution failed");

    assert_eq!(activity.get_call_count(), 1);

    // Second execution: should NOT use cache
    registry
        .execute(
            "test",
            "costly_operation",
            params,
            settings,
            Duration::from_secs(5),
        )
        .await
        .expect("Second execution failed");

    assert_eq!(activity.get_call_count(), 2);
}

#[tokio::test]
async fn test_cache_disabled_when_no_settings() {
    let cache = Arc::new(NoOpCache::new()) as Arc<dyn CacheService>;

    let activity = Arc::new(CostlyActivity::new(Decimal::new(100, 6)));
    let mut registry = ActivityRegistry::new(cache);
    registry.register(activity.clone());

    let params = json!({"test": "no_settings"});

    // No settings (cache disabled by default)
    let settings = None;

    // First execution
    registry
        .execute(
            "test",
            "costly_operation",
            params.clone(),
            settings.clone(),
            Duration::from_secs(5),
        )
        .await
        .expect("First execution failed");

    assert_eq!(activity.get_call_count(), 1);

    // Second execution: should NOT use cache
    registry
        .execute(
            "test",
            "costly_operation",
            params,
            settings,
            Duration::from_secs(5),
        )
        .await
        .expect("Second execution failed");

    assert_eq!(activity.get_call_count(), 2);
}

#[tokio::test]
#[serial]
#[cfg(feature = "redis-cache")]
async fn test_cache_different_activities_different_keys() {
    let cache = match create_redis_cache_with_prefix("diff_acts").await {
        Some(c) => c,
        None => return,
    };

    // Create two different activities
    let activity1 = Arc::new(CostlyActivity::new(Decimal::new(100, 6)));
    let mut registry = ActivityRegistry::new(cache);
    registry.register(activity1.clone());

    // Create second activity type with different name
    struct OtherActivity;

    #[async_trait]
    impl ActivityImpl for OtherActivity {
        async fn execute(&self, parameters: Value) -> Result<ActivityResult> {
            Ok(ActivityResult::value("other_result", parameters))
        }

        fn name(&self) -> &str {
            "other_operation"
        }

        fn worker(&self) -> &str {
            "test"
        }
    }

    registry.register(Arc::new(OtherActivity));

    let params = json!({"same": "params"});

    let settings = Some(ActivitySettings {
        cache: true,
        cache_ttl: Some(60),
        ..Default::default()
    });

    // Execute both activities with same parameters
    let result1 = registry
        .execute(
            "test",
            "costly_operation",
            params.clone(),
            settings.clone(),
            Duration::from_secs(5),
        )
        .await
        .expect("First activity failed");

    let result2 = registry
        .execute(
            "test",
            "other_operation",
            params,
            settings,
            Duration::from_secs(5),
        )
        .await
        .expect("Second activity failed");

    // Extract cache keys
    let cache_key1 = result1
        .metadata
        .as_ref()
        .and_then(|m| m.get("cache_key"))
        .and_then(|k| k.as_str())
        .expect("First result should have cache_key");

    let cache_key2 = result2
        .metadata
        .as_ref()
        .and_then(|m| m.get("cache_key"))
        .and_then(|k| k.as_str())
        .expect("Second result should have cache_key");

    // Cache keys should be different (different activity names)
    assert_ne!(
        cache_key1, cache_key2,
        "Different activities should have different cache keys even with same parameters"
    );
}
