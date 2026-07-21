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

/// Health of one readiness component
///
/// This object shape (`status` + optional `message`) is the contract the
/// `kruxiaflow health` CLI parses — container healthchecks depend on it.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ComponentHealth {
    /// Component status: "healthy" or "unhealthy"
    #[schema(example = "healthy")]
    pub status: String,

    /// Human-readable detail (e.g., the failure reason or lag)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "caught up")]
    pub message: Option<String>,
}

impl ComponentHealth {
    pub fn healthy(message: Option<String>) -> Self {
        Self {
            status: "healthy".to_string(),
            message,
        }
    }

    pub fn unhealthy(message: String) -> Self {
        Self {
            status: "unhealthy".to_string(),
            message: Some(message),
        }
    }

    pub fn is_healthy(&self) -> bool {
        self.status == "healthy"
    }
}

/// Individual health check status
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct HealthCheckStatus {
    /// Database health status
    pub database: ComponentHealth,

    /// Event source health status
    pub event_source: ComponentHealth,

    /// Activity queue health status
    pub queue: ComponentHealth,

    /// Orchestrator health status (event-consumption freshness). Reported for
    /// visibility but does NOT gate the HTTP readiness status: in distributed
    /// deployments the API server must not leave rotation because a separate
    /// orchestrator deployment is down. The `kruxiaflow health` CLI folds it
    /// into its overall verdict, which is the right behavior for the
    /// all-in-one container.
    pub orchestrator: ComponentHealth,
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
