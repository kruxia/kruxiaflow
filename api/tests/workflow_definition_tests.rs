// api/tests/workflow_definition_tests.rs
//! Integration tests for workflow definition management API
//!
//! Tests deployment, retrieval, and listing of workflow definitions.

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use bcrypt::hash;
use serde_json::json;
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use streamflow_api::handlers::workflow_definitions::{
    DeployWorkflowDefinitionResponse, GetWorkflowDefinitionResponse,
    ListWorkflowDefinitionsResponse,
};
use streamflow_api::{AppState, AppStateBuild, app_router};
use streamflow_core::events::PostgresEventSource;
use streamflow_core::queue::{PostgresQueue, QueueConfig};
use streamflow_oauth::{AuthConfig, PostgresAuthService};
use tokio_util::sync::CancellationToken;

/// Helper to create test database pool
async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://streamflow:streamflow_dev@127.0.0.1:5433/streamflow".to_string()
    });

    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    sqlx::migrate!("../migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

/// Load test RSA keys
fn test_rsa_private_key() -> String {
    include_str!("../../oauth/tests/private.pem").to_string()
}

fn test_rsa_public_key() -> String {
    include_str!("../../oauth/tests/public.pem").to_string()
}

/// Helper to create test AppState
async fn setup_test_state() -> AppState {
    let pool = setup_test_pool().await;

    let auth_config = AuthConfig {
        rsa_private_key_pem: test_rsa_private_key(),
        rsa_public_key_pem: Some(test_rsa_public_key()),
        jwt_issuer: "test".to_string(),
        jwt_audience: "test".to_string(),
        token_ttl: 3600,
    };

    let auth_service = PostgresAuthService::new(pool.clone(), auth_config)
        .expect("Failed to create test auth service");

    // Create test client for client_credentials flow
    sqlx::query!(
        "INSERT INTO oauth_clients (client_id, client_secret_hash, name, created_at)
         VALUES ($1, $2, $3, NOW())
         ON CONFLICT (client_id) DO NOTHING",
        "test-client",
        hash("test-secret", bcrypt::DEFAULT_COST).unwrap(),
        "Test Client"
    )
    .execute(&pool)
    .await
    .expect("Failed to create test client");

    let queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let event_source = Arc::new(PostgresEventSource::new(pool.clone()));

    AppState::with_metadata(
        pool,
        Arc::new(auth_service),
        queue,
        event_source,
        CancellationToken::new(),
        "0.2.0-test".to_string(),
        AppStateBuild {
            timestamp: "2025-10-30T00:00:00Z".to_string(),
            git_hash: "test123".to_string(),
        },
        vec!["workflows".to_string()],
    )
}

/// Helper to create test server
async fn setup_test_server() -> TestServer {
    let state = setup_test_state().await;
    let router = app_router(state);
    TestServer::new(router).expect("Failed to create test server")
}

/// Helper to get a valid access token
async fn get_valid_token(server: &TestServer) -> String {
    let response = server
        .post("/api/v1/oauth/token")
        .json(&json!({
            "grant_type": "client_credentials",
            "client_id": "test-client",
            "client_secret": "test-secret"
        }))
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let body: serde_json::Value = response.json();
    body["access_token"].as_str().unwrap().to_string()
}

// ============================================================================
// Workflow Definition Deployment Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_deploy_workflow_definition() {
    let server = setup_test_server().await;
    let token = get_valid_token(&server).await;

    let definition = json!({
        "name": "test_workflow_deploy",
        "activities": [
            {
                "key": "step1",
                "worker": "test",
                "name": "test_activity"
            }
        ]
    });

    let response = server
        .post("/api/v1/workflow_definitions")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&definition)
        .await;

    assert_eq!(response.status_code(), StatusCode::CREATED);

    let body: DeployWorkflowDefinitionResponse = response.json();
    assert_eq!(body.name, "test_workflow_deploy");
    // Version should be in format YYYYmmdd.HHMMSS.uuuuuu (total 21 chars with dots)
    assert!(body.version.len() >= 20); // At least YYYYmmdd.HHMMSS format
    assert!(body.version.contains('.')); // Should have dot separator
}

#[tokio::test]
#[serial]
async fn test_deploy_multiple_versions() {
    let server = setup_test_server().await;
    let token = get_valid_token(&server).await;

    let definition = json!({
        "name": "test_workflow_versions",
        "activities": [
            {
                "key": "step1",
                "worker": "test"
            }
        ]
    });

    // Deploy first version
    let response1 = server
        .post("/api/v1/workflow_definitions")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&definition)
        .await;

    assert_eq!(response1.status_code(), StatusCode::CREATED);
    let body1: DeployWorkflowDefinitionResponse = response1.json();

    // Sleep briefly to ensure different timestamp (microsecond precision should be enough, but be safe)
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    // Deploy second version - should succeed with different timestamp
    let response2 = server
        .post("/api/v1/workflow_definitions")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&definition)
        .await;

    assert_eq!(response2.status_code(), StatusCode::CREATED);
    let body2: DeployWorkflowDefinitionResponse = response2.json();

    // Versions should be different
    assert_ne!(body1.version, body2.version);
    // Second version should be later (lexicographically greater)
    assert!(body2.version > body1.version);
}

#[tokio::test]
#[serial]
async fn test_deploy_invalid_workflow() {
    let server = setup_test_server().await;
    let token = get_valid_token(&server).await;

    // Invalid workflow: references non-existent activity
    let definition = json!({
        "name": "test_workflow_invalid",
        "activities": [
            {
                "key": "step1",
                "worker": "test",
                "following": [
                    {
                        "activity_key": "step2" // Doesn't exist!
                    }
                ]
            }
        ]
    });

    let response = server
        .post("/api/v1/workflow_definitions")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&definition)
        .await;

    assert_eq!(response.status_code(), StatusCode::UNPROCESSABLE_ENTITY);

    let body: serde_json::Value = response.json();
    // Should have validation error with field details
    assert_eq!(body["error"]["code"], "VALIDATION_ERROR");
    // Check that the field_errors contains the error about the missing activity
    let details_str = serde_json::to_string(&body["error"]["details"]).unwrap();
    assert!(details_str.contains("not found") || details_str.contains("step2"));
}

#[tokio::test]
#[serial]
async fn test_deploy_workflow_with_cycle() {
    let server = setup_test_server().await;
    let token = get_valid_token(&server).await;

    // Invalid workflow: contains cycle
    let definition = json!({
        "name": "test_workflow_cycle",
        "activities": [
            {
                "key": "step1",
                "worker": "test",
                "following": [
                    {
                        "activity_key": "step2"
                    }
                ]
            },
            {
                "key": "step2",
                "worker": "test",
                "following": [
                    {
                        "activity_key": "step1" // Creates cycle!
                    }
                ]
            }
        ]
    });

    let response = server
        .post("/api/v1/workflow_definitions")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&definition)
        .await;

    assert_eq!(response.status_code(), StatusCode::UNPROCESSABLE_ENTITY);

    let body: serde_json::Value = response.json();
    assert_eq!(body["error"]["code"], "VALIDATION_ERROR");
    let details_str = serde_json::to_string(&body["error"]["details"]).unwrap();
    assert!(details_str.contains("cycle"));
}

#[tokio::test]
#[serial]
async fn test_deploy_workflow_with_no_activities() {
    let server = setup_test_server().await;
    let token = get_valid_token(&server).await;

    // Invalid workflow: no activities
    let definition = json!({
        "name": "test_workflow_empty",
        "activities": []
    });

    let response = server
        .post("/api/v1/workflow_definitions")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&definition)
        .await;

    assert_eq!(response.status_code(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
#[serial]
async fn test_deploy_workflow_with_duplicate_activity_keys() {
    let server = setup_test_server().await;
    let token = get_valid_token(&server).await;

    // Invalid workflow: duplicate activity keys
    let definition = json!({
        "name": "test_workflow_duplicate",
        "activities": [
            {
                "key": "step1",
                "worker": "test"
            },
            {
                "key": "step1", // Duplicate!
                "worker": "test"
            }
        ]
    });

    let response = server
        .post("/api/v1/workflow_definitions")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&definition)
        .await;

    assert_eq!(response.status_code(), StatusCode::UNPROCESSABLE_ENTITY);

    let body: serde_json::Value = response.json();
    assert_eq!(body["error"]["code"], "VALIDATION_ERROR");
    let details_str = serde_json::to_string(&body["error"]["details"]).unwrap();
    assert!(details_str.contains("Duplicate") || details_str.contains("duplicate"));
}

// ============================================================================
// Workflow Definition Retrieval Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_list_workflow_definitions() {
    let server = setup_test_server().await;
    let token = get_valid_token(&server).await;

    // Deploy a few definitions
    for i in 1..=3 {
        let definition = json!({
            "name": format!("workflow_list_{}", i),
            "activities": [
                {
                    "key": "step1",
                    "worker": "test"
                }
            ]
        });

        let response = server
            .post("/api/v1/workflow_definitions")
            .add_header(
                HeaderName::from_static("authorization"),
                HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
            )
            .json(&definition)
            .await;

        assert_eq!(response.status_code(), StatusCode::CREATED);
    }

    // List all definitions
    let response = server
        .get("/api/v1/workflow_definitions")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let body: ListWorkflowDefinitionsResponse = response.json();
    assert!(body.total >= 3);
    assert!(body.definitions.len() >= 3);

    // Verify summaries have required fields for workflows created by this test
    let our_workflows: Vec<_> = body
        .definitions
        .iter()
        .filter(|d| d.name.starts_with("workflow_list_"))
        .collect();
    assert!(our_workflows.len() >= 3);
    for summary in our_workflows.iter() {
        assert!(!summary.name.is_empty());
        assert!(!summary.version.is_empty());
        assert_eq!(summary.activity_count, 1);
    }
}

#[tokio::test]
#[serial]
async fn test_get_workflow_definition_by_version() {
    let server = setup_test_server().await;
    let token = get_valid_token(&server).await;

    // Deploy workflow
    let definition = json!({
        "name": "test_workflow_get_version",
        "activities": [
            {
                "key": "step1",
                "worker": "test"
            }
        ]
    });

    let deploy_response = server
        .post("/api/v1/workflow_definitions")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&definition)
        .await;

    assert_eq!(deploy_response.status_code(), StatusCode::CREATED);
    let deploy_body: DeployWorkflowDefinitionResponse = deploy_response.json();
    let version = deploy_body.version;

    // Get specific version
    let response = server
        .get(&format!(
            "/api/v1/workflow_definitions/test_workflow_get_version?version={}",
            version
        ))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let body: GetWorkflowDefinitionResponse = response.json();
    assert_eq!(body.name, "test_workflow_get_version");
    assert_eq!(body.version, version);
    assert_eq!(body.activities.len(), 1);
    assert_eq!(body.activities[0].key, "step1");
}

#[tokio::test]
#[serial]
async fn test_get_latest_workflow_definition() {
    let server = setup_test_server().await;
    let token = get_valid_token(&server).await;

    let definition = json!({
        "name": "test_workflow_get_latest",
        "activities": [
            {
                "key": "step1",
                "worker": "test"
            }
        ]
    });

    // Deploy multiple versions
    let mut versions = Vec::new();
    for _ in 0..3 {
        let deploy_response = server
            .post("/api/v1/workflow_definitions")
            .add_header(
                HeaderName::from_static("authorization"),
                HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
            )
            .json(&definition)
            .await;

        assert_eq!(deploy_response.status_code(), StatusCode::CREATED);
        let deploy_body: DeployWorkflowDefinitionResponse = deploy_response.json();
        versions.push(deploy_body.version);

        // Sleep to ensure different timestamps
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    }

    // Get latest (no version parameter)
    let response = server
        .get("/api/v1/workflow_definitions/test_workflow_get_latest")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let body: GetWorkflowDefinitionResponse = response.json();
    assert_eq!(body.name, "test_workflow_get_latest");
    // Latest version should be the last one deployed
    assert_eq!(body.version, versions[2]);
}

#[tokio::test]
#[serial]
async fn test_get_nonexistent_workflow() {
    let server = setup_test_server().await;
    let token = get_valid_token(&server).await;

    let response = server
        .get("/api/v1/workflow_definitions/nonexistent_workflow")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;

    assert_eq!(response.status_code(), StatusCode::NOT_FOUND);

    let body: serde_json::Value = response.json();
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not found")
    );
}

#[tokio::test]
#[serial]
async fn test_get_nonexistent_version() {
    let server = setup_test_server().await;
    let token = get_valid_token(&server).await;

    // Deploy a workflow
    let definition = json!({
        "name": "test_workflow_version_not_found",
        "activities": [
            {
                "key": "step1",
                "worker": "test"
            }
        ]
    });

    server
        .post("/api/v1/workflow_definitions")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&definition)
        .await;

    // Try to get a non-existent version
    let response = server
        .get("/api/v1/workflow_definitions/test_workflow_version_not_found?version=20000101.000000.000000")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;

    assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
}

// ============================================================================
// Authentication Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_deploy_requires_authentication() {
    let server = setup_test_server().await;

    let definition = json!({
        "name": "test_workflow_auth",
        "activities": [
            {
                "key": "step1",
                "worker": "test"
            }
        ]
    });

    // No Authorization header
    let response = server
        .post("/api/v1/workflow_definitions")
        .json(&definition)
        .await;

    assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[serial]
async fn test_list_requires_authentication() {
    let server = setup_test_server().await;

    // No Authorization header
    let response = server.get("/api/v1/workflow_definitions").await;

    assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[serial]
async fn test_get_requires_authentication() {
    let server = setup_test_server().await;

    // No Authorization header
    let response = server
        .get("/api/v1/workflow_definitions/test_workflow")
        .await;

    assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);
}

// ============================================================================
// Complex Workflow Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_deploy_complex_workflow_with_dependencies() {
    let server = setup_test_server().await;
    let token = get_valid_token(&server).await;

    let definition = json!({
        "name": "payment_processing",
        "activities": [
            {
                "key": "validate_payment",
                "worker": "payments",
                "name": "validate_card",
                "parameters": {
                    "card_token": "{{ARG.card_token}}"
                },
                "following": [
                    {
                        "activity_key": "authorize_card",
                        "conditions": ["{{validate_payment.valid}} == true"]
                    }
                ]
            },
            {
                "key": "authorize_card",
                "worker": "payments",
                "name": "authorize",
                "parameters": {
                    "amount": "{{ARG.amount}}"
                },
                "following": [
                    {
                        "activity_key": "capture_payment"
                    }
                ]
            },
            {
                "key": "capture_payment",
                "worker": "payments",
                "name": "capture",
                "parameters": {
                    "authorization_id": "{{authorize_card.authorization_id}}"
                }
            }
        ]
    });

    let response = server
        .post("/api/v1/workflow_definitions")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&definition)
        .await;

    assert_eq!(response.status_code(), StatusCode::CREATED);

    let body: DeployWorkflowDefinitionResponse = response.json();
    assert_eq!(body.name, "payment_processing");

    // Retrieve and verify
    let get_response = server
        .get("/api/v1/workflow_definitions/payment_processing")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;

    assert_eq!(get_response.status_code(), StatusCode::OK);

    let get_body: GetWorkflowDefinitionResponse = get_response.json();
    assert_eq!(get_body.activities.len(), 3);
    assert_eq!(get_body.activities[0].key, "validate_payment");
    assert_eq!(get_body.activities[1].key, "authorize_card");
    assert_eq!(get_body.activities[2].key, "capture_payment");
}
