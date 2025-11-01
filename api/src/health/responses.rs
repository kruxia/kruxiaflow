use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Liveness probe response
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LivenessResponse {
    /// Server liveness status (always "ok" if endpoint responds)
    #[schema(example = "ok")]
    pub status: &'static str,
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
}
