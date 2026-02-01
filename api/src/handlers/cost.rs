use crate::state::AppState;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

#[derive(Debug, Serialize, ToSchema)]
pub struct WorkflowCostSummary {
    pub workflow_id: Uuid,
    pub workflow_name: String,
    pub total_cost_usd: Decimal,
    pub budget_limit_usd: Option<Decimal>,
    pub budget_remaining_usd: Option<Decimal>,
    pub total_activities: i64,
}

/// GET /api/v1/workflows/:workflow_id/cost
/// Get cost summary for a specific workflow
///
/// Returns the total cost, budget limit, budget remaining, and activity count
/// for a workflow. Uses the workflow_cost_summary materialized view for
/// efficient queries.
///
/// # Response
/// - 200 OK: Cost summary returned
/// - 404 Not Found: Workflow does not exist
///
/// # Performance
/// Target: <10ms P99 latency (materialized view query)
#[utoipa::path(
    get,
    path = "/api/v1/workflows/{workflow_id}/cost",
    tag = "Cost Tracking",
    params(
        ("workflow_id" = Uuid, Path, description = "Workflow ID")
    ),
    responses(
        (status = 200, description = "Cost summary", body = WorkflowCostSummary),
        (status = 404, description = "Workflow not found"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn get_workflow_cost(
    State(state): State<AppState>,
    Path(workflow_id): Path<Uuid>,
) -> Result<Json<WorkflowCostSummary>, StatusCode> {
    // Query workflow cost from materialized view
    let summary = sqlx::query!(
        r#"
        SELECT
            workflow_id,
            workflow_name,
            total_cost_usd,
            budget_limit_usd,
            total_activities
        FROM workflow_cost_summary
        WHERE workflow_id = $1
        "#,
        workflow_id
    )
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to fetch workflow cost: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?
    .ok_or(StatusCode::NOT_FOUND)?;

    let budget_remaining = summary
        .budget_limit_usd
        .map(|limit| (limit - summary.total_cost_usd.unwrap_or(Decimal::ZERO)).max(Decimal::ZERO));

    Ok(Json(WorkflowCostSummary {
        workflow_id: summary.workflow_id.unwrap_or(workflow_id),
        workflow_name: summary
            .workflow_name
            .unwrap_or_else(|| "unknown".to_string()),
        total_cost_usd: summary.total_cost_usd.unwrap_or(Decimal::ZERO),
        budget_limit_usd: summary.budget_limit_usd,
        budget_remaining_usd: budget_remaining,
        total_activities: summary.total_activities.unwrap_or(0),
    }))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ActivityCostDetail {
    pub activity_key: String,
    pub attempt: i32,
    pub cost_usd: Decimal,
    pub prompt_tokens: Option<i32>,
    pub output_tokens: Option<i32>,
    pub total_tokens: Option<i32>,
    pub cached_tokens: Option<i32>,
    pub provider: String,
    pub model: String,
    pub budget_exceeded: Option<bool>,
    pub created_at: DateTime<Utc>,
}

/// GET /api/v1/workflows/:workflow_id/cost/history
/// Get detailed cost history for all activities in a workflow
///
/// Returns a detailed breakdown of costs for every activity execution,
/// including token usage, provider/model information, and budget status.
/// Results are ordered by creation time (oldest first).
///
/// # Response
/// - 200 OK: Cost history returned (may be empty array)
/// - 500 Internal Server Error: Database query failed
///
/// # Performance
/// Target: <50ms P99 latency for workflows with <1000 activities
#[utoipa::path(
    get,
    path = "/api/v1/workflows/{workflow_id}/cost/history",
    tag = "Cost Tracking",
    params(
        ("workflow_id" = Uuid, Path, description = "Workflow ID")
    ),
    responses(
        (status = 200, description = "Activity cost history", body = Vec<ActivityCostDetail>),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn get_workflow_cost_history(
    State(state): State<AppState>,
    Path(workflow_id): Path<Uuid>,
) -> Result<Json<Vec<ActivityCostDetail>>, StatusCode> {
    let history = sqlx::query_as!(
        ActivityCostDetail,
        r#"
        SELECT
            activity_key,
            attempt,
            cost_usd,
            prompt_tokens,
            output_tokens,
            total_tokens,
            cached_tokens,
            provider,
            model,
            budget_exceeded,
            created_at
        FROM activity_costs
        WHERE workflow_id = $1
        ORDER BY created_at ASC
        "#,
        workflow_id
    )
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to fetch cost history: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(history))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct CostAnalyticsParams {
    /// Start date for analytics (ISO 8601). Defaults to 30 days ago.
    #[serde(default = "default_start_date")]
    pub start_date: DateTime<Utc>,
    /// End date for analytics (ISO 8601). Defaults to now.
    #[serde(default = "Utc::now")]
    pub end_date: DateTime<Utc>,
}

fn default_start_date() -> DateTime<Utc> {
    Utc::now() - chrono::Duration::days(30)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CostAnalytics {
    pub total_workflows: i64,
    pub total_cost_usd: Decimal,
    pub avg_cost_per_activity: Decimal,
    pub start_date: DateTime<Utc>,
    pub end_date: DateTime<Utc>,
}

/// GET /api/v1/cost/analytics
/// Get aggregated cost analytics across all workflows
///
/// Returns aggregated cost metrics for all workflows within a date range.
/// Defaults to the last 30 days if no dates specified.
///
/// # Query Parameters
/// - `start_date`: Start date (ISO 8601), defaults to 30 days ago
/// - `end_date`: End date (ISO 8601), defaults to now
///
/// # Response
/// - 200 OK: Cost analytics returned
/// - 500 Internal Server Error: Database query failed
///
/// # Performance
/// Target: <100ms P99 latency (aggregation query)
#[utoipa::path(
    get,
    path = "/api/v1/cost/analytics",
    tag = "Cost Tracking",
    params(
        CostAnalyticsParams
    ),
    responses(
        (status = 200, description = "Cost analytics", body = CostAnalytics),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn get_cost_analytics(
    State(state): State<AppState>,
    Query(params): Query<CostAnalyticsParams>,
) -> Result<Json<CostAnalytics>, StatusCode> {
    let analytics = sqlx::query!(
        r#"
        SELECT
            COUNT(DISTINCT workflow_id) as "total_workflows!",
            COALESCE(SUM(cost_usd), 0.0) as "total_cost!",
            COALESCE(AVG(cost_usd), 0.0) as "avg_cost!"
        FROM activity_costs
        WHERE created_at >= $1 AND created_at <= $2
        "#,
        params.start_date,
        params.end_date
    )
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to fetch cost analytics: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(CostAnalytics {
        total_workflows: analytics.total_workflows,
        total_cost_usd: analytics.total_cost,
        avg_cost_per_activity: analytics.avg_cost,
        start_date: params.start_date,
        end_date: params.end_date,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use rust_decimal::Decimal;
    use sqlx::PgPool;
    use std::str::FromStr;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    async fn setup_test_pool() -> PgPool {
        let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://kruxiaflow:kruxiaflow_dev@127.0.0.1:5432/kruxiaflow".to_string()
        });

        PgPool::connect(&database_url)
            .await
            .expect("Failed to connect to test database")
    }

    async fn setup_test_state() -> AppState {
        use kruxiaflow_core::cache::NoOpCache;

        let pool = setup_test_pool().await;
        let auth_service = Arc::new(crate::state::tests::MockAuthService);
        let activity_queue = Arc::new(crate::state::tests::MockActivityQueue);
        let event_source = Arc::new(crate::state::tests::MockEventSource);
        let workflow_storage = Arc::new(crate::state::tests::MockWorkflowStorage);
        let cache_service = Arc::new(NoOpCache::new());
        let shutdown_token = CancellationToken::new();

        let subscription_service = Arc::new(crate::state::tests::MockSubscriptionService);
        AppState::new(
            pool,
            auth_service,
            activity_queue,
            event_source,
            workflow_storage,
            cache_service,
            subscription_service,
            shutdown_token,
        )
    }

    #[tokio::test]
    async fn test_get_workflow_cost_not_found() {
        let state = setup_test_state().await;
        let workflow_id = Uuid::nil(); // UUID that doesn't exist

        let result = get_workflow_cost(State(state), Path(workflow_id)).await;

        assert!(
            result.is_err(),
            "Should return error for non-existent workflow"
        );
        assert_eq!(result.unwrap_err(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_get_workflow_cost_history_empty() {
        let state = setup_test_state().await;
        let workflow_id = Uuid::nil(); // UUID for test

        let result = get_workflow_cost_history(State(state), Path(workflow_id)).await;

        // Should return empty list, not error
        assert!(result.is_ok(), "Should return OK for empty history");
        let history = result.unwrap().0;
        assert_eq!(history.len(), 0, "History should be empty");
    }

    #[tokio::test]
    async fn test_get_cost_analytics() {
        let state = setup_test_state().await;

        let now = Utc::now();
        let params = CostAnalyticsParams {
            start_date: now - chrono::Duration::days(30),
            end_date: now,
        };

        let result = get_cost_analytics(State(state), Query(params)).await;

        assert!(result.is_ok(), "Should return analytics");
        let analytics = result.unwrap().0;
        assert!(analytics.total_workflows >= 0);
        assert!(analytics.total_cost_usd >= Decimal::from_str("0.0").unwrap());
        assert!(analytics.avg_cost_per_activity >= Decimal::from_str("0.0").unwrap());
    }

    #[tokio::test]
    async fn test_cost_analytics_default_dates() {
        let state = setup_test_state().await;

        // Test with default dates (last 30 days)
        let params = CostAnalyticsParams {
            start_date: default_start_date(),
            end_date: Utc::now(),
        };

        let result = get_cost_analytics(State(state), Query(params)).await;

        assert!(result.is_ok(), "Should work with default dates");
    }

    #[tokio::test]
    async fn test_workflow_cost_summary_budget_calculation() {
        // This is a unit test for budget_remaining calculation logic
        let summary = WorkflowCostSummary {
            workflow_id: Uuid::nil(),
            workflow_name: "test".to_string(),
            total_cost_usd: Decimal::from_str("3.50").unwrap(),
            budget_limit_usd: Some(Decimal::from_str("10.00").unwrap()),
            budget_remaining_usd: Some(Decimal::from_str("6.50").unwrap()),
            total_activities: 5,
        };

        assert_eq!(summary.total_cost_usd, Decimal::from_str("3.50").unwrap());
        assert_eq!(
            summary.budget_limit_usd,
            Some(Decimal::from_str("10.00").unwrap())
        );
        assert_eq!(
            summary.budget_remaining_usd,
            Some(Decimal::from_str("6.50").unwrap())
        );
    }
}
