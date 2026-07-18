//! Error types for the worker SDK.

use crate::types::UsageEntry;
use rust_decimal::Decimal;

/// An activity failure, reported to the server via
/// `POST /api/v1/activities/{id}/fail`.
///
/// The `retryable` flag is the contract-critical part: retryable failures are
/// re-queued by the orchestrator (up to the activity's retry limit), terminal
/// failures are not. Construct with [`ActivityError::retryable`] or
/// [`ActivityError::terminal`] so the distinction is always explicit.
///
/// A failed attempt may still have spent money: attach what it cost with
/// [`ActivityError::with_usage`] / [`ActivityError::with_cost`] so the spend
/// is recorded and counted against workflow budgets.
///
/// Any error convertible to [`anyhow::Error`] can be propagated with `?` from
/// handlers returning `Result<_, ActivityError>` via
/// `.map_err(ActivityError::from)?`, or directly from `anyhow::Error`; it
/// becomes a retryable `EXECUTION_ERROR`.
#[derive(Debug, Clone)]
pub struct ActivityError {
    /// Error code (for categorization, e.g. "PAYMENT_DECLINED")
    pub code: String,
    /// Human-readable error message
    pub message: String,
    /// Whether the orchestrator should retry this activity
    pub retryable: bool,
    /// Cost spent before the failure that is NOT covered by `usage` entries
    pub cost_usd: Option<Decimal>,
    /// Per-LLM-call usage made before the failure
    pub usage: Vec<UsageEntry>,
}

impl ActivityError {
    /// A failure the orchestrator should retry (transient conditions:
    /// timeouts, network errors, rate limits).
    pub fn retryable(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: true,
            cost_usd: None,
            usage: Vec::new(),
        }
    }

    /// A terminal failure that will not improve on retry (bad input,
    /// business-rule rejection).
    pub fn terminal(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: false,
            cost_usd: None,
            usage: Vec::new(),
        }
    }

    /// Attach cost spent before the failure that is not covered by usage
    /// entries.
    pub fn with_cost(mut self, cost_usd: Decimal) -> Self {
        self.cost_usd = Some(cost_usd);
        self
    }

    /// Attach per-LLM-call usage made before the failure.
    pub fn with_usage(mut self, usage: Vec<UsageEntry>) -> Self {
        self.usage = usage;
        self
    }
}

impl std::fmt::Display for ActivityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ActivityError {}

impl From<anyhow::Error> for ActivityError {
    fn from(err: anyhow::Error) -> Self {
        Self::retryable("EXECUTION_ERROR", format!("{err:#}"))
    }
}

/// Worker configuration error.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Missing API URL (set KRUXIAFLOW_API_URL)")]
    MissingApiUrl,

    #[error(
        "Partial OAuth credentials: set both KRUXIAFLOW_CLIENT_ID and KRUXIAFLOW_CLIENT_SECRET, \
         or neither (server in dev mode)"
    )]
    PartialCredentials,

    #[error(
        "No worker name: set KRUXIAFLOW_WORKER, call worker(...) on the builder, \
         or register activities that declare one"
    )]
    MissingWorker,

    #[error(
        "Ambiguous worker name: registered activities declare multiple workers ({0:?}); \
         a worker polls one worker name — set it explicitly"
    )]
    AmbiguousWorker(Vec<String>),

    #[error("Invalid value for {var}: {reason}")]
    InvalidValue { var: String, reason: String },
}

/// Error from a worker API call.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    /// The server rejected the request as unauthenticated and no credentials
    /// are configured. Set `KRUXIAFLOW_CLIENT_ID` / `KRUXIAFLOW_CLIENT_SECRET`,
    /// or run the server in dev mode (`--insecure-dev`).
    #[error(
        "Server requires authentication but no credentials are configured \
         (set KRUXIAFLOW_CLIENT_ID and KRUXIAFLOW_CLIENT_SECRET)"
    )]
    AuthRequired,

    #[error("Token request failed: {status} - {body}")]
    Auth { status: u16, body: String },

    /// 409: the activity was already completed, timed out, or was reassigned
    /// to a different worker. Idempotency semantics: log and move on, never
    /// retry.
    #[error("Conflict: {body}")]
    Conflict { body: String },

    #[error("API error: {status} - {body}")]
    Api { status: u16, body: String },
}
