/// Utility to seed OAuth client in benchmark database
///
/// Usage:
///   cargo run --package streamflow-benchmark --bin seed-oauth-client
///
/// Requires environment variables:
///   - DATABASE_URL: Database connection string
///   - STREAMFLOW_CLIENT_ID: OAuth client ID (default: streamflow-dev-client)
///   - STREAMFLOW_CLIENT_SECRET: OAuth client secret
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Get configuration from environment
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");

    let client_id =
        env::var("STREAMFLOW_CLIENT_ID").unwrap_or_else(|_| "streamflow-dev-client".to_string());

    let client_secret =
        env::var("STREAMFLOW_CLIENT_SECRET").expect("STREAMFLOW_CLIENT_SECRET must be set");

    println!("Seeding OAuth Client for Benchmark Database");
    println!("==========================================");
    println!("Database: {}", database_url);
    println!("Client ID: {}", client_id);
    println!();

    // Connect to database
    let pool = sqlx::PgPool::connect(&database_url).await?;

    // Generate bcrypt hash
    println!("Generating bcrypt hash...");
    let client_secret_hash = bcrypt::hash(&client_secret, 12)?;

    // Delete existing client if it exists
    println!("Removing existing client if present...");
    sqlx::query("DELETE FROM oauth_clients WHERE client_id = $1")
        .bind(&client_id)
        .execute(&pool)
        .await?;

    // Insert new client
    println!("Inserting OAuth client...");
    sqlx::query(
        r#"
        INSERT INTO oauth_clients (
            client_id,
            client_secret_hash,
            name,
            description,
            scopes,
            is_active
        ) VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(&client_id)
    .bind(&client_secret_hash)
    .bind("Development Client")
    .bind("OAuth client for development and benchmarking")
    .bind(vec!["workflow:read", "workflow:write", "workflow:execute"])
    .bind(true)
    .execute(&pool)
    .await?;

    // Verify insertion
    println!("\n✅ OAuth client registered successfully!");
    println!();
    println!("Client ID: {}", client_id);
    println!("Database: {}", database_url);

    Ok(())
}
