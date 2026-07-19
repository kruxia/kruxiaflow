/// MCP Control Tools
///
/// Three tools for steering running workflows and managed state:
/// - send_workflow_signal: deliver a signal to a waiting workflow or activity
/// - list_waiting_workflows: find workflows with activities waiting for signals
/// - invalidate_cache: invalidate cached activity results by key or pattern
use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::{CallToolResult, schema_utils::CallToolError};
use rust_mcp_sdk::tool_box;
use sqlx::{PgPool, Row};
use std::collections::HashMap;

use super::{AnyJson, error_response, parse_uuid, text_response};

use kruxiaflow_core::{
    EventSource, NewWorkflowEvent, PostgresEventSource, PostgresSubscriptionService, SignalRequest,
    SubscriptionService, WorkflowEventType,
};

// ============================================================================
// Tool: send_workflow_signal
// ============================================================================

#[mcp_tool(
    name = "send_workflow_signal",
    description = "Send a signal to a workflow that is waiting for one.\n\
        \n\
        Activities with wait_for_signal settings pause execution until they receive \
        the named signal. This tool delivers that signal, optionally with a data \
        payload that the activity can use to continue its work.\n\
        \n\
        If activity_key is provided, only that activity is signaled (it must have an \
        active subscription for the signal_name). If omitted, the signal is delivered \
        to every activity in the workflow currently waiting for that signal_name — \
        matching the REST API's workflow-level signal semantics.\n\
        \n\
        When to use: After list_waiting_workflows identifies a workflow waiting \
        for a specific signal — for example, an approval gate waiting for a human \
        decision.",
    read_only_hint = false,
    destructive_hint = true,
    idempotent_hint = false
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct SendWorkflowSignal {
    /// UUID of the workflow containing the waiting activity
    pub workflow_id: String,

    /// Key of the activity to signal. Omit to signal every activity in the
    /// workflow currently waiting for signal_name.
    pub activity_key: Option<String>,

    /// Name of the signal event the activity is subscribed to
    pub signal_name: String,

    /// Optional data payload delivered to the activity
    pub data: Option<AnyJson>,
}

// ============================================================================
// Tool: list_waiting_workflows
// ============================================================================

#[mcp_tool(
    name = "list_waiting_workflows",
    description = "Find workflows that have activities currently waiting for signals.\n\
        \n\
        Scans active (running) workflows for activities with open signal \
        subscriptions that have not yet been delivered. Optionally filter by \
        signal_name to find only workflows waiting for a specific signal.\n\
        \n\
        When to use: To discover which workflows need human input or external \
        events before they can continue. Combine with send_workflow_signal to \
        deliver the required signal.",
    read_only_hint = true,
    idempotent_hint = true
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ListWaitingWorkflows {
    /// Only return workflows waiting for this specific signal name
    pub signal_name: Option<String>,

    /// Maximum number of workflows to return (default: 20)
    #[serde(default = "default_limit")]
    pub limit: Option<i64>,

    /// Number of workflows to skip for pagination (default: 0)
    #[serde(default)]
    pub offset: Option<i64>,
}

fn default_limit() -> Option<i64> {
    Some(20)
}

// ============================================================================
// Tool: invalidate_cache
// ============================================================================

#[mcp_tool(
    name = "invalidate_cache",
    description = "Invalidate cached activity results by key or pattern.\n\
        \n\
        Kruxia Flow caches activity results (notably llm_prompt responses) to cut \
        repeat costs. When the underlying context has changed and cached responses \
        are stale, use this to remove them. Provide either a specific cache key \
        (from an activity's metadata.cache_key) or a Redis glob pattern such as \
        \"std.llm_prompt:*\" — exactly one of the two.\n\
        \n\
        When to use: After source data or prompt context changes, so subsequent \
        workflow runs recompute instead of serving stale cached results.",
    read_only_hint = false,
    destructive_hint = true,
    idempotent_hint = true
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct InvalidateCache {
    /// Specific cache key to invalidate. Mutually exclusive with pattern.
    pub key: Option<String>,

    /// Redis glob pattern matching cache keys (e.g. "std.llm_prompt:*").
    /// Mutually exclusive with key.
    pub pattern: Option<String>,
}

// ============================================================================
// Enum + routing glue
// ============================================================================

tool_box!(
    ControlTools,
    [SendWorkflowSignal, ListWaitingWorkflows, InvalidateCache]
);

// ============================================================================
// Async runners
// ============================================================================

/// Deliver a signal and publish the orchestrator event.
///
/// With `activity_key` set, signals that one activity. Without it, delivers the
/// signal to every activity in the workflow currently waiting for `signal_name`
/// (workflow-level semantics, matching the REST API).
pub async fn run_send_workflow_signal(
    pool: &PgPool,
    params: &SendWorkflowSignal,
) -> Result<CallToolResult, CallToolError> {
    let workflow_id = parse_uuid(&params.workflow_id)?;

    // Resolve the target activity keys: the explicit one, or every activity in
    // the workflow with an open subscription for this signal name.
    let target_keys: Vec<String> = match &params.activity_key {
        Some(key) => vec![key.clone()],
        None => sqlx::query_scalar(
            "SELECT activity_key FROM activity_event_subscriptions \
             WHERE workflow_id = $1 AND event_name = $2 AND signal_data IS NULL \
             ORDER BY created_at",
        )
        .bind(workflow_id)
        .bind(&params.signal_name)
        .fetch_all(pool)
        .await
        .map_err(|e| {
            CallToolError::from_message(format!(
                "Database error finding waiting activities in workflow '{}': {e}",
                params.workflow_id
            ))
        })?,
    };

    if target_keys.is_empty() {
        return error_response(&serde_json::json!({
            "error": format!(
                "No activities in workflow '{}' are waiting for signal '{}'. \
                 They may have already been signaled or are not in a waiting state.",
                params.workflow_id, params.signal_name
            ),
            "workflow_id": params.workflow_id,
            "signal_name": params.signal_name,
        }));
    }

    let sub_svc = PostgresSubscriptionService::new(pool.clone());
    let event_source = PostgresEventSource::new(pool.clone());
    let mut signaled: Vec<serde_json::Value> = Vec::new();
    let mut not_signaled: Vec<String> = Vec::new();

    for activity_key in &target_keys {
        let request = SignalRequest {
            workflow_id,
            activity_key: activity_key.clone(),
            event_name: params.signal_name.clone(),
            data: params.data.as_ref().map(|v| v.0.clone()),
        };

        let subscription = match sub_svc.signal_activity(request).await {
            Ok(Some(sub)) => sub,
            Ok(None) => {
                // Raced with another signaler or the activity is not waiting
                not_signaled.push(activity_key.clone());
                continue;
            }
            Err(e) => {
                tracing::error!("send_workflow_signal error: {e:?}");
                return Err(CallToolError::from_message(format!(
                    "Error signaling activity '{}' in workflow '{}': {e}",
                    activity_key, params.workflow_id
                )));
            }
        };

        // Publish ActivitySignaled event so the orchestrator picks up the change.
        // This is best-effort: the orchestrator will discover the signal on its
        // next poll cycle even if this publish fails.
        let event_published = match event_source
            .publish(NewWorkflowEvent {
                workflow_id,
                event_type: WorkflowEventType::ActivitySignaled,
                activity_key: Some(activity_key.clone()),
                payload: params
                    .data
                    .as_ref()
                    .map(|v| v.0.clone())
                    .unwrap_or(serde_json::Value::Null),
                iteration: None,
            })
            .await
        {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(
                    "ActivitySignaled event publish failed for activity '{}' in workflow '{}': {e}",
                    activity_key,
                    params.workflow_id
                );
                false
            }
        };

        signaled.push(serde_json::json!({
            "activity_key": activity_key,
            "subscription_id": subscription.id.to_string(),
            "event_published": event_published,
        }));
    }

    if signaled.is_empty() {
        return error_response(&serde_json::json!({
            "error": format!(
                "No active subscription found for signal '{}' in workflow '{}'. \
                 The activity may have already been signaled or is not in a waiting state.",
                params.signal_name, params.workflow_id
            ),
            "workflow_id": params.workflow_id,
            "activity_key": params.activity_key,
            "signal_name": params.signal_name,
        }));
    }

    text_response(&serde_json::json!({
        "status": "signaled",
        "workflow_id": params.workflow_id,
        "signal_name": params.signal_name,
        "signaled_activities": signaled,
        "already_signaled_or_not_waiting": not_signaled,
    }))
}

/// Find workflows with activities that have open signal subscriptions.
pub async fn run_list_waiting_workflows(
    pool: &PgPool,
    params: &ListWaitingWorkflows,
) -> Result<CallToolResult, CallToolError> {
    let limit = params.limit.unwrap_or(20);
    let offset = params.offset.unwrap_or(0);

    // Use runtime sqlx::query() to avoid prepare-cache dependency on
    // the activity_event_subscriptions table.
    // TODO(#9): Migrate to stored procs with compile-time validation (sqlx::query!)
    // per project conventions.
    let signal_filter = params.signal_name.as_deref();

    let (base_query, count_query) = if signal_filter.is_some() {
        (
            "SELECT s.workflow_id, s.activity_key, s.event_name, s.created_at, \
             w.definition_name \
             FROM activity_event_subscriptions s \
             JOIN workflows w ON w.id = s.workflow_id \
             WHERE w.status = 'running' \
               AND s.signal_data IS NULL \
               AND s.event_name = $1 \
             ORDER BY s.created_at DESC \
             LIMIT $2 OFFSET $3",
            "SELECT COUNT(*) FROM activity_event_subscriptions s \
             JOIN workflows w ON w.id = s.workflow_id \
             WHERE w.status = 'running' \
               AND s.signal_data IS NULL \
               AND s.event_name = $1",
        )
    } else {
        (
            "SELECT s.workflow_id, s.activity_key, s.event_name, s.created_at, \
             w.definition_name \
             FROM activity_event_subscriptions s \
             JOIN workflows w ON w.id = s.workflow_id \
             WHERE w.status = 'running' \
               AND s.signal_data IS NULL \
             ORDER BY s.created_at DESC \
             LIMIT $1 OFFSET $2",
            "SELECT COUNT(*) FROM activity_event_subscriptions s \
             JOIN workflows w ON w.id = s.workflow_id \
             WHERE w.status = 'running' \
               AND s.signal_data IS NULL",
        )
    };

    let rows = if let Some(name) = signal_filter {
        sqlx::query(base_query)
            .bind(name)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await
    } else {
        sqlx::query(base_query)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await
    }
    .map_err(|e| {
        CallToolError::from_message(format!("Database error listing waiting workflows: {e}"))
    })?;

    let total: i64 = if let Some(name) = signal_filter {
        sqlx::query_scalar(count_query)
            .bind(name)
            .fetch_one(pool)
            .await
    } else {
        sqlx::query_scalar(count_query).fetch_one(pool).await
    }
    .unwrap_or(0);

    // Group results by workflow_id, preserving row order
    let mut workflow_map: HashMap<String, serde_json::Value> = HashMap::new();
    let mut workflow_order: Vec<String> = Vec::new();

    for row in &rows {
        let wf_id: uuid::Uuid = row.get(0);
        let activity_key: String = row.get(1);
        let event_name: String = row.get(2);
        let created_at: chrono::DateTime<chrono::Utc> = row.get(3);
        let definition_name: String = row.get(4);

        let wf_key = wf_id.to_string();

        if !workflow_map.contains_key(&wf_key) {
            workflow_map.insert(
                wf_key.clone(),
                serde_json::json!({
                    "workflow_id": wf_key,
                    "definition_name": definition_name,
                    "waiting_activities": [],
                }),
            );
            workflow_order.push(wf_key.clone());
        }

        if let Some(entry) = workflow_map.get_mut(&wf_key)
            && let Some(activities) = entry["waiting_activities"].as_array_mut()
        {
            activities.push(serde_json::json!({
                "activity_key": activity_key,
                "signal_name": event_name,
                "waiting_since": created_at.to_rfc3339(),
            }));
        }
    }

    let workflows: Vec<serde_json::Value> = workflow_order
        .iter()
        .filter_map(|k| workflow_map.remove(k))
        .collect();

    text_response(&serde_json::json!({
        "workflows": workflows,
        "total": total,
        "limit": limit,
        "offset": offset,
    }))
}

/// Invalidate cached activity results by key or pattern via the CacheService.
pub async fn run_invalidate_cache(
    cache: &dyn kruxiaflow_core::CacheService,
    params: &InvalidateCache,
) -> Result<CallToolResult, CallToolError> {
    match (&params.key, &params.pattern) {
        (Some(key), None) => match cache.invalidate(key).await {
            Ok(()) => text_response(&serde_json::json!({
                "success": true,
                "invalidated_count": 1,
                "key": key,
            })),
            Err(e) => {
                tracing::error!("invalidate_cache error for key '{key}': {e:?}");
                Err(CallToolError::from_message(format!(
                    "Cache invalidation failed for key '{key}': {e}"
                )))
            }
        },
        (None, Some(pattern)) => match cache.invalidate_pattern(pattern).await {
            Ok(count) => text_response(&serde_json::json!({
                "success": true,
                "invalidated_count": count,
                "pattern": pattern,
            })),
            Err(e) => {
                tracing::error!("invalidate_cache error for pattern '{pattern}': {e:?}");
                Err(CallToolError::from_message(format!(
                    "Cache invalidation failed for pattern '{pattern}': {e}"
                )))
            }
        },
        _ => error_response(&serde_json::json!({
            "error": "Provide exactly one of 'key' or 'pattern'",
            "key": params.key,
            "pattern": params.pattern,
        })),
    }
}
