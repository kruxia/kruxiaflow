//! Workflow API DTOs
//!
//! API-layer wrappers around core workflow types to provide OpenAPI schema
//! generation without coupling core to API concerns.
//!
//! These types mirror the structure of core types and provide bidirectional
//! From/Into conversions, allowing the API layer to derive ToSchema without
//! adding utoipa as a dependency to the core crate.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

/// Workflow definition wrapper
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WorkflowDefinition {
    /// Workflow name (unique per version)
    pub name: String,

    /// Activities in the workflow
    pub activities: Vec<ActivityDefinition>,

    /// Workflow-level settings (timeout, retries, budget)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<WorkflowSettings>,
}

impl From<kruxiaflow_core::workflow::WorkflowDefinition> for WorkflowDefinition {
    fn from(def: kruxiaflow_core::workflow::WorkflowDefinition) -> Self {
        Self {
            name: def.name,
            activities: def.activities.into_iter().map(Into::into).collect(),
            settings: def.settings.map(Into::into),
        }
    }
}

impl From<WorkflowDefinition> for kruxiaflow_core::workflow::WorkflowDefinition {
    fn from(def: WorkflowDefinition) -> Self {
        Self {
            name: def.name,
            activities: def.activities.into_iter().map(Into::into).collect(),
            settings: def.settings.map(Into::into),
        }
    }
}

/// Workflow-level settings wrapper
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WorkflowSettings {
    /// Maximum workflow execution time in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,

    /// Maximum retry attempts for transient failures
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,

    /// Workflow-level budget limit
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget: Option<BudgetSettings>,
}

impl From<kruxiaflow_core::workflow::WorkflowSettings> for WorkflowSettings {
    fn from(settings: kruxiaflow_core::workflow::WorkflowSettings) -> Self {
        Self {
            timeout: settings.timeout,
            max_retries: settings.max_retries,
            budget: settings.budget.map(Into::into),
        }
    }
}

impl From<WorkflowSettings> for kruxiaflow_core::workflow::WorkflowSettings {
    fn from(settings: WorkflowSettings) -> Self {
        Self {
            timeout: settings.timeout,
            max_retries: settings.max_retries,
            budget: settings.budget.map(Into::into),
        }
    }
}

/// Activity definition wrapper
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ActivityDefinition {
    /// Unique key for this activity within the workflow
    pub key: String,

    /// Activity worker type (e.g., "std", "custom-python")
    pub worker: String,

    /// Activity name within worker (e.g., "http_request", "postgres_query")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity_name: Option<String>,

    /// Activity parameters (can include template expressions)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<HashMap<String, serde_json::Value>>,

    /// Activities that must complete before this one
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<Vec<ActivityRelationship>>,

    /// Activities that depend on this one (input only, cleared after normalization)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependency_of: Option<Vec<ActivityRelationship>>,

    /// Activity output definitions (name and type)
    #[serde(rename = "outputs")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_definitions: Option<Vec<ActivityOutputDefinition>>,

    /// Activity-level settings (timeout, retry, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings: Option<ActivitySettings>,

    /// Whether to store separate outputs for each iteration
    #[serde(default)]
    pub iteration_scoped: bool,

    /// Maximum number of iterations (prevents infinite loops)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iteration_limit: Option<u32>,

    /// Cached metadata: whether this activity is part of a loop
    #[serde(default)]
    pub is_loop_activity: bool,

    /// Token streaming configuration for LLM activities
    #[serde(default, skip_serializing_if = "StreamingConfig::is_disabled")]
    pub streaming: StreamingConfig,
}

impl From<kruxiaflow_core::workflow::ActivityDefinition> for ActivityDefinition {
    fn from(def: kruxiaflow_core::workflow::ActivityDefinition) -> Self {
        Self {
            key: def.key,
            worker: def.worker,
            activity_name: def.activity_name,
            parameters: def.parameters,
            depends_on: def
                .depends_on
                .map(|v| v.into_iter().map(Into::into).collect()),
            dependency_of: def
                .dependency_of
                .map(|v| v.into_iter().map(Into::into).collect()),
            output_definitions: def
                .output_definitions
                .map(|v| v.into_iter().map(Into::into).collect()),
            settings: def.settings.map(Into::into),
            iteration_scoped: def.iteration_scoped,
            iteration_limit: def.iteration_limit,
            is_loop_activity: def.is_loop_activity,
            streaming: def.streaming.into(),
        }
    }
}

impl From<ActivityDefinition> for kruxiaflow_core::workflow::ActivityDefinition {
    fn from(def: ActivityDefinition) -> Self {
        Self {
            key: def.key,
            worker: def.worker,
            activity_name: def.activity_name,
            parameters: def.parameters,
            output_definitions: def
                .output_definitions
                .map(|v| v.into_iter().map(Into::into).collect()),
            depends_on: def
                .depends_on
                .map(|v| v.into_iter().map(Into::into).collect()),
            dependency_of: def
                .dependency_of
                .map(|v| v.into_iter().map(Into::into).collect()),
            settings: def.settings.map(Into::into),
            iteration_scoped: def.iteration_scoped,
            iteration_limit: def.iteration_limit,
            is_loop_activity: def.is_loop_activity,
            streaming: def.streaming.into(),
        }
    }
}

/// Token streaming configuration for LLM activities
/// Supports both shorthand `streaming: true` and detailed `streaming: { enabled: true }`
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(untagged)]
#[derive(Default)]
pub enum StreamingConfig {
    /// Streaming disabled (default)
    #[default]
    Disabled,
    /// Shorthand: `streaming: true` or `streaming: false`
    Simple(bool),
    /// Detailed: `streaming: { enabled: true }`
    Detailed(StreamingOptions),
}

impl<'de> Deserialize<'de> for StreamingConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;

        let value = serde_json::Value::deserialize(deserializer)?;

        match value {
            serde_json::Value::Bool(b) => Ok(StreamingConfig::Simple(b)),
            serde_json::Value::Object(_) => {
                let options: StreamingOptions =
                    serde_json::from_value(value).map_err(D::Error::custom)?;
                Ok(StreamingConfig::Detailed(options))
            }
            serde_json::Value::Null => Ok(StreamingConfig::Disabled),
            _ => Err(D::Error::custom(
                "streaming must be a boolean or object with 'enabled' field",
            )),
        }
    }
}

impl StreamingConfig {
    /// Check if streaming is enabled
    pub fn is_enabled(&self) -> bool {
        match self {
            StreamingConfig::Disabled => false,
            StreamingConfig::Simple(enabled) => *enabled,
            StreamingConfig::Detailed(options) => options.enabled,
        }
    }

    /// Check if streaming is disabled (for serde skip_serializing_if)
    pub fn is_disabled(&self) -> bool {
        !self.is_enabled()
    }
}

/// Detailed streaming options
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StreamingOptions {
    /// Whether streaming is enabled
    #[serde(default)]
    pub enabled: bool,
}

impl From<kruxiaflow_core::workflow::StreamingConfig> for StreamingConfig {
    fn from(config: kruxiaflow_core::workflow::StreamingConfig) -> Self {
        match config {
            kruxiaflow_core::workflow::StreamingConfig::Disabled => StreamingConfig::Disabled,
            kruxiaflow_core::workflow::StreamingConfig::Simple(b) => StreamingConfig::Simple(b),
            kruxiaflow_core::workflow::StreamingConfig::Detailed(opts) => {
                StreamingConfig::Detailed(StreamingOptions {
                    enabled: opts.enabled,
                })
            }
        }
    }
}

impl From<StreamingConfig> for kruxiaflow_core::workflow::StreamingConfig {
    fn from(config: StreamingConfig) -> Self {
        match config {
            StreamingConfig::Disabled => kruxiaflow_core::workflow::StreamingConfig::Disabled,
            StreamingConfig::Simple(b) => kruxiaflow_core::workflow::StreamingConfig::Simple(b),
            StreamingConfig::Detailed(opts) => {
                kruxiaflow_core::workflow::StreamingConfig::Detailed(
                    kruxiaflow_core::workflow::StreamingOptions {
                        enabled: opts.enabled,
                    },
                )
            }
        }
    }
}

/// Relationship between activities (edge in the directed graph)
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ActivityRelationship {
    /// Key of the related activity
    pub activity_key: String,

    /// Optional conditions that must be satisfied for this edge to activate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conditions: Option<Vec<String>>,

    /// Cached metadata: whether this dependency is a back-edge (loop)
    #[serde(default)]
    pub is_back_edge: bool,
}

impl From<kruxiaflow_core::workflow::ActivityRelationship> for ActivityRelationship {
    fn from(rel: kruxiaflow_core::workflow::ActivityRelationship) -> Self {
        Self {
            activity_key: rel.activity_key,
            conditions: rel.conditions,
            is_back_edge: rel.is_back_edge,
        }
    }
}

impl From<ActivityRelationship> for kruxiaflow_core::workflow::ActivityRelationship {
    fn from(rel: ActivityRelationship) -> Self {
        Self {
            activity_key: rel.activity_key,
            conditions: rel.conditions,
            is_back_edge: rel.is_back_edge,
        }
    }
}

/// Activity-level settings
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
pub struct ActivitySettings {
    /// Activity timeout in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,

    /// Retry configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetrySettings>,

    /// Budget configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget: Option<BudgetSettings>,

    /// Enable result caching
    #[serde(default)]
    pub cache: bool,

    /// Cache TTL in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_ttl: Option<u64>,

    /// Per-activity iteration limit (can override activity-level iteration_limit)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iteration_limit: Option<u32>,

    /// Relative delay: "500ms", "5s", "30m", "30mi", "2h", "7d", "1w", "2mo", "1y"
    /// Template-aware: "{{INPUT.delay_amount}}m"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay: Option<String>,

    /// Absolute ISO 8601 timestamp for scheduling (template-aware)
    /// Example: "2025-12-01T09:00:00-08:00" or "{{INPUT.report_deadline}}"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduled_for: Option<String>,

    /// Wait for an external signal before running the activity
    /// When set, the activity enters 'waiting' state until signaled or timeout
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait_for_signal: Option<WaitForSignalSettings>,

    /// Token streaming configuration
    #[serde(default, skip_serializing_if = "StreamingConfig::is_disabled")]
    pub streaming: StreamingConfig,
}

/// Settings for activities that wait for external signals
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WaitForSignalSettings {
    /// The event name to wait for (matched against signal requests)
    pub event_name: String,

    /// Timeout in seconds before taking the on_timeout action
    pub timeout_seconds: u64,

    /// Action to take when timeout occurs (default: fail)
    #[serde(default)]
    pub on_timeout: OnTimeout,
}

/// Action to take when a signal wait times out
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum OnTimeout {
    /// Continue with the activity (run with null signal data)
    Continue,
    /// Skip the activity
    Skip,
    /// Fail the activity (default)
    #[default]
    Fail,
}

impl From<kruxiaflow_core::workflow::WaitForSignalSettings> for WaitForSignalSettings {
    fn from(settings: kruxiaflow_core::workflow::WaitForSignalSettings) -> Self {
        Self {
            event_name: settings.event_name,
            timeout_seconds: settings.timeout_seconds,
            on_timeout: settings.on_timeout.into(),
        }
    }
}

impl From<WaitForSignalSettings> for kruxiaflow_core::workflow::WaitForSignalSettings {
    fn from(settings: WaitForSignalSettings) -> Self {
        Self {
            event_name: settings.event_name,
            timeout_seconds: settings.timeout_seconds,
            on_timeout: settings.on_timeout.into(),
        }
    }
}

impl From<kruxiaflow_core::workflow::OnTimeout> for OnTimeout {
    fn from(action: kruxiaflow_core::workflow::OnTimeout) -> Self {
        match action {
            kruxiaflow_core::workflow::OnTimeout::Continue => Self::Continue,
            kruxiaflow_core::workflow::OnTimeout::Skip => Self::Skip,
            kruxiaflow_core::workflow::OnTimeout::Fail => Self::Fail,
        }
    }
}

impl From<OnTimeout> for kruxiaflow_core::workflow::OnTimeout {
    fn from(action: OnTimeout) -> Self {
        match action {
            OnTimeout::Continue => Self::Continue,
            OnTimeout::Skip => Self::Skip,
            OnTimeout::Fail => Self::Fail,
        }
    }
}

/// Budget configuration for activities
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BudgetSettings {
    /// Budget limit in USD
    pub limit: Decimal,

    /// Action when budget exceeded
    #[serde(default)]
    pub action: BudgetAction,
}

/// Action to take when budget is exceeded
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum BudgetAction {
    /// Abort workflow execution
    #[default]
    Abort,

    /// Continue with warning
    Continue,
}

impl From<kruxiaflow_core::workflow::BudgetSettings> for BudgetSettings {
    fn from(settings: kruxiaflow_core::workflow::BudgetSettings) -> Self {
        Self {
            limit: settings.limit,
            action: settings.action.into(),
        }
    }
}

impl From<BudgetSettings> for kruxiaflow_core::workflow::BudgetSettings {
    fn from(settings: BudgetSettings) -> Self {
        Self {
            limit: settings.limit,
            action: settings.action.into(),
        }
    }
}

impl From<kruxiaflow_core::workflow::BudgetAction> for BudgetAction {
    fn from(action: kruxiaflow_core::workflow::BudgetAction) -> Self {
        match action {
            kruxiaflow_core::workflow::BudgetAction::Abort => Self::Abort,
            kruxiaflow_core::workflow::BudgetAction::Continue => Self::Continue,
        }
    }
}

impl From<BudgetAction> for kruxiaflow_core::workflow::BudgetAction {
    fn from(action: BudgetAction) -> Self {
        match action {
            BudgetAction::Abort => Self::Abort,
            BudgetAction::Continue => Self::Continue,
        }
    }
}

impl From<kruxiaflow_core::workflow::ActivitySettings> for ActivitySettings {
    fn from(settings: kruxiaflow_core::workflow::ActivitySettings) -> Self {
        Self {
            timeout_seconds: settings.timeout_seconds,
            retry: settings.retry.map(Into::into),
            budget: settings.budget.map(Into::into),
            cache: settings.cache,
            cache_ttl: settings.cache_ttl,
            iteration_limit: settings.iteration_limit,
            delay: settings.delay,
            scheduled_for: settings.scheduled_for,
            wait_for_signal: settings.wait_for_signal.map(Into::into),
            streaming: settings.streaming.into(),
        }
    }
}

impl From<ActivitySettings> for kruxiaflow_core::workflow::ActivitySettings {
    fn from(settings: ActivitySettings) -> Self {
        Self {
            timeout_seconds: settings.timeout_seconds,
            retry: settings.retry.map(Into::into),
            budget: settings.budget.map(Into::into),
            cache: settings.cache,
            cache_ttl: settings.cache_ttl,
            iteration_limit: settings.iteration_limit,
            delay: settings.delay,
            scheduled_for: settings.scheduled_for,
            wait_for_signal: settings.wait_for_signal.map(Into::into),
            streaming: settings.streaming.into(),
        }
    }
}

/// Retry configuration for activities
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RetrySettings {
    /// Maximum retry attempts
    pub max_attempts: u32,

    /// Backoff strategy
    #[serde(default = "default_backoff")]
    pub strategy: BackoffStrategy,

    /// Base delay in seconds between retries (default: 2)
    #[serde(default = "default_base_seconds")]
    pub base_seconds: u64,

    /// Exponential multiplier (default: 2.0)
    #[serde(default = "default_factor")]
    pub factor: f64,

    /// Maximum backoff delay cap in seconds (default: 300)
    #[serde(default = "default_max_seconds")]
    pub max_seconds: u64,
}

fn default_backoff() -> BackoffStrategy {
    BackoffStrategy::Exponential
}

fn default_base_seconds() -> u64 {
    2
}

fn default_factor() -> f64 {
    2.0
}

fn default_max_seconds() -> u64 {
    300
}

impl From<kruxiaflow_core::workflow::RetryPolicy> for RetrySettings {
    fn from(settings: kruxiaflow_core::workflow::RetryPolicy) -> Self {
        Self {
            max_attempts: settings.max_attempts,
            strategy: settings.strategy.into(),
            base_seconds: settings.base_seconds,
            factor: settings.factor,
            max_seconds: settings.max_seconds,
        }
    }
}

impl From<RetrySettings> for kruxiaflow_core::workflow::RetryPolicy {
    fn from(settings: RetrySettings) -> Self {
        Self {
            max_attempts: settings.max_attempts,
            strategy: settings.strategy.into(),
            base_seconds: settings.base_seconds,
            factor: settings.factor,
            max_seconds: settings.max_seconds,
        }
    }
}

/// Backoff strategy for retries
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum BackoffStrategy {
    /// Fixed delay between retries
    Fixed,
    /// Exponential backoff (delay doubles each retry)
    Exponential,
}

impl From<kruxiaflow_core::workflow::BackoffStrategy> for BackoffStrategy {
    fn from(strategy: kruxiaflow_core::workflow::BackoffStrategy) -> Self {
        match strategy {
            kruxiaflow_core::workflow::BackoffStrategy::Fixed => Self::Fixed,
            kruxiaflow_core::workflow::BackoffStrategy::Exponential => Self::Exponential,
        }
    }
}

impl From<BackoffStrategy> for kruxiaflow_core::workflow::BackoffStrategy {
    fn from(strategy: BackoffStrategy) -> Self {
        match strategy {
            BackoffStrategy::Fixed => Self::Fixed,
            BackoffStrategy::Exponential => Self::Exponential,
        }
    }
}

/// Activity output definition
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ActivityOutputDefinition {
    /// Output name (key for referencing in templates)
    pub name: String,

    /// Output type (default: value)
    #[serde(default)]
    #[serde(rename = "type")]
    pub output_type: OutputType,
}

impl From<kruxiaflow_core::workflow::ActivityOutputDefinition> for ActivityOutputDefinition {
    fn from(output: kruxiaflow_core::workflow::ActivityOutputDefinition) -> Self {
        Self {
            name: output.name,
            output_type: output.output_type.into(),
        }
    }
}

impl From<ActivityOutputDefinition> for kruxiaflow_core::workflow::ActivityOutputDefinition {
    fn from(output: ActivityOutputDefinition) -> Self {
        Self {
            name: output.name,
            output_type: output.output_type.into(),
        }
    }
}

/// Output type enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum OutputType {
    /// Default: JSON value
    #[default]
    Value,

    /// File reference
    File,

    /// Folder reference (post-MVP)
    Folder,
}

impl From<kruxiaflow_core::workflow::OutputType> for OutputType {
    fn from(output_type: kruxiaflow_core::workflow::OutputType) -> Self {
        match output_type {
            kruxiaflow_core::workflow::OutputType::Value => Self::Value,
            kruxiaflow_core::workflow::OutputType::File => Self::File,
            kruxiaflow_core::workflow::OutputType::Folder => Self::Folder,
        }
    }
}

impl From<OutputType> for kruxiaflow_core::workflow::OutputType {
    fn from(output_type: OutputType) -> Self {
        match output_type {
            OutputType::Value => Self::Value,
            OutputType::File => Self::File,
            OutputType::Folder => Self::Folder,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    // --- StreamingConfig tests ---

    #[test]
    fn test_streaming_config_default_is_disabled() {
        let config = StreamingConfig::default();
        assert!(!config.is_enabled());
        assert!(config.is_disabled());
    }

    #[test]
    fn test_streaming_config_simple_true() {
        let config = StreamingConfig::Simple(true);
        assert!(config.is_enabled());
        assert!(!config.is_disabled());
    }

    #[test]
    fn test_streaming_config_simple_false() {
        let config = StreamingConfig::Simple(false);
        assert!(!config.is_enabled());
        assert!(config.is_disabled());
    }

    #[test]
    fn test_streaming_config_detailed_enabled() {
        let config = StreamingConfig::Detailed(StreamingOptions { enabled: true });
        assert!(config.is_enabled());
    }

    #[test]
    fn test_streaming_config_detailed_disabled() {
        let config = StreamingConfig::Detailed(StreamingOptions { enabled: false });
        assert!(!config.is_enabled());
    }

    #[test]
    fn test_streaming_config_deserialize_bool_true() {
        let config: StreamingConfig = serde_json::from_str("true").unwrap();
        assert!(config.is_enabled());
    }

    #[test]
    fn test_streaming_config_deserialize_bool_false() {
        let config: StreamingConfig = serde_json::from_str("false").unwrap();
        assert!(!config.is_enabled());
    }

    #[test]
    fn test_streaming_config_deserialize_object() {
        let config: StreamingConfig = serde_json::from_str(r#"{"enabled": true}"#).unwrap();
        assert!(config.is_enabled());
    }

    #[test]
    fn test_streaming_config_deserialize_null() {
        let config: StreamingConfig = serde_json::from_str("null").unwrap();
        assert!(!config.is_enabled());
    }

    #[test]
    fn test_streaming_config_deserialize_invalid() {
        let result: Result<StreamingConfig, _> = serde_json::from_str("\"invalid\"");
        assert!(result.is_err());
    }

    #[test]
    fn test_streaming_config_deserialize_number_invalid() {
        let result: Result<StreamingConfig, _> = serde_json::from_str("42");
        assert!(result.is_err());
    }

    // --- StreamingConfig From conversions ---

    #[test]
    fn test_streaming_config_from_core_disabled() {
        let core = kruxiaflow_core::workflow::StreamingConfig::Disabled;
        let api: StreamingConfig = core.into();
        assert!(matches!(api, StreamingConfig::Disabled));
    }

    #[test]
    fn test_streaming_config_from_core_simple() {
        let core = kruxiaflow_core::workflow::StreamingConfig::Simple(true);
        let api: StreamingConfig = core.into();
        assert!(matches!(api, StreamingConfig::Simple(true)));
    }

    #[test]
    fn test_streaming_config_from_core_detailed() {
        let core = kruxiaflow_core::workflow::StreamingConfig::Detailed(
            kruxiaflow_core::workflow::StreamingOptions { enabled: true },
        );
        let api: StreamingConfig = core.into();
        assert!(matches!(api, StreamingConfig::Detailed(_)));
    }

    #[test]
    fn test_streaming_config_to_core_disabled() {
        let api = StreamingConfig::Disabled;
        let core: kruxiaflow_core::workflow::StreamingConfig = api.into();
        assert!(matches!(
            core,
            kruxiaflow_core::workflow::StreamingConfig::Disabled
        ));
    }

    #[test]
    fn test_streaming_config_to_core_simple() {
        let api = StreamingConfig::Simple(true);
        let core: kruxiaflow_core::workflow::StreamingConfig = api.into();
        assert!(matches!(
            core,
            kruxiaflow_core::workflow::StreamingConfig::Simple(true)
        ));
    }

    // --- OnTimeout conversions ---

    #[test]
    fn test_on_timeout_default_is_fail() {
        assert_eq!(OnTimeout::default(), OnTimeout::Fail);
    }

    #[test]
    fn test_on_timeout_from_core() {
        assert_eq!(
            OnTimeout::from(kruxiaflow_core::workflow::OnTimeout::Continue),
            OnTimeout::Continue
        );
        assert_eq!(
            OnTimeout::from(kruxiaflow_core::workflow::OnTimeout::Skip),
            OnTimeout::Skip
        );
        assert_eq!(
            OnTimeout::from(kruxiaflow_core::workflow::OnTimeout::Fail),
            OnTimeout::Fail
        );
    }

    #[test]
    fn test_on_timeout_to_core() {
        let core: kruxiaflow_core::workflow::OnTimeout = OnTimeout::Continue.into();
        assert!(matches!(
            core,
            kruxiaflow_core::workflow::OnTimeout::Continue
        ));
        let core: kruxiaflow_core::workflow::OnTimeout = OnTimeout::Skip.into();
        assert!(matches!(core, kruxiaflow_core::workflow::OnTimeout::Skip));
    }

    #[test]
    fn test_on_timeout_serde_roundtrip() {
        let json = serde_json::to_string(&OnTimeout::Continue).unwrap();
        assert_eq!(json, "\"continue\"");
        let back: OnTimeout = serde_json::from_str(&json).unwrap();
        assert_eq!(back, OnTimeout::Continue);
    }

    // --- BudgetAction conversions ---

    #[test]
    fn test_budget_action_default_is_abort() {
        assert_eq!(BudgetAction::default(), BudgetAction::Abort);
    }

    #[test]
    fn test_budget_action_from_core() {
        assert_eq!(
            BudgetAction::from(kruxiaflow_core::workflow::BudgetAction::Abort),
            BudgetAction::Abort
        );
        assert_eq!(
            BudgetAction::from(kruxiaflow_core::workflow::BudgetAction::Continue),
            BudgetAction::Continue
        );
    }

    #[test]
    fn test_budget_action_to_core() {
        let core: kruxiaflow_core::workflow::BudgetAction = BudgetAction::Abort.into();
        assert!(matches!(
            core,
            kruxiaflow_core::workflow::BudgetAction::Abort
        ));
    }

    #[test]
    fn test_budget_settings_roundtrip() {
        let api = BudgetSettings {
            limit: Decimal::from_str("10.50").unwrap(),
            action: BudgetAction::Continue,
        };
        let core: kruxiaflow_core::workflow::BudgetSettings = api.clone().into();
        let back: BudgetSettings = core.into();
        assert_eq!(back.limit, api.limit);
        assert_eq!(back.action, api.action);
    }

    // --- BackoffStrategy conversions ---

    #[test]
    fn test_backoff_strategy_from_core() {
        assert!(matches!(
            BackoffStrategy::from(kruxiaflow_core::workflow::BackoffStrategy::Fixed),
            BackoffStrategy::Fixed
        ));
        assert!(matches!(
            BackoffStrategy::from(kruxiaflow_core::workflow::BackoffStrategy::Exponential),
            BackoffStrategy::Exponential
        ));
    }

    #[test]
    fn test_backoff_strategy_to_core() {
        let core: kruxiaflow_core::workflow::BackoffStrategy = BackoffStrategy::Fixed.into();
        assert!(matches!(
            core,
            kruxiaflow_core::workflow::BackoffStrategy::Fixed
        ));
    }

    // --- RetrySettings conversions ---

    #[test]
    fn test_retry_settings_roundtrip() {
        let api = RetrySettings {
            max_attempts: 3,
            strategy: BackoffStrategy::Exponential,
            base_seconds: 2,
            factor: 2.0,
            max_seconds: 300,
        };
        let core: kruxiaflow_core::workflow::RetryPolicy = api.into();
        let back: RetrySettings = core.into();
        assert_eq!(back.max_attempts, 3);
        assert_eq!(back.base_seconds, 2);
        assert_eq!(back.max_seconds, 300);
    }

    #[test]
    fn test_retry_settings_defaults() {
        let json = r#"{"max_attempts": 3}"#;
        let settings: RetrySettings = serde_json::from_str(json).unwrap();
        assert_eq!(settings.max_attempts, 3);
        assert!(matches!(settings.strategy, BackoffStrategy::Exponential));
        assert_eq!(settings.base_seconds, 2);
        assert_eq!(settings.factor, 2.0);
        assert_eq!(settings.max_seconds, 300);
    }

    // --- OutputType conversions ---

    #[test]
    fn test_output_type_default_is_value() {
        assert!(matches!(OutputType::default(), OutputType::Value));
    }

    #[test]
    fn test_output_type_roundtrip() {
        let api = OutputType::File;
        let core: kruxiaflow_core::workflow::OutputType = api.into();
        let back: OutputType = core.into();
        assert!(matches!(back, OutputType::File));
    }

    #[test]
    fn test_output_type_folder_roundtrip() {
        let api = OutputType::Folder;
        let core: kruxiaflow_core::workflow::OutputType = api.into();
        let back: OutputType = core.into();
        assert!(matches!(back, OutputType::Folder));
    }

    #[test]
    fn test_output_type_serde() {
        let json = serde_json::to_string(&OutputType::File).unwrap();
        assert_eq!(json, "\"file\"");
        let back: OutputType = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, OutputType::File));
    }

    // --- ActivityOutputDefinition conversion ---

    #[test]
    fn test_activity_output_definition_roundtrip() {
        let api = ActivityOutputDefinition {
            name: "document".to_string(),
            output_type: OutputType::File,
        };
        let core: kruxiaflow_core::workflow::ActivityOutputDefinition = api.into();
        let back: ActivityOutputDefinition = core.into();
        assert_eq!(back.name, "document");
        assert!(matches!(back.output_type, OutputType::File));
    }

    // --- ActivityRelationship conversion ---

    #[test]
    fn test_activity_relationship_roundtrip() {
        let api = ActivityRelationship {
            activity_key: "step1".to_string(),
            conditions: Some(vec!["output.status == 'ok'".to_string()]),
            is_back_edge: true,
        };
        let core: kruxiaflow_core::workflow::ActivityRelationship = api.into();
        let back: ActivityRelationship = core.into();
        assert_eq!(back.activity_key, "step1");
        assert!(back.is_back_edge);
        assert_eq!(back.conditions.unwrap().len(), 1);
    }

    #[test]
    fn test_activity_relationship_no_conditions() {
        let api = ActivityRelationship {
            activity_key: "step1".to_string(),
            conditions: None,
            is_back_edge: false,
        };
        let core: kruxiaflow_core::workflow::ActivityRelationship = api.into();
        assert!(core.conditions.is_none());
    }

    // --- WaitForSignalSettings conversion ---

    #[test]
    fn test_wait_for_signal_settings_roundtrip() {
        let api = WaitForSignalSettings {
            event_name: "approval".to_string(),
            timeout_seconds: 3600,
            on_timeout: OnTimeout::Skip,
        };
        let core: kruxiaflow_core::workflow::WaitForSignalSettings = api.into();
        let back: WaitForSignalSettings = core.into();
        assert_eq!(back.event_name, "approval");
        assert_eq!(back.timeout_seconds, 3600);
        assert_eq!(back.on_timeout, OnTimeout::Skip);
    }

    // --- ActivitySettings conversion ---

    #[test]
    fn test_activity_settings_default() {
        let settings = ActivitySettings::default();
        assert!(settings.timeout_seconds.is_none());
        assert!(settings.retry.is_none());
        assert!(settings.budget.is_none());
        assert!(!settings.cache);
        assert!(settings.cache_ttl.is_none());
        assert!(settings.delay.is_none());
        assert!(settings.scheduled_for.is_none());
        assert!(settings.wait_for_signal.is_none());
    }

    #[test]
    fn test_activity_settings_roundtrip_with_all_fields() {
        let api = ActivitySettings {
            timeout_seconds: Some(300),
            retry: Some(RetrySettings {
                max_attempts: 3,
                strategy: BackoffStrategy::Fixed,
                base_seconds: 5,
                factor: 1.0,
                max_seconds: 60,
            }),
            budget: Some(BudgetSettings {
                limit: Decimal::from_str("50.00").unwrap(),
                action: BudgetAction::Continue,
            }),
            cache: true,
            cache_ttl: Some(600),
            iteration_limit: Some(10),
            delay: Some("5s".to_string()),
            scheduled_for: Some("2025-12-01T09:00:00Z".to_string()),
            wait_for_signal: Some(WaitForSignalSettings {
                event_name: "done".to_string(),
                timeout_seconds: 120,
                on_timeout: OnTimeout::Continue,
            }),
            ..Default::default()
        };
        let core: kruxiaflow_core::workflow::ActivitySettings = api.into();
        let back: ActivitySettings = core.into();
        assert_eq!(back.timeout_seconds, Some(300));
        assert!(back.retry.is_some());
        assert!(back.budget.is_some());
        assert!(back.cache);
        assert_eq!(back.cache_ttl, Some(600));
        assert_eq!(back.delay, Some("5s".to_string()));
        assert!(back.wait_for_signal.is_some());
    }

    // --- WorkflowDefinition conversion ---

    #[test]
    fn test_workflow_definition_roundtrip() {
        let api = WorkflowDefinition {
            name: "test-workflow".to_string(),
            settings: None,
            activities: vec![ActivityDefinition {
                key: "step1".to_string(),
                worker: "std".to_string(),
                activity_name: Some("http_request".to_string()),
                parameters: None,
                depends_on: None,
                dependency_of: None,
                output_definitions: None,
                settings: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: StreamingConfig::default(),
            }],
        };
        let core: kruxiaflow_core::workflow::WorkflowDefinition = api.clone().into();
        let back: WorkflowDefinition = core.into();
        assert_eq!(back.name, "test-workflow");
        assert_eq!(back.activities.len(), 1);
        assert_eq!(back.activities[0].key, "step1");
    }
}
