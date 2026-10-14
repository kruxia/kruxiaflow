// api/src/handlers/users.rs
//! User management handlers

use crate::error::{ApiResult, AppError};
use crate::middleware::auth::ValidatedClaims;
use crate::state::AppState;
use axum::{Extension, Json, extract::State, http::StatusCode, response::IntoResponse};
use chrono::{DateTime, Utc};
use kruxiaflow_oauth::RegisterUserRequest;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Request to create a user
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateUserRequest {
    /// Username (must be unique)
    pub username: String,
    /// Email address
    pub email: String,
    /// Password (will be bcrypt hashed)
    pub password: String,
}

/// Response from user creation
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateUserResponse {
    /// User ID
    pub id: Uuid,
    /// Username
    pub username: String,
    /// Email address
    pub email: String,
    /// Whether the user is active
    pub is_active: bool,
    /// When the user was created
    pub created_at: DateTime<Utc>,
}

/// Create a new user (idempotent)
///
/// Endpoint: POST /api/v1/oauth/users
///
/// Creates a user or returns the existing user if the username already exists.
/// Does NOT update the password on conflict (safe for idempotent seeding).
///
/// Requires a client (client_credentials) token: users cannot create users.
#[utoipa::path(
    post,
    path = "/api/v1/oauth/users",
    tag = "OAuth 2.0",
    request_body = CreateUserRequest,
    responses(
        (status = 201, description = "User created successfully", body = CreateUserResponse),
        (status = 400, description = "Validation error", body = String),
        (status = 401, description = "Unauthorized", body = String),
        (status = 403, description = "Caller is a user, not an OAuth client", body = String),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn create_user(
    State(state): State<AppState>,
    Extension(claims): Extension<ValidatedClaims>,
    Json(request): Json<CreateUserRequest>,
) -> ApiResult<impl IntoResponse> {
    // Only OAuth clients (service accounts) may create users. The synthetic
    // insecure-dev principal is allowed so local development stays tokenless.
    let is_dev_principal = state.insecure_dev && claims.subject() == "insecure-dev";
    if !is_dev_principal
        && !state
            .auth_service
            .is_active_client(claims.subject())
            .await
            .map_err(|e| {
                AppError::InternalError(anyhow::anyhow!("Failed to check caller: {}", e))
            })?
    {
        return Err(AppError::Forbidden(
            "Creating users requires an OAuth client token (client_credentials grant)".to_string(),
        ));
    }

    // Validate inputs
    if request.username.trim().is_empty() {
        return Err(AppError::BadRequest("username is required".to_string()));
    }
    if request.email.trim().is_empty() {
        return Err(AppError::BadRequest("email is required".to_string()));
    }
    if request.password.is_empty() {
        return Err(AppError::BadRequest("password is required".to_string()));
    }

    let register_request = RegisterUserRequest {
        username: request.username,
        email: request.email,
        password: request.password,
    };

    let user = state
        .auth_service
        .register_user(register_request)
        .await
        .map_err(|e| {
            tracing::error!("Failed to register user: {:?}", e);
            AppError::InternalError(anyhow::anyhow!("Failed to register user: {}", e))
        })?;

    Ok((
        StatusCode::CREATED,
        Json(CreateUserResponse {
            id: user.id,
            username: user.username,
            email: user.email,
            is_active: user.is_active,
            created_at: user.created_at,
        }),
    ))
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

    /// Claims for a caller that MockAuthService reports as an active client
    fn test_claims() -> ValidatedClaims {
        ValidatedClaims(kruxiaflow_oauth::Claims {
            sub: "test_client".to_string(),
            jti: "test_jti".to_string(),
            iss: "test".to_string(),
            aud: "test".to_string(),
            exp: 9999999999,
            iat: 1000000000,
        })
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_create_user_success(pool: PgPool) {
        let state = setup_test_state(pool);

        let request = CreateUserRequest {
            username: "newuser".to_string(),
            email: "new@example.com".to_string(),
            password: "secret123".to_string(),
        };

        let result = create_user(State(state), Extension(test_claims()), Json(request)).await;
        assert!(result.is_ok());

        let response = result.unwrap().into_response();
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_create_user_empty_username(pool: PgPool) {
        let state = setup_test_state(pool);

        let request = CreateUserRequest {
            username: "".to_string(),
            email: "new@example.com".to_string(),
            password: "secret123".to_string(),
        };

        let result = create_user(State(state), Extension(test_claims()), Json(request)).await;
        assert!(result.is_err());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_create_user_whitespace_username(pool: PgPool) {
        let state = setup_test_state(pool);

        let request = CreateUserRequest {
            username: "   ".to_string(),
            email: "new@example.com".to_string(),
            password: "secret123".to_string(),
        };

        let result = create_user(State(state), Extension(test_claims()), Json(request)).await;
        assert!(result.is_err());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_create_user_empty_email(pool: PgPool) {
        let state = setup_test_state(pool);

        let request = CreateUserRequest {
            username: "newuser".to_string(),
            email: "".to_string(),
            password: "secret123".to_string(),
        };

        let result = create_user(State(state), Extension(test_claims()), Json(request)).await;
        assert!(result.is_err());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_create_user_empty_password(pool: PgPool) {
        let state = setup_test_state(pool);

        let request = CreateUserRequest {
            username: "newuser".to_string(),
            email: "new@example.com".to_string(),
            password: "".to_string(),
        };

        let result = create_user(State(state), Extension(test_claims()), Json(request)).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_create_user_response_serialize() {
        let response = CreateUserResponse {
            id: Uuid::nil(),
            username: "demo".to_string(),
            email: "demo@example.com".to_string(),
            is_active: true,
            created_at: chrono::Utc::now(),
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["username"], "demo");
        assert_eq!(json["email"], "demo@example.com");
        assert_eq!(json["is_active"], true);
    }
}
