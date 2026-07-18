use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Liveness probe response
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LivenessResponse {
    /// Server liveness status (always "ok" if endpoint responds)
    #[schema(example = "ok")]
    pub status: &'static str,

    /// Present (true) only when the server runs in insecure dev mode
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub insecure_dev: bool,
}

/// Individual health check status
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct HealthCheckStatus {
    /// Database health status
    #[schema(example = "ok")]
    pub database: &'static str,

    /// Event source health status
    #[schema(example = "ok")]
    pub event_source: &'static str,

    /// Activity queue health status
    #[schema(example = "ok")]
    pub queue: &'static str,
}

/// Readiness probe response
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReadinessResponse {
    /// Overall readiness status
    #[schema(example = "ready")]
    pub status: &'static str,

    /// Individual health check results
    pub checks: HealthCheckStatus,
}

/// Service information response
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ServiceInfo {
    /// Service version
    #[schema(example = "0.2.0")]
    pub version: String,

    /// Build timestamp
    #[schema(example = "2025-10-31T10:00:00Z")]
    pub build_timestamp: String,

    /// Git commit hash
    #[schema(example = "abc123def")]
    pub build_git_hash: Option<String>,

    /// API version
    #[schema(example = "v1")]
    pub api_version: String,

    /// Enabled features
    pub features: Vec<String>,

    /// Present (true) only when the server runs in insecure dev mode
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub insecure_dev: bool,
}

/// Connection pool metrics response
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PoolMetricsResponse {
    /// Total connections in the pool
    #[schema(example = 10)]
    pub size: u32,

    /// Idle connections available
    #[schema(example = 5)]
    pub idle: u32,

    /// Active connections in use
    #[schema(example = 5)]
    pub active: u32,

    /// Maximum configured connections
    #[schema(example = 20)]
    pub max_connections: u32,

    /// Pool utilization percentage
    #[schema(example = 50.0)]
    pub utilization_percent: f64,

    /// Pool health status
    #[schema(example = "healthy")]
    pub status: &'static str,
}
