use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

/// Liveness probe response
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LivenessResponse {
    /// Server liveness status (always "ok" if endpoint responds)
    #[schema(example = "ok")]
    pub status: String,
}

/// Readiness probe response
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReadinessResponse {
    /// Overall readiness status
    #[schema(example = "ready")]
    pub status: String,

    /// Individual health check results
    pub checks: HashMap<String, String>,
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
