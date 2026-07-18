// api/src/handlers/oauth.rs
//! OAuth 2.0 token issuance handlers
//!
//! Provides OAuth 2.0 compliant token endpoints per RFC 6749.

use crate::error::{ApiResult, AppError};
use crate::state::AppState;
use axum::{
    Form, Json, async_trait,
    extract::{FromRequest, Request, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Custom extractor that accepts both JSON and form-encoded request bodies
///
/// Per OAuth 2.0 spec (RFC 6749), the token endpoint must accept
/// application/x-www-form-urlencoded and may accept application/json.
pub struct JsonOrForm<T>(pub T);

#[async_trait]
impl<S, T> FromRequest<S> for JsonOrForm<T>
where
    S: Send + Sync,
    T: serde::de::DeserializeOwned,
{
    type Rejection = (StatusCode, String);

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let content_type = req
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        // Try JSON first (most common in modern APIs)
        if content_type.starts_with("application/json") {
            Json::<T>::from_request(req, state)
                .await
                .map(|Json(data)| JsonOrForm(data))
                .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid JSON: {}", e)))
        }
        // Then try form-encoded (OAuth 2.0 spec default)
        else if content_type.starts_with("application/x-www-form-urlencoded") {
            Form::<T>::from_request(req, state)
                .await
                .map(|Form(data)| JsonOrForm(data))
                .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid form data: {}", e)))
        }
        // Unsupported content type
        else {
            Err((
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                format!(
                    "Content-Type must be application/json or application/x-www-form-urlencoded, got: {}",
                    content_type
                ),
            ))
        }
    }
}

/// OAuth 2.0 grant types
///
/// Per RFC 6749, grant_type determines which authentication flow to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GrantType {
    /// Client credentials grant (RFC 6749 Section 4.4)
    /// For service accounts and workers
    ClientCredentials,

    /// Resource owner password credentials grant (RFC 6749 Section 4.3)
    /// For human users (testing/admin)
    Password,

    /// Refresh token grant (RFC 6749 Section 6)
    /// For refreshing expired access tokens
    RefreshToken,
}

/// OAuth 2.0 token request
///
/// Compliant with RFC 6749 (OAuth 2.0 specification).
/// Accepts both application/x-www-form-urlencoded (per spec)
/// and application/json (for convenience).
///
/// All fields are flat in the request body. The grant_type field
/// determines which other fields are required.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TokenRequest {
    /// OAuth 2.0 grant type - determines which fields are required
    pub grant_type: GrantType,

    /// Client identifier (required for client_credentials grant)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,

    /// Client secret (required for client_credentials grant)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,

    /// Username (required for password grant)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,

    /// Password (required for password grant)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,

    /// Refresh token (required for refresh_token grant)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,

    /// Requested scope (optional, for future use)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

impl TokenRequest {
    /// Validate that required fields are present for the grant type
    fn validate(&self) -> Result<(), AppError> {
        let mut errors = Vec::new();

        match self.grant_type {
            GrantType::ClientCredentials => {
                if self.client_id.is_none() {
                    errors.push("client_id is required for client_credentials grant".to_string());
                }
                if self.client_secret.is_none() {
                    errors
                        .push("client_secret is required for client_credentials grant".to_string());
                }
            }
            GrantType::Password => {
                if self.username.is_none() {
                    errors.push("username is required for password grant".to_string());
                }
                if self.password.is_none() {
                    errors.push("password is required for password grant".to_string());
                }
            }
            GrantType::RefreshToken => {
                if self.refresh_token.is_none() {
                    errors.push("refresh_token is required for refresh_token grant".to_string());
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(AppError::BadRequest(errors.join(", ")))
        }
    }
}

/// OAuth 2.0 token response
///
/// Compliant with RFC 6749 Section 5.1.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TokenResponse {
    /// JWT access token
    #[schema(example = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9...")]
    pub access_token: String,

    /// Token type (always "Bearer" for JWT)
    #[schema(example = "Bearer")]
    pub token_type: String,

    /// Token expiration time in seconds
    #[schema(example = 86400)]
    pub expires_in: u64,

    /// Refresh token (only for password grant)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,

    /// Granted scope (optional, for future use)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

/// Issue authentication token
///
/// Endpoint: POST /api/v1/oauth/token
/// Content-Type: application/json OR application/x-www-form-urlencoded
///
/// OAuth 2.0 compliant token endpoint per RFC 6749.
/// Accepts both JSON and form-encoded request bodies per spec.
///
/// Supports OAuth 2.0 grant types:
/// - client_credentials: For workers and service accounts
/// - password: For human users (testing/admin)
/// - refresh_token: For refreshing expired access tokens
///
/// Returns JWT access token with configurable TTL.
#[utoipa::path(
    post,
    path = "/api/v1/oauth/token",
    tag = "OAuth 2.0",
    request_body = TokenRequest,
    responses(
        (status = 200, description = "Token issued successfully", body = TokenResponse),
        (status = 401, description = "Invalid credentials", body = String),
        (status = 400, description = "Validation error", body = String)
    )
)]
pub async fn token_handler(
    State(state): State<AppState>,
    JsonOrForm(request): JsonOrForm<TokenRequest>,
) -> ApiResult<Json<TokenResponse>> {
    // Validate required fields for grant type
    request.validate()?;

    // Process based on grant type
    let auth_response = match request.grant_type {
        GrantType::ClientCredentials => {
            let client_id = request.client_id.as_ref().unwrap();
            let client_secret = request.client_secret.as_ref().unwrap();

            state
                .auth_service
                .authenticate_client(client_id, client_secret)
                .await
                .map_err(|e| {
                    tracing::warn!("Client authentication failed: {:?}", e);
                    AppError::Unauthorized("Invalid client credentials".to_string())
                })?
        }
        GrantType::Password => {
            let username = request.username.as_ref().unwrap();
            let password = request.password.as_ref().unwrap();

            state
                .auth_service
                .authenticate_password(username, password)
                .await
                .map_err(|e| {
                    tracing::warn!("Password authentication failed: {:?}", e);
                    AppError::Unauthorized("Invalid username or password".to_string())
                })?
        }
        GrantType::RefreshToken => {
            let refresh_token = request.refresh_token.as_ref().unwrap();

            state
                .auth_service
                .refresh_token(refresh_token)
                .await
                .map_err(|e| {
                    tracing::warn!("Token refresh failed: {:?}", e);
                    AppError::Unauthorized("Invalid or expired refresh token".to_string())
                })?
        }
    };

    Ok(Json(TokenResponse {
        access_token: auth_response.access_token,
        token_type: auth_response.token_type,
        expires_in: auth_response.expires_in,
        refresh_token: auth_response.refresh_token,
        scope: None, // MVP doesn't use scopes
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use crate::state::tests::*;
    use axum::extract::State;
    use kruxiaflow_core::cache::NoOpCache;
    use sqlx::PgPool;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    fn setup_test_state(pool: PgPool) -> AppState {
        AppState::new(
            pool,
            Arc::new(MockAuthService),
            Arc::new(MockActivityQueue),
            Arc::new(MockEventSource),
            Arc::new(MockWorkflowStorage),
            Arc::new(NoOpCache::new()),
            Arc::new(MockSubscriptionService),
            CancellationToken::new(),
        )
    }

    // --- TokenRequest validation tests ---

    #[test]
    fn test_validate_client_credentials_valid() {
        let request = TokenRequest {
            grant_type: GrantType::ClientCredentials,
            client_id: Some("client".to_string()),
            client_secret: Some("secret".to_string()),
            username: None,
            password: None,
            refresh_token: None,
            scope: None,
        };
        assert!(request.validate().is_ok());
    }

    #[test]
    fn test_validate_client_credentials_missing_id() {
        let request = TokenRequest {
            grant_type: GrantType::ClientCredentials,
            client_id: None,
            client_secret: Some("secret".to_string()),
            username: None,
            password: None,
            refresh_token: None,
            scope: None,
        };
        assert!(request.validate().is_err());
    }

    #[test]
    fn test_validate_client_credentials_missing_secret() {
        let request = TokenRequest {
            grant_type: GrantType::ClientCredentials,
            client_id: Some("client".to_string()),
            client_secret: None,
            username: None,
            password: None,
            refresh_token: None,
            scope: None,
        };
        assert!(request.validate().is_err());
    }

    #[test]
    fn test_validate_password_grant_valid() {
        let request = TokenRequest {
            grant_type: GrantType::Password,
            client_id: None,
            client_secret: None,
            username: Some("user".to_string()),
            password: Some("pass".to_string()),
            refresh_token: None,
            scope: None,
        };
        assert!(request.validate().is_ok());
    }

    #[test]
    fn test_validate_password_grant_missing_username() {
        let request = TokenRequest {
            grant_type: GrantType::Password,
            client_id: None,
            client_secret: None,
            username: None,
            password: Some("pass".to_string()),
            refresh_token: None,
            scope: None,
        };
        assert!(request.validate().is_err());
    }

    #[test]
    fn test_validate_password_grant_missing_password() {
        let request = TokenRequest {
            grant_type: GrantType::Password,
            client_id: None,
            client_secret: None,
            username: Some("user".to_string()),
            password: None,
            refresh_token: None,
            scope: None,
        };
        assert!(request.validate().is_err());
    }

    #[test]
    fn test_validate_refresh_token_valid() {
        let request = TokenRequest {
            grant_type: GrantType::RefreshToken,
            client_id: None,
            client_secret: None,
            username: None,
            password: None,
            refresh_token: Some("token".to_string()),
            scope: None,
        };
        assert!(request.validate().is_ok());
    }

    #[test]
    fn test_validate_refresh_token_missing() {
        let request = TokenRequest {
            grant_type: GrantType::RefreshToken,
            client_id: None,
            client_secret: None,
            username: None,
            password: None,
            refresh_token: None,
            scope: None,
        };
        assert!(request.validate().is_err());
    }

    #[test]
    fn test_grant_type_deserialize() {
        let json = r#""client_credentials""#;
        let gt: GrantType = serde_json::from_str(json).unwrap();
        assert_eq!(gt, GrantType::ClientCredentials);

        let json = r#""password""#;
        let gt: GrantType = serde_json::from_str(json).unwrap();
        assert_eq!(gt, GrantType::Password);

        let json = r#""refresh_token""#;
        let gt: GrantType = serde_json::from_str(json).unwrap();
        assert_eq!(gt, GrantType::RefreshToken);
    }

    #[test]
    fn test_token_response_serialize() {
        let response = TokenResponse {
            access_token: "token".to_string(),
            token_type: "Bearer".to_string(),
            expires_in: 3600,
            refresh_token: None,
            scope: None,
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["access_token"], "token");
        assert_eq!(json["token_type"], "Bearer");
        assert!(json.get("refresh_token").is_none());
        assert!(json.get("scope").is_none());
    }

    #[test]
    fn test_token_response_with_refresh() {
        let response = TokenResponse {
            access_token: "token".to_string(),
            token_type: "Bearer".to_string(),
            expires_in: 3600,
            refresh_token: Some("refresh".to_string()),
            scope: None,
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["refresh_token"], "refresh");
    }

    // --- Handler tests ---

    #[sqlx::test(migrations = "../migrations")]
    async fn test_token_handler_client_credentials(pool: PgPool) {
        let state = setup_test_state(pool);

        let request = TokenRequest {
            grant_type: GrantType::ClientCredentials,
            client_id: Some("client".to_string()),
            client_secret: Some("secret".to_string()),
            username: None,
            password: None,
            refresh_token: None,
            scope: None,
        };

        let result = token_handler(State(state), JsonOrForm(request)).await;

        assert!(result.is_ok());
        let Json(response) = result.unwrap();
        assert_eq!(response.access_token, "mock_token");
        assert_eq!(response.token_type, "Bearer");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_token_handler_password_grant(pool: PgPool) {
        let state = setup_test_state(pool);

        let request = TokenRequest {
            grant_type: GrantType::Password,
            client_id: None,
            client_secret: None,
            username: Some("user".to_string()),
            password: Some("pass".to_string()),
            refresh_token: None,
            scope: None,
        };

        let result = token_handler(State(state), JsonOrForm(request)).await;

        assert!(result.is_ok());
        let Json(response) = result.unwrap();
        assert_eq!(response.access_token, "mock_token");
        assert!(response.refresh_token.is_some());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_token_handler_refresh_token(pool: PgPool) {
        let state = setup_test_state(pool);

        let request = TokenRequest {
            grant_type: GrantType::RefreshToken,
            client_id: None,
            client_secret: None,
            username: None,
            password: None,
            refresh_token: Some("old_token".to_string()),
            scope: None,
        };

        let result = token_handler(State(state), JsonOrForm(request)).await;

        assert!(result.is_ok());
        let Json(response) = result.unwrap();
        assert_eq!(response.access_token, "new_token");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_token_handler_validation_error(pool: PgPool) {
        let state = setup_test_state(pool);

        let request = TokenRequest {
            grant_type: GrantType::ClientCredentials,
            client_id: None,
            client_secret: None,
            username: None,
            password: None,
            refresh_token: None,
            scope: None,
        };

        let result = token_handler(State(state), JsonOrForm(request)).await;

        assert!(result.is_err());
    }
}
