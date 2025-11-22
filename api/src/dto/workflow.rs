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
}

impl From<streamflow_core::workflow::WorkflowDefinition> for WorkflowDefinition {
    fn from(def: streamflow_core::workflow::WorkflowDefinition) -> Self {
        Self {
            name: def.name,
            activities: def.activities.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<WorkflowDefinition> for streamflow_core::workflow::WorkflowDefinition {
    fn from(def: WorkflowDefinition) -> Self {
        Self {
            name: def.name,
            activities: def.activities.into_iter().map(Into::into).collect(),
        }
    }
}

/// Activity definition wrapper
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ActivityDefinition {
    /// Unique key for this activity within the workflow
    pub key: String,

    /// Activity worker type (e.g., "builtin", "custom-python")
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
}

impl From<streamflow_core::workflow::ActivityDefinition> for ActivityDefinition {
    fn from(def: streamflow_core::workflow::ActivityDefinition) -> Self {
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
        }
    }
}

impl From<ActivityDefinition> for streamflow_core::workflow::ActivityDefinition {
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

impl From<streamflow_core::workflow::ActivityRelationship> for ActivityRelationship {
    fn from(rel: streamflow_core::workflow::ActivityRelationship) -> Self {
        Self {
            activity_key: rel.activity_key,
            conditions: rel.conditions,
            is_back_edge: rel.is_back_edge,
        }
    }
}

impl From<ActivityRelationship> for streamflow_core::workflow::ActivityRelationship {
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

impl From<streamflow_core::workflow::BudgetSettings> for BudgetSettings {
    fn from(settings: streamflow_core::workflow::BudgetSettings) -> Self {
        Self {
            limit: settings.limit,
            action: settings.action.into(),
        }
    }
}

impl From<BudgetSettings> for streamflow_core::workflow::BudgetSettings {
    fn from(settings: BudgetSettings) -> Self {
        Self {
            limit: settings.limit,
            action: settings.action.into(),
        }
    }
}

impl From<streamflow_core::workflow::BudgetAction> for BudgetAction {
    fn from(action: streamflow_core::workflow::BudgetAction) -> Self {
        match action {
            streamflow_core::workflow::BudgetAction::Abort => Self::Abort,
            streamflow_core::workflow::BudgetAction::Continue => Self::Continue,
        }
    }
}

impl From<BudgetAction> for streamflow_core::workflow::BudgetAction {
    fn from(action: BudgetAction) -> Self {
        match action {
            BudgetAction::Abort => Self::Abort,
            BudgetAction::Continue => Self::Continue,
        }
    }
}

impl From<streamflow_core::workflow::ActivitySettings> for ActivitySettings {
    fn from(settings: streamflow_core::workflow::ActivitySettings) -> Self {
        Self {
            timeout_seconds: settings.timeout_seconds,
            retry: settings.retry.map(Into::into),
            budget: settings.budget.map(Into::into),
            cache: settings.cache,
            cache_ttl: settings.cache_ttl,
            iteration_limit: settings.iteration_limit,
        }
    }
}

impl From<ActivitySettings> for streamflow_core::workflow::ActivitySettings {
    fn from(settings: ActivitySettings) -> Self {
        Self {
            timeout_seconds: settings.timeout_seconds,
            retry: settings.retry.map(Into::into),
            budget: settings.budget.map(Into::into),
            cache: settings.cache,
            cache_ttl: settings.cache_ttl,
            iteration_limit: settings.iteration_limit,
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

impl From<streamflow_core::workflow::RetryPolicy> for RetrySettings {
    fn from(settings: streamflow_core::workflow::RetryPolicy) -> Self {
        Self {
            max_attempts: settings.max_attempts,
            strategy: settings.strategy.into(),
            base_seconds: settings.base_seconds,
            factor: settings.factor,
            max_seconds: settings.max_seconds,
        }
    }
}

impl From<RetrySettings> for streamflow_core::workflow::RetryPolicy {
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

impl From<streamflow_core::workflow::BackoffStrategy> for BackoffStrategy {
    fn from(strategy: streamflow_core::workflow::BackoffStrategy) -> Self {
        match strategy {
            streamflow_core::workflow::BackoffStrategy::Fixed => Self::Fixed,
            streamflow_core::workflow::BackoffStrategy::Exponential => Self::Exponential,
        }
    }
}

impl From<BackoffStrategy> for streamflow_core::workflow::BackoffStrategy {
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

impl From<streamflow_core::workflow::ActivityOutputDefinition> for ActivityOutputDefinition {
    fn from(output: streamflow_core::workflow::ActivityOutputDefinition) -> Self {
        Self {
            name: output.name,
            output_type: output.output_type.into(),
        }
    }
}

impl From<ActivityOutputDefinition> for streamflow_core::workflow::ActivityOutputDefinition {
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

impl From<streamflow_core::workflow::OutputType> for OutputType {
    fn from(output_type: streamflow_core::workflow::OutputType) -> Self {
        match output_type {
            streamflow_core::workflow::OutputType::Value => Self::Value,
            streamflow_core::workflow::OutputType::File => Self::File,
            streamflow_core::workflow::OutputType::Folder => Self::Folder,
        }
    }
}

impl From<OutputType> for streamflow_core::workflow::OutputType {
    fn from(output_type: OutputType) -> Self {
        match output_type {
            OutputType::Value => Self::Value,
            OutputType::File => Self::File,
            OutputType::Folder => Self::Folder,
        }
    }
}
