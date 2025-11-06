//! Workflow API DTOs
//!
//! API-layer wrappers around core workflow types to provide OpenAPI schema
//! generation without coupling core to API concerns.
//!
//! These types mirror the structure of core types and provide bidirectional
//! From/Into conversions, allowing the API layer to derive ToSchema without
//! adding utoipa as a dependency to the core crate.

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

    /// Activity namespace (e.g., "payments", "llm")
    pub namespace: String,

    /// Activity name within namespace (e.g., "authorize", "complete")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Activity parameters (can include template expressions)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<HashMap<String, serde_json::Value>>,

    /// Activities that must complete before this one
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preceding: Option<Vec<ActivityRelationship>>,

    /// Activities that should run after this one
    #[serde(skip_serializing_if = "Option::is_none")]
    pub following: Option<Vec<ActivityRelationship>>,

    /// Activity-level settings (timeout, retry, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings: Option<ActivitySettings>,
}

impl From<streamflow_core::workflow::ActivityDefinition> for ActivityDefinition {
    fn from(def: streamflow_core::workflow::ActivityDefinition) -> Self {
        Self {
            key: def.key,
            namespace: def.namespace,
            name: def.name,
            parameters: def.parameters,
            preceding: def
                .preceding
                .map(|v| v.into_iter().map(Into::into).collect()),
            following: def
                .following
                .map(|v| v.into_iter().map(Into::into).collect()),
            settings: def.settings.map(Into::into),
        }
    }
}

impl From<ActivityDefinition> for streamflow_core::workflow::ActivityDefinition {
    fn from(def: ActivityDefinition) -> Self {
        Self {
            key: def.key,
            namespace: def.namespace,
            name: def.name,
            parameters: def.parameters,
            preceding: def
                .preceding
                .map(|v| v.into_iter().map(Into::into).collect()),
            following: def
                .following
                .map(|v| v.into_iter().map(Into::into).collect()),
            settings: def.settings.map(Into::into),
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
}

impl From<streamflow_core::workflow::ActivityRelationship> for ActivityRelationship {
    fn from(rel: streamflow_core::workflow::ActivityRelationship) -> Self {
        Self {
            activity_key: rel.activity_key,
            conditions: rel.conditions,
        }
    }
}

impl From<ActivityRelationship> for streamflow_core::workflow::ActivityRelationship {
    fn from(rel: ActivityRelationship) -> Self {
        Self {
            activity_key: rel.activity_key,
            conditions: rel.conditions,
        }
    }
}

/// Activity-level settings
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ActivitySettings {
    /// Activity timeout in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,

    /// Retry configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetrySettings>,
}

impl From<streamflow_core::workflow::ActivitySettings> for ActivitySettings {
    fn from(settings: streamflow_core::workflow::ActivitySettings) -> Self {
        Self {
            timeout: settings.timeout,
            retry: settings.retry.map(Into::into),
        }
    }
}

impl From<ActivitySettings> for streamflow_core::workflow::ActivitySettings {
    fn from(settings: ActivitySettings) -> Self {
        Self {
            timeout: settings.timeout,
            retry: settings.retry.map(Into::into),
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
    pub backoff: BackoffStrategy,
}

fn default_backoff() -> BackoffStrategy {
    BackoffStrategy::Exponential
}

impl From<streamflow_core::workflow::RetrySettings> for RetrySettings {
    fn from(settings: streamflow_core::workflow::RetrySettings) -> Self {
        Self {
            max_attempts: settings.max_attempts,
            backoff: settings.backoff.into(),
        }
    }
}

impl From<RetrySettings> for streamflow_core::workflow::RetrySettings {
    fn from(settings: RetrySettings) -> Self {
        Self {
            max_attempts: settings.max_attempts,
            backoff: settings.backoff.into(),
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
