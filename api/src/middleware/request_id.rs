use axum::{extract::Request, http::HeaderValue, middleware::Next, response::Response};
use uuid::Uuid;

/// Header name for request ID
pub const REQUEST_ID_HEADER: &str = "X-Request-ID";

/// Middleware that adds a unique request ID to each request
///
/// The request ID is:
/// - Generated as a UUID v7
/// - Added to the response headers as `X-Request-ID`
/// - Injected into the tracing context for log correlation
///
/// If the client provides an `X-Request-ID` header, we use that instead
/// of generating a new one (useful for distributed tracing).
pub async fn request_id_middleware(mut request: Request, next: Next) -> Response {
    // Check if client provided a request ID
    let request_id = request
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::now_v7().to_string());

    // Add request ID to tracing context
    let span = tracing::info_span!("request", request_id = %request_id);
    let _guard = span.enter();

    // Store request ID in extensions for access by handlers
    request
        .extensions_mut()
        .insert(RequestId(request_id.clone()));

    // Call next middleware/handler
    let mut response = next.run(request).await;

    // Add request ID to response headers
    if let Ok(header_value) = HeaderValue::from_str(&request_id) {
        response
            .headers_mut()
            .insert(REQUEST_ID_HEADER, header_value);
    }

    response
}

/// Request ID extracted from headers
#[derive(Debug, Clone)]
pub struct RequestId(pub String);

impl RequestId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
