use sqlx::PgPool;
use std::time::Duration;

/// Orchestrator configuration
#[derive(Clone)]
pub struct OrchestratorConfig {
    pub pool: PgPool,
    pub poll_interval_min: Duration,
    pub poll_interval_max: Duration,
    pub backoff_multiplier: f64,
    pub workflow_timeout: Duration,
    pub timeout_check_interval: Duration,
}

impl OrchestratorConfig {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            poll_interval_min: Duration::from_millis(10),
            poll_interval_max: Duration::from_millis(500), // Optimized for low latency
            backoff_multiplier: 1.3,                       // Optimized for faster convergence
            workflow_timeout: Duration::from_secs(300),    // 5 minutes default timeout
            timeout_check_interval: Duration::from_secs(30), // Check every 30 seconds
        }
    }

    pub fn with_poll_interval(mut self, min: Duration, max: Duration) -> Self {
        self.poll_interval_min = min;
        self.poll_interval_max = max;
        self
    }

    pub fn with_backoff_multiplier(mut self, multiplier: f64) -> Self {
        self.backoff_multiplier = multiplier;
        self
    }

    pub fn with_workflow_timeout(mut self, timeout: Duration) -> Self {
        self.workflow_timeout = timeout;
        self
    }

    pub fn with_timeout_check_interval(mut self, interval: Duration) -> Self {
        self.timeout_check_interval = interval;
        self
    }
}
