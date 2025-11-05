// oauth/src/lib.rs
//! OAuth 2.0 authentication service for StreamFlow
//!
//! This module provides OAuth 2.0 compliant authentication services with
//! RSA256 signed JWT tokens.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod postgres;
pub use postgres::{hash_refresh_token, PostgresAuthService};

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
///
/// For MVP, we use only standard JWT claims. The `sub` (subject) claim
/// uniquely identifies the authenticated entity (user_id or client_id).
///
/// The `jti` (JWT ID) ensures each token is unique, even when issued
/// at the same timestamp for the same subject.
///
/// Additional claims can be added post-MVP for authorization:
/// - scopes: Vec<String> for permissions
/// - tenant_id: String for multi-tenancy
/// - roles: Vec<String> for RBAC
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Subject (user_id or client_id) - uniquely identifies authenticated entity
    pub sub: String,

    /// JWT ID - unique identifier for this token
    pub jti: String,

    /// Issuer (who issued this token)
    pub iss: String,

    /// Audience (who this token is intended for)
    pub aud: String,

    /// Expiration time (Unix timestamp)
    pub exp: i64,

    /// Issued at (Unix timestamp)
    pub iat: i64,
}

/// Authentication response containing access token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
}

/// Authentication service interface
///
/// Provides OAuth 2.0 compliant authentication with JWT tokens.
/// Implementations handle token issuance and validation.
#[async_trait]
pub trait AuthenticationService: Send + Sync {
    /// Authenticate client credentials and issue token
    ///
    /// OAuth 2.0 client_credentials flow for service accounts and workers.
    async fn authenticate_client(
        &self,
        client_id: &str,
        client_secret: &str,
    ) -> AuthResult<AuthResponse>;

    /// Authenticate user password and issue token
    ///
    /// OAuth 2.0 password flow for human users.
    async fn authenticate_password(
        &self,
        username: &str,
        password: &str,
    ) -> AuthResult<AuthResponse>;

    /// Refresh access token
    ///
    /// OAuth 2.0 refresh_token flow.
    async fn refresh_token(&self, refresh_token: &str) -> AuthResult<AuthResponse>;

    /// Validate access token and return claims
    ///
    /// Used by API middleware to validate Bearer tokens.
    async fn validate_token(&self, token: &str) -> AuthResult<Claims>;

    /// Get RSA public keys for external verification (JWKS)
    ///
    /// Returns public keys in JWK format for external token verification.
    async fn get_signing_keys(&self) -> AuthResult<Vec<JwtKey>>;
}

/// JWT key for external verification (JWKS format)
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

    /// RSA public key (PEM format) for JWT verification
    /// If not provided, will attempt to derive from private key (may not work with all key formats)
    pub rsa_public_key_pem: Option<String>,

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
            rsa_public_key_pem: None,
            jwt_issuer: "streamflow".to_string(),
            jwt_audience: "streamflow-api".to_string(),
            token_ttl: 86400, // 24 hours
        }
    }
}
