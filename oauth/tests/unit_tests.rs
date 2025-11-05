// oauth/tests/unit_tests.rs
//! Unit tests for OAuth JWT signing and verification
//!
//! These tests use the test keys in oauth/tests/*.pem and do not require a database.

use chrono::{Duration, Utc};
use sqlx::PgPool;
use streamflow_oauth::{AuthConfig, AuthError, Claims, PostgresAuthService};

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
