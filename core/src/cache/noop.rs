//! No-op cache implementation for graceful degradation
//!
//! This implementation is used when Redis is not configured or unavailable.
//! All operations succeed immediately without actually caching anything.

use super::{CacheService, CachedResult};
use async_trait::async_trait;
use std::time::Duration;

/// No-op cache that always returns cache miss
///
/// Used when Redis is not configured or unavailable. This allows workflows
/// to continue executing without caching, ensuring graceful degradation.
///
/// All cache operations succeed immediately:
/// - `get()` always returns `None` (cache miss)
/// - `set()` silently succeeds without storing anything
/// - `invalidate()` silently succeeds
/// - `is_available()` returns `false` to indicate no actual caching
#[derive(Debug, Clone)]
pub struct NoOpCache;

impl NoOpCache {
    /// Create a new NoOpCache instance
    pub fn new() -> Self {
        Self
    }
}

impl Default for NoOpCache {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CacheService for NoOpCache {
    async fn get(&self, _key: &str) -> anyhow::Result<Option<CachedResult>> {
        // Always return cache miss
        Ok(None)
    }

    async fn set(&self, _key: &str, _result: &CachedResult, _ttl: Duration) -> anyhow::Result<()> {
        // No-op - silently succeed
        Ok(())
    }

    async fn invalidate(&self, _key: &str) -> anyhow::Result<()> {
        // No-op - silently succeed
        Ok(())
    }

    async fn invalidate_pattern(&self, _pattern: &str) -> anyhow::Result<usize> {
        // No-op - return 0 invalidated keys
        Ok(0)
    }

    fn is_available(&self) -> bool {
        // NoOp cache is not actually available for caching
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use rust_decimal::Decimal;
    use serde_json::json;

    #[tokio::test]
    async fn test_noop_cache_always_misses() {
        let cache = NoOpCache::new();

        // Cache should always return None (miss)
        let result = cache.get("test_key").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_noop_cache_set_succeeds() {
        let cache = NoOpCache::new();

        let cached_result = CachedResult {
            output: json!({"test": "value"}),
            cached_at: Utc::now(),
            original_cost_usd: Some(Decimal::new(123, 6)), // 0.000123
        };

        // Set should succeed without error
        let result = cache
            .set("test_key", &cached_result, Duration::from_secs(3600))
            .await;
        assert!(result.is_ok());

        // But subsequent get should still return None (no actual caching)
        let get_result = cache.get("test_key").await.unwrap();
        assert!(get_result.is_none());
    }

    #[tokio::test]
    async fn test_noop_cache_invalidate_succeeds() {
        let cache = NoOpCache::new();

        // Invalidate should succeed without error
        let result = cache.invalidate("test_key").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_noop_cache_invalidate_pattern_succeeds() {
        let cache = NoOpCache::new();

        // Invalidate pattern should succeed and return 0
        let count = cache.invalidate_pattern("test:*").await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_noop_cache_not_available() {
        let cache = NoOpCache::new();

        // NoOp cache should report as not available
        assert!(!cache.is_available());
    }
}
