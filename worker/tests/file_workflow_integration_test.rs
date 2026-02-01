/// End-to-end file workflow integration test
///
/// This test validates the complete file workflow:
/// 1. HTTP request downloads file
/// 2. Worker uploads file to storage
/// 3. File can be retrieved from storage
use anyhow::Result;
use kruxiaflow_api::{AppState, app_router};
use kruxiaflow_core::events::{EventSource, PostgresEventSource};
use kruxiaflow_core::queue::{ActivityQueue, PostgresQueue, QueueConfig};
use kruxiaflow_core::storage::{PostgresStorage, WorkflowStorage};
use kruxiaflow_core::{
    OrchestratorConfig, PostgresSubscriptionService, SubscriptionService, run_orchestrator,
};
use kruxiaflow_oauth::{AuthenticationService, PostgresAuthService};
use kruxiaflow_worker::{WorkerConfig, WorkerManager, register_std_activities};
use serde_json::json;
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Helper to create test database pool
async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://kruxiaflow:kruxiaflow_dev@127.0.0.1:5432/kruxiaflow".to_string()
    });

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

/// Generate test RSA private key
fn test_rsa_private_key() -> String {
    include_str!("../../oauth/tests/private.pem").to_string()
}

/// Generate test RSA public key
fn test_rsa_public_key() -> String {
    include_str!("../../oauth/tests/public.pem").to_string()
}

/// Helper to setup test services
async fn setup_test_services(
    pool: PgPool,
) -> (
    Arc<dyn AuthenticationService>,
    Arc<dyn ActivityQueue>,
    Arc<dyn EventSource>,
    Arc<dyn WorkflowStorage>,
    CancellationToken,
) {
    use kruxiaflow_oauth::AuthConfig;
    let auth_config = AuthConfig {
        rsa_private_key_pem: test_rsa_private_key(),
        rsa_public_key_pem: Some(test_rsa_public_key()),
        jwt_issuer: "test".to_string(),
        jwt_audience: "test".to_string(),
        token_ttl: 3600,
    };

    let auth_service = PostgresAuthService::new(pool.clone(), auth_config)
        .expect("Failed to create test auth service");

    let auth_service: Arc<dyn AuthenticationService> = Arc::new(auth_service);

    let queue_config = QueueConfig::default();
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), queue_config));

    let event_source: Arc<dyn EventSource> = Arc::new(PostgresEventSource::new(pool.clone()));

    let workflow_storage: Arc<dyn WorkflowStorage> = Arc::new(PostgresStorage::new(pool.clone()));

    let shutdown_token = CancellationToken::new();

    (
        auth_service,
        activity_queue,
        event_source,
        workflow_storage,
        shutdown_token,
    )
}

/// Helper to create test client credentials
async fn create_test_client(pool: &PgPool) -> (String, String) {
    let client_id = format!("test_client_{}", Uuid::now_v7());
    let client_secret = "test_secret";

    // Hash the secret
    let hashed = bcrypt::hash(client_secret, bcrypt::DEFAULT_COST).expect("Failed to hash secret");

    // Insert client
    sqlx::query!(
        "INSERT INTO oauth_clients (client_id, client_secret_hash, name)
         VALUES ($1, $2, $3)
         ON CONFLICT (client_id) DO NOTHING",
        client_id,
        hashed,
        "Test Client"
    )
    .execute(pool)
    .await
    .expect("Failed to create test client");

    (client_id, client_secret.to_string())
}

/// Helper to cleanup test data
async fn cleanup_test_data(pool: &PgPool, workflow_id: Uuid) {
    // Delete workflow files
    let oids: Vec<i32> =
        sqlx::query_scalar("SELECT oid::int4 FROM workflow_files WHERE workflow_id = $1")
            .bind(workflow_id)
            .fetch_all(pool)
            .await
            .expect("Failed to fetch OIDs");

    for oid in oids {
        let _ = sqlx::query("SELECT lo_unlink($1)")
            .bind(oid)
            .execute(pool)
            .await;
    }

    sqlx::query("DELETE FROM workflow_files WHERE workflow_id = $1")
        .bind(workflow_id)
        .execute(pool)
        .await
        .expect("Failed to delete workflow files");

    // Delete activity queue entries
    sqlx::query("DELETE FROM activity_queue WHERE workflow_id = $1")
        .bind(workflow_id)
        .execute(pool)
        .await
        .expect("Failed to delete activity queue");

    // Delete workflow events
    sqlx::query("DELETE FROM workflow_events WHERE workflow_id = $1")
        .bind(workflow_id)
        .execute(pool)
        .await
        .expect("Failed to delete workflow events");
}

#[tokio::test]
#[serial]
async fn test_end_to_end_file_workflow() -> Result<()> {
    // Setup
    let pool = setup_test_pool().await;
    let (auth_service, activity_queue, event_source, workflow_storage, shutdown_token) =
        setup_test_services(pool.clone()).await;

    let (client_id, client_secret) = create_test_client(&pool).await;

    // Create API state
    let cache_service = Arc::new(kruxiaflow_core::cache::NoOpCache::new());
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));
    let state = AppState::new(
        pool.clone(),
        auth_service.clone(),
        activity_queue.clone(),
        event_source.clone(),
        workflow_storage.clone(),
        cache_service,
        subscription_service.clone(),
        shutdown_token.clone(),
    );

    // Create and start real server on random port
    let app = app_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind to port");
    let addr = listener.local_addr().expect("Failed to get local address");
    let server_url = format!("http://{}", addr);

    // Spawn the server in the background
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("Server failed to start");
    });

    // Give the server a moment to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Start orchestrator
    let orchestrator_config = OrchestratorConfig::new(pool.clone());
    let subscription_service: Arc<dyn SubscriptionService> =
        Arc::new(PostgresSubscriptionService::new(pool.clone()));
    let orchestrator_handle = tokio::spawn({
        let event_source = event_source.clone();
        let activity_queue = activity_queue.clone();
        let shutdown_token = shutdown_token.clone();
        let subscription_service = subscription_service.clone();
        async move {
            run_orchestrator(
                event_source,
                activity_queue,
                subscription_service,
                orchestrator_config,
                Some(shutdown_token),
            )
            .await
        }
    });

    // Wait for orchestrator to start
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Get OAuth token
    let client = reqwest::Client::new();
    let token_response = client
        .post(format!("{}/api/v1/oauth/token", server_url))
        .form(&[
            ("grant_type", "client_credentials"),
            ("client_id", &client_id),
            ("client_secret", &client_secret),
            ("scope", "workflow:write activity:execute"),
        ])
        .send()
        .await?;

    assert_eq!(token_response.status().as_u16(), 200);
    let token_body: serde_json::Value = token_response.json().await?;
    let access_token = token_body["access_token"].as_str().unwrap();

    // Start worker with storage
    let cache_service = Arc::new(kruxiaflow_core::cache::NoOpCache::new());
    let registry = register_std_activities(cache_service);
    #[allow(deprecated)]
    let worker_config = WorkerConfig {
        api_url: server_url.clone(),
        worker_id: "test_worker".to_string(),
        worker: "std".to_string(),
        poll_interval: Duration::from_millis(100),
        poll_max_activities: 5,
        max_concurrent_activities: 16,
        concurrency: 1,
        activity_timeout: Duration::from_secs(30),
        heartbeat_interval: Duration::from_secs(10),
        client_id: client_id.clone(),
        client_secret: client_secret.clone(),
    };

    let manager = WorkerManager::new(worker_config, registry, workflow_storage.clone());
    let worker_handles = manager.start().await?;

    // Wait for worker to start
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Define a workflow that downloads a file from the API's OpenAPI spec endpoint
    // This ensures we don't depend on external services (per CLAUDE.md guidelines)
    let openapi_url = format!("{}/api/v1/openapi.json", server_url);

    // First, deploy the workflow definition
    let workflow_def = json!({
        "name": "test_file_download",
        "activities": [
            {
                "key": "fetch_file",
                "worker": "std",
                "activity_name": "http_request",
                "parameters": {
                    "method": "GET",
                    "url": openapi_url,
                    "download_to_file": "openapi.json"
                },
                "outputs": [
                    {
                        "name": "response",
                        "type": "value"
                    },
                    {
                        "name": "openapi.json",
                        "type": "file"
                    }
                ]
            }
        ]
    });

    let definition_response = client
        .post(format!("{}/api/v1/workflow_definitions", server_url))
        .header("Authorization", format!("Bearer {}", access_token))
        .json(&workflow_def)
        .send()
        .await?;

    assert_eq!(
        definition_response.status().as_u16(),
        201,
        "Failed to deploy workflow definition"
    );

    // Now submit a workflow instance
    let workflow_response = client
        .post(format!("{}/api/v1/workflows", server_url))
        .header("Authorization", format!("Bearer {}", access_token))
        .json(&json!({
            "definition_name": "test_file_download",
            "input": {}
        }))
        .send()
        .await?;

    assert_eq!(
        workflow_response.status().as_u16(),
        201,
        "Failed to submit workflow"
    );
    let workflow_body: serde_json::Value = workflow_response.json().await?;
    let workflow_id = Uuid::parse_str(workflow_body["workflow_id"].as_str().unwrap())?;

    // Wait for workflow to complete
    let mut attempts = 0;
    let max_attempts = 30; // 15 seconds max
    let mut workflow_completed = false;

    while attempts < max_attempts {
        tokio::time::sleep(Duration::from_millis(500)).await;
        attempts += 1;

        // Check workflow status via events
        let events: Vec<(String,)> = sqlx::query_as(
            "SELECT event_type::text FROM workflow_events WHERE workflow_id = $1 ORDER BY timestamp"
        )
        .bind(workflow_id)
        .fetch_all(&pool)
        .await?;

        for (event_type,) in &events {
            if event_type == "WorkflowCompleted" {
                workflow_completed = true;
                break;
            }
        }

        if workflow_completed {
            break;
        }
    }

    assert!(workflow_completed, "Workflow did not complete in time");

    // Verify file was uploaded to storage
    let files = workflow_storage
        .list_files(workflow_id, "fetch_file")
        .await?;

    assert_eq!(files.len(), 1, "Expected 1 file to be uploaded");
    assert_eq!(files[0].filename, "openapi.json");
    assert!(files[0].size > 0, "File should have content");

    tracing::info!("File uploaded: {:?}", files[0]);

    // Download file from storage and verify content
    let mut download_stream = workflow_storage
        .download_file(workflow_id, "fetch_file", "openapi.json")
        .await?;

    let mut downloaded_content = Vec::new();
    use futures::StreamExt;
    while let Some(chunk_result) = download_stream.next().await {
        let chunk = chunk_result?;
        downloaded_content.extend_from_slice(&chunk);
    }

    // Verify it's valid JSON (OpenAPI spec)
    let json_content: serde_json::Value = serde_json::from_slice(&downloaded_content)?;
    tracing::info!(
        "Downloaded file content (first 100 bytes): {:?}",
        String::from_utf8_lossy(
            &downloaded_content[..std::cmp::min(100, downloaded_content.len())]
        )
    );

    // OpenAPI spec should have "openapi" and "info" fields
    assert!(
        json_content.is_object(),
        "Downloaded content should be JSON object"
    );
    assert!(
        json_content.get("openapi").is_some(),
        "OpenAPI spec should have 'openapi' field"
    );
    assert!(
        json_content.get("info").is_some(),
        "OpenAPI spec should have 'info' field"
    );

    // Cleanup
    for handle in worker_handles {
        handle.abort();
    }
    server_handle.abort();
    orchestrator_handle.abort();

    cleanup_test_data(&pool, workflow_id).await;

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_file_workflow_with_multiple_outputs() -> Result<()> {
    // This test verifies that an activity can produce both value and file outputs
    let pool = setup_test_pool().await;
    let workflow_storage: Arc<dyn WorkflowStorage> = Arc::new(PostgresStorage::new(pool.clone()));

    let workflow_id = Uuid::now_v7();
    let activity_key = "test_activity";

    // Simulate activity creating a file
    use kruxiaflow_core::workflow::{ActivityOutputDefinition, OutputType};

    let output_definitions = vec![
        ActivityOutputDefinition {
            name: "status".to_string(),
            output_type: OutputType::Value,
        },
        ActivityOutputDefinition {
            name: "document".to_string(),
            output_type: OutputType::File,
        },
    ];

    // Create FileExecutor
    use kruxiaflow_worker::file_executor::FileExecutor;
    let executor = FileExecutor::new(
        workflow_id,
        activity_key.to_string(),
        workflow_storage.clone(),
    )
    .await?;

    // Simulate activity writing a file
    let file_path = executor.output_file_path("document");
    tokio::fs::write(&file_path, b"Test file content").await?;

    // Process outputs
    let activity_outputs = json!({
        "status": "success"
    });

    let outputs = executor
        .process_file_outputs(&output_definitions, activity_outputs)
        .await?;

    // Verify we have both outputs
    assert_eq!(outputs.len(), 2);

    let status_output = outputs.iter().find(|o| o.name == "status").unwrap();
    assert_eq!(status_output.output_type, OutputType::Value);
    assert_eq!(status_output.value, json!("success"));

    let file_output = outputs.iter().find(|o| o.name == "document").unwrap();
    assert_eq!(file_output.output_type, OutputType::File);
    let file_ref = file_output.value.as_str().unwrap();
    assert!(file_ref.contains(&workflow_id.to_string()));
    assert!(file_ref.contains(activity_key));
    assert!(file_ref.contains("document"));

    // Verify file was uploaded to storage
    let metadata = workflow_storage
        .get_file_metadata(workflow_id, activity_key, "document")
        .await?;
    assert_eq!(metadata.size, 17); // "Test file content" length

    // Cleanup
    executor.cleanup().await?;
    workflow_storage.delete_workflow_files(workflow_id).await?;

    Ok(())
}
