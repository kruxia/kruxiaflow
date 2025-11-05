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
    use super::*;
    use sqlx::PgPool;
    use streamflow_oauth::{AuthenticationService, AuthResult, AuthResponse, Claims, JwtKey};
    use async_trait::async_trait;

    // Mock authentication service for testing
    struct MockAuthService;

    #[async_trait]
    impl AuthenticationService for MockAuthService {
        async fn authenticate_client(&self, _client_id: &str, _client_secret: &str) -> AuthResult<AuthResponse> {
            Ok(AuthResponse {
                access_token: "mock_token".to_string(),
                token_type: "Bearer".to_string(),
                expires_in: 3600,
                refresh_token: None,
            })
        }

        async fn authenticate_password(&self, _username: &str, _password: &str) -> AuthResult<AuthResponse> {
            Ok(AuthResponse {
                access_token: "mock_token".to_string(),
                token_type: "Bearer".to_string(),
                expires_in: 3600,
                refresh_token: Some("mock_refresh".to_string()),
            })
        }

        async fn refresh_token(&self, _refresh_token: &str) -> AuthResult<AuthResponse> {
            Ok(AuthResponse {
                access_token: "new_token".to_string(),
                token_type: "Bearer".to_string(),
                expires_in: 3600,
                refresh_token: Some("new_refresh".to_string()),
            })
        }

        async fn validate_token(&self, _token: &str) -> AuthResult<Claims> {
            Ok(Claims {
                sub: "test_user".to_string(),
                jti: "test_jti".to_string(),
                iss: "test".to_string(),
                aud: "test".to_string(),
                exp: 9999999999,
                iat: 1000000000,
            })
        }

        async fn get_signing_keys(&self) -> AuthResult<Vec<JwtKey>> {
            Ok(vec![])
        }
    }

    async fn mock_pool() -> PgPool {
        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://streamflow:streamflow_dev@127.0.0.1:5433/streamflow".to_string());
        PgPool::connect(&database_url).await.expect("Failed to connect to test database")
    }

    #[tokio::test]
    async fn test_app_state_new_creates_with_defaults() {
        let pool = mock_pool().await;
        let auth_service = Arc::new(MockAuthService);

        let state = AppState::new(pool, auth_service);

        assert_eq!(state.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(state.features.len(), 4);
        assert!(state.features.contains(&"workflows".to_string()));
        assert!(state.features.contains(&"workers".to_string()));
        assert!(state.features.contains(&"websockets".to_string()));
        assert!(state.features.contains(&"authentication".to_string()));
    }

    #[tokio::test]
    async fn test_app_state_with_metadata_uses_custom_values() {
        let pool = mock_pool().await;
        let auth_service = Arc::new(MockAuthService);

        let build = AppStateBuild {
            timestamp: "2025-01-01T00:00:00Z".to_string(),
            git_hash: "abc123".to_string(),
        };

        let features = vec!["custom_feature".to_string()];

        let state = AppState::with_metadata(
            pool,
            auth_service,
            "1.2.3".to_string(),
            build.clone(),
            features.clone(),
        );

        assert_eq!(state.version, "1.2.3");
        assert_eq!(state.build.timestamp, "2025-01-01T00:00:00Z");
        assert_eq!(state.build.git_hash, "abc123");
        assert_eq!(state.features, features);
    }

    #[tokio::test]
    async fn test_app_state_clone() {
        let pool = mock_pool().await;
        let auth_service = Arc::new(MockAuthService);

        let state1 = AppState::new(pool, auth_service);
        let state2 = state1.clone();

        assert_eq!(state1.version, state2.version);
        assert_eq!(state1.build.timestamp, state2.build.timestamp);
        assert_eq!(state1.build.git_hash, state2.build.git_hash);
        assert_eq!(state1.features, state2.features);
    }

    #[tokio::test]
    async fn test_app_state_build_clone() {
        let build1 = AppStateBuild {
            timestamp: "2025-01-01T00:00:00Z".to_string(),
            git_hash: "abc123".to_string(),
        };

        let build2 = build1.clone();

        assert_eq!(build1.timestamp, build2.timestamp);
        assert_eq!(build1.git_hash, build2.git_hash);
    }

    #[tokio::test]
    async fn test_app_state_new_captures_build_metadata() {
        let pool = mock_pool().await;
        let auth_service = Arc::new(MockAuthService);

        let state = AppState::new(pool, auth_service);

        // Build metadata should be set (either from env vars or "unknown")
        assert!(!state.build.timestamp.is_empty());
        assert!(!state.build.git_hash.is_empty());
    }

    #[tokio::test]
    async fn test_app_state_with_empty_features() {
        let pool = mock_pool().await;
        let auth_service = Arc::new(MockAuthService);

        let build = AppStateBuild {
            timestamp: "test".to_string(),
            git_hash: "test".to_string(),
        };

        let state = AppState::with_metadata(
            pool,
            auth_service,
            "1.0.0".to_string(),
            build,
            vec![],
        );

        assert_eq!(state.features.len(), 0);
    }

    #[tokio::test]
    async fn test_app_state_auth_service_accessible() {
        let pool = mock_pool().await;
        let auth_service = Arc::new(MockAuthService);

        let state = AppState::new(pool, auth_service);

        // Verify we can access the auth service
        let result = state.auth_service.authenticate_client("test", "test").await;
        assert!(result.is_ok());
    }
}
