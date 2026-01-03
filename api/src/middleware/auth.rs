// api/src/middleware/auth.rs
//! Authentication middleware for JWT Bearer token validation

use crate::error::AppError;
use crate::state::AppState;
use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use kruxiaflow_oauth::Claims;

/// Extract Bearer token from Authorization header
///
/// Expected format: `Authorization: Bearer <token>`
fn extract_bearer_token(request: &Request) -> Option<String> {
    request
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| {
            // RFC 6750: Bearer token scheme is case-insensitive
            if s.len() > 7 && s[..7].eq_ignore_ascii_case("bearer ") {
                Some(s[7..].to_string())
            } else {
                None
            }
        })
}

/// Authentication middleware
///
/// Validates JWT Bearer token and stores claims in request extensions.
///
/// This middleware:
/// - Extracts Bearer token from Authorization header
/// - Validates token signature using cached signing keys
/// - Checks token expiration
/// - Stores validated claims in request extensions
/// - Returns 401 Unauthorized for missing/invalid tokens
///
/// The middleware uses cached JWT signing keys loaded at server startup,
/// ensuring validation overhead is <1ms per request.
pub async fn auth_middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, AppError> {
    // Extract Bearer token from Authorization header
    let token = extract_bearer_token(&request).ok_or_else(|| {
        AppError::Unauthorized(
            "Missing or invalid Authorization header. Expected: Authorization: Bearer <token>"
                .to_string(),
        )
    })?;

    // Validate token using AuthenticationService
    let claims = state
        .auth_service
        .validate_token(&token)
        .await
        .map_err(|e| {
            tracing::warn!("Token validation failed: {:?}", e);
            AppError::Unauthorized(format!("Invalid token: {}", e))
        })?;

    // Store validated claims in request extensions for handler access
    request.extensions_mut().insert(ValidatedClaims(claims));

    // Token is valid - proceed to handler
    Ok(next.run(request).await)
}

/// Validated JWT claims extracted from request
///
/// This type is inserted into request extensions by the auth middleware
/// and can be extracted by handlers that need the authenticated subject.
///
/// For MVP, this simply wraps the standard Claims structure. Post-MVP,
/// handlers can use this to access authorization data (scopes, tenant_id, etc.)
#[derive(Debug, Clone)]
pub struct ValidatedClaims(pub Claims);

impl ValidatedClaims {
    /// Get the subject (user_id or client_id) - the authenticated entity
    pub fn subject(&self) -> &str {
        &self.0.sub
    }

    /// Get the full claims for future authorization use
    pub fn claims(&self) -> &Claims {
        &self.0
    }
}
