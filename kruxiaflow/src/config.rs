use anyhow::{Context, Result};
use clap::Args;
use kruxiaflow_core::cache::CacheService;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;

/// Cache configuration
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Cache provider: "redis" or "noop"
    pub provider: String,

    /// Redis connection URL (used when provider=redis)
    pub redis_url: Option<String>,

    /// Redis key prefix for namespace isolation
    pub redis_key_prefix: Option<String>,
}

impl CacheConfig {
    /// Create CacheConfig with precedence: Environment variables > Defaults
    pub fn new() -> Self {
        let provider = std::env::var("KRUXIAFLOW_CACHE_PROVIDER")
            .unwrap_or_else(|_| "noop".to_string())
            .to_lowercase();

        let redis_url = std::env::var("KRUXIAFLOW_REDIS_URL").ok();

        let redis_key_prefix = std::env::var("KRUXIAFLOW_REDIS_KEY_PREFIX").ok();

        Self {
            provider,
            redis_url,
            redis_key_prefix,
        }
    }

    /// Create cache service based on configuration
    pub async fn create_cache_service(&self) -> Arc<dyn CacheService> {
        match self.provider.as_str() {
            "redis" => self.create_redis_cache().await,
            _ => {
                tracing::info!("Cache disabled (using NoOpCache)");
                Arc::new(kruxiaflow_core::NoOpCache::new())
            }
        }
    }

    #[cfg(feature = "redis-cache")]
    async fn create_redis_cache(&self) -> Arc<dyn CacheService> {
        let redis_url = self
            .redis_url
            .as_deref()
            .unwrap_or("redis://localhost:6379");

        match kruxiaflow_core::RedisCache::new(redis_url, self.redis_key_prefix.clone()) {
            Ok(cache) => {
                // Test connectivity
                match cache.ping().await {
                    Ok(_) => {
                        tracing::info!(
                            redis_url = %self.redact_redis_url(redis_url),
                            "Redis cache initialized successfully"
                        );
                        Arc::new(cache)
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "Redis ping failed, falling back to NoOpCache"
                        );
                        Arc::new(kruxiaflow_core::NoOpCache::new())
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "Failed to create Redis cache, falling back to NoOpCache"
                );
                Arc::new(kruxiaflow_core::NoOpCache::new())
            }
        }
    }

    #[cfg(not(feature = "redis-cache"))]
    async fn create_redis_cache(&self) -> Arc<dyn CacheService> {
        tracing::warn!(
            "Redis caching requested but redis-cache feature not enabled, falling back to NoOpCache"
        );
        Arc::new(kruxiaflow_core::NoOpCache::new())
    }

    /// Redact password from Redis URL for logging
    fn redact_redis_url(&self, url: &str) -> String {
        // Format: redis://[:password@]host:port[/db]
        if let Some(at_pos) = url.find('@')
            && let Some(colon_pos) = url[..at_pos].rfind(':')
        {
            let mut redacted = url.to_string();
            redacted.replace_range(colon_pos + 1..at_pos, "***");
            return redacted;
        }
        url.to_string()
    }

    /// Log configuration
    pub fn log_config(&self) {
        tracing::info!("Cache Configuration:");
        tracing::info!("  Provider: {}", self.provider);
        if let Some(url) = &self.redis_url {
            tracing::info!("  Redis URL: {}", self.redact_redis_url(url));
        }
        if let Some(prefix) = &self.redis_key_prefix {
            tracing::info!("  Key prefix: {}", prefix);
        }
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// True if the bind address is loopback (127.0.0.0/8, ::1, or "localhost")
pub fn is_loopback_bind(bind: &str) -> bool {
    if bind.eq_ignore_ascii_case("localhost") {
        return true;
    }
    bind.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

/// Database connection pool sizing for the long-running commands (`serve`,
/// `api`, `orchestrator`, `worker`). Each command supplies its own defaults;
/// these options override them, e.g. to shrink the pool on small hosts.
#[derive(Args, Debug, Clone, Default)]
pub struct DbPoolArgs {
    /// Maximum database pool connections
    #[arg(
        long,
        env = "KRUXIAFLOW_DB_MAX_CONNECTIONS",
        help = "Maximum database pool connections (default depends on command)",
        long_help = "Maximum number of connections in the database pool.\n\
Keep this below PostgreSQL's max_connections minus\n\
superuser_reserved_connections.\n\n\
Defaults: serve 100, api 200, orchestrator 50, worker 10\n\
Example: --db-max-connections 15"
    )]
    pub db_max_connections: Option<u32>,

    /// Minimum (idle) database pool connections
    #[arg(
        long,
        env = "KRUXIAFLOW_DB_MIN_CONNECTIONS",
        help = "Minimum idle database pool connections (default depends on command)",
        long_help = "Number of connections the database pool keeps open while idle.\n\
Each idle connection is a PostgreSQL backend process (~2MB).\n\
If only --db-max-connections is set, the default minimum is\n\
capped to it.\n\n\
Defaults: serve 10, api 20, orchestrator 10, worker 2\n\
Example: --db-min-connections 2"
    )]
    pub db_min_connections: Option<u32>,
}

impl DbPoolArgs {
    /// Resolve `(max, min)` pool sizes against a command's defaults.
    pub fn resolve(&self, default_max: u32, default_min: u32) -> Result<(u32, u32)> {
        let max = self.db_max_connections.unwrap_or(default_max);
        if max == 0 {
            anyhow::bail!("Database max connections must be at least 1");
        }
        let min = match self.db_min_connections {
            Some(min) if min > max => anyhow::bail!(
                "Database min connections ({}) cannot exceed max connections ({})",
                min,
                max
            ),
            Some(min) => min,
            None => default_min.min(max),
        };
        Ok((max, min))
    }

    /// Build pool options sized from these args and a command's defaults.
    pub fn pool_options(&self, default_max: u32, default_min: u32) -> Result<PgPoolOptions> {
        let (max, min) = self.resolve(default_max, default_min)?;
        tracing::info!(
            max_connections = max,
            min_connections = min,
            "Database pool configuration"
        );
        Ok(PgPoolOptions::new()
            .max_connections(max)
            .min_connections(min))
    }
}

/// How event consumers learn that new workflow events have committed
#[derive(clap::ValueEnum, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EventWake {
    /// LISTEN for commit notifications; the poll timer stays as a safety net
    #[default]
    Notify,
    /// Poll on the backoff timer alone
    Poll,
}

/// Event wake-up selection for the commands that consume workflow events
/// (`serve`, `api`, `orchestrator`).
#[derive(Args, Debug, Clone, Default)]
pub struct EventWakeArgs {
    /// Event wake mode
    #[arg(
        long,
        env = "KRUXIAFLOW_EVENT_WAKE",
        value_enum,
        default_value_t = EventWake::Notify,
        help = "How event consumers learn of new events: notify or poll",
        long_help = "How event consumers learn that new workflow events have committed\n\n\
notify: LISTEN on the workflow_events channel and poll as soon as a\n\
commit notification arrives; the poll timer stays as a safety net.\n\
Falls back to poll (with a warning) if LISTEN fails, e.g. behind a\n\
transaction-mode connection pooler.\n\
poll: poll on the backoff timer alone.\n\n\
Delivery never depends on notifications: every consumer reads from its\n\
durable cursor in both modes.\n\n\
Default: notify\n\
Example: --event-wake poll"
    )]
    pub event_wake: EventWake,
}

impl EventWakeArgs {
    /// The core wake mode for these args
    pub fn mode(&self) -> kruxiaflow_core::EventWakeMode {
        match self.event_wake {
            EventWake::Notify => kruxiaflow_core::EventWakeMode::Notify,
            EventWake::Poll => kruxiaflow_core::EventWakeMode::Poll,
        }
    }

    /// Build the PostgreSQL event source in this mode (logs which mode is
    /// active). Fails if the stored event cursors cannot belong to this
    /// database cluster; the error says how to repair them.
    pub async fn event_source(
        &self,
        pool: sqlx::PgPool,
    ) -> Result<kruxiaflow_core::PostgresEventSource> {
        kruxiaflow_core::PostgresEventSource::connect(pool, self.mode())
            .await
            .context("Cannot start consuming workflow events")
    }
}

/// How a worker learns that work may be available
#[derive(clap::ValueEnum, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WorkerWake {
    /// Listen on the API server's wake stream; the poll timer stays as a safety net
    #[default]
    Sse,
    /// Poll on the timer alone
    Poll,
}

/// Worker wake-up selection for the commands that run a worker (`serve`'s
/// built-in worker, `worker`).
#[derive(Args, Debug, Clone)]
pub struct WorkerWakeArgs {
    /// Worker wake mode
    #[arg(
        long,
        env = "KRUXIAFLOW_WORKER_WAKE",
        value_enum,
        default_value_t = WorkerWake::Sse,
        help = "How the worker learns of new activities: sse or poll",
        long_help = "How the worker learns that an activity may be claimable\n\n\
sse: keep a server-sent event stream open to the API server\n\
(GET /api/v1/workers/wake) and poll as soon as it says an activity was\n\
scheduled. While the stream is connected the poll timer is only a safety\n\
net (--worker-wake-poll-interval). Whenever it is not connected, the\n\
worker polls at its normal interval.\n\
poll: poll on the timer alone.\n\n\
Behind a proxy that buffers responses, use poll or disable buffering for\n\
the wake path.\n\n\
Default: sse\n\
Example: --worker-wake poll"
    )]
    pub worker_wake: WorkerWake,

    /// Idle poll interval in seconds while the wake stream is connected
    #[arg(
        long,
        env = "KRUXIAFLOW_WORKER_WAKE_POLL_INTERVAL",
        default_value_t = 5.0,
        help = "Idle poll interval in seconds while the wake stream is connected",
        long_help = "How long the worker waits between polls while its wake stream is\n\
connected and nothing is happening. The stream normally triggers polls;\n\
this timer only catches a lost wake, and a running activity whose timeout\n\
expired.\n\n\
Default: 5\n\
Example: --worker-wake-poll-interval 2"
    )]
    pub worker_wake_poll_interval: f64,
}

impl Default for WorkerWakeArgs {
    fn default() -> Self {
        Self {
            worker_wake: WorkerWake::Sse,
            worker_wake_poll_interval: 5.0,
        }
    }
}

impl WorkerWakeArgs {
    /// Reject a non-positive or non-finite interval
    pub fn validate(&self) -> Result<()> {
        if !self.worker_wake_poll_interval.is_finite() || self.worker_wake_poll_interval <= 0.0 {
            anyhow::bail!("Worker wake poll interval must be greater than 0 seconds");
        }
        Ok(())
    }

    /// The SDK wake mode for these args
    pub fn mode(&self) -> kruxiaflow_std_worker::WakeMode {
        match self.worker_wake {
            WorkerWake::Sse => kruxiaflow_std_worker::WakeMode::Sse,
            WorkerWake::Poll => kruxiaflow_std_worker::WakeMode::Poll,
        }
    }

    /// The idle poll interval while the wake stream is connected
    pub fn poll_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f64(self.worker_wake_poll_interval)
    }
}

/// Where workflow file content is stored
#[derive(clap::ValueEnum, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StorageProvider {
    /// PostgreSQL Large Objects (no extra service needed)
    #[default]
    #[value(alias = "postgres")]
    Postgresql,
    /// S3-compatible object storage (AWS S3, MinIO, Cloudflare R2, ...)
    S3,
}

/// Workflow file storage selection for the commands that read or write
/// workflow files (`serve`, `api`, `worker`). File metadata always lives in
/// PostgreSQL; the provider decides where file content goes.
#[derive(Args, Debug, Clone, Default)]
pub struct StorageArgs {
    /// Storage provider for workflow file content
    #[arg(
        long,
        env = "KRUXIAFLOW_STORAGE_PROVIDER",
        value_enum,
        default_value_t = StorageProvider::Postgresql,
        help = "Storage provider for workflow files: postgresql or s3",
        long_help = "Where workflow file content is stored.\n\
File metadata stays in PostgreSQL for every provider, and files written\n\
by the postgresql provider stay readable after switching to s3.\n\n\
postgresql: PostgreSQL Large Objects (default, no extra service)\n\
s3: S3-compatible object storage; requires --storage-s3-bucket.\n    \
Credentials come from AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY,\n    \
web identity, ECS task roles, or EC2 instance metadata."
    )]
    pub storage_provider: StorageProvider,

    /// S3 bucket name
    #[arg(
        long,
        env = "KRUXIAFLOW_STORAGE_S3_BUCKET",
        help = "S3 bucket for workflow files (required with --storage-provider s3)"
    )]
    pub storage_s3_bucket: Option<String>,

    /// S3 region
    #[arg(
        long,
        env = "KRUXIAFLOW_STORAGE_S3_REGION",
        help = "S3 region (default: AWS_REGION, AWS_DEFAULT_REGION, then us-east-1)"
    )]
    pub storage_s3_region: Option<String>,

    /// S3 endpoint for non-AWS services
    #[arg(
        long,
        env = "KRUXIAFLOW_STORAGE_S3_ENDPOINT",
        help = "S3 endpoint URL for S3-compatible services, e.g. http://minio:9000",
        long_help = "Endpoint URL for S3-compatible services such as MinIO,\n\
Cloudflare R2, or Ceph RGW. Omit for AWS S3.\n\
Plain http:// endpoints are allowed.\n\n\
Example: --storage-s3-endpoint http://minio:9000"
    )]
    pub storage_s3_endpoint: Option<String>,

    /// Key prefix for all objects
    #[arg(
        long,
        env = "KRUXIAFLOW_STORAGE_S3_PREFIX",
        help = "Key prefix for workflow file objects, e.g. kruxiaflow/artifacts"
    )]
    pub storage_s3_prefix: Option<String>,
}

impl StorageArgs {
    /// Build the configured workflow storage. Fails on missing or invalid
    /// settings; no request is made to the storage service.
    pub fn create_storage(
        &self,
        pool: sqlx::PgPool,
    ) -> Result<Arc<dyn kruxiaflow_core::WorkflowStorage>> {
        match self.storage_provider {
            StorageProvider::Postgresql => {
                tracing::info!("Workflow storage: PostgreSQL Large Objects");
                Ok(Arc::new(kruxiaflow_core::PostgresStorage::new(pool)))
            }
            StorageProvider::S3 => {
                let bucket = self.storage_s3_bucket.clone().context(
                    "--storage-provider s3 requires --storage-s3-bucket \
                     (or KRUXIAFLOW_STORAGE_S3_BUCKET)",
                )?;
                let config = kruxiaflow_core::S3Config {
                    bucket,
                    region: self.storage_s3_region.clone(),
                    endpoint: self.storage_s3_endpoint.clone(),
                    prefix: self.storage_s3_prefix.clone(),
                };
                let storage = kruxiaflow_core::S3Storage::new(pool, &config)
                    .context("Failed to configure S3 workflow storage")?;
                tracing::info!(
                    bucket = %config.bucket,
                    region = config.region.as_deref().unwrap_or("(from AWS environment)"),
                    endpoint = config.endpoint.as_deref().unwrap_or("(AWS)"),
                    prefix = config.prefix.as_deref().unwrap_or(""),
                    "Workflow storage: S3-compatible object storage"
                );
                Ok(Arc::new(storage))
            }
        }
    }
}

/// Sandboxed code execution (`run_code` on Pydantic Monty) for the commands
/// that run a worker (`serve`'s built-in worker, `worker`). Compiled in with
/// the `monty` feature; enabled at run time by `--monty-binary`.
#[cfg(feature = "monty")]
#[derive(Args, Debug, Clone)]
pub struct MontyArgs {
    /// Path to the `monty` worker binary; enables `std.run_code`
    #[arg(
        long,
        env = "KRUXIAFLOW_MONTY_BINARY",
        help = "Absolute path to the monty worker binary; enables the run_code activity",
        long_help = "Absolute path to the `monty` worker binary (crate monty-runtime, the\n\
same version as this build). Setting it enables the std.run_code activity,\n\
which runs untrusted Python in pooled Monty sandbox subprocesses. The path is\n\
never looked up on PATH, and the binary's version is checked at start-up.\n\n\
Install: cargo install monty-runtime --version 1.6.0 --locked\n\
Default: unset (run_code disabled)\n\
Example: --monty-binary /usr/local/bin/monty"
    )]
    pub monty_binary: Option<std::path::PathBuf>,

    /// Prewarmed Monty workers
    #[arg(
        long,
        env = "KRUXIAFLOW_MONTY_MIN_PROCESSES",
        default_value_t = 1,
        help = "Monty worker processes kept warm"
    )]
    pub monty_min_processes: usize,

    /// Upper bound on live Monty workers
    #[arg(
        long,
        env = "KRUXIAFLOW_MONTY_MAX_PROCESSES",
        help = "Most Monty worker processes alive at once (default: the worker's activity concurrency)"
    )]
    pub monty_max_processes: Option<usize>,

    /// Host-side kill switch for any single request to a Monty worker
    #[arg(
        long,
        env = "KRUXIAFLOW_MONTY_REQUEST_TIMEOUT_SECS",
        default_value_t = 30,
        help = "Seconds before a Monty worker that has not answered is killed",
        long_help = "Seconds before a Monty worker that has not answered a request is\n\
killed. A backstop behind the per-run limits: it must be at least twice\n\
--monty-max-duration-secs plus 2 seconds.\n\n\
Default: 30"
    )]
    pub monty_request_timeout_secs: u64,

    /// Checkouts per Monty worker before it is replaced
    #[arg(
        long,
        env = "KRUXIAFLOW_MONTY_RECYCLE_AFTER",
        default_value_t = 1,
        help = "Runs per Monty worker before it is replaced (1 = never reused)",
        long_help = "How many run_code runs a Monty worker serves before it is replaced.\n\
1 (the default) gives every run a fresh process, so nothing can carry over\n\
between workflows or tenants; it costs a few milliseconds per run.\n\n\
Default: 1"
    )]
    pub monty_recycle_after: u32,

    /// Memory ceiling per run, in bytes
    #[arg(
        long,
        env = "KRUXIAFLOW_MONTY_MAX_MEMORY_BYTES",
        default_value_t = 64 * 1024 * 1024,
        help = "Most memory a run may use (a workflow may ask for less, never more)"
    )]
    pub monty_max_memory_bytes: usize,

    /// Execution-time ceiling per run, in seconds
    #[arg(
        long,
        env = "KRUXIAFLOW_MONTY_MAX_DURATION_SECS",
        default_value_t = 10.0,
        help = "Most execution time (and, separately, sleep) a run may use, in seconds"
    )]
    pub monty_max_duration_secs: f64,

    /// Recursion-depth ceiling per run
    #[arg(
        long,
        env = "KRUXIAFLOW_MONTY_MAX_RECURSION_DEPTH",
        default_value_t = 1000,
        help = "Deepest recursion a run may use",
        long_help = "Deepest recursion a run may use. Higher values do not crash the\n\
sandbox but let runaway recursion run until the request timeout.\n\n\
Default: 1000"
    )]
    pub monty_max_recursion_depth: usize,
}

#[cfg(feature = "monty")]
impl Default for MontyArgs {
    fn default() -> Self {
        Self {
            monty_binary: None,
            monty_min_processes: 1,
            monty_max_processes: None,
            monty_request_timeout_secs: 30,
            monty_recycle_after: 1,
            monty_max_memory_bytes: 64 * 1024 * 1024,
            monty_max_duration_secs: 10.0,
            monty_max_recursion_depth: 1000,
        }
    }
}

#[cfg(feature = "monty")]
impl MontyArgs {
    /// The run_code configuration, or `None` when `--monty-binary` is unset.
    /// `default_max_processes` is the worker's activity concurrency.
    pub fn to_config(
        &self,
        default_max_processes: usize,
    ) -> Result<Option<kruxiaflow_std_worker::MontyConfig>> {
        // Unset or empty (the Docker image sets it; an empty value disables it)
        let Some(binary) = self
            .monty_binary
            .clone()
            .filter(|path| !path.as_os_str().is_empty())
        else {
            tracing::info!(
                "run_code is compiled in but disabled: set KRUXIAFLOW_MONTY_BINARY to enable it"
            );
            return Ok(None);
        };
        let max_duration = std::time::Duration::try_from_secs_f64(self.monty_max_duration_secs)
            .map_err(|_| {
                anyhow::anyhow!(
                    "KRUXIAFLOW_MONTY_MAX_DURATION_SECS must be a positive number of seconds"
                )
            })?;
        Ok(Some(kruxiaflow_std_worker::MontyConfig {
            binary,
            min_processes: self.monty_min_processes,
            max_processes: self.monty_max_processes.unwrap_or(default_max_processes),
            request_timeout: std::time::Duration::from_secs(self.monty_request_timeout_secs),
            recycle_after: self.monty_recycle_after,
            max_memory_bytes: self.monty_max_memory_bytes,
            max_duration,
            max_recursion_depth: self.monty_max_recursion_depth,
        }))
    }
}

/// API Server configuration
#[derive(Debug, Clone)]
pub struct ApiConfig {
    /// PostgreSQL connection URL
    pub database_url: String,

    /// Port to bind to
    pub port: u16,

    /// Address to bind to
    pub bind: String,

    /// Insecure dev mode: accept unauthenticated requests (local dev only)
    pub insecure_dev: bool,
}

impl ApiConfig {
    /// Create ApiConfig with precedence: CLI flags > Environment variables > Defaults
    pub fn new(
        database_url_cli: Option<String>,
        port_cli: Option<u16>,
        bind_cli: Option<String>,
        insecure_dev: bool,
        insecure_dev_allow_nonloopback: bool,
    ) -> Result<Self> {
        // Database URL: Required
        let database_url = database_url_cli
            .or_else(|| std::env::var("DATABASE_URL").ok())
            .context("Database URL is required (--database-url or DATABASE_URL)")?;

        // Port: CLI > Env > Default (8080)
        let port = port_cli
            .or_else(|| {
                std::env::var("KRUXIAFLOW_API_PORT")
                    .ok()
                    .and_then(|s| s.parse().ok())
            })
            .unwrap_or(8080);

        // Bind address: CLI > Env > Default (0.0.0.0)
        let bind = bind_cli
            .or_else(|| std::env::var("KRUXIAFLOW_API_BIND").ok())
            .unwrap_or_else(|| "0.0.0.0".to_string());

        // Validate configuration
        if port == 0 {
            anyhow::bail!("Port must be between 1 and 65535");
        }

        if insecure_dev && !insecure_dev_allow_nonloopback && !is_loopback_bind(&bind) {
            anyhow::bail!(
                "--insecure-dev requires a loopback bind address (got '{}').\n\
                 Either bind to 127.0.0.1, or — for containers — also set\n\
                 --insecure-dev-allow-nonloopback and publish the port bound to\n\
                 127.0.0.1 on the host (e.g. \"127.0.0.1:8080:8080\").",
                bind
            );
        }

        Ok(Self {
            database_url,
            port,
            bind,
            insecure_dev,
        })
    }

    /// Get bind address for Axum server
    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.bind, self.port)
    }

    /// Log configuration (redact sensitive values)
    pub fn log_config(&self) {
        tracing::info!("API Server Configuration:");
        tracing::info!("  Bind address: {}", self.bind_address());
        tracing::info!("  Database: {}", self.redact_database_url());
    }

    /// Redact password from database URL for logging
    fn redact_database_url(&self) -> String {
        // Simple redaction: Replace password with ***
        // Format: postgres://user:password@host:port/db
        if let Some(at_pos) = self.database_url.rfind('@')
            && let Some(colon_pos) = self.database_url[..at_pos].rfind(':')
        {
            let mut redacted = self.database_url.clone();
            redacted.replace_range(colon_pos + 1..at_pos, "***");
            return redacted;
        }
        "***".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    // =========================================================================
    // CacheConfig tests
    // =========================================================================

    #[test]
    #[serial]
    fn test_cache_config_default_provider() {
        // Clean environment
        unsafe {
            std::env::remove_var("KRUXIAFLOW_CACHE_PROVIDER");
            std::env::remove_var("KRUXIAFLOW_REDIS_URL");
            std::env::remove_var("KRUXIAFLOW_REDIS_KEY_PREFIX");
        }

        let config = CacheConfig::new();

        assert_eq!(config.provider, "noop");
        assert!(config.redis_url.is_none());
        assert!(config.redis_key_prefix.is_none());
    }

    #[test]
    #[serial]
    fn test_cache_config_from_environment() {
        unsafe {
            std::env::set_var("KRUXIAFLOW_CACHE_PROVIDER", "Redis"); // Mixed case
            std::env::set_var("KRUXIAFLOW_REDIS_URL", "redis://localhost:6380");
            std::env::set_var("KRUXIAFLOW_REDIS_KEY_PREFIX", "test:");
        }

        let config = CacheConfig::new();

        assert_eq!(config.provider, "redis"); // Normalized to lowercase
        assert_eq!(config.redis_url, Some("redis://localhost:6380".to_string()));
        assert_eq!(config.redis_key_prefix, Some("test:".to_string()));

        // Clean up
        unsafe {
            std::env::remove_var("KRUXIAFLOW_CACHE_PROVIDER");
            std::env::remove_var("KRUXIAFLOW_REDIS_URL");
            std::env::remove_var("KRUXIAFLOW_REDIS_KEY_PREFIX");
        }
    }

    #[test]
    #[serial]
    fn test_cache_config_default_is_same_as_new() {
        unsafe {
            std::env::remove_var("KRUXIAFLOW_CACHE_PROVIDER");
            std::env::remove_var("KRUXIAFLOW_REDIS_URL");
            std::env::remove_var("KRUXIAFLOW_REDIS_KEY_PREFIX");
        }

        let config_new = CacheConfig::new();
        let config_default = CacheConfig::default();

        assert_eq!(config_new.provider, config_default.provider);
        assert_eq!(config_new.redis_url, config_default.redis_url);
        assert_eq!(config_new.redis_key_prefix, config_default.redis_key_prefix);
    }

    #[test]
    fn test_cache_config_redact_redis_url_with_password() {
        let config = CacheConfig {
            provider: "redis".to_string(),
            redis_url: Some("redis://:secretpassword@localhost:6379".to_string()),
            redis_key_prefix: None,
        };

        let redacted = config.redact_redis_url("redis://:secretpassword@localhost:6379");
        assert!(redacted.contains("***"));
        assert!(!redacted.contains("secretpassword"));
        assert!(redacted.contains("localhost:6379"));
    }

    #[test]
    fn test_cache_config_redact_redis_url_with_user_and_password() {
        let config = CacheConfig {
            provider: "redis".to_string(),
            redis_url: Some("redis://user:password123@redis.example.com:6379/0".to_string()),
            redis_key_prefix: None,
        };

        let redacted = config.redact_redis_url("redis://user:password123@redis.example.com:6379/0");
        assert!(redacted.contains("***"));
        assert!(!redacted.contains("password123"));
        assert!(redacted.contains("redis.example.com"));
    }

    #[test]
    fn test_cache_config_redact_redis_url_without_password() {
        let config = CacheConfig {
            provider: "redis".to_string(),
            redis_url: Some("redis://localhost:6379".to_string()),
            redis_key_prefix: None,
        };

        let redacted = config.redact_redis_url("redis://localhost:6379");
        // Should return URL unchanged when no password
        assert_eq!(redacted, "redis://localhost:6379");
    }

    #[test]
    #[serial]
    fn test_cache_config_create_noop_cache() {
        unsafe {
            std::env::remove_var("KRUXIAFLOW_CACHE_PROVIDER");
        }

        let config = CacheConfig::new();
        let _cache = config.create_cache_service();

        // NoOpCache should be created - we can't directly test the type
        // but we can verify it doesn't panic and returns an Arc<dyn CacheService>
    }

    #[test]
    #[serial]
    fn test_cache_config_create_cache_with_unknown_provider() {
        let config = CacheConfig {
            provider: "unknown_provider".to_string(),
            redis_url: None,
            redis_key_prefix: None,
        };

        // Should fall back to NoOpCache for unknown providers
        let _cache = config.create_cache_service();
    }

    // =========================================================================
    // ApiConfig tests
    // =========================================================================

    #[test]
    fn test_database_url_redaction() {
        let config = ApiConfig {
            database_url: "postgres://user:secret123@localhost:5432/db".to_string(),
            port: 8080,
            bind: "0.0.0.0".to_string(),
            insecure_dev: false,
        };

        let redacted = config.redact_database_url();
        assert!(redacted.contains("***"));
        assert!(!redacted.contains("secret123"));
    }

    #[test]
    fn test_bind_address() {
        let config = ApiConfig {
            database_url: "postgres://localhost/db".to_string(),
            port: 9090,
            bind: "127.0.0.1".to_string(),
            insecure_dev: false,
        };

        assert_eq!(config.bind_address(), "127.0.0.1:9090");
    }

    #[test]
    #[serial]
    fn test_defaults() {
        // Set database URL via environment for test
        unsafe {
            std::env::set_var("DATABASE_URL", "postgres://localhost/test");
        }

        let config = ApiConfig::new(None, None, None, false, false).unwrap();

        assert_eq!(config.port, 8080);
        assert_eq!(config.bind, "0.0.0.0");

        // Clean up
        unsafe {
            std::env::remove_var("DATABASE_URL");
        }
    }

    #[test]
    #[serial]
    fn test_cli_overrides_env() {
        unsafe {
            std::env::set_var("DATABASE_URL", "postgres://localhost/env_db");
            std::env::set_var("KRUXIAFLOW_API_PORT", "9090");
            std::env::set_var("KRUXIAFLOW_API_BIND", "127.0.0.1");
        }

        let config = ApiConfig::new(
            Some("postgres://localhost/cli_db".to_string()),
            Some(8888),
            Some("192.168.1.1".to_string()),
            false,
            false,
        )
        .unwrap();

        assert!(config.database_url.contains("cli_db"));
        assert_eq!(config.port, 8888);
        assert_eq!(config.bind, "192.168.1.1");

        // Clean up
        unsafe {
            std::env::remove_var("DATABASE_URL");
            std::env::remove_var("KRUXIAFLOW_API_PORT");
            std::env::remove_var("KRUXIAFLOW_API_BIND");
        }
    }

    #[test]
    #[serial]
    fn test_env_overrides_defaults() {
        unsafe {
            std::env::set_var("DATABASE_URL", "postgres://localhost/test");
            std::env::set_var("KRUXIAFLOW_API_PORT", "9000");
            std::env::set_var("KRUXIAFLOW_API_BIND", "localhost");
        }

        let config = ApiConfig::new(None, None, None, false, false).unwrap();

        assert_eq!(config.port, 9000);
        assert_eq!(config.bind, "localhost");

        // Clean up
        unsafe {
            std::env::remove_var("DATABASE_URL");
            std::env::remove_var("KRUXIAFLOW_API_PORT");
            std::env::remove_var("KRUXIAFLOW_API_BIND");
        }
    }

    #[test]
    #[serial]
    fn test_invalid_port() {
        unsafe {
            std::env::set_var("DATABASE_URL", "postgres://localhost/test");
        }

        let result = ApiConfig::new(None, Some(0), None, false, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Port must be"));

        unsafe {
            std::env::remove_var("DATABASE_URL");
        }
    }

    #[test]
    #[serial]
    fn test_database_url_required() {
        unsafe {
            std::env::remove_var("DATABASE_URL");
        }

        let result = ApiConfig::new(None, None, None, false, false);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Database URL is required")
        );
    }

    #[test]
    fn test_database_url_redaction_without_password() {
        // Database URL without password (e.g., local dev with trust auth)
        let config = ApiConfig {
            database_url: "postgres://localhost/testdb".to_string(),
            port: 8080,
            bind: "0.0.0.0".to_string(),
            insecure_dev: false,
        };

        let redacted = config.redact_database_url();
        // When no @ symbol with password, returns "***" as fallback
        assert_eq!(redacted, "***");
    }

    #[test]
    fn test_database_url_redaction_with_at_but_no_password() {
        // Edge case: URL with @ but user only (no password)
        // The function finds the protocol colon and redacts from there to @
        let config = ApiConfig {
            database_url: "postgres://user@localhost:5432/db".to_string(),
            port: 8080,
            bind: "0.0.0.0".to_string(),
            insecure_dev: false,
        };

        let redacted = config.redact_database_url();
        // Finds protocol colon and redacts "//user" portion
        assert!(redacted.contains("***"));
        assert!(redacted.contains("@localhost"));
    }

    #[test]
    #[serial]
    fn test_invalid_port_from_environment() {
        unsafe {
            std::env::set_var("DATABASE_URL", "postgres://localhost/test");
            std::env::set_var("KRUXIAFLOW_API_PORT", "invalid_port");
        }

        // Invalid port string should fall back to default
        let config = ApiConfig::new(None, None, None, false, false).unwrap();
        assert_eq!(config.port, 8080); // Default port

        // Clean up
        unsafe {
            std::env::remove_var("DATABASE_URL");
            std::env::remove_var("KRUXIAFLOW_API_PORT");
        }
    }

    #[test]
    fn test_api_config_clone() {
        let config = ApiConfig {
            database_url: "postgres://localhost/db".to_string(),
            port: 9090,
            bind: "127.0.0.1".to_string(),
            insecure_dev: false,
        };

        let cloned = config.clone();
        assert_eq!(cloned.database_url, config.database_url);
        assert_eq!(cloned.port, config.port);
        assert_eq!(cloned.bind, config.bind);
    }

    #[test]
    fn test_cache_config_log_config_with_redis() {
        let config = CacheConfig {
            provider: "redis".to_string(),
            redis_url: Some("redis://:secret@localhost:6379".to_string()),
            redis_key_prefix: Some("test:".to_string()),
        };
        // Should not panic
        config.log_config();
    }

    #[test]
    fn test_cache_config_log_config_without_redis() {
        let config = CacheConfig {
            provider: "noop".to_string(),
            redis_url: None,
            redis_key_prefix: None,
        };
        // Should not panic
        config.log_config();
    }

    #[test]
    fn test_api_config_log_config() {
        let config = ApiConfig {
            database_url: "postgres://user:pass@localhost:5432/db".to_string(),
            port: 8080,
            bind: "0.0.0.0".to_string(),
            insecure_dev: false,
        };
        // Should not panic
        config.log_config();
    }

    #[test]
    fn test_api_config_debug() {
        let config = ApiConfig {
            database_url: "postgres://localhost/db".to_string(),
            port: 8080,
            bind: "0.0.0.0".to_string(),
            insecure_dev: false,
        };

        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("ApiConfig"));
        assert!(debug_str.contains("8080"));
    }
}

#[cfg(all(test, feature = "monty"))]
mod monty_args_tests {
    use super::*;
    use clap::Parser;
    use std::path::PathBuf;
    use std::time::Duration;

    /// The flags exactly as `serve` and `worker` flatten them
    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        monty: MontyArgs,
    }

    #[test]
    fn test_unset_binary_disables_run_code() {
        assert!(MontyArgs::default().to_config(16).unwrap().is_none());
    }

    #[test]
    fn test_empty_binary_disables_run_code() {
        // The Docker image sets KRUXIAFLOW_MONTY_BINARY; an empty value turns it off
        let args = MontyArgs {
            monty_binary: Some(PathBuf::new()),
            ..MontyArgs::default()
        };
        assert!(args.to_config(16).unwrap().is_none());
    }

    #[test]
    fn test_defaults_become_the_config() {
        let args = MontyArgs {
            monty_binary: Some(PathBuf::from("/monty")),
            ..MontyArgs::default()
        };
        let config = args.to_config(16).unwrap().expect("enabled");
        assert_eq!(config.binary, PathBuf::from("/monty"));
        assert_eq!(config.min_processes, 1);
        assert_eq!(
            config.max_processes, 16,
            "defaults to the worker's concurrency"
        );
        assert_eq!(config.request_timeout, Duration::from_secs(30));
        assert_eq!(config.recycle_after, 1);
        assert_eq!(config.max_memory_bytes, 64 * 1024 * 1024);
        assert_eq!(config.max_duration, Duration::from_secs(10));
        assert_eq!(config.max_recursion_depth, 1000);
    }

    #[test]
    fn test_flags_parse_into_the_config() {
        let cli = TestCli::try_parse_from([
            "test",
            "--monty-binary",
            "/opt/monty",
            "--monty-min-processes",
            "2",
            "--monty-max-processes",
            "6",
            "--monty-request-timeout-secs",
            "40",
            "--monty-recycle-after",
            "5",
            "--monty-max-memory-bytes",
            "1000000",
            "--monty-max-duration-secs",
            "2.5",
            "--monty-max-recursion-depth",
            "300",
        ])
        .expect("valid flags");
        let config = cli.monty.to_config(16).unwrap().expect("enabled");
        assert_eq!(config.binary, PathBuf::from("/opt/monty"));
        assert_eq!(config.min_processes, 2);
        assert_eq!(config.max_processes, 6);
        assert_eq!(config.request_timeout, Duration::from_secs(40));
        assert_eq!(config.recycle_after, 5);
        assert_eq!(config.max_memory_bytes, 1_000_000);
        assert_eq!(config.max_duration, Duration::from_millis(2500));
        assert_eq!(config.max_recursion_depth, 300);
    }

    #[test]
    fn test_invalid_duration_is_an_error() {
        let args = MontyArgs {
            monty_binary: Some(PathBuf::from("/monty")),
            monty_max_duration_secs: -1.0,
            ..MontyArgs::default()
        };
        let err = args.to_config(16).unwrap_err().to_string();
        assert!(err.contains("KRUXIAFLOW_MONTY_MAX_DURATION_SECS"), "{err}");
    }

    #[test]
    fn test_non_numeric_flag_is_rejected() {
        assert!(TestCli::try_parse_from(["test", "--monty-max-memory-bytes", "lots"]).is_err());
    }
}
