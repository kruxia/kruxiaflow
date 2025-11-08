// oauth/tests/integration_tests.rs
//! Integration tests for OAuth flows
//!
//! These tests require a running PostgreSQL database configured via DATABASE_URL.
//! Run with: scripts/test.sh

use bcrypt::hash;
use chrono::{Duration, Utc};
use serial_test::serial;
use sqlx::PgPool;
use streamflow_oauth::{
    AuthConfig, AuthError, AuthenticationService, PostgresAuthService, hash_refresh_token,
};
use uuid::Uuid;

/// Load test private key
fn test_private_key() -> String {
    include_str!("private.pem").to_string()
}

/// Setup test database pool and run migrations
async fn setup_test_pool() -> PgPool {
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");

    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    // Run migrations
    sqlx::migrate!("../migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

/// Load test public key
fn test_public_key() -> String {
    include_str!("public.pem").to_string()
}

/// Create test auth service
async fn setup_auth_service(pool: PgPool) -> PostgresAuthService {
    let config = AuthConfig {
        rsa_private_key_pem: test_private_key(),
        rsa_public_key_pem: Some(test_public_key()),
        jwt_issuer: "test".to_string(),
        jwt_audience: "test-api".to_string(),
        token_ttl: 3600,
    };

    PostgresAuthService::new(pool, config).expect("Failed to create auth service")
}

/// Helper to create a test client in the database
async fn create_test_client(pool: &PgPool, client_id: &str, client_secret: &str) -> Uuid {
    let secret_hash = hash(client_secret, bcrypt::DEFAULT_COST).unwrap();

    let row = sqlx::query!(
        r#"
        INSERT INTO oauth_clients (client_id, client_secret_hash, name, scopes)
        VALUES ($1, $2, $3, $4)
        RETURNING id
        "#,
        client_id,
        secret_hash,
        "Test Client",
        &[] as &[String]
    )
    .fetch_one(pool)
    .await
    .expect("Failed to create test client");

    row.id
}

/// Helper to create a test user in the database
async fn create_test_user(pool: &PgPool, username: &str, email: &str, password: &str) -> Uuid {
    let password_hash = hash(password, bcrypt::DEFAULT_COST).unwrap();

    let row = sqlx::query!(
        r#"
        INSERT INTO oauth_users (username, email, password_hash, is_active)
        VALUES ($1, $2, $3, true)
        ON CONFLICT (email) DO UPDATE
        SET username = EXCLUDED.username, password_hash = EXCLUDED.password_hash, is_active = true
        RETURNING id
        "#,
        username,
        email,
        password_hash
    )
    .fetch_one(pool)
    .await
    .expect("Failed to create test user");

    row.id
}

/// Cleanup test data
async fn cleanup_test_data(pool: &PgPool, client_id: Option<&str>, username: Option<&str>) {
    if let Some(uname) = username {
        // Delete refresh tokens first (foreign key constraint)
        sqlx::query!("DELETE FROM oauth_refresh_tokens WHERE user_id IN (SELECT id FROM oauth_users WHERE username = $1)", uname)
            .execute(pool)
            .await
            .ok();

        sqlx::query!("DELETE FROM oauth_users WHERE username = $1", uname)
            .execute(pool)
            .await
            .ok();
    }

    if let Some(cid) = client_id {
        sqlx::query!("DELETE FROM oauth_clients WHERE client_id = $1", cid)
            .execute(pool)
            .await
            .ok();
    }
}

/// Cleanup test data at the start of a test (in case previous run failed)
async fn pre_cleanup_test_data(pool: &PgPool, client_id: Option<&str>, username: Option<&str>) {
    cleanup_test_data(pool, client_id, username).await;
}

// ============================================================================
// Client Credentials Flow Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_client_credentials_success() {
    let pool = setup_test_pool().await;
    let service = setup_auth_service(pool.clone()).await;

    let client_id = "test-client-success";
    let client_secret = "test-secret-123";

    create_test_client(&pool, client_id, client_secret).await;

    // Authenticate with client credentials
    let result = service.authenticate_client(client_id, client_secret).await;

    assert!(result.is_ok(), "Client authentication should succeed");

    let response = result.unwrap();
    assert_eq!(response.token_type, "Bearer");
    assert_eq!(response.expires_in, 3600);
    assert!(
        response.refresh_token.is_none(),
        "Client credentials should not return refresh token"
    );
    assert!(!response.access_token.is_empty());

    // Validate the token
    let claims = service.validate_token(&response.access_token).await;
    assert!(claims.is_ok(), "Token validation should succeed");

    cleanup_test_data(&pool, Some(client_id), None).await;
}

#[tokio::test]
#[serial]
async fn test_client_credentials_invalid_secret() {
    let pool = setup_test_pool().await;
    let service = setup_auth_service(pool.clone()).await;

    let client_id = "test-client-invalid-secret";
    let client_secret = "correct-secret";

    create_test_client(&pool, client_id, client_secret).await;

    // Try to authenticate with wrong secret
    let result = service.authenticate_client(client_id, "wrong-secret").await;

    assert!(
        matches!(result, Err(AuthError::InvalidCredentials)),
        "Should reject invalid secret"
    );

    cleanup_test_data(&pool, Some(client_id), None).await;
}

#[tokio::test]
#[serial]
async fn test_client_credentials_nonexistent_client() {
    let pool = setup_test_pool().await;
    let service = setup_auth_service(pool.clone()).await;

    // Try to authenticate with non-existent client
    let result = service
        .authenticate_client("nonexistent-client", "any-secret")
        .await;

    assert!(
        matches!(result, Err(AuthError::InvalidCredentials)),
        "Should reject non-existent client"
    );
}

#[tokio::test]
#[serial]
async fn test_client_credentials_inactive_client() {
    let pool = setup_test_pool().await;
    let service = setup_auth_service(pool.clone()).await;

    let client_id = "test-client-inactive";
    let client_secret = "test-secret";

    let client_uuid = create_test_client(&pool, client_id, client_secret).await;

    // Deactivate the client
    sqlx::query!(
        "UPDATE oauth_clients SET is_active = false WHERE id = $1",
        client_uuid
    )
    .execute(&pool)
    .await
    .unwrap();

    // Try to authenticate
    let result = service.authenticate_client(client_id, client_secret).await;

    assert!(
        matches!(result, Err(AuthError::InvalidCredentials)),
        "Should reject inactive client"
    );

    cleanup_test_data(&pool, Some(client_id), None).await;
}

// ============================================================================
// Password Flow Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_password_flow_success() {
    let pool = setup_test_pool().await;
    let service = setup_auth_service(pool.clone()).await;

    let username = "testuser-password-success";
    let email = "testuser@example.com";
    let password = "secure-password-123";

    create_test_user(&pool, username, email, password).await;

    // Authenticate with password
    let result = service.authenticate_password(username, password).await;

    assert!(
        result.is_ok(),
        "Password authentication should succeed: {:?}",
        result.err()
    );

    let response = result.unwrap();
    assert_eq!(response.token_type, "Bearer");
    assert_eq!(response.expires_in, 3600);
    assert!(
        response.refresh_token.is_some(),
        "Password flow should return refresh token"
    );
    assert!(!response.access_token.is_empty());

    // Validate the token
    let claims = service.validate_token(&response.access_token).await;
    assert!(claims.is_ok(), "Token validation should succeed");

    cleanup_test_data(&pool, None, Some(username)).await;
}

#[tokio::test]
#[serial]
async fn test_password_flow_invalid_password() {
    let pool = setup_test_pool().await;
    let service = setup_auth_service(pool.clone()).await;

    let username = "testuser-invalid-password";
    let email = "testuser2@example.com";
    let password = "correct-password";

    create_test_user(&pool, username, email, password).await;

    // Try to authenticate with wrong password
    let result = service
        .authenticate_password(username, "wrong-password")
        .await;

    assert!(
        matches!(result, Err(AuthError::InvalidCredentials)),
        "Should reject invalid password"
    );

    cleanup_test_data(&pool, None, Some(username)).await;
}

#[tokio::test]
#[serial]
async fn test_password_flow_nonexistent_user() {
    let pool = setup_test_pool().await;
    let service = setup_auth_service(pool).await;

    // Try to authenticate with non-existent user
    let result = service
        .authenticate_password("nonexistent-user", "any-password")
        .await;

    assert!(
        matches!(result, Err(AuthError::InvalidCredentials)),
        "Should reject non-existent user"
    );
}

#[tokio::test]
#[serial]
async fn test_password_flow_inactive_user() {
    let pool = setup_test_pool().await;
    let service = setup_auth_service(pool.clone()).await;

    let username = "testuser-inactive";
    let email = "testuser3@example.com";
    let password = "test-password";

    let user_uuid = create_test_user(&pool, username, email, password).await;

    // Deactivate the user
    sqlx::query!(
        "UPDATE oauth_users SET is_active = false WHERE id = $1",
        user_uuid
    )
    .execute(&pool)
    .await
    .unwrap();

    // Try to authenticate
    let result = service.authenticate_password(username, password).await;

    assert!(
        matches!(result, Err(AuthError::InvalidCredentials)),
        "Should reject inactive user"
    );

    cleanup_test_data(&pool, None, Some(username)).await;
}

// ============================================================================
// Refresh Token Flow Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_refresh_token_success() {
    let pool = setup_test_pool().await;
    let service = setup_auth_service(pool.clone()).await;

    let username = "testuser-refresh";
    let email = "testuser4@example.com";
    let password = "test-password";

    // Cleanup any leftover data from previous failed runs
    pre_cleanup_test_data(&pool, None, Some(username)).await;

    create_test_user(&pool, username, email, password).await;

    // Get initial token with refresh token
    let initial_response = service
        .authenticate_password(username, password)
        .await
        .unwrap();

    let refresh_token = initial_response.refresh_token.unwrap();

    // Wait 1 second so the new JWT has a different iat (issued at) timestamp
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // Use refresh token to get new access token
    let result = service.refresh_token(&refresh_token).await;

    assert!(result.is_ok(), "Token refresh should succeed");

    let response = result.unwrap();
    assert_eq!(response.token_type, "Bearer");
    assert_eq!(response.expires_in, 3600);
    assert!(!response.access_token.is_empty());
    assert_ne!(
        response.access_token, initial_response.access_token,
        "New access token should be different"
    );

    // Verify refresh token rotation - new refresh token should be different
    let new_refresh_token = response.refresh_token.as_ref().unwrap();
    assert_ne!(
        new_refresh_token, &refresh_token,
        "New refresh token should be different (rotation)"
    );

    // Validate the new access token
    let claims = service.validate_token(&response.access_token).await;
    assert!(claims.is_ok(), "New token should be valid");

    // Verify old refresh token is now revoked
    let old_token_result = service.refresh_token(&refresh_token).await;
    assert!(
        matches!(old_token_result, Err(AuthError::RevokedToken)),
        "Old refresh token should be revoked after rotation"
    );

    cleanup_test_data(&pool, None, Some(username)).await;
}

#[tokio::test]
#[serial]
async fn test_refresh_token_invalid() {
    let pool = setup_test_pool().await;
    let service = setup_auth_service(pool).await;

    // Try to use invalid refresh token
    let result = service.refresh_token("invalid-refresh-token").await;

    assert!(
        matches!(result, Err(AuthError::InvalidToken(_))),
        "Should reject invalid refresh token"
    );
}

#[tokio::test]
#[serial]
async fn test_refresh_token_expired() {
    let pool = setup_test_pool().await;
    let service = setup_auth_service(pool.clone()).await;

    let username = "testuser-expired-refresh";
    let email = "testuser5@example.com";
    let password = "test-password";

    // Cleanup any leftover data from previous failed runs
    pre_cleanup_test_data(&pool, None, Some(username)).await;

    let user_uuid = create_test_user(&pool, username, email, password).await;

    // Create an expired refresh token
    let refresh_token = Uuid::now_v7().to_string();
    let refresh_token_hash = hash_refresh_token(&refresh_token); // Use SHA-256, not bcrypt

    sqlx::query!(
        r#"
        INSERT INTO oauth_refresh_tokens (token_hash, user_id, expires_at)
        VALUES ($1, $2, $3)
        "#,
        refresh_token_hash,
        user_uuid,
        Utc::now() - Duration::days(1) // Expired yesterday
    )
    .execute(&pool)
    .await
    .unwrap();

    // Try to use expired refresh token
    let result = service.refresh_token(&refresh_token).await;

    assert!(
        matches!(result, Err(AuthError::ExpiredToken)),
        "Should reject expired refresh token, got {:?}",
        result
    );

    cleanup_test_data(&pool, None, Some(username)).await;
}

// ============================================================================
// Token Validation Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_validate_token_success() {
    let pool = setup_test_pool().await;
    let service = setup_auth_service(pool.clone()).await;

    let client_id = "test-client-validate";
    let client_secret = "test-secret";

    create_test_client(&pool, client_id, client_secret).await;

    // Get a token
    let response = service
        .authenticate_client(client_id, client_secret)
        .await
        .unwrap();

    // Validate the token
    let result = service.validate_token(&response.access_token).await;

    assert!(result.is_ok(), "Token validation should succeed");

    let claims = result.unwrap();
    assert_eq!(claims.iss, "test");
    assert_eq!(claims.aud, "test-api");
    assert!(!claims.sub.is_empty());

    // Expiration should be in the future
    let now = Utc::now().timestamp();
    assert!(claims.exp > now, "Token should not be expired");

    cleanup_test_data(&pool, Some(client_id), None).await;
}

#[tokio::test]
#[serial]
async fn test_validate_token_malformed() {
    let pool = setup_test_pool().await;
    let service = setup_auth_service(pool).await;

    // Try to validate malformed token
    let result = service.validate_token("not-a-valid-jwt").await;

    assert!(
        matches!(result, Err(AuthError::InvalidToken(_))),
        "Should reject malformed token"
    );
}

#[tokio::test]
#[serial]
async fn test_validate_token_wrong_signature() {
    let pool = setup_test_pool().await;
    let service = setup_auth_service(pool.clone()).await;

    let client_id = "test-client-wrong-sig";
    let client_secret = "test-secret";

    create_test_client(&pool, client_id, client_secret).await;

    // Get a valid token
    let response = service
        .authenticate_client(client_id, client_secret)
        .await
        .unwrap();

    // Tamper with the signature
    let mut tampered_token = response.access_token.clone();
    tampered_token.pop();
    tampered_token.push('X');

    // Try to validate tampered token
    let result = service.validate_token(&tampered_token).await;

    assert!(
        matches!(result, Err(AuthError::InvalidToken(_))),
        "Should reject token with wrong signature"
    );

    cleanup_test_data(&pool, Some(client_id), None).await;
}

// ============================================================================
// Cross-flow Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_multiple_clients_different_tokens() {
    let pool = setup_test_pool().await;
    let service = setup_auth_service(pool.clone()).await;

    let client1_id = "test-client-1";
    let client1_secret = "secret-1";
    let client2_id = "test-client-2";
    let client2_secret = "secret-2";

    create_test_client(&pool, client1_id, client1_secret).await;
    create_test_client(&pool, client2_id, client2_secret).await;

    // Get tokens for both clients
    let response1 = service
        .authenticate_client(client1_id, client1_secret)
        .await
        .unwrap();

    let response2 = service
        .authenticate_client(client2_id, client2_secret)
        .await
        .unwrap();

    // Tokens should be different
    assert_ne!(
        response1.access_token, response2.access_token,
        "Different clients should get different tokens"
    );

    // Both tokens should be valid
    let claims1 = service
        .validate_token(&response1.access_token)
        .await
        .unwrap();
    let claims2 = service
        .validate_token(&response2.access_token)
        .await
        .unwrap();

    // Subjects should be different (different client IDs)
    assert_ne!(claims1.sub, claims2.sub, "Subjects should be different");

    cleanup_test_data(&pool, Some(client1_id), None).await;
    cleanup_test_data(&pool, Some(client2_id), None).await;
}

#[tokio::test]
#[serial]
async fn test_user_and_client_tokens_are_distinct() {
    let pool = setup_test_pool().await;
    let service = setup_auth_service(pool.clone()).await;

    let client_id = "test-client-distinct";
    let client_secret = "secret";
    let username = "testuser-distinct";
    let email = "distinct@example.com";
    let password = "password";

    create_test_client(&pool, client_id, client_secret).await;
    create_test_user(&pool, username, email, password).await;

    // Get token for client
    let client_response = service
        .authenticate_client(client_id, client_secret)
        .await
        .unwrap();

    // Get token for user
    let user_response = service
        .authenticate_password(username, password)
        .await
        .unwrap();

    // Tokens should be different
    assert_ne!(
        client_response.access_token, user_response.access_token,
        "User and client tokens should be different"
    );

    // Client token should not have refresh token
    assert!(
        client_response.refresh_token.is_none(),
        "Client credentials should not get refresh token"
    );

    // User token should have refresh token
    assert!(
        user_response.refresh_token.is_some(),
        "Password flow should get refresh token"
    );

    // Both tokens should validate
    let client_claims = service
        .validate_token(&client_response.access_token)
        .await
        .unwrap();
    let user_claims = service
        .validate_token(&user_response.access_token)
        .await
        .unwrap();

    // Subjects should be different
    assert_ne!(
        client_claims.sub, user_claims.sub,
        "Client and user should have different subjects"
    );

    cleanup_test_data(&pool, Some(client_id), Some(username)).await;
}
