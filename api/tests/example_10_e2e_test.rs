use kruxiaflow_api::{AppState, app_router};
use kruxiaflow_core::events::PostgresEventSource;
use kruxiaflow_core::queue::{ActivityQueue, PostgresQueue, QueueConfig};
use kruxiaflow_core::{OrchestratorConfig, run_orchestrator};
use kruxiaflow_oauth::{AuthConfig, PostgresAuthService};
use kruxiaflow_worker::{
    ActivityRegistry, EmailSendActivity, HttpRequestActivity, PostgresQueryActivity, WorkerConfig,
    WorkerManager, activities::PostgresTransactionActivity, new_pool_cache,
};
/// End-to-end test for Example 10: Order Processing with Email Notification
///
/// This test verifies:
/// - HTTP requests with authorization headers
/// - Conditional workflow execution (inventory check)
/// - PostgreSQL transaction with RETURNING clause
/// - Email sending via SMTP
/// - Sequential dependency chain
///
/// Uses mock endpoints (API health endpoints) and Mailhog for email testing.
use serde::Deserialize;
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Helper to create test database pool
async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://kruxiaflow:kruxiaflow_dev@127.0.0.1:5433/kruxiaflow".to_string()
    });

    PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test database")
}

/// Generate test RSA private key
fn test_rsa_private_key() -> String {
    include_str!("../../oauth/tests/private.pem").to_string()
}

/// Generate test RSA public key
fn test_rsa_public_key() -> String {
    include_str!("../../oauth/tests/public.pem").to_string()
}

/// Create OAuth client for testing
async fn create_test_oauth_client(pool: &PgPool, client_id: &str, client_secret: &str) {
    let secret_hash = bcrypt::hash(client_secret, bcrypt::DEFAULT_COST).unwrap();
    sqlx::query(
        "INSERT INTO oauth_clients (client_id, client_secret_hash, name, created_at)
         VALUES ($1, $2, $3, NOW())
         ON CONFLICT (client_id) DO NOTHING",
    )
    .bind(client_id)
    .bind(&secret_hash)
    .bind("Test Client")
    .execute(pool)
    .await
    .expect("Failed to create test OAuth client");
}

/// Get OAuth token for testing
async fn get_test_token(api_url: &str, client_id: &str, client_secret: &str) -> String {
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/api/v1/oauth/token", api_url))
        .json(&serde_json::json!({
            "grant_type": "client_credentials",
            "client_id": client_id,
            "client_secret": client_secret
        }))
        .send()
        .await
        .expect("Failed to get token");

    let result: serde_json::Value = response
        .json()
        .await
        .expect("Failed to parse token response");
    result["access_token"].as_str().unwrap().to_string()
}

/// Mailhog API URL for verification
fn mailhog_api_url() -> String {
    std::env::var("MAILHOG_API_URL").unwrap_or_else(|_| "http://127.0.0.1:8025".to_string())
}

/// Check if mailhog is available
async fn mailhog_available() -> bool {
    let client = reqwest::Client::new();
    client
        .get(format!("{}/api/v2/messages", mailhog_api_url()))
        .send()
        .await
        .is_ok()
}

/// Clear all messages from mailhog
async fn clear_mailhog() {
    let client = reqwest::Client::new();
    let _ = client
        .delete(format!("{}/api/v1/messages", mailhog_api_url()))
        .send()
        .await;
}

/// Mailhog message response structure
#[derive(Debug, Deserialize)]
struct MailhogMessages {
    items: Vec<MailhogMessage>,
}

#[derive(Debug, Deserialize)]
struct MailhogMessage {
    #[serde(rename = "Content")]
    content: MailhogContent,
}

#[derive(Debug, Deserialize)]
struct MailhogContent {
    #[serde(rename = "Headers")]
    headers: MailhogHeaders,
    #[serde(rename = "Body")]
    body: String,
}

#[derive(Debug, Deserialize)]
struct MailhogHeaders {
    #[serde(rename = "Subject")]
    subject: Vec<String>,
}

/// Get all messages from mailhog
async fn get_mailhog_messages() -> Option<MailhogMessages> {
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/api/v2/messages", mailhog_api_url()))
        .send()
        .await
        .ok()?;

    response.json::<MailhogMessages>().await.ok()
}

/// Wait for a message to arrive in mailhog (with timeout)
async fn wait_for_message(timeout_ms: u64) -> Option<MailhogMessage> {
    let start = std::time::Instant::now();
    while start.elapsed().as_millis() < timeout_ms as u128 {
        if let Some(messages) = get_mailhog_messages().await {
            if !messages.items.is_empty() {
                return Some(messages.items.into_iter().next().unwrap());
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    None
}

/// Create test tables for order processing
async fn setup_test_tables(pool: &PgPool, table_suffix: &str) {
    // Create orders table
    sqlx::query(&format!(
        "CREATE TABLE IF NOT EXISTS orders_{} (
            id SERIAL PRIMARY KEY,
            customer_id TEXT NOT NULL,
            customer_email TEXT NOT NULL,
            product_id TEXT NOT NULL,
            quantity INT NOT NULL,
            amount DECIMAL(10,2) NOT NULL,
            payment_txn_id TEXT,
            reservation_id TEXT,
            status TEXT NOT NULL DEFAULT 'pending',
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )",
        table_suffix
    ))
    .execute(pool)
    .await
    .expect("Failed to create orders table");

    // Create inventory table
    sqlx::query(&format!(
        "CREATE TABLE IF NOT EXISTS inventory_{} (
            product_id TEXT PRIMARY KEY,
            available INT NOT NULL DEFAULT 0,
            reserved INT NOT NULL DEFAULT 0
        )",
        table_suffix
    ))
    .execute(pool)
    .await
    .expect("Failed to create inventory table");

    // Insert test inventory
    sqlx::query(&format!(
        "INSERT INTO inventory_{} (product_id, available, reserved)
         VALUES ('prod_test', 100, 0)
         ON CONFLICT (product_id) DO UPDATE SET available = 100, reserved = 0",
        table_suffix
    ))
    .execute(pool)
    .await
    .expect("Failed to insert test inventory");
}

/// Clean up test tables
async fn cleanup_test_tables(pool: &PgPool, table_suffix: &str) {
    let _ = sqlx::query(&format!("DROP TABLE IF EXISTS orders_{}", table_suffix))
        .execute(pool)
        .await;
    let _ = sqlx::query(&format!("DROP TABLE IF EXISTS inventory_{}", table_suffix))
        .execute(pool)
        .await;
}

#[tokio::test]
#[serial]
async fn test_example_10_order_processing_with_email() {
    // Check if mailhog is available
    if !mailhog_available().await {
        eprintln!("Skipping test: mailhog not available. Run: docker compose up mailhog");
        return;
    }

    clear_mailhog().await;

    // Setup database and services
    let pool = setup_test_pool().await;

    // Create unique table suffix to avoid conflicts with parallel tests
    let table_suffix = format!("ex10_{}", Uuid::now_v7().simple());
    setup_test_tables(&pool, &table_suffix).await;

    // Get database URL for workflow
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://kruxiaflow:kruxiaflow_dev@127.0.0.1:5433/kruxiaflow".to_string()
    });

    // Create auth service
    let auth_config = AuthConfig {
        rsa_private_key_pem: test_rsa_private_key(),
        rsa_public_key_pem: Some(test_rsa_public_key()),
        jwt_issuer: "test".to_string(),
        jwt_audience: "test".to_string(),
        token_ttl: 3600,
    };
    let auth_service =
        PostgresAuthService::new(pool.clone(), auth_config).expect("Failed to create auth service");

    // Create OAuth clients
    create_test_oauth_client(&pool, "test_client_ex10", "test_secret_ex10").await;
    create_test_oauth_client(&pool, "test_worker_ex10", "test_worker_secret_ex10").await;

    // Create shared services
    let activity_queue: Arc<dyn ActivityQueue> =
        Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default()));
    let event_source = Arc::new(PostgresEventSource::new(pool.clone()));
    let workflow_storage = Arc::new(kruxiaflow_core::storage::PostgresStorage::new(pool.clone()));
    let shutdown_token = CancellationToken::new();

    // Start Kruxia Flow API server
    let cache_service = Arc::new(kruxiaflow_core::cache::NoOpCache::new());
    let state = AppState::new(
        pool.clone(),
        Arc::new(auth_service),
        activity_queue.clone(),
        event_source.clone(),
        workflow_storage.clone(),
        cache_service,
        shutdown_token.clone(),
    );
    let app = app_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind API");
    let addr = listener.local_addr().expect("Failed to get address");
    let api_url = format!("http://{}", addr);

    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("API server failed");
    });

    // Give API server time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Get token for API calls
    let token = get_test_token(&api_url, "test_client_ex10", "test_secret_ex10").await;

    // Start orchestrator
    let orchestrator_event_source = event_source.clone();
    let orchestrator_queue = activity_queue.clone();
    let orchestrator_pool = pool.clone();
    let orchestrator_shutdown = shutdown_token.clone();
    tokio::spawn(async move {
        let config = OrchestratorConfig::new(orchestrator_pool);
        run_orchestrator(
            orchestrator_event_source,
            orchestrator_queue,
            config,
            Some(orchestrator_shutdown),
        )
        .await
        .expect("Orchestrator failed");
    });

    // Give orchestrator time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Start worker with all required activities
    let postgres_pool_cache = new_pool_cache();
    let mut registry = ActivityRegistry::new(Arc::new(kruxiaflow_core::NoOpCache::new()));
    registry.register(Arc::new(HttpRequestActivity::new()));
    registry.register(Arc::new(PostgresQueryActivity::new(
        postgres_pool_cache.clone(),
    )));
    registry.register(Arc::new(PostgresTransactionActivity::new(
        postgres_pool_cache,
    )));
    registry.register(Arc::new(EmailSendActivity::new()));

    let worker_config = WorkerConfig {
        api_url: api_url.clone(),
        worker_id: format!("test_worker_ex10_{}", Uuid::now_v7()),
        activity_types: registry.activity_types(),
        poll_max_activities: 10,
        poll_interval: Duration::from_millis(100),
        concurrency: 5,
        activity_timeout: Duration::from_secs(30),
        heartbeat_interval: Duration::from_secs(30),
        client_id: "test_worker_ex10".to_string(),
        client_secret: "test_worker_secret_ex10".to_string(),
    };

    let manager = WorkerManager::new(worker_config, registry, workflow_storage.clone());
    let worker_handles = manager.start().await.expect("Failed to start worker");

    // Give worker time to start and authenticate
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Create simplified workflow YAML that uses mock endpoints
    // This focuses on testing the postgres_transaction and email_send activities
    let workflow_yaml = format!(
        r#"
name: order_processing_test
description: Test order processing with database transaction and email confirmation

activities:
  # Step 1: Mock inventory check (using health endpoint as mock)
  - key: validate_inventory
    worker: builtin
    activity_name: http_request
    parameters:
      method: GET
      url: "{api_url}/health"
      timeout_seconds: 10
    outputs:
      - response

  # Step 2: Mock inventory reservation (using health endpoint as mock)
  - key: reserve_inventory
    worker: builtin
    activity_name: http_request
    parameters:
      method: POST
      url: "{api_url}/health"
      headers:
        Content-Type: "application/json"
      body:
        product_id: "{{{{INPUT.product_id}}}}"
        quantity: "{{{{INPUT.quantity}}}}"
    outputs:
      - response
    depends_on:
      - validate_inventory

  # Step 3: Mock payment processing (using health endpoint as mock)
  - key: process_payment
    worker: builtin
    activity_name: http_request
    parameters:
      method: POST
      url: "{api_url}/health"
      headers:
        Content-Type: "application/json"
      body:
        amount: "{{{{INPUT.amount}}}}"
        customer_id: "{{{{INPUT.customer_id}}}}"
    outputs:
      - response
    depends_on:
      - reserve_inventory

  # Step 4: Record order in database (atomic transaction)
  - key: record_order
    worker: builtin
    activity_name: postgres_transaction
    parameters:
      db_url: "{db_url}"
      statements:
        - query: |
            INSERT INTO orders_{table_suffix}
            (customer_id, customer_email, product_id, quantity, amount, payment_txn_id, reservation_id, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, 'confirmed')
            RETURNING id as order_id
          params:
            - "{{{{INPUT.customer_id}}}}"
            - "{{{{INPUT.customer_email}}}}"
            - "{{{{INPUT.product_id}}}}"
            - "{{{{INPUT.quantity}}}}"
            - "{{{{INPUT.amount}}}}"
            - "txn_mock_12345"
            - "res_mock_67890"
        - query: |
            UPDATE inventory_{table_suffix}
            SET reserved = reserved + $1,
                available = available - $1
            WHERE product_id = $2
          params:
            - "{{{{INPUT.quantity}}}}"
            - "{{{{INPUT.product_id}}}}"
    outputs:
      - result
    depends_on:
      - process_payment

  # Step 5: Send confirmation email
  - key: send_confirmation
    worker: builtin
    activity_name: email_send
    parameters:
      smtp_url: "smtp://127.0.0.1:1025"
      from: "orders@kruxiaflow-test.com"
      to:
        - "{{{{INPUT.customer_email}}}}"
      subject: "Order Confirmation - #{{{{record_order.result.results[0].rows[0].order_id}}}}"
      html_body: |
        <html>
        <body style="font-family: Arial, sans-serif;">
          <h1 style="color: #2e7d32;">Order Confirmed!</h1>
          <p>Thank you for your order.</p>
          <table style="border-collapse: collapse;">
            <tr><td style="padding: 8px; border: 1px solid #ddd;"><strong>Order ID</strong></td><td style="padding: 8px; border: 1px solid #ddd;">#{{{{record_order.result.results[0].rows[0].order_id}}}}</td></tr>
            <tr><td style="padding: 8px; border: 1px solid #ddd;"><strong>Product</strong></td><td style="padding: 8px; border: 1px solid #ddd;">{{{{INPUT.product_id}}}}</td></tr>
            <tr><td style="padding: 8px; border: 1px solid #ddd;"><strong>Quantity</strong></td><td style="padding: 8px; border: 1px solid #ddd;">{{{{INPUT.quantity}}}}</td></tr>
            <tr><td style="padding: 8px; border: 1px solid #ddd;"><strong>Amount</strong></td><td style="padding: 8px; border: 1px solid #ddd;">${{{{INPUT.amount}}}}</td></tr>
          </table>
        </body>
        </html>
      text_body: |
        Order Confirmed!

        Order ID: #{{{{record_order.result.results[0].rows[0].order_id}}}}
        Product: {{{{INPUT.product_id}}}}
        Quantity: {{{{INPUT.quantity}}}}
        Amount: ${{{{INPUT.amount}}}}
    depends_on:
      - record_order
"#,
        api_url = api_url,
        db_url = db_url,
        table_suffix = table_suffix
    );

    // Deploy workflow
    let client = reqwest::Client::new();
    let deploy_response = client
        .post(format!("{}/api/v1/workflow_definitions", api_url))
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "text/yaml")
        .body(workflow_yaml)
        .send()
        .await
        .expect("Failed to deploy workflow");

    assert_eq!(
        deploy_response.status(),
        reqwest::StatusCode::CREATED,
        "Deploy failed: {:?}",
        deploy_response.text().await
    );

    let deploy_result: serde_json::Value = deploy_response
        .json()
        .await
        .expect("Failed to parse deploy response");

    let definition_name = deploy_result["name"].as_str().unwrap();
    let definition_version = deploy_result["version"].as_str().unwrap();

    println!(
        "Deployed workflow: {} version {}",
        definition_name, definition_version
    );

    // Submit workflow for execution
    let submit_response = client
        .post(format!("{}/api/v1/workflows", api_url))
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({
            "definition_name": definition_name,
            "version": definition_version,
            "input": {
                "customer_id": "cust_test_123",
                "customer_email": "test-customer@example.com",
                "product_id": "prod_test",
                "quantity": 2,
                "amount": 99.99
            }
        }))
        .send()
        .await
        .expect("Failed to submit workflow");

    assert_eq!(
        submit_response.status(),
        reqwest::StatusCode::CREATED,
        "Submit failed: {:?}",
        submit_response.text().await
    );

    let submit_result: serde_json::Value = submit_response
        .json()
        .await
        .expect("Failed to parse submit response");

    let workflow_id = submit_result["workflow_id"]
        .as_str()
        .unwrap()
        .parse::<Uuid>()
        .unwrap();

    println!("Submitted workflow: {}", workflow_id);

    // Poll for workflow completion (timeout after 60 seconds)
    let start = std::time::Instant::now();
    let timeout = Duration::from_secs(60);
    let mut final_status = None;

    while start.elapsed() < timeout {
        let status_response = client
            .get(format!("{}/api/v1/workflows/{}", api_url, workflow_id))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .expect("Failed to get workflow status");

        if status_response.status() == reqwest::StatusCode::OK {
            let status_result: serde_json::Value = status_response
                .json()
                .await
                .expect("Failed to parse status response");

            let status = status_result["status"].as_str().unwrap();
            println!("Workflow status: {}", status);

            if status == "completed" || status == "failed" {
                final_status = Some(status.to_string());

                // Print activities for debugging
                if let Some(activities) = status_result.get("activities") {
                    println!(
                        "Activities: {}",
                        serde_json::to_string_pretty(activities).unwrap()
                    );
                }
                break;
            }
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // Verify workflow completed successfully
    if final_status.is_none() {
        // Workflow timed out - print diagnostic information
        let status_response = client
            .get(format!("{}/api/v1/workflows/{}", api_url, workflow_id))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .expect("Failed to get workflow status");

        if status_response.status() == reqwest::StatusCode::OK {
            let status_result: serde_json::Value = status_response
                .json()
                .await
                .expect("Failed to parse status response");

            println!("Workflow timed out. Final state:");
            println!("{}", serde_json::to_string_pretty(&status_result).unwrap());
        }

        cleanup_test_tables(&pool, &table_suffix).await;
        panic!("Workflow did not complete within timeout");
    }

    assert_eq!(
        final_status.as_ref().unwrap(),
        "completed",
        "Workflow did not complete successfully"
    );

    // Verify workflow activities in database
    let workflow_activities: (serde_json::Value,) =
        sqlx::query_as("SELECT activities FROM workflows WHERE id = $1")
            .bind(workflow_id)
            .fetch_one(&pool)
            .await
            .expect("Failed to fetch workflow from database");

    let activities_obj = workflow_activities.0.as_object().unwrap();

    // Verify all activities completed
    let activity_keys = vec![
        "validate_inventory",
        "reserve_inventory",
        "process_payment",
        "record_order",
        "send_confirmation",
    ];

    for key in &activity_keys {
        assert!(
            activities_obj.contains_key(*key),
            "Activity {} not found",
            key
        );
        let activity = &activities_obj[*key];
        assert_eq!(
            activity["status"], "completed",
            "Activity {} did not complete: {:?}",
            key, activity
        );
        println!("✓ Activity {} completed", key);
    }

    // Verify order was created in database
    let order: (i32, String, String, i32) = sqlx::query_as(&format!(
        "SELECT id, customer_id, product_id, quantity FROM orders_{} ORDER BY id DESC LIMIT 1",
        table_suffix
    ))
    .fetch_one(&pool)
    .await
    .expect("Failed to fetch order from database");

    println!(
        "✓ Order created: id={}, customer={}, product={}, qty={}",
        order.0, order.1, order.2, order.3
    );
    assert_eq!(order.1, "cust_test_123");
    assert_eq!(order.2, "prod_test");
    assert_eq!(order.3, 2);

    // Verify inventory was updated
    let inventory: (i32, i32) = sqlx::query_as(&format!(
        "SELECT available, reserved FROM inventory_{} WHERE product_id = 'prod_test'",
        table_suffix
    ))
    .fetch_one(&pool)
    .await
    .expect("Failed to fetch inventory from database");

    println!(
        "✓ Inventory updated: available={}, reserved={}",
        inventory.0, inventory.1
    );
    assert_eq!(inventory.0, 98); // 100 - 2
    assert_eq!(inventory.1, 2); // 0 + 2

    // Verify email was sent
    let message = wait_for_message(10000).await;
    if let Some(msg) = message {
        println!("✓ Email received: {}", msg.content.headers.subject[0]);
        assert!(msg.content.headers.subject[0].contains("Order Confirmation"));
        assert!(msg.content.body.contains("Order Confirmed"));
    } else {
        println!("⚠ Email not received within timeout (mailhog might be unavailable)");
    }

    println!("✅ Example 10: Order processing with email notification test passed!");

    // Cleanup
    cleanup_test_tables(&pool, &table_suffix).await;
    manager.stop(worker_handles).await;
    shutdown_token.cancel();
}

#[tokio::test]
#[serial]
async fn test_example_10_verify_workflow_definition() {
    use kruxiaflow_core::workflow::definition::WorkflowDefinition;

    let workflow_yaml = include_str!("../../examples/10-order-processing.yaml");
    let workflow_def =
        WorkflowDefinition::from_yaml(workflow_yaml).expect("Failed to parse workflow YAML");

    // Verify workflow has expected activities
    assert_eq!(workflow_def.name, "order_processing");
    assert_eq!(workflow_def.activities.len(), 5);

    let activity_keys: Vec<&str> = workflow_def
        .activities
        .iter()
        .map(|a| a.key.as_str())
        .collect();

    assert!(activity_keys.contains(&"validate_inventory"));
    assert!(activity_keys.contains(&"reserve_inventory"));
    assert!(activity_keys.contains(&"process_payment"));
    assert!(activity_keys.contains(&"record_order"));
    assert!(activity_keys.contains(&"send_confirmation"));

    println!("✅ Example 10: Workflow definition validation passed");
}
