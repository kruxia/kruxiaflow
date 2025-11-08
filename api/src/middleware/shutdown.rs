use axum::{
    Json,
    extract::State,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde_json::json;

use crate::AppState;

/// Middleware to check if server is shutting down
///
/// Returns 503 Service Unavailable when shutdown has been initiated.
/// This allows the API server to gracefully reject new requests while
/// draining in-flight requests during shutdown.
pub async fn shutdown_check(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    if state.is_shutting_down() {
        // Return 503 Service Unavailable during shutdown
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "error": {
                    "code": "service_unavailable",
                    "message": "Server is shutting down, please retry later"
                }
            })),
        )
            .into_response();
    }

    next.run(request).await
}
