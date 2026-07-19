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
/// for a workflow. Uses the workflow_cost_summary view.
///
/// # Response
/// - 200 OK: Cost summary returned
/// - 404 Not Found: Workflow does not exist
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
    // Query workflow cost from view
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
    /// None for lump-sum and non-LLM cost line items
    pub provider: Option<String>,
    /// None for lump-sum and non-LLM cost line items
    pub model: Option<String>,
    pub budget_limit_usd: Option<Decimal>,
    pub budget_exceeded: Option<bool>,
    /// Budget enforcement outcome: "abort" (activity aborted before execution,
    /// zero-cost row) or "downgrade" (fallback chain skipped models for budget
    /// reasons). None for ordinary cost rows.
    pub budget_event: Option<String>,
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
            activity_budget_limit_usd as budget_limit_usd,
            budget_exceeded,
            budget_event,
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
    /// Group costs by one dimension: provider | model | definition | day.
    #[serde(default)]
    pub group_by: Option<String>,
    /// Row limit for top_workflows / top_definitions (default 10, max 10000).
    #[serde(default)]
    pub limit: Option<i64>,
}

fn default_start_date() -> DateTime<Utc> {
    Utc::now() - chrono::Duration::days(30)
}

/// One bucket of a `group_by` aggregation.
#[derive(Debug, Serialize, ToSchema)]
pub struct CostGroup {
    /// Group key: provider name, "provider/model", definition name, or
    /// "YYYY-MM-DD" (UTC). None groups lump-sum line items with no
    /// provider/model.
    pub key: Option<String>,
    pub total_cost_usd: Decimal,
    pub activities: i64,
    pub workflows: i64,
    pub total_tokens: i64,
}

/// One of the most expensive workflows in the period.
#[derive(Debug, Serialize, ToSchema)]
pub struct TopWorkflow {
    pub workflow_id: Uuid,
    pub definition_name: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub total_cost_usd: Decimal,
    pub activities: i64,
    pub budget_limit_usd: Option<Decimal>,
}

/// One of the most expensive workflow definitions in the period.
#[derive(Debug, Serialize, ToSchema)]
pub struct TopDefinition {
    pub definition_name: String,
    pub workflows: i64,
    pub total_cost_usd: Decimal,
    pub avg_cost_per_workflow: Decimal,
}

/// A budget enforcement event (abort or downgrade) recorded in the period.
#[derive(Debug, Serialize, ToSchema)]
pub struct BudgetEvent {
    pub workflow_id: Uuid,
    pub definition_name: String,
    pub activity_key: String,
    /// "abort" or "downgrade"
    pub event: String,
    pub estimated_cost_usd: Option<Decimal>,
    pub budget_limit_usd: Option<Decimal>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CostAnalytics {
    pub total_workflows: i64,
    pub total_cost_usd: Decimal,
    pub avg_cost_per_activity: Decimal,
    pub start_date: DateTime<Utc>,
    pub end_date: DateTime<Utc>,
    /// Cost line items in the period (budget aborts excluded).
    pub total_activities: i64,
    pub avg_cost_per_workflow: Decimal,
    pub total_tokens: i64,
    pub cached_tokens: i64,
    /// Share of prompt tokens served from provider prompt caches
    /// (cached_tokens / prompt_tokens). None when no prompt tokens recorded.
    pub cache_hit_rate: Option<f64>,
    /// Activities aborted before execution by budget enforcement.
    pub budget_aborts: i64,
    /// Completed activities whose fallback chain skipped models for budget
    /// reasons (ran on a cheaper model).
    pub budget_downgrades: i64,
    /// Dimension used for `groups` (echoed from the request). None when no
    /// group_by was requested.
    pub group_by: Option<String>,
    /// Aggregation buckets, present when `group_by` was requested. Ordered by
    /// cost descending (chronological for group_by=day).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<CostGroup>>,
    /// Most expensive workflows in the period, cost descending.
    pub top_workflows: Vec<TopWorkflow>,
    /// Most expensive definitions in the period, cost descending.
    pub top_definitions: Vec<TopDefinition>,
    /// Recent budget enforcement events in the period, newest first (max 50).
    pub budget_events: Vec<BudgetEvent>,
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
    // Validate group_by before touching the database
    match params.group_by.as_deref() {
        None | Some("provider") | Some("model") | Some("definition") | Some("day") => {}
        Some(other) => {
            tracing::debug!("Invalid group_by value: {}", other);
            return Err(StatusCode::BAD_REQUEST);
        }
    }
    let limit = params.limit.unwrap_or(10).clamp(1, 10_000);

    let summary = sqlx::query!(
        r#"
        SELECT
            COUNT(DISTINCT workflow_id) FILTER (WHERE budget_event IS DISTINCT FROM 'abort') as "total_workflows!",
            COUNT(*) FILTER (WHERE budget_event IS DISTINCT FROM 'abort') as "total_activities!",
            COALESCE(SUM(cost_usd), 0.0) as "total_cost!",
            COALESCE(AVG(cost_usd) FILTER (WHERE budget_event IS DISTINCT FROM 'abort'), 0.0) as "avg_cost!",
            COALESCE(SUM(total_tokens), 0)::BIGINT as "total_tokens!",
            COALESCE(SUM(cached_tokens), 0)::BIGINT as "cached_tokens!",
            COALESCE(SUM(prompt_tokens), 0)::BIGINT as "prompt_tokens!",
            COUNT(*) FILTER (WHERE budget_event = 'abort') as "budget_aborts!",
            COUNT(*) FILTER (WHERE budget_event = 'downgrade') as "budget_downgrades!"
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

    let groups = match params.group_by.as_deref() {
        Some(dimension) => {
            Some(fetch_cost_groups(&state, dimension, params.start_date, params.end_date).await?)
        }
        None => None,
    };

    let top_workflows = sqlx::query_as!(
        TopWorkflow,
        r#"
        SELECT
            ac.workflow_id as "workflow_id!",
            w.definition_name as "definition_name!",
            w.status::TEXT as "status!",
            w.created_at as "created_at!",
            COALESCE(SUM(ac.cost_usd), 0.0) as "total_cost_usd!",
            COUNT(*) FILTER (WHERE ac.budget_event IS DISTINCT FROM 'abort') as "activities!",
            w.budget_limit_usd
        FROM activity_costs ac
        JOIN workflows w ON w.id = ac.workflow_id
        WHERE ac.created_at >= $1 AND ac.created_at <= $2
        GROUP BY ac.workflow_id, w.definition_name, w.status, w.created_at, w.budget_limit_usd
        ORDER BY COALESCE(SUM(ac.cost_usd), 0.0) DESC
        LIMIT $3
        "#,
        params.start_date,
        params.end_date,
        limit
    )
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to fetch top workflows: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let top_definitions = sqlx::query!(
        r#"
        SELECT
            w.definition_name as "definition_name!",
            COUNT(DISTINCT ac.workflow_id) as "workflows!",
            COALESCE(SUM(ac.cost_usd), 0.0) as "total_cost_usd!"
        FROM activity_costs ac
        JOIN workflows w ON w.id = ac.workflow_id
        WHERE ac.created_at >= $1 AND ac.created_at <= $2
        GROUP BY w.definition_name
        ORDER BY COALESCE(SUM(ac.cost_usd), 0.0) DESC
        LIMIT $3
        "#,
        params.start_date,
        params.end_date,
        limit
    )
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to fetch top definitions: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?
    .into_iter()
    .map(|row| {
        let avg = if row.workflows > 0 {
            row.total_cost_usd / Decimal::from(row.workflows)
        } else {
            Decimal::ZERO
        };
        TopDefinition {
            definition_name: row.definition_name,
            workflows: row.workflows,
            total_cost_usd: row.total_cost_usd,
            avg_cost_per_workflow: avg,
        }
    })
    .collect();

    let budget_events = sqlx::query_as!(
        BudgetEvent,
        r#"
        SELECT
            ac.workflow_id as "workflow_id!",
            w.definition_name as "definition_name!",
            ac.activity_key as "activity_key!",
            ac.budget_event as "event!",
            ac.estimated_cost_usd,
            COALESCE(ac.activity_budget_limit_usd, ac.workflow_budget_limit_usd) as budget_limit_usd,
            ac.created_at as "created_at!"
        FROM activity_costs ac
        JOIN workflows w ON w.id = ac.workflow_id
        WHERE ac.budget_event IS NOT NULL
          AND ac.created_at >= $1 AND ac.created_at <= $2
        ORDER BY ac.created_at DESC
        LIMIT 50
        "#,
        params.start_date,
        params.end_date
    )
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to fetch budget events: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let avg_cost_per_workflow = if summary.total_workflows > 0 {
        summary.total_cost / Decimal::from(summary.total_workflows)
    } else {
        Decimal::ZERO
    };
    let cache_hit_rate = if summary.prompt_tokens > 0 {
        Some(summary.cached_tokens as f64 / summary.prompt_tokens as f64)
    } else {
        None
    };

    Ok(Json(CostAnalytics {
        total_workflows: summary.total_workflows,
        total_cost_usd: summary.total_cost,
        avg_cost_per_activity: summary.avg_cost,
        start_date: params.start_date,
        end_date: params.end_date,
        total_activities: summary.total_activities,
        avg_cost_per_workflow,
        total_tokens: summary.total_tokens,
        cached_tokens: summary.cached_tokens,
        cache_hit_rate,
        budget_aborts: summary.budget_aborts,
        budget_downgrades: summary.budget_downgrades,
        group_by: params.group_by.clone(),
        groups,
        top_workflows,
        top_definitions,
        budget_events,
    }))
}

/// Server-side aggregation for `group_by` — one static query per dimension so
/// sqlx can verify each at compile time. Shared semantics: budget-abort rows
/// are excluded (zero-cost enforcement markers, not spend), buckets are
/// ordered by cost descending except `day`, which is chronological.
async fn fetch_cost_groups(
    state: &AppState,
    dimension: &str,
    start_date: DateTime<Utc>,
    end_date: DateTime<Utc>,
) -> Result<Vec<CostGroup>, StatusCode> {
    let result = match dimension {
        "provider" => {
            sqlx::query_as!(
                CostGroup,
                r#"
                SELECT
                    provider as key,
                    COALESCE(SUM(cost_usd), 0.0) as "total_cost_usd!",
                    COUNT(*) as "activities!",
                    COUNT(DISTINCT workflow_id) as "workflows!",
                    COALESCE(SUM(total_tokens), 0)::BIGINT as "total_tokens!"
                FROM activity_costs
                WHERE created_at >= $1 AND created_at <= $2
                  AND budget_event IS DISTINCT FROM 'abort'
                GROUP BY provider
                ORDER BY COALESCE(SUM(cost_usd), 0.0) DESC
                "#,
                start_date,
                end_date
            )
            .fetch_all(&state.db_pool)
            .await
        }
        "model" => {
            sqlx::query_as!(
                CostGroup,
                r#"
                SELECT
                    CASE WHEN model IS NULL THEN NULL
                         ELSE COALESCE(provider || '/', '') || model
                    END as key,
                    COALESCE(SUM(cost_usd), 0.0) as "total_cost_usd!",
                    COUNT(*) as "activities!",
                    COUNT(DISTINCT workflow_id) as "workflows!",
                    COALESCE(SUM(total_tokens), 0)::BIGINT as "total_tokens!"
                FROM activity_costs
                WHERE created_at >= $1 AND created_at <= $2
                  AND budget_event IS DISTINCT FROM 'abort'
                GROUP BY 1
                ORDER BY COALESCE(SUM(cost_usd), 0.0) DESC
                "#,
                start_date,
                end_date
            )
            .fetch_all(&state.db_pool)
            .await
        }
        "definition" => {
            sqlx::query_as!(
                CostGroup,
                r#"
                SELECT
                    w.definition_name as "key?",
                    COALESCE(SUM(ac.cost_usd), 0.0) as "total_cost_usd!",
                    COUNT(*) as "activities!",
                    COUNT(DISTINCT ac.workflow_id) as "workflows!",
                    COALESCE(SUM(ac.total_tokens), 0)::BIGINT as "total_tokens!"
                FROM activity_costs ac
                JOIN workflows w ON w.id = ac.workflow_id
                WHERE ac.created_at >= $1 AND ac.created_at <= $2
                  AND ac.budget_event IS DISTINCT FROM 'abort'
                GROUP BY w.definition_name
                ORDER BY COALESCE(SUM(ac.cost_usd), 0.0) DESC
                "#,
                start_date,
                end_date
            )
            .fetch_all(&state.db_pool)
            .await
        }
        "day" => {
            sqlx::query_as!(
                CostGroup,
                r#"
                SELECT
                    TO_CHAR(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD') as key,
                    COALESCE(SUM(cost_usd), 0.0) as "total_cost_usd!",
                    COUNT(*) as "activities!",
                    COUNT(DISTINCT workflow_id) as "workflows!",
                    COALESCE(SUM(total_tokens), 0)::BIGINT as "total_tokens!"
                FROM activity_costs
                WHERE created_at >= $1 AND created_at <= $2
                  AND budget_event IS DISTINCT FROM 'abort'
                GROUP BY 1
                ORDER BY 1 ASC
                "#,
                start_date,
                end_date
            )
            .fetch_all(&state.db_pool)
            .await
        }
        // Validated by the caller
        _ => return Err(StatusCode::BAD_REQUEST),
    };

    result.map_err(|e| {
        tracing::error!("Failed to fetch cost groups by {}: {}", dimension, e);
        StatusCode::INTERNAL_SERVER_ERROR
    })
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

    fn setup_test_state(pool: PgPool) -> AppState {
        use kruxiaflow_core::cache::NoOpCache;

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

    #[sqlx::test(migrations = "../migrations")]
    async fn test_get_workflow_cost_not_found(pool: PgPool) {
        let state = setup_test_state(pool);
        let workflow_id = Uuid::nil(); // UUID that doesn't exist

        let result = get_workflow_cost(State(state), Path(workflow_id)).await;

        assert!(
            result.is_err(),
            "Should return error for non-existent workflow"
        );
        assert_eq!(result.unwrap_err(), StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_get_workflow_cost_history_empty(pool: PgPool) {
        let state = setup_test_state(pool);
        let workflow_id = Uuid::nil(); // UUID for test

        let result = get_workflow_cost_history(State(state), Path(workflow_id)).await;

        // Should return empty list, not error
        assert!(result.is_ok(), "Should return OK for empty history");
        let history = result.unwrap().0;
        assert_eq!(history.len(), 0, "History should be empty");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_get_cost_analytics(pool: PgPool) {
        let state = setup_test_state(pool);

        let now = Utc::now();
        let params = CostAnalyticsParams {
            start_date: now - chrono::Duration::days(30),
            end_date: now,
            group_by: None,
            limit: None,
        };

        let result = get_cost_analytics(State(state), Query(params)).await;

        assert!(result.is_ok(), "Should return analytics");
        let analytics = result.unwrap().0;
        assert!(analytics.total_workflows >= 0);
        assert!(analytics.total_cost_usd >= Decimal::from_str("0.0").unwrap());
        assert!(analytics.avg_cost_per_activity >= Decimal::from_str("0.0").unwrap());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_cost_analytics_default_dates(pool: PgPool) {
        let state = setup_test_state(pool);

        // Test with default dates (last 30 days)
        let params = CostAnalyticsParams {
            start_date: default_start_date(),
            end_date: Utc::now(),
            group_by: None,
            limit: None,
        };

        let result = get_cost_analytics(State(state), Query(params)).await;

        assert!(result.is_ok(), "Should work with default dates");
    }

    #[test]
    fn test_cost_analytics_params_defaults() {
        let start = default_start_date();
        let now = Utc::now();
        // default start should be ~30 days ago
        let diff = now - start;
        assert!(diff.num_days() >= 29 && diff.num_days() <= 31);
    }

    #[test]
    fn test_workflow_cost_summary_serialize() {
        let summary = WorkflowCostSummary {
            workflow_id: Uuid::nil(),
            workflow_name: "test".to_string(),
            total_cost_usd: Decimal::from_str("1.50").unwrap(),
            budget_limit_usd: None,
            budget_remaining_usd: None,
            total_activities: 2,
        };
        let json = serde_json::to_value(&summary).unwrap();
        assert_eq!(json["total_activities"], 2);
        assert!(json["budget_limit_usd"].is_null());
    }

    #[test]
    fn test_activity_cost_detail_serialize() {
        let detail = ActivityCostDetail {
            activity_key: "llm_prompt".to_string(),
            attempt: 1,
            cost_usd: Decimal::from_str("0.0023").unwrap(),
            prompt_tokens: Some(150),
            output_tokens: Some(50),
            total_tokens: Some(200),
            cached_tokens: Some(0),
            provider: Some("anthropic".to_string()),
            model: Some("claude-3-sonnet".to_string()),
            budget_limit_usd: Some(Decimal::from_str("1.00").unwrap()),
            budget_exceeded: Some(false),
            budget_event: None,
            created_at: Utc::now(),
        };
        let json = serde_json::to_value(&detail).unwrap();
        assert_eq!(json["activity_key"], "llm_prompt");
        assert_eq!(json["prompt_tokens"], 150);
        assert_eq!(json["provider"], "anthropic");
    }

    #[test]
    fn test_cost_analytics_serialize() {
        let analytics = CostAnalytics {
            total_workflows: 10,
            total_cost_usd: Decimal::from_str("25.50").unwrap(),
            avg_cost_per_activity: Decimal::from_str("0.0255").unwrap(),
            start_date: Utc::now() - chrono::Duration::days(30),
            end_date: Utc::now(),
            total_activities: 1000,
            avg_cost_per_workflow: Decimal::from_str("2.55").unwrap(),
            total_tokens: 500_000,
            cached_tokens: 100_000,
            cache_hit_rate: Some(0.25),
            budget_aborts: 2,
            budget_downgrades: 3,
            group_by: None,
            groups: None,
            top_workflows: vec![],
            top_definitions: vec![],
            budget_events: vec![],
        };
        let json = serde_json::to_value(&analytics).unwrap();
        assert_eq!(json["total_workflows"], 10);
        assert_eq!(json["budget_aborts"], 2);
        assert!(json.get("groups").is_none());
    }

    #[test]
    fn test_budget_remaining_calculation_no_budget() {
        // When budget_limit_usd is None, budget_remaining should be None
        let budget_limit: Option<Decimal> = None;
        let budget_remaining = budget_limit
            .map(|limit| (limit - Decimal::from_str("5.00").unwrap()).max(Decimal::ZERO));
        assert!(budget_remaining.is_none());
    }

    #[test]
    fn test_budget_remaining_calculation_with_overspend() {
        let budget_limit = Some(Decimal::from_str("10.00").unwrap());
        let total_cost = Decimal::from_str("15.00").unwrap();
        let budget_remaining = budget_limit.map(|limit| (limit - total_cost).max(Decimal::ZERO));
        assert_eq!(budget_remaining, Some(Decimal::ZERO));
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
