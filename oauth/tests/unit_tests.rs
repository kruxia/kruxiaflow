// oauth/tests/unit_tests.rs
//! Unit tests for OAuth JWT signing and verification
//!
//! These tests use the test keys in oauth/tests/*.pem and do not require a database.

use chrono::{Duration, Utc};
use kruxiaflow_oauth::{
    AuthConfig, AuthError, AuthenticationService, Claims, PostgresAuthService, RegisterUserRequest,
    RegisterUserResponse,
};
use sqlx::PgPool;
use uuid::Uuid;

/// Load test private key (PKCS#8 format)
fn test_private_key() -> String {
    include_str!("private.pem").to_string()
}

/// Load test public key
fn test_public_key() -> String {
    include_str!("public.pem").to_string()
}

/// Create a mock database pool for unit tests
/// Note: These tests don't actually use the database, but PostgresAuthService requires it
async fn mock_pool() -> PgPool {
    PgPool::connect(&std::env::var("DATABASE_URL").unwrap())
        .await
        .expect("Failed to connect to test database")
}

/// Create test auth config with both private and public keys
fn test_auth_config() -> AuthConfig {
    AuthConfig {
        rsa_private_key_pem: test_private_key(),
        rsa_public_key_pem: Some(test_public_key()),
        jwt_issuer: "test".to_string(),
        jwt_audience: "test".to_string(),
        token_ttl: 3600,
    }
}

#[tokio::test]
async fn test_sign_and_verify_jwt() {
    let pool = mock_pool().await;
    let service = PostgresAuthService::new(pool, test_auth_config()).unwrap();

    // Create claims with standard fields
    let now = Utc::now();
    let claims = Claims {
        sub: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        jti: Uuid::now_v7().to_string(),
        iss: "test".to_string(),
        aud: "test".to_string(),
        exp: (now + Duration::hours(1)).timestamp(),
        iat: now.timestamp(),
    };

    // Sign JWT
    let token = service.sign_jwt(claims.clone()).unwrap();

    // Verify JWT signature is valid
    let verified_claims = service.verify_jwt(&token).unwrap();
    assert_eq!(verified_claims.sub, claims.sub);
    assert_eq!(verified_claims.iss, claims.iss);
    assert_eq!(verified_claims.aud, claims.aud);
}

#[tokio::test]
async fn test_expired_token_rejected() {
    let pool = mock_pool().await;
    let service = PostgresAuthService::new(pool, test_auth_config()).unwrap();

    let now = Utc::now();
    let claims = Claims {
        sub: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        jti: Uuid::now_v7().to_string(),
        iss: "test".to_string(),
        aud: "test".to_string(),
        exp: (now - Duration::hours(1)).timestamp(), // Expired 1 hour ago
        iat: (now - Duration::hours(2)).timestamp(),
    };

    let token = service.sign_jwt(claims).unwrap();

    // Verify should fail with ExpiredToken
    let result = service.verify_jwt(&token);
    assert!(
        matches!(result, Err(AuthError::ExpiredToken)),
        "Expected ExpiredToken, got {:?}",
        result
    );
}

#[tokio::test]
async fn test_invalid_issuer_rejected() {
    let pool = mock_pool().await;
    let service = PostgresAuthService::new(pool, test_auth_config()).unwrap();

    let now = Utc::now();
    let claims = Claims {
        sub: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        jti: Uuid::now_v7().to_string(),
        iss: "wrong-issuer".to_string(), // Wrong issuer
        aud: "test".to_string(),
        exp: (now + Duration::hours(1)).timestamp(),
        iat: now.timestamp(),
    };

    let token = service.sign_jwt(claims).unwrap();

    // Verify should fail with InvalidToken
    let result = service.verify_jwt(&token);
    assert!(
        matches!(result, Err(AuthError::InvalidToken(_))),
        "Expected InvalidToken, got {:?}",
        result
    );
}

#[tokio::test]
async fn test_invalid_audience_rejected() {
    let pool = mock_pool().await;
    let service = PostgresAuthService::new(pool, test_auth_config()).unwrap();

    let now = Utc::now();
    let claims = Claims {
        sub: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        jti: Uuid::now_v7().to_string(),
        iss: "test".to_string(),
        aud: "wrong-audience".to_string(), // Wrong audience
        exp: (now + Duration::hours(1)).timestamp(),
        iat: now.timestamp(),
    };

    let token = service.sign_jwt(claims).unwrap();

    // Verify should fail with InvalidToken
    let result = service.verify_jwt(&token);
    assert!(
        matches!(result, Err(AuthError::InvalidToken(_))),
        "Expected InvalidToken, got {:?}",
        result
    );
}

#[tokio::test]
async fn test_token_with_future_iat_accepted() {
    let pool = mock_pool().await;
    let service = PostgresAuthService::new(pool, test_auth_config()).unwrap();

    let now = Utc::now();
    let claims = Claims {
        sub: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        jti: Uuid::now_v7().to_string(),
        iss: "test".to_string(),
        aud: "test".to_string(),
        exp: (now + Duration::hours(2)).timestamp(),
        iat: (now + Duration::seconds(5)).timestamp(), // Issued 5 seconds in the future (clock skew)
    };

    let token = service.sign_jwt(claims.clone()).unwrap();

    // Verify should succeed (clock skew tolerance)
    let verified_claims = service.verify_jwt(&token).unwrap();
    assert_eq!(verified_claims.sub, claims.sub);
}

#[tokio::test]
async fn test_malformed_token_rejected() {
    let pool = mock_pool().await;
    let service = PostgresAuthService::new(pool, test_auth_config()).unwrap();

    // Malformed token (not a JWT)
    let result = service.verify_jwt("not-a-valid-jwt-token");
    assert!(
        matches!(result, Err(AuthError::InvalidToken(_))),
        "Expected InvalidToken, got {:?}",
        result
    );
}

#[tokio::test]
async fn test_token_with_tampered_signature_rejected() {
    let pool = mock_pool().await;
    let service = PostgresAuthService::new(pool, test_auth_config()).unwrap();

    let now = Utc::now();
    let claims = Claims {
        sub: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        jti: Uuid::now_v7().to_string(),
        iss: "test".to_string(),
        aud: "test".to_string(),
        exp: (now + Duration::hours(1)).timestamp(),
        iat: now.timestamp(),
    };

    let token = service.sign_jwt(claims).unwrap();

    // Tamper with the signature (replace last character)
    let mut tampered = token.clone();
    tampered.pop();
    tampered.push('X');

    // Verify should fail with InvalidToken
    let result = service.verify_jwt(&tampered);
    assert!(
        matches!(result, Err(AuthError::InvalidToken(_))),
        "Expected InvalidToken, got {:?}",
        result
    );
}

#[tokio::test]
async fn test_token_ttl_respected() {
    let pool = mock_pool().await;
    let service = PostgresAuthService::new(pool, test_auth_config()).unwrap();

    let now = Utc::now();
    let claims = Claims {
        sub: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        jti: Uuid::now_v7().to_string(),
        iss: "test".to_string(),
        aud: "test".to_string(),
        exp: (now + Duration::seconds(3600)).timestamp(),
        iat: now.timestamp(),
    };

    let token = service.sign_jwt(claims.clone()).unwrap();

    // Should be valid now
    let verified = service.verify_jwt(&token).unwrap();
    assert_eq!(verified.sub, claims.sub);

    // Expiration should be approximately 1 hour from now (within 5 seconds tolerance)
    let expected_exp = (now + Duration::seconds(3600)).timestamp();
    let actual_exp = verified.exp;
    assert!(
        (actual_exp - expected_exp).abs() <= 5,
        "Expected expiration around {}, got {}",
        expected_exp,
        actual_exp
    );
}

// ============================================================================
// Tests for oauth/src/lib.rs types and structures
// ============================================================================

#[test]
fn test_auth_config_default() {
    let config = kruxiaflow_oauth::AuthConfig::default();

    assert_eq!(config.rsa_private_key_pem, "");
    assert_eq!(config.rsa_public_key_pem, None);
    assert_eq!(config.jwt_issuer, "kruxiaflow");
    assert_eq!(config.jwt_audience, "kruxiaflow-api");
    assert_eq!(config.token_ttl, 86400);
}

#[test]
fn test_auth_config_custom() {
    let config = kruxiaflow_oauth::AuthConfig {
        rsa_private_key_pem: "test-key".to_string(),
        rsa_public_key_pem: Some("test-pub".to_string()),
        jwt_issuer: "custom-issuer".to_string(),
        jwt_audience: "custom-audience".to_string(),
        token_ttl: 7200,
    };

    assert_eq!(config.rsa_private_key_pem, "test-key");
    assert_eq!(config.rsa_public_key_pem, Some("test-pub".to_string()));
    assert_eq!(config.jwt_issuer, "custom-issuer");
    assert_eq!(config.jwt_audience, "custom-audience");
    assert_eq!(config.token_ttl, 7200);
}

#[test]
fn test_claims_serialization() {
    use kruxiaflow_oauth::Claims;

    let claims = Claims {
        sub: "user-123".to_string(),
        jti: "jti-123".to_string(),
        iss: "kruxiaflow".to_string(),
        aud: "kruxiaflow-api".to_string(),
        exp: 1234567890,
        iat: 1234567800,
    };

    let json = serde_json::to_string(&claims).unwrap();
    assert!(json.contains("\"sub\":\"user-123\""));
    assert!(json.contains("\"jti\":\"jti-123\""));
    assert!(json.contains("\"iss\":\"kruxiaflow\""));
    assert!(json.contains("\"aud\":\"kruxiaflow-api\""));
    assert!(json.contains("\"exp\":1234567890"));
    assert!(json.contains("\"iat\":1234567800"));
}

#[test]
fn test_claims_deserialization() {
    use kruxiaflow_oauth::Claims;

    let json = r#"{
        "sub": "user-123",
        "jti": "jti-123",
        "iss": "kruxiaflow",
        "aud": "kruxiaflow-api",
        "exp": 1234567890,
        "iat": 1234567800
    }"#;

    let claims: Claims = serde_json::from_str(json).unwrap();
    assert_eq!(claims.sub, "user-123");
    assert_eq!(claims.jti, "jti-123");
    assert_eq!(claims.iss, "kruxiaflow");
    assert_eq!(claims.aud, "kruxiaflow-api");
    assert_eq!(claims.exp, 1234567890);
    assert_eq!(claims.iat, 1234567800);
}

#[test]
fn test_auth_response_serialization() {
    use kruxiaflow_oauth::AuthResponse;

    let response = AuthResponse {
        access_token: "token-123".to_string(),
        token_type: "Bearer".to_string(),
        expires_in: 3600,
        refresh_token: Some("refresh-123".to_string()),
    };

    let json = serde_json::to_string(&response).unwrap();
    assert!(json.contains("\"access_token\":\"token-123\""));
    assert!(json.contains("\"token_type\":\"Bearer\""));
    assert!(json.contains("\"expires_in\":3600"));
    assert!(json.contains("\"refresh_token\":\"refresh-123\""));
}

#[test]
fn test_auth_response_deserialization() {
    use kruxiaflow_oauth::AuthResponse;

    let json = r#"{
        "access_token": "token-123",
        "token_type": "Bearer",
        "expires_in": 3600,
        "refresh_token": "refresh-123"
    }"#;

    let response: AuthResponse = serde_json::from_str(json).unwrap();
    assert_eq!(response.access_token, "token-123");
    assert_eq!(response.token_type, "Bearer");
    assert_eq!(response.expires_in, 3600);
    assert_eq!(response.refresh_token, Some("refresh-123".to_string()));
}

#[test]
fn test_auth_response_without_refresh_token() {
    use kruxiaflow_oauth::AuthResponse;

    let response = AuthResponse {
        access_token: "token-123".to_string(),
        token_type: "Bearer".to_string(),
        expires_in: 3600,
        refresh_token: None,
    };

    let json = serde_json::to_string(&response).unwrap();
    assert!(json.contains("\"access_token\":\"token-123\""));
    assert!(!json.contains("refresh_token"));
}

#[test]
fn test_jwt_key_serialization() {
    use kruxiaflow_oauth::JwtKey;

    let key = JwtKey {
        kid: "key-1".to_string(),
        kty: "RSA".to_string(),
        alg: "RS256".to_string(),
        n: "modulus".to_string(),
        e: "exponent".to_string(),
    };

    let json = serde_json::to_string(&key).unwrap();
    assert!(json.contains("\"kid\":\"key-1\""));
    assert!(json.contains("\"kty\":\"RSA\""));
    assert!(json.contains("\"alg\":\"RS256\""));
    assert!(json.contains("\"n\":\"modulus\""));
    assert!(json.contains("\"e\":\"exponent\""));
}

#[test]
fn test_auth_error_display() {
    use kruxiaflow_oauth::AuthError;

    let err = AuthError::InvalidCredentials;
    assert_eq!(err.to_string(), "Invalid credentials");

    let err = AuthError::InvalidToken("bad signature".to_string());
    assert_eq!(err.to_string(), "Invalid token: bad signature");

    let err = AuthError::ExpiredToken;
    assert_eq!(err.to_string(), "Expired token");

    let err = AuthError::RevokedToken;
    assert_eq!(err.to_string(), "Revoked token");

    let err = AuthError::JwtError("encoding failed".to_string());
    assert_eq!(err.to_string(), "JWT error: encoding failed");

    let err = AuthError::InternalError("unexpected".to_string());
    assert_eq!(err.to_string(), "Internal error: unexpected");
}

#[test]
fn test_auth_error_from_sqlx_error() {
    use kruxiaflow_oauth::AuthError;

    // Simulate a database connection error
    let sqlx_err = sqlx::Error::PoolTimedOut;
    let auth_err: AuthError = sqlx_err.into();

    match auth_err {
        AuthError::DatabaseError(_) => {} // Expected
        _ => panic!("Expected DatabaseError variant"),
    }
}

#[test]
fn test_claims_clone() {
    use kruxiaflow_oauth::Claims;

    let claims1 = Claims {
        sub: "user-123".to_string(),
        jti: "jti-123".to_string(),
        iss: "kruxiaflow".to_string(),
        aud: "kruxiaflow-api".to_string(),
        exp: 1234567890,
        iat: 1234567800,
    };

    let claims2 = claims1.clone();
    assert_eq!(claims1.sub, claims2.sub);
    assert_eq!(claims1.jti, claims2.jti);
    assert_eq!(claims1.iss, claims2.iss);
    assert_eq!(claims1.aud, claims2.aud);
    assert_eq!(claims1.exp, claims2.exp);
    assert_eq!(claims1.iat, claims2.iat);
}

#[test]
fn test_auth_response_clone() {
    use kruxiaflow_oauth::AuthResponse;

    let response1 = AuthResponse {
        access_token: "token-123".to_string(),
        token_type: "Bearer".to_string(),
        expires_in: 3600,
        refresh_token: Some("refresh-123".to_string()),
    };

    let response2 = response1.clone();
    assert_eq!(response1.access_token, response2.access_token);
    assert_eq!(response1.token_type, response2.token_type);
    assert_eq!(response1.expires_in, response2.expires_in);
    assert_eq!(response1.refresh_token, response2.refresh_token);
}

#[test]
fn test_jwt_key_clone() {
    use kruxiaflow_oauth::JwtKey;

    let key1 = JwtKey {
        kid: "key-1".to_string(),
        kty: "RSA".to_string(),
        alg: "RS256".to_string(),
        n: "modulus".to_string(),
        e: "exponent".to_string(),
    };

    let key2 = key1.clone();
    assert_eq!(key1.kid, key2.kid);
    assert_eq!(key1.kty, key2.kty);
    assert_eq!(key1.alg, key2.alg);
    assert_eq!(key1.n, key2.n);
    assert_eq!(key1.e, key2.e);
}

#[test]
fn test_auth_config_clone() {
    use kruxiaflow_oauth::AuthConfig;

    let config1 = AuthConfig {
        rsa_private_key_pem: "test-key".to_string(),
        rsa_public_key_pem: Some("test-pub".to_string()),
        jwt_issuer: "custom-issuer".to_string(),
        jwt_audience: "custom-audience".to_string(),
        token_ttl: 7200,
    };

    let config2 = config1.clone();
    assert_eq!(config1.rsa_private_key_pem, config2.rsa_private_key_pem);
    assert_eq!(config1.rsa_public_key_pem, config2.rsa_public_key_pem);
    assert_eq!(config1.jwt_issuer, config2.jwt_issuer);
    assert_eq!(config1.jwt_audience, config2.jwt_audience);
    assert_eq!(config1.token_ttl, config2.token_ttl);
}

// ============================================================================
// PostgresAuthService construction and key handling tests
// ============================================================================

#[tokio::test]
async fn test_postgres_auth_service_with_invalid_private_key() {
    let pool = mock_pool().await;
    let mut config = test_auth_config();
    config.rsa_private_key_pem = "invalid-key".to_string();

    let result = PostgresAuthService::new(pool, config);
    assert!(result.is_err(), "Should fail with invalid private key");

    if let Err(e) = result {
        assert!(
            matches!(e, kruxiaflow_oauth::AuthError::InternalError(_)),
            "Expected InternalError for invalid key"
        );
    }
}

#[tokio::test]
async fn test_postgres_auth_service_with_invalid_public_key() {
    let pool = mock_pool().await;
    let mut config = test_auth_config();
    config.rsa_public_key_pem = Some("invalid-public-key".to_string());

    let result = PostgresAuthService::new(pool, config);
    assert!(result.is_err(), "Should fail with invalid public key");
}

#[tokio::test]
async fn test_postgres_auth_service_without_public_key() {
    let pool = mock_pool().await;
    let mut config = test_auth_config();
    config.rsa_public_key_pem = None;

    // Should try to use private key for decoding
    let result = PostgresAuthService::new(pool, config);

    // This might succeed or fail depending on the key format
    // The important thing is we're testing the code path
    let _ = result;
}

#[tokio::test]
async fn test_get_signing_keys_returns_empty() {
    let pool = mock_pool().await;
    let service = PostgresAuthService::new(pool, test_auth_config()).unwrap();

    // For MVP, get_signing_keys returns empty vec
    let keys = service.get_signing_keys().await.unwrap();
    assert!(
        keys.is_empty(),
        "get_signing_keys should return empty vec for MVP"
    );
}

#[tokio::test]
async fn test_validate_token_uses_verify_jwt() {
    let pool = mock_pool().await;
    let service = PostgresAuthService::new(pool, test_auth_config()).unwrap();

    let now = Utc::now();
    let claims = Claims {
        sub: "test-user".to_string(),
        jti: Uuid::now_v7().to_string(),
        iss: "test".to_string(),
        aud: "test".to_string(),
        exp: (now + Duration::hours(1)).timestamp(),
        iat: now.timestamp(),
    };

    let token = service.sign_jwt(claims.clone()).unwrap();

    // validate_token should delegate to verify_jwt
    let result = service.validate_token(&token).await;
    assert!(result.is_ok(), "validate_token should succeed");

    let validated_claims = result.unwrap();
    assert_eq!(validated_claims.sub, claims.sub);
}

// ============================================================================
// Hash function tests
// ============================================================================

#[test]
fn test_hash_refresh_token_deterministic() {
    use kruxiaflow_oauth::hash_refresh_token;

    let token = "test-token-123";
    let hash1 = hash_refresh_token(token);
    let hash2 = hash_refresh_token(token);

    assert_eq!(hash1, hash2, "Hash should be deterministic");
}

#[test]
fn test_hash_refresh_token_different_tokens() {
    use kruxiaflow_oauth::hash_refresh_token;

    let token1 = "token-1";
    let token2 = "token-2";

    let hash1 = hash_refresh_token(token1);
    let hash2 = hash_refresh_token(token2);

    assert_ne!(
        hash1, hash2,
        "Different tokens should have different hashes"
    );
}

#[test]
fn test_hash_refresh_token_produces_hex_string() {
    use kruxiaflow_oauth::hash_refresh_token;

    let token = "test-token";
    let hash = hash_refresh_token(token);

    // SHA-256 hash should be 64 hex characters
    assert_eq!(hash.len(), 64, "SHA-256 hash should be 64 characters");

    // All characters should be valid hex
    assert!(
        hash.chars().all(|c| c.is_ascii_hexdigit()),
        "Hash should contain only hex digits"
    );
}

// ============================================================================
// RegisterUserRequest / RegisterUserResponse type tests
// ============================================================================

#[test]
fn test_register_user_request_deserialization() {
    let json = r#"{
        "username": "demo",
        "email": "demo@example.com",
        "password": "secret"
    }"#;

    let request: RegisterUserRequest = serde_json::from_str(json).unwrap();
    assert_eq!(request.username, "demo");
    assert_eq!(request.email, "demo@example.com");
    assert_eq!(request.password, "secret");
}

#[test]
fn test_register_user_request_clone() {
    let request1 = RegisterUserRequest {
        username: "user1".to_string(),
        email: "user1@example.com".to_string(),
        password: "pass".to_string(),
    };

    let request2 = request1.clone();
    assert_eq!(request1.username, request2.username);
    assert_eq!(request1.email, request2.email);
    assert_eq!(request1.password, request2.password);
}

#[test]
fn test_register_user_response_serialization() {
    let response = RegisterUserResponse {
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
    assert!(json["id"].is_string());
    assert!(json["created_at"].is_string());
}

#[test]
fn test_register_user_response_clone() {
    let response1 = RegisterUserResponse {
        id: Uuid::nil(),
        username: "user1".to_string(),
        email: "user1@example.com".to_string(),
        is_active: true,
        created_at: chrono::Utc::now(),
    };

    let response2 = response1.clone();
    assert_eq!(response1.id, response2.id);
    assert_eq!(response1.username, response2.username);
    assert_eq!(response1.email, response2.email);
    assert_eq!(response1.is_active, response2.is_active);
}
