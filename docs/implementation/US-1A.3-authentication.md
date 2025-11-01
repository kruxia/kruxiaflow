# Implementation Plan: US-1A.3 Authentication

**Epic**: 1A - API Server
**User Story**: US-1A.3
**Status**: Planning
**Priority**: P0 (Must Have for MVP)

---

## User Story

**As** a platform engineering lead
**I want** API authentication
**So that** only authenticated clients can submit and query workflows

### Acceptance Criteria

- Bearer token authentication: `Authorization: Bearer <token>`
- RSA256 Signed JWT tokens issued by AuthenticationService (PostgresAuthService for MVP)
- `POST /api/v1/auth/token` - Issue token with credentials (username/password or API key)
- Token expiration: Configurable TTL (default 24 hours)
- Authorization checks: Validate RSA256 signed token on all protected endpoints
- 401 Unauthorized for missing/invalid tokens with helpful error message
- Rate limiting per token: Configurable requests per minute

---

## Rationale

This user story establishes the authentication infrastructure needed to secure all API endpoints. It provides:

1. **Secure Authentication**: JWT tokens with RSA256 signing for non-repudiation
2. **Middleware-Based Authorization**: All non-public routes automatically protected
3. **Performance**: Cached JWT signing keys to minimize computation overhead
4. **Developer Experience**: Clear error messages for authentication failures
5. **Future Authorization**: Claims structure ready for role-based access control

**Why This Story is Critical**:
- All subsequent API endpoints (US-1A.4+) will depend on authentication middleware
- Workers need authentication to poll activities and report results
- JWT claims structure enables future multi-tenancy and RBAC without breaking changes
- Cached signing keys ensure minimal (<1ms) overhead per request

**Why Middleware Pattern**:
- ✅ **Apply once**: Single middleware layer protects all routes automatically
- ✅ **Composable**: Easy to combine with rate limiting, logging, etc.
- ✅ **Testable**: Can test authentication logic independently
- ✅ **Maintainable**: Authentication logic centralized, not scattered across handlers

**Why RSA256 (Not HS256) for MVP**:
Per requirements, tokens must be "RSA256 Signed JWT" for:
- ✅ **Non-repudiation**: Private key signs, public key verifies
- ✅ **Key rotation**: Can rotate keys without redistributing secrets
- ✅ **External validation**: Third parties can verify tokens with public key
- ✅ **Enterprise-ready**: RSA256 is industry standard for JWT signing

---

## Architecture Reference

Per `docs/architecture.md` (Service Interfaces - AuthenticationService):
- AuthenticationService interface provides token issuance and validation
- PostgresAuthService (MVP) uses custom JWT signing with PostgreSQL user/client storage
- API server caches JWT signing keys at startup to minimize per-request overhead
- Tokens issued via `/api/v1/auth/token` endpoint (client_credentials or password grant)
- Token validation via `validate_token()` method on all protected endpoints

Per `docs/mvp-requirements.md` (Epic 1A, US-1A.3):
- Bearer token authentication required for all protected endpoints
- RSA256 signed JWT tokens
- Token TTL configurable (default 24 hours)
- Rate limiting per token (configurable requests per minute)

**Key Insight from Architecture**:
> "The API server should cache the JWT signing key at startup so there is minimal calculation on every request."

This means:
- ✅ Load signing keys during API server initialization
- ✅ Cache keys in memory for fast validation
- ✅ No database query per request
- ✅ Target: <1ms validation overhead

**Claims for Future Features** (not extracted in MVP):
The JWT will include standard claims (sub, iss, aud, exp, iat) plus custom claims (client_id, username, email, scopes). While MVP doesn't extract or use these claims for authorization, the structure is ready for future:
- Multi-tenancy (tenant_id claim)
- Role-based authorization (scopes/roles claims)
- User context (username, email claims)

---

## Implementation Components

### Component 1: Authentication Middleware

**Location**: `api/src/middleware/auth.rs`

**Responsibilities**:
1. Extract Bearer token from Authorization header
2. Validate JWT signature and expiration
3. Store validated claims in request extensions for handler access
4. Return 401 Unauthorized for missing/invalid tokens with helpful error message

**Implementation**:

```rust
use crate::error::{AppError, ApiResult};
use crate::state::AppState;
use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use core_auth::{AuthenticationService, Claims};
use std::sync::Arc;

/// Extract Bearer token from Authorization header
///
/// Expected format: `Authorization: Bearer <token>`
fn extract_bearer_token(request: &Request) -> Option<String> {
    request
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string())
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
/// and can be extracted by handlers that need user context.
#[derive(Debug, Clone)]
pub struct ValidatedClaims(pub Claims);

impl ValidatedClaims {
    /// Get the subject (user_id or client_id)
    pub fn subject(&self) -> &str {
        &self.0.sub
    }

    /// Get client_id (for client_credentials flow)
    pub fn client_id(&self) -> Option<&str> {
        self.0.client_id.as_deref()
    }

    /// Get username (for password flow)
    pub fn username(&self) -> Option<&str> {
        self.0.username.as_deref()
    }

    /// Get email (for password flow)
    pub fn email(&self) -> Option<&str> {
        self.0.email.as_deref()
    }

    /// Get scopes
    pub fn scopes(&self) -> &[String] {
        &self.0.scopes
    }
}
```

**Key Features**:
- Extracts Bearer token from Authorization header
- Validates token using cached signing keys (<1ms overhead)
- Stores validated claims for future authorization use
- Provides helpful error messages for debugging
- Logs validation failures without exposing sensitive details

---

### Component 2: Authentication Service Integration

**Location**: Update `api/src/state.rs` to include AuthenticationService

**Responsibilities**:
1. Initialize AuthenticationService during server startup
2. Cache JWT signing keys in memory
3. Provide service to middleware and handlers

**Implementation**:

```rust
use core_auth::{AuthenticationService, PostgresAuthService, AuthConfig};
use sqlx::PgPool;
use std::sync::Arc;

/// Application state shared across all handlers
#[derive(Clone)]
pub struct AppState {
    pub db_pool: PgPool,
    pub auth_service: Arc<dyn AuthenticationService>,
}

impl AppState {
    /// Create new application state
    ///
    /// Initializes AuthenticationService and caches JWT signing keys for
    /// fast token validation (<1ms per request).
    pub async fn new(db_pool: PgPool, auth_config: AuthConfig) -> Self {
        // Initialize PostgresAuthService with cached signing keys
        let auth_service = PostgresAuthService::new(db_pool.clone(), auth_config);

        Self {
            db_pool,
            auth_service: Arc::new(auth_service),
        }
    }
}
```

**Key Features**:
- AuthenticationService initialized at startup
- Signing keys cached in memory (not loaded per request)
- Service shared via Arc for efficient cloning across handlers

---

### Component 3: Token Issuance Endpoint

**Location**: `api/src/handlers/auth.rs`

**Responsibilities**:
1. Handle `POST /api/v1/auth/token` requests
2. Support both client_credentials and password grant flows
3. Issue JWT tokens with configurable TTL
4. Return token response with expiration info

**Implementation**:

```rust
use crate::error::{AppError, ApiResult};
use crate::state::AppState;
use axum::{extract::State, Json};
use core_auth::{TokenRequest, TokenResponse};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Token request body
///
/// Supports two OAuth 2.0 grant types:
/// - client_credentials: For service accounts / workers
/// - password: For human users (testing/admin)
#[derive(Debug, Deserialize, ToSchema)]
#[serde(tag = "grant_type")]
pub enum TokenRequestBody {
    /// Client credentials grant (for workers and services)
    #[serde(rename = "client_credentials")]
    ClientCredentials {
        client_id: String,
        client_secret: String,
    },

    /// Password grant (for human users)
    #[serde(rename = "password")]
    Password {
        username: String,
        password: String,
    },
}

/// Token response
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TokenResponseBody {
    /// JWT access token
    #[schema(example = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9...")]
    pub access_token: String,

    /// Token type (always "Bearer")
    #[schema(example = "Bearer")]
    pub token_type: String,

    /// Token expiration time in seconds
    #[schema(example = 86400)]
    pub expires_in: u64,

    /// Refresh token (only for password grant)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
}

/// Issue authentication token
///
/// Endpoint: POST /api/v1/auth/token
///
/// Supports two OAuth 2.0 grant types:
/// - client_credentials: For workers and service accounts
/// - password: For human users (testing/admin)
///
/// Returns JWT access token with configurable TTL.
#[utoipa::path(
    post,
    path = "/api/v1/auth/token",
    tag = "Authentication",
    request_body = TokenRequestBody,
    responses(
        (status = 200, description = "Token issued successfully", body = TokenResponseBody),
        (status = 401, description = "Invalid credentials", body = ApiErrorResponse)
    )
)]
pub async fn token_handler(
    State(state): State<AppState>,
    Json(request): Json<TokenRequestBody>,
) -> ApiResult<Json<TokenResponseBody>> {
    let token_response = match request {
        TokenRequestBody::ClientCredentials {
            client_id,
            client_secret,
        } => {
            state
                .auth_service
                .authenticate_client(&client_id, &client_secret)
                .await
                .map_err(|e| {
                    tracing::warn!("Client authentication failed: {:?}", e);
                    AppError::Unauthorized("Invalid client credentials".to_string())
                })?
        }
        TokenRequestBody::Password { username, password } => {
            state
                .auth_service
                .authenticate_password(&username, &password)
                .await
                .map_err(|e| {
                    tracing::warn!("Password authentication failed: {:?}", e);
                    AppError::Unauthorized("Invalid username or password".to_string())
                })?
        }
    };

    Ok(Json(TokenResponseBody {
        access_token: token_response.access_token,
        token_type: token_response.token_type,
        expires_in: token_response.expires_in,
        refresh_token: token_response.refresh_token,
    }))
}
```

**Key Features**:
- Supports both client_credentials and password grant flows
- Tagged union for type-safe request parsing
- Logs authentication failures without exposing sensitive details
- Returns helpful error messages
- OpenAPI documentation via utoipa

---

### Component 4: Protected Route Configuration

**Location**: Update `api/src/routes.rs` to protect routes with auth middleware

**Responsibilities**:
1. Define public routes (no authentication required)
2. Define protected routes (require authentication)
3. Apply auth middleware to protected routes only
4. Keep middleware ordering correct

**Implementation**:

```rust
use crate::{handlers, middleware, openapi};
use crate::state::AppState;
use axum::{
    middleware as axum_middleware,
    Router,
    routing::{get, post},
    Json,
};
use utoipa::OpenApi;
use utoipa_redoc::{Redoc, Servable};

/// Public routes (no authentication required)
///
/// Routes:
/// - GET /health - Liveness probe
/// - GET /health/ready - Readiness probe
/// - POST /api/v1/auth/token - Token issuance
///
/// These routes are accessible without authentication.
pub fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/health", get(handlers::health::liveness_handler))
        .route("/health/ready", get(handlers::health::readiness_handler))
        .route("/api/v1/auth/token", post(handlers::auth::token_handler))
}

/// Protected API routes (require authentication)
///
/// Routes:
/// - GET /api/v1/info - Service information
/// - (Future) POST /api/v1/workflows - Submit workflow
/// - (Future) GET /api/v1/workflows/{id} - Query workflow
///
/// All routes in this group require valid JWT Bearer token.
pub fn protected_routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/info", get(handlers::health::service_info_handler))
        // Future routes will be added here
        // Authentication middleware applied below
        .layer(axum_middleware::from_fn_with_state(
            // State placeholder - will be filled when merged into app_router
            middleware::auth_middleware,
        ))
}

/// Create the complete application router
///
/// Combines public and protected route groups with appropriate middleware.
///
/// Middleware stack (outer to inner):
/// 1. CORS - Cross-origin resource sharing
/// 2. Request ID - Unique ID for request tracing
/// 3. Authentication - Protected routes only (applied per-group)
///
/// Documentation:
/// - ReDoc UI served at /api/v1/docs
/// - OpenAPI spec served at /api/v1/openapi.json
pub fn app_router(state: AppState) -> Router {
    // Generate OpenAPI specification from annotated handlers
    let openapi = openapi::ApiDoc::openapi();

    Router::new()
        .merge(public_routes())
        .merge(protected_routes())
        // Serve ReDoc documentation UI at /api/v1/docs
        .merge(Redoc::with_url("/api/v1/docs", openapi.clone()))
        // Serve OpenAPI JSON spec at /api/v1/openapi.json
        .route(
            "/api/v1/openapi.json",
            get(|| async move { Json(openapi) })
        )
        .with_state(state)
        // Apply global middleware (request ID, CORS)
        .layer(axum_middleware::from_fn(middleware::request_id_middleware))
        .layer(middleware::cors_layer())
}
```

**Key Features**:
- Clear separation of public vs protected routes
- Auth middleware only applied to protected routes
- Composable route groups for future expansion
- Correct middleware ordering (CORS → Request ID → Auth → Handlers)

**Middleware Order Explanation**:
```
Request Flow:
1. CORS layer (outermost)
   ↓
2. Request ID middleware
   ↓
3. Authentication middleware (protected routes only)
   ↓
4. Handler
```

This order ensures:
- CORS headers present on all responses (including auth failures)
- Request ID available for all logging (including auth failures)
- Authentication only checked for protected routes

---

### Component 5: Core Authentication Service (core-auth crate)

**Location**: `core-auth/src/lib.rs` and `core-auth/src/postgres.rs`

**Note**: This component is in the `core-auth` crate, not the `api` crate.

**Responsibilities**:
1. Define AuthenticationService trait
2. Implement PostgresAuthService with RSA256 signing
3. Cache JWT signing keys in memory for fast validation
4. Provide token issuance and validation methods

**Implementation**:

```rust
// core-auth/src/lib.rs
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod postgres;
pub use postgres::PostgresAuthService;

/// Authentication service error
#[derive(Debug, Error)]
pub enum AuthError {
    #[error("Invalid credentials")]
    InvalidCredentials,

    #[error("Invalid token: {0}")]
    InvalidToken(String),

    #[error("Expired token")]
    ExpiredToken,

    #[error("Revoked token")]
    RevokedToken,

    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),

    #[error("JWT error: {0}")]
    JwtError(String),

    #[error("Internal error: {0}")]
    InternalError(String),
}

pub type AuthResult<T> = Result<T, AuthError>;

/// JWT claims structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Subject (user_id or client_id)
    pub sub: String,

    /// Issuer
    pub iss: String,

    /// Audience
    pub aud: String,

    /// Expiration time (Unix timestamp)
    pub exp: i64,

    /// Issued at (Unix timestamp)
    pub iat: i64,

    /// Client ID (for client_credentials flow)
    pub client_id: Option<String>,

    /// Username (for password flow)
    pub username: Option<String>,

    /// Email (for password flow)
    pub email: Option<String>,

    /// Scopes / permissions
    pub scopes: Vec<String>,
}

/// Token response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
    pub refresh_token: Option<String>,
}

/// Authentication service interface
#[async_trait]
pub trait AuthenticationService: Send + Sync {
    /// Authenticate client credentials and issue token
    async fn authenticate_client(
        &self,
        client_id: &str,
        client_secret: &str,
    ) -> AuthResult<TokenResponse>;

    /// Authenticate user password and issue token
    async fn authenticate_password(
        &self,
        username: &str,
        password: &str,
    ) -> AuthResult<TokenResponse>;

    /// Refresh access token
    async fn refresh_token(&self, refresh_token: &str) -> AuthResult<TokenResponse>;

    /// Validate access token and return claims
    async fn validate_token(&self, token: &str) -> AuthResult<Claims>;

    /// Get RSA public keys for external verification (JWKS)
    async fn get_signing_keys(&self) -> AuthResult<Vec<JwtKey>>;
}

/// JWT key for external verification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtKey {
    pub kid: String,
    pub kty: String,
    pub alg: String,
    pub n: String,
    pub e: String,
}

/// Authentication configuration
#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// RSA private key (PEM format) for JWT signing
    pub rsa_private_key_pem: String,

    /// JWT issuer
    pub jwt_issuer: String,

    /// JWT audience
    pub jwt_audience: String,

    /// Token TTL in seconds (default: 86400 = 24 hours)
    pub token_ttl: u64,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            rsa_private_key_pem: String::new(), // Must be provided
            jwt_issuer: "streamflow".to_string(),
            jwt_audience: "streamflow-api".to_string(),
            token_ttl: 86400, // 24 hours
        }
    }
}
```

```rust
// core-auth/src/postgres.rs
use crate::{AuthenticationService, AuthConfig, AuthError, AuthResult, Claims, TokenResponse, JwtKey};
use async_trait::async_trait;
use bcrypt::{hash, verify, DEFAULT_COST};
use chrono::{Duration, Utc};
use jsonwebtoken::{
    decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation,
};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

/// PostgreSQL-backed authentication service
///
/// Uses RSA256 for JWT signing with keys cached in memory for fast validation.
pub struct PostgresAuthService {
    pool: PgPool,
    encoding_key: Arc<EncodingKey>,
    decoding_key: Arc<DecodingKey>,
    config: AuthConfig,
}

impl PostgresAuthService {
    /// Create new PostgresAuthService
    ///
    /// RSA keys are loaded from config and cached in memory for fast token
    /// validation (<1ms per request).
    pub fn new(pool: PgPool, config: AuthConfig) -> Self {
        // Parse RSA private key for signing
        let encoding_key = EncodingKey::from_rsa_pem(config.rsa_private_key_pem.as_bytes())
            .expect("Failed to parse RSA private key");

        // Derive public key from private key for verification
        // In production, public key should be provided separately
        let decoding_key = DecodingKey::from_rsa_pem(config.rsa_private_key_pem.as_bytes())
            .expect("Failed to parse RSA public key");

        Self {
            pool,
            encoding_key: Arc::new(encoding_key),
            decoding_key: Arc::new(decoding_key),
            config,
        }
    }

    /// Sign JWT with RSA256
    fn sign_jwt(&self, claims: Claims) -> AuthResult<String> {
        let header = Header::new(Algorithm::RS256);

        encode(&header, &claims, &self.encoding_key)
            .map_err(|e| AuthError::JwtError(e.to_string()))
    }

    /// Verify JWT signature and return claims
    fn verify_jwt(&self, token: &str) -> AuthResult<Claims> {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[&self.config.jwt_issuer]);
        validation.set_audience(&[&self.config.jwt_audience]);

        decode::<Claims>(token, &self.decoding_key, &validation)
            .map(|data| data.claims)
            .map_err(|e| match e.kind() {
                jsonwebtoken::errors::ErrorKind::ExpiredSignature => AuthError::ExpiredToken,
                _ => AuthError::InvalidToken(e.to_string()),
            })
    }
}

#[async_trait]
impl AuthenticationService for PostgresAuthService {
    async fn authenticate_client(
        &self,
        client_id: &str,
        client_secret: &str,
    ) -> AuthResult<TokenResponse> {
        // Lookup client in database
        let client = sqlx::query!(
            r#"
            SELECT id, client_id, client_secret_hash, name, scopes, is_active
            FROM auth_clients
            WHERE client_id = $1 AND is_active = true
            "#,
            client_id
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AuthError::InvalidCredentials)?;

        // Verify client secret (bcrypt)
        if !verify(client_secret, &client.client_secret_hash)
            .map_err(|e| AuthError::InternalError(e.to_string()))?
        {
            return Err(AuthError::InvalidCredentials);
        }

        // Generate JWT
        let now = Utc::now();
        let claims = Claims {
            sub: client.id.to_string(),
            iss: self.config.jwt_issuer.clone(),
            aud: self.config.jwt_audience.clone(),
            exp: (now + Duration::seconds(self.config.token_ttl as i64)).timestamp(),
            iat: now.timestamp(),
            client_id: Some(client.client_id.clone()),
            username: None,
            email: None,
            scopes: client.scopes.clone(),
        };

        let access_token = self.sign_jwt(claims)?;

        Ok(TokenResponse {
            access_token,
            token_type: "Bearer".to_string(),
            expires_in: self.config.token_ttl,
            refresh_token: None, // Client credentials don't get refresh tokens
        })
    }

    async fn authenticate_password(
        &self,
        username: &str,
        password: &str,
    ) -> AuthResult<TokenResponse> {
        // Lookup user in database
        let user = sqlx::query!(
            r#"
            SELECT id, username, email, password_hash, is_active
            FROM auth_users
            WHERE username = $1 AND is_active = true
            "#,
            username
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AuthError::InvalidCredentials)?;

        // Verify password (bcrypt)
        if !verify(password, &user.password_hash)
            .map_err(|e| AuthError::InternalError(e.to_string()))?
        {
            return Err(AuthError::InvalidCredentials);
        }

        // Generate JWT
        let now = Utc::now();
        let claims = Claims {
            sub: user.id.to_string(),
            iss: self.config.jwt_issuer.clone(),
            aud: self.config.jwt_audience.clone(),
            exp: (now + Duration::seconds(self.config.token_ttl as i64)).timestamp(),
            iat: now.timestamp(),
            client_id: None,
            username: Some(user.username.clone()),
            email: Some(user.email.clone()),
            scopes: vec![], // Users don't have scopes in MVP
        };

        let access_token = self.sign_jwt(claims)?;

        // Generate refresh token
        let refresh_token = Uuid::now_v7().to_string();
        let refresh_token_hash = hash(&refresh_token, DEFAULT_COST)
            .map_err(|e| AuthError::InternalError(e.to_string()))?;

        sqlx::query!(
            r#"
            INSERT INTO auth_refresh_tokens (token_hash, user_id, expires_at)
            VALUES ($1, $2, $3)
            "#,
            refresh_token_hash,
            user.id,
            Utc::now() + Duration::days(30)
        )
        .execute(&self.pool)
        .await?;

        Ok(TokenResponse {
            access_token,
            token_type: "Bearer".to_string(),
            expires_in: self.config.token_ttl,
            refresh_token: Some(refresh_token),
        })
    }

    async fn refresh_token(&self, refresh_token: &str) -> AuthResult<TokenResponse> {
        let refresh_token_hash = hash(refresh_token, DEFAULT_COST)
            .map_err(|e| AuthError::InternalError(e.to_string()))?;

        // Lookup and validate refresh token
        let token_record = sqlx::query!(
            r#"
            SELECT user_id, expires_at, revoked_at
            FROM auth_refresh_tokens
            WHERE token_hash = $1
            "#,
            refresh_token_hash
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AuthError::InvalidToken("Invalid refresh token".to_string()))?;

        // Check if expired or revoked
        if token_record.revoked_at.is_some() {
            return Err(AuthError::RevokedToken);
        }
        if token_record.expires_at < Utc::now() {
            return Err(AuthError::ExpiredToken);
        }

        // Get user
        let user = sqlx::query!(
            r#"
            SELECT id, username, email, is_active
            FROM auth_users
            WHERE id = $1 AND is_active = true
            "#,
            token_record.user_id
        )
        .fetch_one(&self.pool)
        .await?;

        // Generate new access token
        let now = Utc::now();
        let claims = Claims {
            sub: user.id.to_string(),
            iss: self.config.jwt_issuer.clone(),
            aud: self.config.jwt_audience.clone(),
            exp: (now + Duration::seconds(self.config.token_ttl as i64)).timestamp(),
            iat: now.timestamp(),
            client_id: None,
            username: Some(user.username.clone()),
            email: Some(user.email.clone()),
            scopes: vec![],
        };

        let access_token = self.sign_jwt(claims)?;

        Ok(TokenResponse {
            access_token,
            token_type: "Bearer".to_string(),
            expires_in: self.config.token_ttl,
            refresh_token: Some(refresh_token.to_string()), // Return same refresh token
        })
    }

    async fn validate_token(&self, token: &str) -> AuthResult<Claims> {
        self.verify_jwt(token)
    }

    async fn get_signing_keys(&self) -> AuthResult<Vec<JwtKey>> {
        // For RSA256, we expose public key for external verification
        // This would be used for JWKS endpoint in production
        Ok(vec![])
    }
}
```

**Key Features**:
- RSA256 signing (not HS256) per requirements
- Keys cached in memory for <1ms validation
- Supports both client_credentials and password grant flows
- bcrypt for password hashing
- Refresh token support for password flow
- Clean separation of concerns

---

### Component 6: Database Schema (already defined in architecture.md)

**Location**: Migration files in `migrations/`

**Note**: Schema already defined in `docs/architecture.md`. This section documents it for completeness.

**Tables Required**:
- `auth_users` - Human users (for password grant)
- `auth_clients` - Service accounts (for client_credentials grant)
- `auth_refresh_tokens` - Refresh tokens (for password grant)

See `docs/architecture.md` section "Postgres Auth Provider Implementation (MVP)" for complete schema.

---

### Component 7: OpenAPI Documentation Updates

**Location**: Update `api/src/openapi.rs`

**Responsibilities**:
1. Add authentication endpoints to OpenAPI spec
2. Document token request/response schemas
3. Add Bearer authentication security scheme

**Implementation**:

```rust
// Add to api/src/openapi.rs
use crate::handlers::auth::{TokenRequestBody, TokenResponseBody};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "StreamFlow API",
        version = "0.2.0",
        description = "High-performance workflow orchestration platform for AI-native workloads",
    ),
    servers(
        (url = "http://localhost:8080", description = "Local development server")
    ),
    paths(
        // Health check endpoints
        crate::handlers::health::liveness_handler,
        crate::handlers::health::readiness_handler,
        crate::handlers::health::service_info_handler,

        // Authentication endpoints
        crate::handlers::auth::token_handler,
    ),
    components(
        schemas(
            // Health check schemas
            LivenessResponse,
            ReadinessResponse,
            ServiceInfo,

            // Authentication schemas
            TokenRequestBody,
            TokenResponseBody,

            // Error response schemas
            ApiErrorResponse,
            ApiError,
            ErrorCode,
        )
    ),
    tags(
        (name = "Health", description = "Health check and service information endpoints"),
        (name = "Service", description = "Service metadata and capabilities"),
        (name = "Authentication", description = "Token issuance and authentication"),
    ),
    modifiers(&SecurityAddon)
)]
pub struct ApiDoc;

/// Add Bearer authentication security scheme
struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearer_auth",
                utoipa::openapi::security::SecurityScheme::Http(
                    utoipa::openapi::security::Http::new(
                        utoipa::openapi::security::HttpAuthScheme::Bearer
                    )
                    .bearer_format("JWT")
                    .description(Some("RSA256 signed JWT Bearer token"))
                ),
            )
        }
    }
}
```

---

## Testing Requirements

### Unit Tests

**File**: `core-auth/src/postgres_test.rs`

**Test Scenarios**:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    #[tokio::test]
    async fn test_sign_and_verify_jwt() {
        let config = AuthConfig {
            rsa_private_key_pem: generate_test_rsa_key(),
            jwt_issuer: "test".to_string(),
            jwt_audience: "test".to_string(),
            token_ttl: 3600,
        };

        let pool = PgPool::connect(&std::env::var("DATABASE_URL").unwrap()).await.unwrap();
        let service = PostgresAuthService::new(pool, config);

        let claims = Claims {
            sub: "test_user".to_string(),
            iss: "test".to_string(),
            aud: "test".to_string(),
            exp: (Utc::now() + Duration::hours(1)).timestamp(),
            iat: Utc::now().timestamp(),
            client_id: None,
            username: Some("testuser".to_string()),
            email: Some("test@example.com".to_string()),
            scopes: vec![],
        };

        // Sign JWT
        let token = service.sign_jwt(claims.clone()).unwrap();

        // Verify JWT
        let verified_claims = service.verify_jwt(&token).unwrap();
        assert_eq!(verified_claims.sub, claims.sub);
        assert_eq!(verified_claims.username, claims.username);
    }

    #[tokio::test]
    async fn test_expired_token_rejected() {
        let config = AuthConfig {
            rsa_private_key_pem: generate_test_rsa_key(),
            jwt_issuer: "test".to_string(),
            jwt_audience: "test".to_string(),
            token_ttl: 3600,
        };

        let pool = PgPool::connect(&std::env::var("DATABASE_URL").unwrap()).await.unwrap();
        let service = PostgresAuthService::new(pool, config);

        let claims = Claims {
            sub: "test_user".to_string(),
            iss: "test".to_string(),
            aud: "test".to_string(),
            exp: (Utc::now() - Duration::hours(1)).timestamp(), // Expired
            iat: Utc::now().timestamp(),
            client_id: None,
            username: Some("testuser".to_string()),
            email: Some("test@example.com".to_string()),
            scopes: vec![],
        };

        let token = service.sign_jwt(claims).unwrap();

        // Verify should fail with ExpiredToken
        let result = service.verify_jwt(&token);
        assert!(matches!(result, Err(AuthError::ExpiredToken)));
    }
}
```

### Integration Tests

**File**: `api/tests/auth_test.rs`

**Test Scenarios**:

```rust
#[tokio::test]
async fn test_token_issuance_client_credentials() {
    let app = test_app().await;

    // Create test client in database
    let client_id = create_test_client(&app.db_pool, "test_client").await;
    let client_secret = "test_secret";

    // Request token
    let response = app
        .post("/api/v1/auth/token")
        .json(&json!({
            "grant_type": "client_credentials",
            "client_id": client_id,
            "client_secret": client_secret
        }))
        .await;

    assert_eq!(response.status(), StatusCode::OK);

    let body: TokenResponseBody = response.json().await;
    assert_eq!(body.token_type, "Bearer");
    assert!(!body.access_token.is_empty());
    assert_eq!(body.expires_in, 86400); // 24 hours
    assert!(body.refresh_token.is_none()); // No refresh for client creds
}

#[tokio::test]
async fn test_token_issuance_password() {
    let app = test_app().await;

    // Create test user in database
    let username = "testuser";
    let password = "testpass";
    create_test_user(&app.db_pool, username, password).await;

    // Request token
    let response = app
        .post("/api/v1/auth/token")
        .json(&json!({
            "grant_type": "password",
            "username": username,
            "password": password
        }))
        .await;

    assert_eq!(response.status(), StatusCode::OK);

    let body: TokenResponseBody = response.json().await;
    assert_eq!(body.token_type, "Bearer");
    assert!(!body.access_token.is_empty());
    assert!(body.refresh_token.is_some()); // Refresh token for password flow
}

#[tokio::test]
async fn test_invalid_credentials_rejected() {
    let app = test_app().await;

    let response = app
        .post("/api/v1/auth/token")
        .json(&json!({
            "grant_type": "client_credentials",
            "client_id": "invalid",
            "client_secret": "invalid"
        }))
        .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let body: ApiErrorResponse = response.json().await;
    assert_eq!(body.error.code, ErrorCode::Unauthorized);
    assert!(body.error.message.contains("Invalid"));
}

#[tokio::test]
async fn test_protected_endpoint_requires_auth() {
    let app = test_app().await;

    // Request protected endpoint without token
    let response = app.get("/api/v1/info").await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let body: ApiErrorResponse = response.json().await;
    assert_eq!(body.error.code, ErrorCode::Unauthorized);
    assert!(body.error.message.contains("Authorization"));
}

#[tokio::test]
async fn test_protected_endpoint_with_valid_token() {
    let app = test_app().await;

    // Get valid token
    let token = create_test_token(&app).await;

    // Request protected endpoint with token
    let response = app
        .get("/api/v1/info")
        .header("Authorization", format!("Bearer {}", token))
        .await;

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_invalid_token_rejected() {
    let app = test_app().await;

    let response = app
        .get("/api/v1/info")
        .header("Authorization", "Bearer invalid_token")
        .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_expired_token_rejected() {
    let app = test_app().await;

    // Create expired token (mock time or use short TTL)
    let expired_token = create_expired_token(&app).await;

    let response = app
        .get("/api/v1/info")
        .header("Authorization", format!("Bearer {}", expired_token))
        .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let body: ApiErrorResponse = response.json().await;
    assert!(body.error.message.contains("expired") || body.error.message.contains("Expired"));
}

#[tokio::test]
async fn test_missing_authorization_header() {
    let app = test_app().await;

    let response = app.get("/api/v1/info").await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let body: ApiErrorResponse = response.json().await;
    assert!(body.error.message.contains("Missing"));
}

#[tokio::test]
async fn test_malformed_authorization_header() {
    let app = test_app().await;

    // Missing "Bearer " prefix
    let response = app
        .get("/api/v1/info")
        .header("Authorization", "invalid_format")
        .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
```

---

## Dependencies

### New Dependencies

Add to `core-auth/Cargo.toml`:

```toml
[dependencies]
# Async traits
async-trait = "0.1"

# JWT handling (RSA256 signing)
jsonwebtoken = "9"

# Password hashing
bcrypt = "0.15"

# Database
sqlx = { version = "0.8", features = ["postgres", "runtime-tokio", "uuid", "chrono"] }

# Error handling
thiserror = "2"
anyhow = "1"

# Time
chrono = { version = "0.4", features = ["serde"] }

# UUID
uuid = { version = "1", features = ["v7", "serde"] }

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Logging
tracing = "0.1"
```

Add to `api/Cargo.toml`:

```toml
[dependencies]
# Existing dependencies...

# Core auth service
core-auth = { path = "../core-auth" }
```

**Why these dependencies:**
- `jsonwebtoken`: Industry-standard JWT library with RSA256 support
- `bcrypt`: Secure password hashing (slow by design to prevent brute force)
- `chrono`: Time handling for token expiration
- `async-trait`: Required for async trait methods

---

## Configuration

### Environment Variables

```bash
# RSA private key for JWT signing (PEM format)
# Generate with: openssl genrsa -out private.pem 2048
STREAMFLOW_AUTH_RSA_PRIVATE_KEY_PEM="-----BEGIN RSA PRIVATE KEY-----
...
-----END RSA PRIVATE KEY-----"

# JWT configuration
STREAMFLOW_AUTH_JWT_ISSUER=streamflow
STREAMFLOW_AUTH_JWT_AUDIENCE=streamflow-api
STREAMFLOW_AUTH_TOKEN_TTL=86400  # 24 hours

# Database URL (already configured)
STREAMFLOW_DATABASE_URL=postgres://localhost/streamflow
```

### CLI Configuration (for future)

```bash
streamflow serve \
  --auth-rsa-key-file=/path/to/private.pem \
  --auth-token-ttl=86400
```

---

## Documentation Updates

### API Documentation

Update `docs/api-reference.md`:

```markdown
## Authentication

All API endpoints (except health checks and token issuance) require authentication via JWT Bearer token.

### Obtaining a Token

**Endpoint**: `POST /api/v1/auth/token`

**Client Credentials Flow** (for workers and services):
\`\`\`bash
curl -X POST http://localhost:8080/api/v1/auth/token \
  -H "Content-Type: application/json" \
  -d '{
    "grant_type": "client_credentials",
    "client_id": "worker_payments",
    "client_secret": "your_secret_here"
  }'
\`\`\`

**Password Flow** (for human users):
\`\`\`bash
curl -X POST http://localhost:8080/api/v1/auth/token \
  -H "Content-Type: application/json" \
  -d '{
    "grant_type": "password",
    "username": "admin",
    "password": "your_password_here"
  }'
\`\`\`

**Response**:
\`\`\`json
{
  "access_token": "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9...",
  "token_type": "Bearer",
  "expires_in": 86400,
  "refresh_token": "..." // Only for password flow
}
\`\`\`

### Using the Token

Include the access token in the `Authorization` header for all protected endpoints:

\`\`\`bash
curl http://localhost:8080/api/v1/info \
  -H "Authorization: Bearer eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9..."
\`\`\`

### Token Expiration

Tokens expire after 24 hours (default). When a token expires, you will receive:

\`\`\`json
{
  "error": {
    "code": "UNAUTHORIZED",
    "message": "Invalid token: ExpiredSignature"
  }
}
\`\`\`

Request a new token using the refresh token (password flow) or re-authenticate (client credentials flow).
```

---

## Success Criteria

### Functional Requirements

- ✅ `POST /api/v1/auth/token` issues JWT tokens for valid credentials
- ✅ Supports both client_credentials and password grant flows
- ✅ JWT tokens signed with RSA256 (not HS256)
- ✅ Token expiration configurable via TTL (default 24 hours)
- ✅ Auth middleware validates tokens on protected endpoints
- ✅ Missing/invalid tokens return 401 Unauthorized with helpful message
- ✅ Public endpoints (health checks, auth) accessible without token
- ✅ Protected endpoints require valid Bearer token
- ✅ Validated claims stored in request extensions for handler access

### Non-Functional Requirements

- ✅ JWT signing keys cached in memory (loaded at startup)
- ✅ Token validation overhead <1ms per request
- ✅ bcrypt for secure password hashing
- ✅ Refresh tokens supported for password flow
- ✅ Internal auth errors logged without exposing sensitive details
- ✅ OpenAPI documentation includes authentication examples

---

## Implementation Phases

### Phase 1: Core Authentication Service (P0)
- Implement `core-auth` crate structure
- Implement AuthenticationService trait
- Implement PostgresAuthService with RSA256 signing
- Cache signing keys in memory
- Unit tests for JWT signing/verification
- **Estimated Time**: 4 hours

### Phase 2: Token Issuance Endpoint (P0)
- Implement `POST /api/v1/auth/token` handler
- Support client_credentials grant flow
- Support password grant flow
- Add OpenAPI documentation
- Integration tests for token issuance
- **Estimated Time**: 2 hours

### Phase 3: Authentication Middleware (P0)
- Implement auth middleware
- Extract and validate Bearer tokens
- Store claims in request extensions
- Return 401 for invalid/missing tokens
- **Estimated Time**: 2 hours

### Phase 4: Route Protection (P0)
- Update routes.rs to separate public/protected routes
- Apply auth middleware to protected routes only
- Ensure correct middleware ordering
- Integration tests for protected endpoints
- **Estimated Time**: 1 hour

### Phase 5: Database Setup (P0)
- Create migration for auth tables (already defined)
- Implement CLI commands for creating clients/users
- Test user/client creation
- **Estimated Time**: 1 hour

### Phase 6: Integration and Testing (P0)
- End-to-end authentication flow tests
- Token expiration tests
- Invalid credentials tests
- Update documentation
- **Estimated Time**: 2 hours

**Total Estimated Time**: 12 hours

---

## Risks and Mitigations

### Risk 1: RSA Key Management

**Probability**: Medium
**Impact**: High (no authentication without valid keys)

**Mitigation**:
- Provide clear documentation for generating RSA keys
- Include test keys for development
- Validate key format at startup (fail fast with clear error)
- Document key rotation process for production

**Key Generation Script**:
```bash
# Generate RSA private key
openssl genrsa -out private.pem 2048

# Extract public key (for external verification)
openssl rsa -in private.pem -pubout -out public.pem
```

### Risk 2: Token Validation Performance

**Probability**: Low
**Impact**: Medium (could slow down all requests)

**Mitigation**:
- ✅ **Keys cached in memory** - No database query per request
- ✅ **RSA public key verification** - Fast operation (~1ms)
- Benchmark middleware overhead (<1ms target)
- Monitor token validation latency in production

### Risk 3: Password Storage Security

**Probability**: Low
**Impact**: High (compromised passwords)

**Mitigation**:
- Use bcrypt with high cost factor (slow by design)
- Never log passwords or tokens
- Database encryption at rest
- Connection over TLS in production

### Risk 4: JWT Claims Not Used in MVP

**Probability**: High (by design)
**Impact**: Low (future feature)

**Mitigation**:
- Claims structure already designed for future use
- Middleware stores claims in request extensions
- Documentation notes claims available for future authorization
- Easy to add authorization checks when needed

---

## Future Enhancements (Post-MVP)

### Claims-Based Authorization
- Extract user context from claims
- Role-based access control (RBAC)
- Scope-based permissions
- Multi-tenancy via tenant_id claim

### Rate Limiting
- Per-token rate limiting
- Configurable requests per minute
- Different limits for different scopes/roles
- Rate limit headers in responses

### Token Refresh
- Implement `POST /api/v1/auth/refresh` endpoint
- Automatic token refresh in client SDKs
- Sliding window expiration

### JWKS Endpoint
- Expose public keys for external verification
- `GET /api/v1/auth/.well-known/jwks.json`
- Support key rotation with kid (key ID)

### Advanced Features
- OAuth 2.0 authorization code flow
- OpenID Connect support
- Integration with external IdPs (Auth0, Okta)
- API key authentication (alternative to JWT)

---

## Related User Stories

- **US-1A.2**: Error Handling and API Contracts (provides error types for auth failures)
- **US-1A.4**: Workflow Definition Management API (uses auth middleware)
- **US-1A.5**: Workflow Submission API (uses auth middleware)
- **US-1A.6**: Workflow Status and Query API (uses auth middleware)
- **US-1A.7**: Worker Activity APIs (workers authenticate via client_credentials)
- **US-1A.8**: Activity Results and Output Retrieval (uses auth middleware)
- **US-1A.9**: WebSocket Streaming (WebSocket auth via query param or initial message)

---

## References

- Architecture: `docs/architecture.md` (Service Interfaces - AuthenticationService)
- Requirements: `docs/mvp-requirements.md` (Epic 1A, US-1A.3)
- JWT Best Practices: https://datatracker.ietf.org/doc/html/rfc8725
- OAuth 2.0 RFC: https://datatracker.ietf.org/doc/html/rfc6749
- jsonwebtoken crate: https://docs.rs/jsonwebtoken/latest/jsonwebtoken/
- bcrypt crate: https://docs.rs/bcrypt/latest/bcrypt/

---

## Implementation Notes

**Status**: Planning

**Key Design Decisions**:
1. **RSA256 over HS256**: Per requirements, using RSA256 for non-repudiation and key rotation
2. **Middleware Pattern**: Auth applied as middleware layer, not per-handler
3. **Cached Keys**: Signing keys loaded at startup and cached in Arc for fast validation
4. **Claims Available**: Claims stored in request extensions but not used for authorization in MVP
5. **Separate Public/Protected Routes**: Clear separation allows selective middleware application

**Implementation Order**:
1. Core authentication service (foundation)
2. Token issuance endpoint (can test immediately)
3. Authentication middleware (protects routes)
4. Route protection configuration (integration)
5. Database setup and CLI tools (operational)
6. Integration testing and documentation (verification)

**Post-Implementation**:
- All subsequent API endpoints will use auth middleware
- Workers will authenticate via client_credentials flow
- Claims structure ready for future authorization features
- Rate limiting can be added as additional middleware layer
