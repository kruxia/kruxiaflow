//! Redis-backed cache implementation with TTL support
//!
//! This implementation uses Redis for distributed caching with automatic
//! TTL-based expiration.

use super::{CacheService, CachedResult};
use async_trait::async_trait;
use redis::{AsyncCommands, Client, aio::MultiplexedConnection};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Redis-backed cache implementation
///
/// Provides persistent, distributed caching with automatic TTL expiration.
/// Connection pooling via multiplexed connections ensures efficient resource usage.
///
/// # Configuration
///
/// - Redis URL: `redis://localhost:6379` or `redis://user:pass@host:port/db`
/// - Key prefix: For namespace isolation (default: `kruxiaflow:cache:`)
///
/// # Examples
///
/// ```no_run
/// use kruxiaflow_core::cache::{RedisCache, CacheService};
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let cache = RedisCache::new("redis://localhost:6379", None)?;
///
///     // Test connectivity
///     cache.ping().await?;
///
///     println!("Redis cache initialized");
///     Ok(())
/// }
/// ```
pub struct RedisCache {
    client: Client,
    connection: Arc<Mutex<Option<MultiplexedConnection>>>,
    /// Optional key prefix for namespace isolation
    key_prefix: String,
}

impl RedisCache {
    /// Create a new RedisCache instance
    ///
    /// # Arguments
    ///
    /// * `redis_url` - Redis connection URL (e.g., "redis://localhost:6379")
    /// * `key_prefix` - Optional key prefix for namespace isolation
    ///
    /// # Errors
    ///
    /// Returns error if Redis client creation fails
    pub fn new(redis_url: &str, key_prefix: Option<String>) -> anyhow::Result<Self> {
        let client = Client::open(redis_url)?;
        Ok(Self {
            client,
            connection: Arc::new(Mutex::new(None)),
            key_prefix: key_prefix.unwrap_or_else(|| "kruxiaflow:cache:".to_string()),
        })
    }

    /// Build full Redis key with prefix
    fn build_key(&self, key: &str) -> String {
        format!("{}{}", self.key_prefix, key)
    }

    /// Get or create multiplexed connection
    async fn get_connection(&self) -> anyhow::Result<MultiplexedConnection> {
        let mut conn_guard = self.connection.lock().await;

        if let Some(conn) = conn_guard.as_ref() {
            // Clone the connection (MultiplexedConnection is cheaply cloneable)
            return Ok(conn.clone());
        }

        // Create new connection
        let conn = self.client.get_multiplexed_async_connection().await?;
        *conn_guard = Some(conn.clone());
        Ok(conn)
    }

    /// Test Redis connectivity
    ///
    /// Sends a PING command to verify the Redis server is reachable.
    pub async fn ping(&self) -> anyhow::Result<()> {
        let mut conn = self.get_connection().await?;
        let _: () = redis::cmd("PING").query_async(&mut conn).await?;
        Ok(())
    }
}

#[async_trait]
impl CacheService for RedisCache {
    async fn get(&self, key: &str) -> anyhow::Result<Option<CachedResult>> {
        let mut conn = self.get_connection().await?;
        let redis_key = self.build_key(key);

        let value: Option<String> = conn.get(&redis_key).await?;

        match value {
            Some(json_str) => {
                let result: CachedResult = serde_json::from_str(&json_str)?;
                Ok(Some(result))
            }
            None => Ok(None),
        }
    }

    async fn set(&self, key: &str, result: &CachedResult, ttl: Duration) -> anyhow::Result<()> {
        let mut conn = self.get_connection().await?;
        let redis_key = self.build_key(key);
        let json_str = serde_json::to_string(result)?;

        // Use SETEX for atomic set with TTL
        let _: () = conn.set_ex(&redis_key, json_str, ttl.as_secs()).await?;

        Ok(())
    }

    async fn invalidate(&self, key: &str) -> anyhow::Result<()> {
        let mut conn = self.get_connection().await?;
        let redis_key = self.build_key(key);
        let _: () = conn.del(&redis_key).await?;
        Ok(())
    }

    async fn invalidate_pattern(&self, pattern: &str) -> anyhow::Result<usize> {
        let mut conn = self.get_connection().await?;
        let redis_pattern = self.build_key(pattern);

        // Use SCAN for safe pattern matching (not KEYS which blocks)
        // SCAN returns (cursor, keys) - we use SCAN 0 MATCH pattern
        let (_, keys): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(0)
            .arg("MATCH")
            .arg(&redis_pattern)
            .query_async(&mut conn)
            .await?;

        if keys.is_empty() {
            return Ok(0);
        }

        let count = keys.len();

        // Delete all matched keys
        for key in keys {
            let _: () = conn.del(&key).await?;
        }

        Ok(count)
    }

    fn is_available(&self) -> bool {
        // Cache is available if we can create client
        // Actual connectivity checked lazily on first operation
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use rust_decimal::Decimal;
    use serde_json::json;

    // Helper to check if Redis is available for testing
    async fn redis_available() -> bool {
        match RedisCache::new("redis://localhost:6379", None) {
            Ok(cache) => cache.ping().await.is_ok(),
            Err(_) => false,
        }
    }

    #[tokio::test]
    async fn test_redis_cache_set_get() {
        if !redis_available().await {
            eprintln!("Skipping test: Redis not available");
            return;
        }

        let cache = RedisCache::new("redis://localhost:6379", Some("test:".to_string()))
            .expect("Failed to create Redis cache");

        let cached_result = CachedResult {
            output: json!({"test": "value"}),
            cached_at: Utc::now(),
            original_cost_usd: Some(Decimal::new(123, 6)),
        };

        // Set cache entry
        cache
            .set("test_key", &cached_result, Duration::from_secs(60))
            .await
            .expect("Failed to set cache");

        // Get cache entry
        let result = cache.get("test_key").await.expect("Failed to get cache");
        assert!(result.is_some());

        let retrieved = result.unwrap();
        assert_eq!(retrieved.output, cached_result.output);
        assert_eq!(retrieved.original_cost_usd, cached_result.original_cost_usd);
    }

    #[tokio::test]
    async fn test_redis_cache_miss() {
        if !redis_available().await {
            eprintln!("Skipping test: Redis not available");
            return;
        }

        let cache = RedisCache::new("redis://localhost:6379", Some("test:".to_string()))
            .expect("Failed to create Redis cache");

        // Get non-existent key
        let result = cache
            .get("nonexistent_key")
            .await
            .expect("Failed to get cache");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_redis_cache_invalidate() {
        if !redis_available().await {
            eprintln!("Skipping test: Redis not available");
            return;
        }

        let cache = RedisCache::new("redis://localhost:6379", Some("test:".to_string()))
            .expect("Failed to create Redis cache");

        let cached_result = CachedResult {
            output: json!({"test": "value"}),
            cached_at: Utc::now(),
            original_cost_usd: None,
        };

        // Set and verify
        cache
            .set("test_key", &cached_result, Duration::from_secs(60))
            .await
            .expect("Failed to set cache");

        let result = cache.get("test_key").await.expect("Failed to get cache");
        assert!(result.is_some());

        // Invalidate
        cache
            .invalidate("test_key")
            .await
            .expect("Failed to invalidate cache");

        // Verify deletion
        let result = cache.get("test_key").await.expect("Failed to get cache");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_redis_cache_ttl_expiration() {
        if !redis_available().await {
            eprintln!("Skipping test: Redis not available");
            return;
        }

        let cache = RedisCache::new("redis://localhost:6379", Some("test:".to_string()))
            .expect("Failed to create Redis cache");

        let cached_result = CachedResult {
            output: json!({"test": "value"}),
            cached_at: Utc::now(),
            original_cost_usd: None,
        };

        // Set with 1 second TTL
        cache
            .set("ttl_test_key", &cached_result, Duration::from_secs(1))
            .await
            .expect("Failed to set cache");

        // Immediate get should succeed
        let result = cache
            .get("ttl_test_key")
            .await
            .expect("Failed to get cache");
        assert!(result.is_some());

        // Wait for expiration
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Should be expired now
        let result = cache
            .get("ttl_test_key")
            .await
            .expect("Failed to get cache");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_redis_cache_pattern_invalidation() {
        if !redis_available().await {
            eprintln!("Skipping test: Redis not available");
            return;
        }

        let cache = RedisCache::new("redis://localhost:6379", Some("test:pattern:".to_string()))
            .expect("Failed to create Redis cache");

        let cached_result = CachedResult {
            output: json!({"test": "value"}),
            cached_at: Utc::now(),
            original_cost_usd: None,
        };

        // Set multiple entries
        cache
            .set("key1", &cached_result, Duration::from_secs(60))
            .await
            .expect("Failed to set cache");
        cache
            .set("key2", &cached_result, Duration::from_secs(60))
            .await
            .expect("Failed to set cache");

        // Invalidate by pattern
        let count = cache
            .invalidate_pattern("key*")
            .await
            .expect("Failed to invalidate pattern");

        // Should have deleted at least the 2 keys we created
        assert!(count >= 2);

        // Verify deletion
        let result1 = cache.get("key1").await.expect("Failed to get cache");
        let result2 = cache.get("key2").await.expect("Failed to get cache");
        assert!(result1.is_none());
        assert!(result2.is_none());
    }

    // --- Unit tests that don't require a running Redis instance ---

    #[test]
    fn test_redis_cache_new_with_default_prefix() {
        let cache =
            RedisCache::new("redis://localhost:6379", None).expect("Failed to create Redis cache");
        assert_eq!(cache.key_prefix, "kruxiaflow:cache:");
    }

    #[test]
    fn test_redis_cache_new_with_custom_prefix() {
        let cache = RedisCache::new("redis://localhost:6379", Some("custom:prefix:".to_string()))
            .expect("Failed to create Redis cache");
        assert_eq!(cache.key_prefix, "custom:prefix:");
    }

    #[test]
    fn test_redis_cache_new_with_empty_prefix() {
        let cache = RedisCache::new("redis://localhost:6379", Some(String::new()))
            .expect("Failed to create Redis cache");
        assert_eq!(cache.key_prefix, "");
    }

    #[test]
    fn test_redis_cache_new_invalid_url() {
        let result = RedisCache::new("not-a-valid-url", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_redis_cache_new_empty_url() {
        let result = RedisCache::new("", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_redis_cache_build_key_with_default_prefix() {
        let cache = RedisCache::new("redis://localhost:6379", None).unwrap();
        assert_eq!(cache.build_key("my_key"), "kruxiaflow:cache:my_key");
    }

    #[test]
    fn test_redis_cache_build_key_with_custom_prefix() {
        let cache = RedisCache::new("redis://localhost:6379", Some("test:".to_string())).unwrap();
        assert_eq!(cache.build_key("my_key"), "test:my_key");
    }

    #[test]
    fn test_redis_cache_build_key_with_empty_prefix() {
        let cache = RedisCache::new("redis://localhost:6379", Some(String::new())).unwrap();
        assert_eq!(cache.build_key("my_key"), "my_key");
    }

    #[test]
    fn test_redis_cache_build_key_with_empty_key() {
        let cache = RedisCache::new("redis://localhost:6379", None).unwrap();
        assert_eq!(cache.build_key(""), "kruxiaflow:cache:");
    }

    #[test]
    fn test_redis_cache_build_key_special_characters() {
        let cache = RedisCache::new("redis://localhost:6379", None).unwrap();
        assert_eq!(
            cache.build_key("workflow:123:activity:*"),
            "kruxiaflow:cache:workflow:123:activity:*"
        );
    }

    #[test]
    fn test_redis_cache_is_available() {
        let cache = RedisCache::new("redis://localhost:6379", None).unwrap();
        // is_available always returns true (connectivity checked lazily)
        assert!(cache.is_available());
    }

    #[tokio::test]
    async fn test_redis_cache_get_connection_fails_with_bad_url() {
        // Use a URL that resolves but isn't Redis
        let cache = RedisCache::new("redis://127.0.0.1:1", None).unwrap();
        let result = cache.get_connection().await;
        assert!(result.is_err());
    }

    #[test]
    fn test_redis_cache_connection_arc_clone() {
        let cache = RedisCache::new("redis://localhost:6379", None).unwrap();
        // Verify the connection Arc can be cloned (for concurrent access)
        let _clone = Arc::clone(&cache.connection);
    }

    #[test]
    fn test_redis_cache_new_with_auth_url() {
        let cache = RedisCache::new("redis://user:password@localhost:6379/0", None);
        assert!(cache.is_ok());
    }

    #[test]
    fn test_redis_cache_new_with_db_number() {
        let cache = RedisCache::new("redis://localhost:6379/2", None);
        assert!(cache.is_ok());
    }

    #[test]
    fn test_cached_result_serialization_roundtrip() {
        let result = CachedResult {
            output: json!({"key": "value", "nested": {"a": 1}}),
            cached_at: Utc::now(),
            original_cost_usd: Some(Decimal::new(5050, 4)), // 0.5050
        };

        let json_str = serde_json::to_string(&result).unwrap();
        let deserialized: CachedResult = serde_json::from_str(&json_str).unwrap();

        assert_eq!(deserialized.output, result.output);
        assert_eq!(deserialized.original_cost_usd, result.original_cost_usd);
    }

    #[test]
    fn test_cached_result_serialization_no_cost() {
        let result = CachedResult {
            output: json!("simple string"),
            cached_at: Utc::now(),
            original_cost_usd: None,
        };

        let json_str = serde_json::to_string(&result).unwrap();
        let deserialized: CachedResult = serde_json::from_str(&json_str).unwrap();

        assert_eq!(deserialized.output, json!("simple string"));
        assert!(deserialized.original_cost_usd.is_none());
    }

    #[test]
    fn test_cached_result_with_null_output() {
        let result = CachedResult {
            output: json!(null),
            cached_at: Utc::now(),
            original_cost_usd: None,
        };

        let json_str = serde_json::to_string(&result).unwrap();
        let deserialized: CachedResult = serde_json::from_str(&json_str).unwrap();

        assert!(deserialized.output.is_null());
    }

    #[test]
    fn test_cached_result_with_array_output() {
        let result = CachedResult {
            output: json!([1, 2, 3, "test"]),
            cached_at: Utc::now(),
            original_cost_usd: Some(Decimal::ZERO),
        };

        let json_str = serde_json::to_string(&result).unwrap();
        let deserialized: CachedResult = serde_json::from_str(&json_str).unwrap();

        assert_eq!(deserialized.output, json!([1, 2, 3, "test"]));
        assert_eq!(deserialized.original_cost_usd, Some(Decimal::ZERO));
    }
}
