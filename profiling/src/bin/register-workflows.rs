/// Utility to register benchmark workflow definitions via API
///
/// Usage:
///   cargo run --package kruxiaflow-profiling --bin register-workflows
///
/// Requires environment variables:
///   - KRUXIAFLOW_BASE_URL: API base URL (default: http://localhost:8080)
///   - KRUXIAFLOW_CLIENT_ID: OAuth client ID
///   - KRUXIAFLOW_CLIENT_SECRET: OAuth client secret
use reqwest::Client;
use std::env;
use kruxiaflow_profiling::{create_parallel_workflow, create_sequential_workflow};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base_url =
        env::var("KRUXIAFLOW_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());

    let client_id = env::var("KRUXIAFLOW_CLIENT_ID").expect("KRUXIAFLOW_CLIENT_ID must be set");

    let client_secret =
        env::var("KRUXIAFLOW_CLIENT_SECRET").expect("KRUXIAFLOW_CLIENT_SECRET must be set");

    println!("Registering Benchmark Workflow Definitions");
    println!("=========================================");
    println!("API URL: {}", base_url);
    println!();

    let client = Client::new();

    // Get OAuth token
    println!("Authenticating...");
    let token_response = client
        .post(format!("{}/api/v1/oauth/token", base_url))
        .form(&[
            ("grant_type", "client_credentials"),
            ("client_id", &client_id),
            ("client_secret", &client_secret),
        ])
        .send()
        .await?;

    if !token_response.status().is_success() {
        let error_text = token_response.text().await?;
        return Err(format!("Failed to get OAuth token: {}", error_text).into());
    }

    let token_data: serde_json::Value = token_response.json().await?;
    let access_token = token_data["access_token"]
        .as_str()
        .ok_or("No access_token in response")?;

    println!("✓ Authenticated");
    println!();

    // Register workflow definitions
    let workflows = vec![
        ("sequential_bench_3", create_sequential_workflow(3)),
        ("sequential_bench_5", create_sequential_workflow(5)),
        ("parallel_bench_10", create_parallel_workflow(10)),
    ];

    for (name, definition) in workflows {
        println!("Registering workflow: {}", name);

        let response = client
            .post(format!("{}/api/v1/workflow_definitions", base_url))
            .header("Authorization", format!("Bearer {}", access_token))
            .json(&definition)
            .send()
            .await?;

        if response.status().is_success() {
            println!("  ✓ Registered successfully");
        } else if response.status().as_u16() == 409 {
            println!("  ⚠ Already exists (skipping)");
        } else {
            let status = response.status();
            let error_text = response.text().await?;
            println!("  ✗ Failed: {} - {}", status, error_text);
        }
    }

    println!();
    println!("✅ Workflow definitions registered!");
    println!();
    println!("You can now run the benchmarks:");
    println!(
        "  cargo test --package kruxiaflow-profiling --release --test load_tests -- --nocapture"
    );

    Ok(())
}
