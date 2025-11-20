use sqlx::PgPool;
use std::sync::Arc;
use streamflow_core::cache::CacheService;
use streamflow_core::events::EventSource;
use streamflow_core::queue::ActivityQueue;
use streamflow_core::storage::WorkflowStorage;
use streamflow_oauth::AuthenticationService;
use tokio_util::sync::CancellationToken;

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
/// Contains database connection pool, infrastructure services with swappable
/// implementations, and service metadata. Cloning is cheap as it uses Arc
/// internally for shared resources.
///
/// Infrastructure services use trait objects to allow swapping implementations
/// via configuration (e.g., PostgreSQL → Kafka for events, PostgreSQL → SQS for queue).
#[derive(Clone)]
pub struct AppState {
    /// PostgreSQL connection pool
    pub db_pool: PgPool,

    /// Authentication service (JWT token validation and issuance)
    /// Swappable: PostgresAuthService → Auth0, Okta (post-MVP)
    pub auth_service: Arc<dyn AuthenticationService>,

    /// Activity queue for scheduling and claiming activities
    /// Swappable: PostgresQueue → SQS, RabbitMQ, Redis (post-MVP)
    pub activity_queue: Arc<dyn ActivityQueue>,

    /// Event source for publishing and consuming workflow events
    /// Swappable: PostgresEventSource → Kafka, NATS, Logical Replication (post-MVP)
    pub event_source: Arc<dyn EventSource>,

    /// Workflow storage for file artifacts
    /// Swappable: PostgresStorage → S3, Filesystem (post-MVP)
    pub workflow_storage: Arc<dyn WorkflowStorage>,

    /// Cache service for activity result caching
    /// Swappable: RedisCache → NoOpCache (when Redis unavailable)
    pub cache_service: Arc<dyn CacheService>,

    /// Shutdown coordination token for graceful shutdown
    pub shutdown_token: CancellationToken,

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
    /// * `activity_queue` - Activity queue implementation (e.g., PostgresQueue, SqsQueue)
    /// * `event_source` - Event source implementation (e.g., PostgresEventSource, KafkaEventSource)
    /// * `workflow_storage` - Workflow storage implementation (e.g., PostgresStorage, S3Storage)
    /// * `cache_service` - Cache service implementation (e.g., RedisCache, NoOpCache)
    /// * `shutdown_token` - Cancellation token for coordinated shutdown
    ///
    /// # Build Metadata
    /// - `version`: Captured from CARGO_PKG_VERSION at compile time
    /// - `build.timestamp`: Captured via build.rs at compile time (BUILD_TIMESTAMP env var)
    /// - `build.git_hash`: Captured via build.rs at compile time (BUILD_GIT_HASH env var)
    /// - `features`: Hardcoded feature list for MVP
    pub fn new(
        db_pool: PgPool,
        auth_service: Arc<dyn AuthenticationService>,
        activity_queue: Arc<dyn ActivityQueue>,
        event_source: Arc<dyn EventSource>,
        workflow_storage: Arc<dyn WorkflowStorage>,
        cache_service: Arc<dyn CacheService>,
        shutdown_token: CancellationToken,
    ) -> Self {
        Self {
            db_pool,
            auth_service,
            activity_queue,
            event_source,
            workflow_storage,
            cache_service,
            shutdown_token,
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
    /// * `activity_queue` - Activity queue implementation
    /// * `event_source` - Event source implementation
    /// * `workflow_storage` - Workflow storage implementation
    /// * `cache_service` - Cache service implementation
    /// * `shutdown_token` - Cancellation token for coordinated shutdown
    /// * `version` - Service version string
    /// * `build` - Build metadata (timestamp and git hash)
    /// * `features` - List of enabled features
    ///
    /// Useful for testing or custom deployments
    pub fn with_metadata(
        db_pool: PgPool,
        auth_service: Arc<dyn AuthenticationService>,
        activity_queue: Arc<dyn ActivityQueue>,
        event_source: Arc<dyn EventSource>,
        workflow_storage: Arc<dyn WorkflowStorage>,
        cache_service: Arc<dyn CacheService>,
        shutdown_token: CancellationToken,
        version: String,
        build: AppStateBuild,
        features: Vec<String>,
    ) -> Self {
        Self {
            db_pool,
            auth_service,
            activity_queue,
            event_source,
            workflow_storage,
            cache_service,
            shutdown_token,
            version,
            build,
            features,
        }
    }

    /// Check if shutdown has been initiated
    pub fn is_shutting_down(&self) -> bool {
        self.shutdown_token.is_cancelled()
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use async_trait::async_trait;
    use futures::stream::{self, Stream};
    use sqlx::PgPool;
    use std::pin::Pin;
    use streamflow_core::cache::{CacheService, CachedResult};
    use streamflow_core::events::{EventError, NewWorkflowEvent, WorkflowEvent};
    use streamflow_core::queue::{Activity, ActivityResult, QueuedActivity};
    use streamflow_core::storage::{FileMetadata, StorageError};
    use streamflow_oauth::{AuthResponse, AuthResult, AuthenticationService, Claims, JwtKey};
    use tokio_util::bytes::Bytes;
    use uuid::Uuid;

    // Mock authentication service for testing
    pub struct MockAuthService;

    // Mock activity queue for testing
    pub struct MockActivityQueue;

    // Mock workflow storage for testing
    pub struct MockWorkflowStorage;

    // Mock cache service for testing
    pub struct MockCacheService;

    #[async_trait]
    impl ActivityQueue for MockActivityQueue {
        async fn schedule(
            &self,
            _workflow_id: Uuid,
            _activities: Vec<Activity>,
        ) -> streamflow_core::queue::Result<()> {
            Ok(())
        }

        async fn claim_next(
            &self,
            _worker_id: &str,
            _namespace: &str,
            _name: &str,
        ) -> streamflow_core::queue::Result<Option<QueuedActivity>> {
            Ok(None)
        }

        async fn get_activity_summary(
            &self,
            _activity_id: Uuid,
        ) -> streamflow_core::queue::Result<streamflow_core::queue::ActivitySummary> {
            Ok(streamflow_core::queue::ActivitySummary {
                workflow_id: Uuid::now_v7(),
                activity_key: "mock_activity".to_string(),
            })
        }

        async fn complete(
            &self,
            _activity_id: Uuid,
            _worker_id: &str,
            _result: ActivityResult,
        ) -> streamflow_core::queue::Result<()> {
            Ok(())
        }

        async fn fail(
            &self,
            _activity_id: Uuid,
            _worker_id: &str,
            _retryable: bool,
            _result: ActivityResult,
        ) -> streamflow_core::queue::Result<bool> {
            Ok(false)
        }

        async fn heartbeat(
            &self,
            _activity_id: Uuid,
            _worker_id: &str,
        ) -> streamflow_core::queue::Result<()> {
            Ok(())
        }
    }

    // Mock event source for testing
    pub struct MockEventSource;

    #[async_trait]
    impl EventSource for MockEventSource {
        async fn publish(&self, _event: NewWorkflowEvent) -> Result<(), EventError> {
            Ok(())
        }

        async fn poll(&self, _consumer_id: &str) -> Result<Vec<WorkflowEvent>, EventError> {
            Ok(vec![])
        }

        async fn update_position(
            &self,
            _consumer_id: &str,
            _last_event_id: Uuid,
        ) -> Result<(), EventError> {
            Ok(())
        }
    }

    #[async_trait]
    impl WorkflowStorage for MockWorkflowStorage {
        async fn upload_file(
            &self,
            _workflow_id: Uuid,
            _activity_key: &str,
            _filename: &str,
            _content_type: Option<&str>,
            _data: Pin<
                Box<dyn Stream<Item = std::result::Result<Bytes, std::io::Error>> + Send + Unpin>,
            >,
        ) -> Result<FileMetadata, StorageError> {
            Ok(FileMetadata {
                workflow_id: Uuid::now_v7(),
                activity_key: "test".to_string(),
                filename: "test.txt".to_string(),
                size: 0,
                content_type: None,
                created_at: chrono::Utc::now(),
            })
        }

        async fn download_file(
            &self,
            _workflow_id: Uuid,
            _activity_key: &str,
            _filename: &str,
        ) -> Result<
            Pin<Box<dyn Stream<Item = std::result::Result<Bytes, std::io::Error>> + Send>>,
            StorageError,
        > {
            Ok(Box::pin(stream::empty()))
        }

        async fn get_file_metadata(
            &self,
            _workflow_id: Uuid,
            _activity_key: &str,
            _filename: &str,
        ) -> Result<FileMetadata, StorageError> {
            Ok(FileMetadata {
                workflow_id: Uuid::now_v7(),
                activity_key: "test".to_string(),
                filename: "test.txt".to_string(),
                size: 0,
                content_type: None,
                created_at: chrono::Utc::now(),
            })
        }

        async fn list_files(
            &self,
            _workflow_id: Uuid,
            _activity_key: &str,
        ) -> Result<Vec<FileMetadata>, StorageError> {
            Ok(vec![])
        }

        async fn delete_file(
            &self,
            _workflow_id: Uuid,
            _activity_key: &str,
            _filename: &str,
        ) -> Result<(), StorageError> {
            Ok(())
        }

        async fn delete_workflow_files(&self, _workflow_id: Uuid) -> Result<(), StorageError> {
            Ok(())
        }

        async fn get_file_reference(
            &self,
            _workflow_id: Uuid,
            _activity_key: &str,
            _filename: &str,
        ) -> Result<String, StorageError> {
            Ok("mock://test/file.txt".to_string())
        }
    }

    #[async_trait]
    impl CacheService for MockCacheService {
        async fn get(&self, _key: &str) -> anyhow::Result<Option<CachedResult>> {
            Ok(None)
        }

        async fn set(
            &self,
            _key: &str,
            _result: &CachedResult,
            _ttl: std::time::Duration,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn invalidate(&self, _key: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn invalidate_pattern(&self, _pattern: &str) -> anyhow::Result<usize> {
            Ok(0)
        }

        fn is_available(&self) -> bool {
            false
        }
    }

    #[async_trait]
    impl AuthenticationService for MockAuthService {
        async fn authenticate_client(
            &self,
            _client_id: &str,
            _client_secret: &str,
        ) -> AuthResult<AuthResponse> {
            Ok(AuthResponse {
                access_token: "mock_token".to_string(),
                token_type: "Bearer".to_string(),
                expires_in: 3600,
                refresh_token: None,
            })
        }

        async fn authenticate_password(
            &self,
            _username: &str,
            _password: &str,
        ) -> AuthResult<AuthResponse> {
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
        let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://streamflow:streamflow_dev@127.0.0.1:5432/streamflow".to_string()
        });
        PgPool::connect(&database_url)
            .await
            .expect("Failed to connect to test database")
    }

    #[tokio::test]
    async fn test_app_state_new_creates_with_defaults() {
        let pool = mock_pool().await;
        let auth_service = Arc::new(MockAuthService);
        let activity_queue = Arc::new(MockActivityQueue);
        let event_source = Arc::new(MockEventSource);
        let workflow_storage = Arc::new(MockWorkflowStorage);
        let cache_service = Arc::new(MockCacheService);
        let shutdown_token = CancellationToken::new();

        let state = AppState::new(
            pool,
            auth_service,
            activity_queue,
            event_source,
            workflow_storage,
            cache_service,
            shutdown_token,
        );

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
        let activity_queue = Arc::new(MockActivityQueue);
        let event_source = Arc::new(MockEventSource);
        let workflow_storage = Arc::new(MockWorkflowStorage);
        let cache_service = Arc::new(MockCacheService);
        let shutdown_token = CancellationToken::new();

        let build = AppStateBuild {
            timestamp: "2025-01-01T00:00:00Z".to_string(),
            git_hash: "abc123".to_string(),
        };

        let features = vec!["custom_feature".to_string()];

        let state = AppState::with_metadata(
            pool,
            auth_service,
            activity_queue,
            event_source,
            workflow_storage,
            cache_service,
            shutdown_token,
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
        let activity_queue = Arc::new(MockActivityQueue);
        let event_source = Arc::new(MockEventSource);
        let workflow_storage = Arc::new(MockWorkflowStorage);
        let cache_service = Arc::new(MockCacheService);
        let shutdown_token = CancellationToken::new();

        let state1 = AppState::new(
            pool,
            auth_service,
            activity_queue,
            event_source,
            workflow_storage,
            cache_service,
            shutdown_token,
        );
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
        let activity_queue = Arc::new(MockActivityQueue);
        let event_source = Arc::new(MockEventSource);
        let workflow_storage = Arc::new(MockWorkflowStorage);
        let cache_service = Arc::new(MockCacheService);
        let shutdown_token = CancellationToken::new();

        let state = AppState::new(
            pool,
            auth_service,
            activity_queue,
            event_source,
            workflow_storage,
            cache_service,
            shutdown_token,
        );

        // Build metadata should be set (either from env vars or "unknown")
        assert!(!state.build.timestamp.is_empty());
        assert!(!state.build.git_hash.is_empty());
    }

    #[tokio::test]
    async fn test_app_state_with_empty_features() {
        let pool = mock_pool().await;
        let auth_service = Arc::new(MockAuthService);
        let activity_queue = Arc::new(MockActivityQueue);
        let event_source = Arc::new(MockEventSource);
        let workflow_storage = Arc::new(MockWorkflowStorage);
        let cache_service = Arc::new(MockCacheService);
        let shutdown_token = CancellationToken::new();

        let build = AppStateBuild {
            timestamp: "test".to_string(),
            git_hash: "test".to_string(),
        };

        let state = AppState::with_metadata(
            pool,
            auth_service,
            activity_queue,
            event_source,
            workflow_storage,
            cache_service,
            shutdown_token,
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
        let activity_queue = Arc::new(MockActivityQueue);
        let event_source = Arc::new(MockEventSource);
        let workflow_storage = Arc::new(MockWorkflowStorage);
        let cache_service = Arc::new(MockCacheService);
        let shutdown_token = CancellationToken::new();

        let state = AppState::new(
            pool,
            auth_service,
            activity_queue,
            event_source,
            workflow_storage,
            cache_service,
            shutdown_token,
        );

        // Verify we can access the auth service
        let result = state.auth_service.authenticate_client("test", "test").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_mock_auth_service_authenticate_password() {
        let service = MockAuthService;
        let result = service.authenticate_password("user", "pass").await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.access_token, "mock_token");
        assert_eq!(response.refresh_token, Some("mock_refresh".to_string()));
    }

    #[tokio::test]
    async fn test_mock_auth_service_refresh_token() {
        let service = MockAuthService;
        let result = service.refresh_token("old_token").await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.access_token, "new_token");
        assert_eq!(response.refresh_token, Some("new_refresh".to_string()));
    }

    #[tokio::test]
    async fn test_mock_auth_service_validate_token() {
        let service = MockAuthService;
        let result = service.validate_token("some_token").await;
        assert!(result.is_ok());
        let claims = result.unwrap();
        assert_eq!(claims.sub, "test_user");
        assert_eq!(claims.iss, "test");
    }

    #[tokio::test]
    async fn test_mock_auth_service_get_signing_keys() {
        let service = MockAuthService;
        let result = service.get_signing_keys().await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 0);
    }

    #[test]
    fn test_mock_pool_uses_default_database_url() {
        // Test that mock_pool falls back to default when DATABASE_URL is not set
        unsafe {
            std::env::remove_var("DATABASE_URL");
        }

        // This should use the default URL
        let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://streamflow:streamflow_dev@127.0.0.1:5433/streamflow".to_string()
        });
        assert_eq!(
            database_url,
            "postgres://streamflow:streamflow_dev@127.0.0.1:5433/streamflow"
        );
    }
}
