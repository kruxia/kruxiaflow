/// Axum extractors for StreamFlow API
///
/// This module provides custom extractors that integrate StreamFlow core types
/// with Axum's handler system.
use crate::state::AppState;
use axum::{async_trait, extract::FromRequestParts, http::request::Parts};
use streamflow_core::workflow::{WorkflowDefinitionRepository, WorkflowService};

/// Axum extractor for WorkflowDefinitionRepository
///
/// Allows WorkflowDefinitionRepository to be extracted directly in handler signatures.
/// Automatically creates repository from AppState's db_pool.
///
/// # Example
/// ```rust,ignore
/// async fn handler(repo: WorkflowDefinitionRepository) -> impl IntoResponse {
///     // Use repo directly
/// }
/// ```
#[async_trait]
impl FromRequestParts<AppState> for WorkflowDefinitionRepository {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        _parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(WorkflowDefinitionRepository::new(state.db_pool.clone()))
    }
}

/// Axum extractor for WorkflowService
///
/// Allows WorkflowService to be extracted directly in handler signatures.
/// Automatically creates service from AppState's db_pool.
///
/// # Example
/// ```rust,ignore
/// async fn handler(service: WorkflowService) -> impl IntoResponse {
///     // Use service directly
/// }
/// ```
#[async_trait]
impl FromRequestParts<AppState> for WorkflowService {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        _parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(WorkflowService::new(state.db_pool.clone()))
    }
}
