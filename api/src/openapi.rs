use crate::error::{ApiError, ApiErrorResponse, ErrorCode};
use crate::handlers::cost::{ActivityCostDetail, CostAnalytics, WorkflowCostSummary};
use crate::handlers::llm_catalog::{
    ModelResponse, ModelSearchCriterion, ModelSearchRequest, ModelSearchResponse,
    ProviderResponse,
};
use crate::handlers::oauth::{GrantType, TokenRequest, TokenResponse};
use crate::handlers::workers::{
    ActivityError, ActivityHeartbeatRequest, ActivityHeartbeatResponse, CompleteActivityRequest,
    CompleteActivityResponse, FailActivityRequest, FailActivityResponse, PendingActivity,
    PollActivitiesRequest, PollActivitiesResponse,
};
use crate::handlers::workflow_definitions::{
    DeployWorkflowDefinitionRequest, DeployWorkflowDefinitionResponse,
    GetWorkflowDefinitionResponse, ListWorkflowDefinitionsResponse, WorkflowDefinitionSummary,
};
use crate::handlers::workflows::{
    ActivityState, GetWorkflowResponse, ListWorkflowsResponse, SubmitWorkflowRequest,
    SubmitWorkflowResponse, WorkflowSummary,
};
use crate::health::{LivenessResponse, ReadinessResponse, ServiceInfo};
use utoipa::OpenApi;

/// OpenAPI specification for StreamFlow API
///
/// This struct defines the complete API documentation using utoipa macros.
/// Schemas are automatically generated from Rust types at compile time.
///
/// Note: Contact info should match workspace authors in Cargo.toml
#[derive(OpenApi)]
#[openapi(
    info(
        title = "StreamFlow API",
        version = env!("CARGO_PKG_VERSION"),
        description = env!("CARGO_PKG_DESCRIPTION"),
        contact(
            name = "Sean Harrison",
            email = "sah@kruxia.com",
        )
    ),
    servers(
        (url = "http://localhost:8080", description = "Local development server")
    ),
    paths(
        // Health check endpoints
        crate::handlers::health::liveness_handler,
        crate::handlers::health::readiness_handler,
        crate::handlers::health::service_info_handler,

        // OAuth 2.0 endpoints
        crate::handlers::oauth::token_handler,

        // Workflow Definition Management
        crate::handlers::workflow_definitions::deploy_workflow_definition,
        crate::handlers::workflow_definitions::list_workflow_definitions,
        crate::handlers::workflow_definitions::get_workflow_definition,

        // Workflow Submission and Query
        crate::handlers::workflows::submit_workflow,
        crate::handlers::workflows::get_workflow,
        crate::handlers::workflows::list_workflows,

        // Worker Activity APIs
        crate::handlers::workers::poll_activities,
        crate::handlers::workers::heartbeat_activity,
        crate::handlers::workers::complete_activity,
        crate::handlers::workers::fail_activity,

        // Cost Tracking APIs
        crate::handlers::cost::get_workflow_cost,
        crate::handlers::cost::get_workflow_cost_history,
        crate::handlers::cost::get_cost_analytics,

        // LLM Catalog APIs
        crate::handlers::llm_catalog::list_providers,
        crate::handlers::llm_catalog::search_models,
    ),
    components(
        schemas(
            // Health check schemas
            LivenessResponse,
            ReadinessResponse,
            ServiceInfo,

            // OAuth 2.0 schemas
            TokenRequest,
            TokenResponse,
            GrantType,

            // Workflow Definition Management schemas
            DeployWorkflowDefinitionRequest,
            DeployWorkflowDefinitionResponse,
            GetWorkflowDefinitionResponse,
            ListWorkflowDefinitionsResponse,
            WorkflowDefinitionSummary,

            // Workflow Submission and Query schemas
            SubmitWorkflowRequest,
            SubmitWorkflowResponse,
            GetWorkflowResponse,
            ActivityState,
            ListWorkflowsResponse,
            WorkflowSummary,

            // Worker Activity schemas
            PollActivitiesRequest,
            PollActivitiesResponse,
            PendingActivity,
            ActivityHeartbeatRequest,
            ActivityHeartbeatResponse,
            CompleteActivityRequest,
            CompleteActivityResponse,
            FailActivityRequest,
            FailActivityResponse,
            ActivityError,

            // Cost Tracking schemas
            WorkflowCostSummary,
            ActivityCostDetail,
            CostAnalytics,

            // LLM Catalog schemas
            ProviderResponse,
            ModelResponse,
            ModelSearchCriterion,
            ModelSearchRequest,
            ModelSearchResponse,

            // Error response schemas
            ApiErrorResponse,
            ApiError,
            ErrorCode,
        )
    ),
    tags(
        (name = "Health", description = "Health check and service information endpoints"),
        (name = "Service", description = "Service metadata and capabilities"),
        (name = "OAuth 2.0", description = "OAuth 2.0 compliant token issuance (RFC 6749)"),
        (name = "Workflow Definitions", description = "Workflow definition deployment and management"),
        (name = "Workflows", description = "Workflow submission, query, and management"),
        (name = "Workers", description = "Worker activity polling and execution"),
        (name = "Cost Tracking", description = "LLM and activity cost tracking, budget enforcement, and analytics"),
        (name = "LLM Catalog", description = "LLM provider and model discovery with pricing information"),
    ),
    modifiers(&SecurityAddon)
)]
pub struct ApiDoc;

/// Add security scheme to OpenAPI spec
struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};

            let mut http = Http::new(HttpAuthScheme::Bearer);
            http.bearer_format = Some("JWT".to_string());
            http.description = Some("JWT Bearer token authentication".to_string());

            components.add_security_scheme("bearer_auth", SecurityScheme::Http(http))
        }
    }
}
