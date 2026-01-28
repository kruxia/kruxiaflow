//! Integration tests for workflow query API
//!
//! Tests workflow query endpoints including get workflow and list workflows.

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use bcrypt::hash;
use kruxiaflow_api::handlers::workflows::{
    GetWorkflowResponse, ListWorkflowsResponse, SubmitWorkflowResponse,
};
use kruxiaflow_api::{AppState, AppStateBuild, app_router};
use kruxiaflow_core::PostgresSubscriptionService;
use kruxiaflow_core::events::PostgresEventSource;
use kruxiaflow_core::queue::{PostgresQueue, QueueConfig};
use kruxiaflow_oauth::{AuthConfig, PostgresAuthService};
use serde_json::json;
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Helper to create test database pool
async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://kruxiaflow:kruxiaflow_dev@127.0.0.1:5433/kruxiaflow".to_string()
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
    sqlx::query(
        "INSERT INTO oauth_clients (client_id, client_secret_hash, name, created_at)
         VALUES ($1, $2, $3, NOW())
         ON CONFLICT (client_id) DO NOTHING",
    )
    .bind("test-client")
    .bind(hash("test-secret", bcrypt::DEFAULT_COST).unwrap())
    .bind("Test Client")
    .execute(&pool)
    .await
    .expect("Failed to create test client");

    let queue = Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let event_source = Arc::new(PostgresEventSource::new(pool.clone()));
    let workflow_storage = Arc::new(kruxiaflow_core::storage::PostgresStorage::new(pool.clone()));
    let cache_service = Arc::new(kruxiaflow_core::cache::NoOpCache::new());

    let subscription_service = Arc::new(PostgresSubscriptionService::new(pool.clone()));
    AppState::with_metadata(
        pool,
        Arc::new(auth_service),
        queue,
        event_source,
        workflow_storage,
        cache_service,
        subscription_service,
        CancellationToken::new(),
        "0.2.0-test".to_string(),
        AppStateBuild {
            timestamp: "2025-10-30T00:00:00Z".to_string(),
            git_hash: "test123".to_string(),
        },
        vec!["workflows".to_string(), "testing".to_string()],
    )
}

/// Helper to create test server
async fn setup_test_server() -> TestServer {
    let state = setup_test_state().await;
    let app = app_router(state);
    TestServer::new(app).expect("Failed to create test server")
}

/// Helper to get OAuth token
async fn get_test_token(server: &TestServer) -> String {
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

/// Helper to deploy a test workflow definition
async fn deploy_test_workflow(server: &TestServer, token: &str, name: &str) -> String {
    let response = server
        .post("/api/v1/workflow_definitions")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "name": name,
            "activities": [
                {
                    "key": "step1",
                    "worker": "test",
                    "name": "echo",
                    "parameters": {}
                }
            ]
        }))
        .await;

    if response.status_code() != StatusCode::CREATED {
        let error_text = response.text();
        eprintln!(
            "Workflow definition deployment failed with status {}: {}",
            response.status_code(),
            error_text
        );
        panic!(
            "Expected status CREATED (201), got {}",
            response.status_code()
        );
    }

    let body: serde_json::Value = response.json();
    body["version"].as_str().unwrap().to_string()
}

/// Helper to submit a workflow
async fn submit_workflow(server: &TestServer, token: &str, def_name: &str) -> uuid::Uuid {
    let response = server
        .post("/api/v1/workflows")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .json(&json!({
            "definition_name": def_name,
            "input": {"key": "value"}
        }))
        .await;

    if response.status_code() != StatusCode::CREATED {
        let error_text = response.text();
        eprintln!(
            "Workflow submission failed with status {}: {}",
            response.status_code(),
            error_text
        );
        panic!(
            "Expected status CREATED (201), got {}",
            response.status_code()
        );
    }

    let body: SubmitWorkflowResponse = response.json();
    body.workflow_id
}

#[tokio::test]
#[serial]
async fn test_get_workflow_success() {
    let server = setup_test_server().await;
    let token = get_test_token(&server).await;

    // Deploy workflow definition and submit workflow
    let def_name = "test_get_workflow";
    deploy_test_workflow(&server, &token, def_name).await;
    let workflow_id = submit_workflow(&server, &token, def_name).await;

    // Get workflow
    let response = server
        .get(&format!("/api/v1/workflows/{}", workflow_id))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let body: GetWorkflowResponse = response.json();
    assert_eq!(body.id, workflow_id);
    assert_eq!(body.definition_name, def_name);
    // Activities should be an array of ActivityState objects (may be empty for new workflows)
    // Verify activities have proper structure if any exist
    for activity in &body.activities {
        assert!(!activity.activity_key.is_empty());
    }
    assert!(body.state_data.is_object());
}

#[tokio::test]
#[serial]
async fn test_get_workflow_not_found() {
    let server = setup_test_server().await;
    let token = get_test_token(&server).await;

    let nonexistent_id = uuid::Uuid::now_v7();

    let response = server
        .get(&format!("/api/v1/workflows/{}", nonexistent_id))
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
async fn test_get_workflow_requires_authentication() {
    let server = setup_test_server().await;

    let workflow_id = uuid::Uuid::now_v7();

    let response = server
        .get(&format!("/api/v1/workflows/{}", workflow_id))
        .await;

    assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[serial]
async fn test_list_workflows_no_filters() {
    let server = setup_test_server().await;
    let token = get_test_token(&server).await;

    // Deploy and submit multiple workflows
    let def_name = "test_list_workflows";
    deploy_test_workflow(&server, &token, def_name).await;

    let _wf1 = submit_workflow(&server, &token, def_name).await;
    let _wf2 = submit_workflow(&server, &token, def_name).await;
    let _wf3 = submit_workflow(&server, &token, def_name).await;

    let response = server
        .get("/api/v1/workflows")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let body: ListWorkflowsResponse = response.json();
    assert!(body.workflows.len() >= 3);
    assert!(body.total >= 3);
    assert_eq!(body.count, body.workflows.len() as i64);
    assert_eq!(body.limit, 100); // default limit
    assert_eq!(body.offset, 0);
}

#[tokio::test]
#[serial]
async fn test_list_workflows_filter_by_status() {
    let server = setup_test_server().await;
    let token = get_test_token(&server).await;

    // Deploy and submit workflows
    let def_name = "test_filter_status";
    deploy_test_workflow(&server, &token, def_name).await;
    submit_workflow(&server, &token, def_name).await;
    submit_workflow(&server, &token, def_name).await;

    let response = server
        .get("/api/v1/workflows?status=created")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let body: ListWorkflowsResponse = response.json();
    assert!(!body.workflows.is_empty());

    // Verify all returned workflows have the filtered status
    for workflow in &body.workflows {
        assert_eq!(workflow.status, kruxiaflow_core::WorkflowStatus::Created);
    }
}

#[tokio::test]
#[serial]
async fn test_list_workflows_filter_by_definition_name() {
    let server = setup_test_server().await;
    let token = get_test_token(&server).await;

    // Deploy two different workflow definitions
    let def_name1 = "test_filter_def_1";
    let def_name2 = "test_filter_def_2";
    deploy_test_workflow(&server, &token, def_name1).await;
    deploy_test_workflow(&server, &token, def_name2).await;

    submit_workflow(&server, &token, def_name1).await;
    submit_workflow(&server, &token, def_name1).await;
    submit_workflow(&server, &token, def_name2).await;

    let response = server
        .get(&format!("/api/v1/workflows?definition_name={}", def_name1))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let body: ListWorkflowsResponse = response.json();
    assert!(!body.workflows.is_empty());

    // Verify all returned workflows have the filtered definition_name
    for workflow in &body.workflows {
        assert_eq!(workflow.definition_name, def_name1);
    }
}

#[tokio::test]
#[serial]
async fn test_list_workflows_pagination() {
    let server = setup_test_server().await;
    let token = get_test_token(&server).await;

    // Deploy and submit 10 workflows
    let def_name = "test_pagination";
    deploy_test_workflow(&server, &token, def_name).await;

    for _ in 0..10 {
        submit_workflow(&server, &token, def_name).await;
    }

    // Get first page (limit=5)
    let response = server
        .get("/api/v1/workflows?limit=5&offset=0")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let body: ListWorkflowsResponse = response.json();
    assert_eq!(body.count, 5);
    assert_eq!(body.limit, 5);
    assert_eq!(body.offset, 0);
    assert!(body.total >= 10);

    // Get second page (limit=5, offset=5)
    let response2 = server
        .get("/api/v1/workflows?limit=5&offset=5")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;

    assert_eq!(response2.status_code(), StatusCode::OK);

    let body2: ListWorkflowsResponse = response2.json();
    assert_eq!(body2.limit, 5);
    assert_eq!(body2.offset, 5);

    // Ensure we got different workflows on the second page
    let first_page_ids: Vec<_> = body.workflows.iter().map(|w| w.id).collect();
    let second_page_ids: Vec<_> = body2.workflows.iter().map(|w| w.id).collect();

    // No overlap between pages
    for id in &second_page_ids {
        assert!(!first_page_ids.contains(id));
    }
}

#[tokio::test]
#[serial]
async fn test_list_workflows_invalid_limit() {
    let server = setup_test_server().await;
    let token = get_test_token(&server).await;

    // Test limit too high (max is 1000)
    let response = server
        .get("/api/v1/workflows?limit=2000")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;

    assert_eq!(response.status_code(), StatusCode::UNPROCESSABLE_ENTITY);

    let body: serde_json::Value = response.json();
    assert_eq!(body["error"]["code"], "VALIDATION_ERROR");
}

#[tokio::test]
#[serial]
async fn test_list_workflows_invalid_offset() {
    let server = setup_test_server().await;
    let token = get_test_token(&server).await;

    // Test negative offset
    let response = server
        .get("/api/v1/workflows?offset=-1")
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;

    assert_eq!(response.status_code(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
#[serial]
async fn test_list_workflows_requires_authentication() {
    let server = setup_test_server().await;

    let response = server.get("/api/v1/workflows").await;

    assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[serial]
async fn test_workflow_query_end_to_end() {
    let server = setup_test_server().await;
    let token = get_test_token(&server).await;

    // Deploy workflow definition
    let def_name = "test_e2e_query";
    deploy_test_workflow(&server, &token, def_name).await;

    // Submit workflow
    let workflow_id = submit_workflow(&server, &token, def_name).await;

    // Get workflow by ID
    let get_response = server
        .get(&format!("/api/v1/workflows/{}", workflow_id))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;

    assert_eq!(get_response.status_code(), StatusCode::OK);
    let get_body: GetWorkflowResponse = get_response.json();
    assert_eq!(get_body.id, workflow_id);
    // Workflow response includes structured activities array (may be empty for new workflows)
    for activity in &get_body.activities {
        assert!(!activity.activity_key.is_empty());
    }

    // List workflows and verify our workflow is in the list
    let list_response = server
        .get(&format!("/api/v1/workflows?definition_name={}", def_name))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;

    assert_eq!(list_response.status_code(), StatusCode::OK);
    let list_body: ListWorkflowsResponse = list_response.json();
    assert!(list_body.workflows.iter().any(|w| w.id == workflow_id));
}
