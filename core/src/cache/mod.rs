//! Activity result caching service
//!
//! This module provides caching capabilities for activity execution results to reduce
//! costs and improve performance for repeated activity executions with identical parameters.
//!
//! ## Architecture
//!
//! - **CacheService**: Trait defining the caching interface
//! - **RedisCache**: Redis-backed implementation with TTL support (optional)
//! - **NoOpCache**: Fallback implementation when caching is disabled
//! - **CachedResult**: Wrapper for cached activity results with metadata
//!
//! ## Usage
//!
//! Caching is integrated at the ActivityRegistry execution layer, making it transparent
//! to individual activity implementations. All activity types (LLM, HTTP, PostgreSQL, etc.)
//! automatically benefit from caching when enabled via activity settings.

pub mod key_generator;
pub mod noop;

#[cfg(feature = "redis-cache")]
pub mod redis;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Cached activity result with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedResult {
    /// The cached output value
    pub output: serde_json::Value,
    /// When this cache entry was created
    pub cached_at: DateTime<Utc>,
    /// Original cost (for metrics/debugging)
    pub original_cost_usd: Option<Decimal>,
}

/// Cache service interface for activity result caching
///
/// This trait abstracts caching operations to support multiple backends
/// (Redis when available, NoOp fallback when not).
#[async_trait]
pub trait CacheService: Send + Sync {
    /// Get cached result by key
    ///
    /// Returns `Ok(Some(result))` on cache hit, `Ok(None)` on cache miss,
    /// and `Err` on cache service errors.
    async fn get(&self, key: &str) -> anyhow::Result<Option<CachedResult>>;

    /// Store result with TTL
    ///
    /// # Arguments
    ///
    /// * `key` - Cache key (typically SHA256 hash of activity name + parameters)
    /// * `result` - The activity result to cache
    /// * `ttl` - Time-to-live duration for automatic expiration
    async fn set(&self, key: &str, result: &CachedResult, ttl: Duration) -> anyhow::Result<()>;

    /// Invalidate cache entry by key
    ///
    /// Removes a specific cache entry. Succeeds silently if key doesn't exist.
    async fn invalidate(&self, key: &str) -> anyhow::Result<()>;

    /// Invalidate all cache entries matching pattern
    ///
    /// # Arguments
    ///
    /// * `pattern` - Pattern to match cache keys (e.g., "activity_name:*")
    ///
    /// # Returns
    ///
    /// Number of cache entries invalidated
    async fn invalidate_pattern(&self, pattern: &str) -> anyhow::Result<usize>;

    /// Check if cache is available/healthy
    ///
    /// Returns `true` if the cache backend is operational, `false` otherwise.
    /// For NoOpCache, this always returns `false` to indicate no actual caching.
    fn is_available(&self) -> bool;
}

// Re-export implementations
pub use noop::NoOpCache;

#[cfg(feature = "redis-cache")]
pub use redis::RedisCache;
