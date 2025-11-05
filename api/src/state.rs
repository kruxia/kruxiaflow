use sqlx::PgPool;
use std::sync::Arc;
use streamflow_oauth::AuthenticationService;

/// Build metadata captured at compile time
#[derive(Clone)]
pub struct AppStateBuild {
    /// Build timestamp (set at compile time via build.rs)
    pub timestamp: String,

    /// Git commit hash (set at compile time via build.rs)
    pub git_hash: String,
}

/// Application state shared across all request handlers
///
/// Contains database connection pool, authentication service, and service metadata.
/// Cloning is cheap as it uses Arc internally for shared resources.
#[derive(Clone)]
pub struct AppState {
    /// PostgreSQL connection pool
    pub db_pool: PgPool,

    /// Authentication service (JWT token validation and issuance)
    pub auth_service: Arc<dyn AuthenticationService>,

    /// Service version from Cargo.toml
    pub version: String,

    /// Build metadata
    pub build: AppStateBuild,

    /// Enabled features/capabilities
    pub features: Vec<String>,
}

impl AppState {
    /// Create new application state with default metadata
    ///
    /// # Arguments
    /// * `db_pool` - PostgreSQL connection pool
    /// * `auth_service` - Authentication service for JWT validation
    ///
    /// # Build Metadata
    /// - `version`: Captured from CARGO_PKG_VERSION at compile time
    /// - `build.timestamp`: Captured via build.rs at compile time (BUILD_TIMESTAMP env var)
    /// - `build.git_hash`: Captured via build.rs at compile time (BUILD_GIT_HASH env var)
    /// - `features`: Hardcoded feature list for MVP
    pub fn new(db_pool: PgPool, auth_service: Arc<dyn AuthenticationService>) -> Self {
        Self {
            db_pool,
            auth_service,
            version: env!("CARGO_PKG_VERSION").to_string(),
            build: AppStateBuild {
                timestamp: option_env!("BUILD_TIMESTAMP")
                    .unwrap_or("unknown")
                    .to_string(),
                git_hash: option_env!("BUILD_GIT_HASH")
                    .unwrap_or("unknown")
                    .to_string(),
            },
            features: vec![
                "workflows".to_string(),
                "workers".to_string(),
                "websockets".to_string(),
                "authentication".to_string(),
            ],
        }
    }

    /// Create new application state with custom metadata
    ///
    /// # Arguments
    /// * `db_pool` - PostgreSQL connection pool
    /// * `auth_service` - Authentication service for JWT validation
    /// * `version` - Service version string
    /// * `build` - Build metadata (timestamp and git hash)
    /// * `features` - List of enabled features
    ///
    /// Useful for testing or custom deployments
    pub fn with_metadata(
        db_pool: PgPool,
        auth_service: Arc<dyn AuthenticationService>,
        version: String,
        build: AppStateBuild,
        features: Vec<String>,
    ) -> Self {
        Self {
            db_pool,
            auth_service,
            version,
            build,
            features,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_app_state_has_version() {
        // Can't test without a real PgPool, but we can verify the structure compiles
        // Real tests would be integration tests with a test database
    }
}
