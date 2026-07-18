// oauth/src/postgres.rs
//! PostgreSQL-backed authentication service with RSA256 JWT signing

use crate::{
    AuthConfig, AuthError, AuthResponse, AuthResult, AuthenticationService, Claims, JwtKey,
    RegisterUserRequest, RegisterUserResponse,
};
use async_trait::async_trait;
use bcrypt::{DEFAULT_COST, hash, verify};
use chrono::{Duration, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

/// Hash a refresh token for storage using SHA-256
///
/// Refresh tokens are already high-entropy (UUIDv7 = 128 bits), so we use
/// a fast deterministic hash (SHA-256) instead of bcrypt for efficient indexed lookups.
///
/// This is safe because:
/// - Refresh tokens have cryptographic randomness (UUIDv7)
/// - We check expiration and revocation server-side
/// - Tokens transmitted over HTTPS only
/// - SHA-256 enables O(1) indexed lookup vs O(n) bcrypt verification
///
/// Public for testing purposes.
pub fn hash_refresh_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

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
    pub fn new(pool: PgPool, config: AuthConfig) -> Result<Self, AuthError> {
        // Parse RSA private key for signing
        let encoding_key = EncodingKey::from_rsa_pem(config.rsa_private_key_pem.as_bytes())
            .map_err(|e| {
                AuthError::InternalError(format!("Failed to parse RSA private key: {}", e))
            })?;

        // Parse RSA public key for verification
        // If public key is provided, use it; otherwise try to use private key
        let decoding_key = if let Some(ref public_key_pem) = config.rsa_public_key_pem {
            DecodingKey::from_rsa_pem(public_key_pem.as_bytes()).map_err(|e| {
                AuthError::InternalError(format!("Failed to parse RSA public key: {}", e))
            })?
        } else {
            // Try to use private key for decoding (works with some key formats)
            DecodingKey::from_rsa_pem(config.rsa_private_key_pem.as_bytes())
                .map_err(|e| AuthError::InternalError(format!("Failed to create RSA decoding key from private key: {}. Provide rsa_public_key_pem in config.", e)))?
        };

        Ok(Self {
            pool,
            encoding_key: Arc::new(encoding_key),
            decoding_key: Arc::new(decoding_key),
            config,
        })
    }

    /// Sign JWT with RSA256
    ///
    /// Public for testing purposes. In production, use authenticate_* methods instead.
    pub fn sign_jwt(&self, claims: Claims) -> AuthResult<String> {
        let header = Header::new(Algorithm::RS256);

        encode(&header, &claims, &self.encoding_key).map_err(|e| AuthError::JwtError(e.to_string()))
    }

    /// Verify JWT signature and return claims
    ///
    /// Public for testing purposes. In production, use validate_token instead.
    pub fn verify_jwt(&self, token: &str) -> AuthResult<Claims> {
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
    ) -> AuthResult<AuthResponse> {
        // Lookup client in database
        let client = sqlx::query!(
            r#"
            SELECT id, client_id, client_secret_hash, name, scopes, is_active
            FROM oauth_clients
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

        // Generate JWT with standard claims only
        // The `sub` (subject) uniquely identifies the client
        // The `jti` (JWT ID) ensures each token is unique
        let now = Utc::now();
        let claims = Claims {
            sub: client.id.to_string(),      // Uniquely identifies this client
            jti: Uuid::now_v7().to_string(), // Unique token ID
            iss: self.config.jwt_issuer.clone(),
            aud: self.config.jwt_audience.clone(),
            exp: (now + Duration::seconds(self.config.token_ttl as i64)).timestamp(),
            iat: now.timestamp(),
        };

        let access_token = self.sign_jwt(claims)?;

        Ok(AuthResponse {
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
    ) -> AuthResult<AuthResponse> {
        // Lookup user in database
        let user = sqlx::query!(
            r#"
            SELECT id, username, email, password_hash, is_active
            FROM oauth_users
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

        // Generate JWT with standard claims only
        // The `sub` (subject) uniquely identifies the user
        // The `jti` (JWT ID) ensures each token is unique
        let now = Utc::now();
        let claims = Claims {
            sub: user.id.to_string(),        // Uniquely identifies this user
            jti: Uuid::now_v7().to_string(), // Unique token ID
            iss: self.config.jwt_issuer.clone(),
            aud: self.config.jwt_audience.clone(),
            exp: (now + Duration::seconds(self.config.token_ttl as i64)).timestamp(),
            iat: now.timestamp(),
        };

        let access_token = self.sign_jwt(claims)?;

        // Generate refresh token (high-entropy UUIDv7)
        let refresh_token = Uuid::now_v7().to_string();
        let refresh_token_hash = hash_refresh_token(&refresh_token);

        sqlx::query!(
            r#"
            INSERT INTO oauth_refresh_tokens (token_hash, user_id, expires_at)
            VALUES ($1, $2, $3)
            "#,
            refresh_token_hash,
            user.id,
            Utc::now() + Duration::days(30)
        )
        .execute(&self.pool)
        .await?;

        Ok(AuthResponse {
            access_token,
            token_type: "Bearer".to_string(),
            expires_in: self.config.token_ttl,
            refresh_token: Some(refresh_token),
        })
    }

    async fn refresh_token(&self, refresh_token: &str) -> AuthResult<AuthResponse> {
        // Hash the provided token for lookup (SHA-256 is deterministic and fast)
        let token_hash = hash_refresh_token(refresh_token);

        // Direct indexed lookup by token hash (O(1) instead of O(n))
        let token_record = sqlx::query!(
            r#"
            SELECT id, token_hash, user_id, expires_at, revoked_at
            FROM oauth_refresh_tokens
            WHERE token_hash = $1
            "#,
            token_hash
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
            FROM oauth_users
            WHERE id = $1 AND is_active = true
            "#,
            token_record.user_id
        )
        .fetch_one(&self.pool)
        .await?;

        // Generate new access token with standard claims only
        // The `jti` (JWT ID) ensures each token is unique
        let now = Utc::now();
        let claims = Claims {
            sub: user.id.to_string(),        // Uniquely identifies this user
            jti: Uuid::now_v7().to_string(), // Unique token ID
            iss: self.config.jwt_issuer.clone(),
            aud: self.config.jwt_audience.clone(),
            exp: (now + Duration::seconds(self.config.token_ttl as i64)).timestamp(),
            iat: now.timestamp(),
        };

        let access_token = self.sign_jwt(claims)?;

        // MVP: Strict refresh token rotation (immediate revocation)
        // Generate new refresh token (high-entropy UUIDv7)
        let new_refresh_token = Uuid::now_v7().to_string();
        let new_refresh_token_hash = hash_refresh_token(&new_refresh_token);

        // Transaction: revoke old token + insert new token atomically
        // This prevents race conditions and ensures security
        let mut tx = self.pool.begin().await?;

        // Revoke old refresh token immediately
        sqlx::query!(
            r#"
            UPDATE oauth_refresh_tokens
            SET revoked_at = NOW()
            WHERE id = $1
            "#,
            token_record.id
        )
        .execute(&mut *tx)
        .await?;

        // Insert new refresh token
        sqlx::query!(
            r#"
            INSERT INTO oauth_refresh_tokens (token_hash, user_id, expires_at)
            VALUES ($1, $2, $3)
            "#,
            new_refresh_token_hash,
            user.id,
            Utc::now() + Duration::days(30)
        )
        .execute(&mut *tx)
        .await?;

        // Commit transaction
        tx.commit().await?;

        // Return new access token and NEW refresh token
        // Client MUST store the new refresh token and discard the old one
        Ok(AuthResponse {
            access_token,
            token_type: "Bearer".to_string(),
            expires_in: self.config.token_ttl,
            refresh_token: Some(new_refresh_token), // New token - RFC 6749 Section 6
        })
    }

    async fn validate_token(&self, token: &str) -> AuthResult<Claims> {
        self.verify_jwt(token)
    }

    async fn register_user(
        &self,
        request: RegisterUserRequest,
    ) -> AuthResult<RegisterUserResponse> {
        let password_hash = hash(&request.password, DEFAULT_COST)
            .map_err(|e| AuthError::InternalError(format!("Failed to hash password: {}", e)))?;

        let row = sqlx::query!(
            r#"
            INSERT INTO oauth_users (username, email, password_hash)
            VALUES ($1, $2, $3)
            ON CONFLICT (username) DO UPDATE SET updated_at = NOW()
            RETURNING id, username, email, is_active, created_at
            "#,
            request.username,
            request.email,
            password_hash,
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(RegisterUserResponse {
            id: row.id,
            username: row.username,
            email: row.email,
            is_active: row.is_active,
            created_at: row.created_at,
        })
    }

    async fn get_signing_keys(&self) -> AuthResult<Vec<JwtKey>> {
        // For RSA256, we expose public key for external verification
        // This would be used for JWKS endpoint in production
        // For MVP, we return empty vec
        Ok(vec![])
    }
}

// Unit tests are in oauth/tests/unit_tests.rs
// Integration tests are in oauth/tests/integration_tests.rs
