//! Integration tests for output retrieval API
//!
//! Tests activity output, workflow output, and file download endpoints.

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use bcrypt::hash;
use kruxiaflow_api::dto::{
    GetActivityOutputResponse, GetWorkflowOutputResponse, UploadActivityFileResponse,
};
use kruxiaflow_api::handlers::workflows::SubmitWorkflowResponse;
use kruxiaflow_api::{AppState, AppStateBuild, app_router};
use kruxiaflow_core::PostgresSubscriptionService;
use kruxiaflow_core::WorkflowStatus;
use kruxiaflow_core::events::PostgresEventSource;
use kruxiaflow_core::queue::{PostgresQueue, QueueConfig};
use kruxiaflow_oauth::{AuthConfig, PostgresAuthService};
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Load test RSA keys
fn test_rsa_private_key() -> String {
    include_str!("../../oauth/tests/private.pem").to_string()
}

fn test_rsa_public_key() -> String {
    include_str!("../../oauth/tests/public.pem").to_string()
}

/// Helper to create test AppState
async fn setup_test_state(pool: PgPool) -> AppState {
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
    .bind(hash("test-secret", 4).unwrap()) // min cost: real hashing strength is not under test
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
        "0.3.0-test".to_string(),
        AppStateBuild {
            timestamp: "2025-12-05T00:00:00Z".to_string(),
            git_hash: "test123".to_string(),
        },
        vec!["workflows".to_string(), "testing".to_string()],
    )
}

/// Helper to create test server
async fn setup_test_server(pool: PgPool) -> TestServer {
    let state = setup_test_state(pool).await;
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
                },
                {
                    "key": "step2",
                    "worker": "test",
                    "name": "echo",
                    "parameters": {},
                    "depends_on": ["step1"]
                }
            ]
        }))
        .await;

    // Accept both 201 Created (new) and 200 OK (already exists/unchanged)
    if response.status_code() != StatusCode::CREATED && response.status_code() != StatusCode::OK {
        let error_text = response.text();
        panic!(
            "Workflow definition deployment failed with status {}: {}",
            response.status_code(),
            error_text
        );
    }

    let body: serde_json::Value = response.json();
    body["version"].as_str().unwrap().to_string()
}

/// Helper to submit a workflow
async fn submit_workflow(server: &TestServer, token: &str, def_name: &str) -> Uuid {
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
        panic!(
            "Workflow submission failed with status {}: {}",
            response.status_code(),
            error_text
        );
    }

    let body: SubmitWorkflowResponse = response.json();
    body.workflow_id
}

/// Helper to mark workflow as completed with activity outputs
async fn complete_workflow_with_output(pool: &PgPool, workflow_id: Uuid) {
    // Update workflow to completed status with activity outputs
    sqlx::query(
        r#"
        UPDATE workflows
        SET status = 'completed',
            activities = jsonb_set(
                jsonb_set(
                    activities,
                    '{step1}',
                    '{"status": "completed", "outputs": {"result": "step1 output"}, "completed_at": "2025-12-05T10:00:00Z", "depends_on": []}'::jsonb
                ),
                '{step2}',
                '{"status": "completed", "outputs": {"result": "step2 output"}, "completed_at": "2025-12-05T10:01:00Z", "depends_on": [{"activity_key": "step1"}]}'::jsonb
            ),
            updated_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(workflow_id)
    .execute(pool)
    .await
    .expect("Failed to update workflow");
}

/// Helper to mark a single activity as completed
async fn complete_activity_with_output(pool: &PgPool, workflow_id: Uuid, activity_key: &str) {
    let output_json = json!({
        "status": "completed",
        "outputs": {"result": format!("{} output", activity_key)},
        "completed_at": "2025-12-05T10:00:00Z",
        "depends_on": []
    });

    sqlx::query(
        r#"
        UPDATE workflows
        SET activities = jsonb_set(activities, $2::text[], $3::jsonb),
            updated_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(workflow_id)
    .bind([activity_key])
    .bind(output_json)
    .execute(pool)
    .await
    .expect("Failed to update activity");
}

/// Helper to add an activity in pending state (not completed)
async fn add_pending_activity(
    pool: &PgPool,
    workflow_id: Uuid,
    activity_key: &str,
    depends_on: Vec<&str>,
) {
    let deps: Vec<serde_json::Value> = depends_on
        .iter()
        .map(|k| json!({"activity_key": *k}))
        .collect();

    let state_json = json!({
        "status": "pending",
        "depends_on": deps
    });

    sqlx::query(
        r#"
        UPDATE workflows
        SET activities = jsonb_set(activities, $2::text[], $3::jsonb),
            updated_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(workflow_id)
    .bind([activity_key])
    .bind(state_json)
    .execute(pool)
    .await
    .expect("Failed to add pending activity");
}

// ============================================================================
// Activity Output Tests
// ============================================================================

#[sqlx::test(migrations = "../migrations")]
async fn test_get_activity_output_success(pool: PgPool) {
    let server = setup_test_server(pool.clone()).await;
    let token = get_test_token(&server).await;

    // Deploy and submit workflow
    let def_name = "test_activity_output";
    deploy_test_workflow(&server, &token, def_name).await;
    let workflow_id = submit_workflow(&server, &token, def_name).await;

    // Complete the activity
    complete_activity_with_output(&pool, workflow_id, "step1").await;

    // Get activity output
    let response = server
        .get(&format!(
            "/api/v1/workflows/{}/activities/step1/output",
            workflow_id
        ))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let body: GetActivityOutputResponse = response.json();
    assert_eq!(body.workflow_id, workflow_id);
    assert_eq!(body.activity_key, "step1");
    assert!(body.output.is_some());
    assert_eq!(body.output.as_ref().unwrap()["result"], "step1 output");
}

#[sqlx::test(migrations = "../migrations")]
async fn test_get_activity_output_not_completed(pool: PgPool) {
    let server = setup_test_server(pool.clone()).await;
    let token = get_test_token(&server).await;

    // Deploy and submit workflow
    let def_name = "test_activity_output_not_completed";
    deploy_test_workflow(&server, &token, def_name).await;
    let workflow_id = submit_workflow(&server, &token, def_name).await;

    // Add activity in pending state (not completed)
    add_pending_activity(&pool, workflow_id, "step1", vec![]).await;

    // Try to get output for non-completed activity
    let response = server
        .get(&format!(
            "/api/v1/workflows/{}/activities/step1/output",
            workflow_id
        ))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);

    let body: serde_json::Value = response.json();
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not completed")
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn test_get_activity_output_workflow_not_found(pool: PgPool) {
    let server = setup_test_server(pool).await;
    let token = get_test_token(&server).await;

    let nonexistent_id = Uuid::now_v7();

    let response = server
        .get(&format!(
            "/api/v1/workflows/{}/activities/step1/output",
            nonexistent_id
        ))
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

#[sqlx::test(migrations = "../migrations")]
async fn test_get_activity_output_activity_not_found(pool: PgPool) {
    let server = setup_test_server(pool).await;
    let token = get_test_token(&server).await;

    // Deploy and submit workflow
    let def_name = "test_activity_not_found";
    deploy_test_workflow(&server, &token, def_name).await;
    let workflow_id = submit_workflow(&server, &token, def_name).await;

    // Try to get output for non-existent activity
    let response = server
        .get(&format!(
            "/api/v1/workflows/{}/activities/nonexistent/output",
            workflow_id
        ))
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

#[sqlx::test(migrations = "../migrations")]
async fn test_get_activity_output_requires_authentication(pool: PgPool) {
    let server = setup_test_server(pool).await;

    let workflow_id = Uuid::now_v7();

    let response = server
        .get(&format!(
            "/api/v1/workflows/{}/activities/step1/output",
            workflow_id
        ))
        .await;

    assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);
}

// ============================================================================
// Workflow Output Tests
// ============================================================================

#[sqlx::test(migrations = "../migrations")]
async fn test_get_workflow_output_success(pool: PgPool) {
    let server = setup_test_server(pool.clone()).await;
    let token = get_test_token(&server).await;

    // Deploy and submit workflow
    let def_name = "test_workflow_output";
    deploy_test_workflow(&server, &token, def_name).await;
    let workflow_id = submit_workflow(&server, &token, def_name).await;

    // Complete the workflow
    complete_workflow_with_output(&pool, workflow_id).await;

    // Get workflow output
    let response = server
        .get(&format!("/api/v1/workflows/{}/output", workflow_id))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let body: GetWorkflowOutputResponse = response.json();
    assert_eq!(body.workflow_id, workflow_id);
    assert_eq!(body.status, WorkflowStatus::Completed);
    assert!(body.outputs.contains_key("step1"));
    assert!(body.outputs.contains_key("step2"));

    // step2 should be terminal (step1 is depended on by step2)
    assert!(body.terminal_outputs.contains(&"step2".to_string()));
    assert!(!body.terminal_outputs.contains(&"step1".to_string()));
}

#[sqlx::test(migrations = "../migrations")]
async fn test_get_workflow_output_not_completed(pool: PgPool) {
    let server = setup_test_server(pool).await;
    let token = get_test_token(&server).await;

    // Deploy and submit workflow (not completed)
    let def_name = "test_workflow_output_not_completed";
    deploy_test_workflow(&server, &token, def_name).await;
    let workflow_id = submit_workflow(&server, &token, def_name).await;

    // Try to get output for non-completed workflow
    let response = server
        .get(&format!("/api/v1/workflows/{}/output", workflow_id))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);

    let body: serde_json::Value = response.json();
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not completed")
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn test_get_workflow_output_not_found(pool: PgPool) {
    let server = setup_test_server(pool).await;
    let token = get_test_token(&server).await;

    let nonexistent_id = Uuid::now_v7();

    let response = server
        .get(&format!("/api/v1/workflows/{}/output", nonexistent_id))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;

    assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_get_workflow_output_requires_authentication(pool: PgPool) {
    let server = setup_test_server(pool).await;

    let workflow_id = Uuid::now_v7();

    let response = server
        .get(&format!("/api/v1/workflows/{}/output", workflow_id))
        .await;

    assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);
}

// ============================================================================
// File Download Tests
// ============================================================================

#[sqlx::test(migrations = "../migrations")]
async fn test_download_file_not_found(pool: PgPool) {
    let server = setup_test_server(pool).await;
    let token = get_test_token(&server).await;

    // Deploy and submit workflow
    let def_name = "test_file_download";
    deploy_test_workflow(&server, &token, def_name).await;
    let workflow_id = submit_workflow(&server, &token, def_name).await;

    // Try to download non-existent file
    let response = server
        .get(&format!(
            "/api/v1/workflows/{}/activities/step1/files/nonexistent.txt",
            workflow_id
        ))
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

#[sqlx::test(migrations = "../migrations")]
async fn test_download_file_requires_authentication(pool: PgPool) {
    let server = setup_test_server(pool).await;

    let workflow_id = Uuid::now_v7();

    let response = server
        .get(&format!(
            "/api/v1/workflows/{}/activities/step1/files/test.txt",
            workflow_id
        ))
        .await;

    assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);
}

// ============================================================================
// End-to-End Tests
// ============================================================================

#[sqlx::test(migrations = "../migrations")]
async fn test_output_retrieval_end_to_end(pool: PgPool) {
    let server = setup_test_server(pool.clone()).await;
    let token = get_test_token(&server).await;

    // Deploy workflow definition
    let def_name = "test_output_e2e";
    deploy_test_workflow(&server, &token, def_name).await;

    // Submit workflow
    let workflow_id = submit_workflow(&server, &token, def_name).await;

    // Add both activities to the workflow state (simulating orchestrator scheduling)
    add_pending_activity(&pool, workflow_id, "step1", vec![]).await;
    add_pending_activity(&pool, workflow_id, "step2", vec!["step1"]).await;

    // Complete first activity only
    complete_activity_with_output(&pool, workflow_id, "step1").await;

    // Can get output for completed activity
    let response1 = server
        .get(&format!(
            "/api/v1/workflows/{}/activities/step1/output",
            workflow_id
        ))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;
    assert_eq!(response1.status_code(), StatusCode::OK);

    // Cannot get output for incomplete activity (returns 400 Bad Request)
    let response2 = server
        .get(&format!(
            "/api/v1/workflows/{}/activities/step2/output",
            workflow_id
        ))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;
    assert_eq!(response2.status_code(), StatusCode::BAD_REQUEST);

    // Cannot get workflow output (workflow not completed)
    let response3 = server
        .get(&format!("/api/v1/workflows/{}/output", workflow_id))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;
    assert_eq!(response3.status_code(), StatusCode::BAD_REQUEST);

    // Complete the workflow
    complete_workflow_with_output(&pool, workflow_id).await;

    // Now can get workflow output
    let response4 = server
        .get(&format!("/api/v1/workflows/{}/output", workflow_id))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;
    assert_eq!(response4.status_code(), StatusCode::OK);

    let body: GetWorkflowOutputResponse = response4.json();
    assert_eq!(body.outputs.len(), 2);
    assert!(body.terminal_outputs.contains(&"step2".to_string()));
}

// ============================================================================
// File Upload Tests
// ============================================================================

#[sqlx::test(migrations = "../migrations")]
async fn test_upload_file_success(pool: PgPool) {
    let server = setup_test_server(pool).await;
    let token = get_test_token(&server).await;

    // Deploy and submit workflow
    let def_name = "test_file_upload";
    deploy_test_workflow(&server, &token, def_name).await;
    let workflow_id = submit_workflow(&server, &token, def_name).await;

    // Upload a file
    let file_content = b"Hello, World! This is a test file.";
    let response = server
        .post(&format!(
            "/api/v1/workflows/{}/activities/step1/files/test.txt",
            workflow_id
        ))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .add_header(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("text/plain"),
        )
        .bytes(file_content.to_vec().into())
        .await;

    assert_eq!(response.status_code(), StatusCode::CREATED);

    let body: UploadActivityFileResponse = response.json();
    assert_eq!(body.workflow_id, workflow_id);
    assert_eq!(body.activity_key, "step1");
    assert_eq!(body.filename, "test.txt");
    assert_eq!(body.size, file_content.len() as i64);
    assert_eq!(body.content_type, Some("text/plain".to_string()));
}

#[sqlx::test(migrations = "../migrations")]
async fn test_upload_and_download_file_roundtrip(pool: PgPool) {
    let server = setup_test_server(pool).await;
    let token = get_test_token(&server).await;

    // Deploy and submit workflow
    let def_name = "test_file_roundtrip";
    deploy_test_workflow(&server, &token, def_name).await;
    let workflow_id = submit_workflow(&server, &token, def_name).await;

    // Upload a JSONL file
    let file_content = br#"{"id":1,"text":"first passage"}
{"id":2,"text":"second passage"}
{"id":3,"text":"third passage"}"#;

    let upload_response = server
        .post(&format!(
            "/api/v1/workflows/{}/activities/step1/files/passages.jsonl",
            workflow_id
        ))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .add_header(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("application/x-ndjson"),
        )
        .bytes(file_content.to_vec().into())
        .await;

    assert_eq!(upload_response.status_code(), StatusCode::CREATED);

    let upload_body: UploadActivityFileResponse = upload_response.json();
    assert_eq!(upload_body.filename, "passages.jsonl");
    assert_eq!(
        upload_body.content_type,
        Some("application/x-ndjson".to_string())
    );

    // Download the file
    let download_response = server
        .get(&format!(
            "/api/v1/workflows/{}/activities/step1/files/passages.jsonl",
            workflow_id
        ))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;

    assert_eq!(download_response.status_code(), StatusCode::OK);
    assert_eq!(
        download_response
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap(),
        "application/x-ndjson"
    );
    assert_eq!(download_response.as_bytes().as_ref(), file_content);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_upload_file_requires_authentication(pool: PgPool) {
    let server = setup_test_server(pool).await;

    let workflow_id = Uuid::now_v7();

    let response = server
        .post(&format!(
            "/api/v1/workflows/{}/activities/step1/files/test.txt",
            workflow_id
        ))
        .bytes(b"test content".to_vec().into())
        .await;

    assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "../migrations")]
async fn test_upload_file_binary_content(pool: PgPool) {
    let server = setup_test_server(pool).await;
    let token = get_test_token(&server).await;

    // Deploy and submit workflow
    let def_name = "test_binary_upload";
    deploy_test_workflow(&server, &token, def_name).await;
    let workflow_id = submit_workflow(&server, &token, def_name).await;

    // Upload binary content (simulating PDF or image)
    let binary_content: Vec<u8> = (0..256).map(|i| i as u8).collect();

    let response = server
        .post(&format!(
            "/api/v1/workflows/{}/activities/step1/files/data.bin",
            workflow_id
        ))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .add_header(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("application/octet-stream"),
        )
        .bytes(binary_content.clone().into())
        .await;

    assert_eq!(response.status_code(), StatusCode::CREATED);

    let body: UploadActivityFileResponse = response.json();
    assert_eq!(body.size, binary_content.len() as i64);

    // Verify download returns same content
    let download_response = server
        .get(&format!(
            "/api/v1/workflows/{}/activities/step1/files/data.bin",
            workflow_id
        ))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;

    assert_eq!(download_response.status_code(), StatusCode::OK);
    assert_eq!(
        download_response.as_bytes().as_ref(),
        binary_content.as_slice()
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn test_upload_file_overwrite_existing(pool: PgPool) {
    let server = setup_test_server(pool).await;
    let token = get_test_token(&server).await;

    // Deploy and submit workflow
    let def_name = "test_file_overwrite";
    deploy_test_workflow(&server, &token, def_name).await;
    let workflow_id = submit_workflow(&server, &token, def_name).await;

    // Upload initial file
    let initial_content = b"initial content";
    let response1 = server
        .post(&format!(
            "/api/v1/workflows/{}/activities/step1/files/test.txt",
            workflow_id
        ))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .bytes(initial_content.to_vec().into())
        .await;

    assert_eq!(response1.status_code(), StatusCode::CREATED);

    // Upload new content to same filename (overwrite)
    let new_content = b"updated content with more data";
    let response2 = server
        .post(&format!(
            "/api/v1/workflows/{}/activities/step1/files/test.txt",
            workflow_id
        ))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .bytes(new_content.to_vec().into())
        .await;

    assert_eq!(response2.status_code(), StatusCode::CREATED);

    let body: UploadActivityFileResponse = response2.json();
    assert_eq!(body.size, new_content.len() as i64);

    // Verify download returns new content
    let download_response = server
        .get(&format!(
            "/api/v1/workflows/{}/activities/step1/files/test.txt",
            workflow_id
        ))
        .add_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        )
        .await;

    assert_eq!(download_response.status_code(), StatusCode::OK);
    assert_eq!(download_response.as_bytes().as_ref(), new_content);
}
