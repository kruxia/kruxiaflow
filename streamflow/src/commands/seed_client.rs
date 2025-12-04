use anyhow::Result;
use clap::Args;
use sqlx::PgPool;

/// Seed client command - Seed OAuth client credentials
#[derive(Args)]
pub struct SeedClientCommand {
    /// OAuth client ID
    #[arg(
        long,
        env = "STREAMFLOW_CLIENT_ID",
        default_value = "streamflow_internal_worker",
        help = "OAuth client ID to seed",
        long_help = "OAuth client ID to seed in the database.\n\n\
Default: streamflow_internal_worker\n\
Example: --client-id my-app-client"
    )]
    pub client_id: String,

    /// OAuth client secret
    #[arg(
        long,
        env = "STREAMFLOW_CLIENT_SECRET",
        help = "OAuth client secret (required)",
        long_help = "OAuth client secret to hash and store.\n\n\
Required: Must be provided via flag or STREAMFLOW_CLIENT_SECRET env var\n\
Example: --client-secret my-secret-key"
    )]
    pub client_secret: Option<String>,

    /// Force re-seed even if client exists
    #[arg(
        long,
        help = "Delete and re-create client even if it exists",
        long_help = "By default, seed-client skips seeding if the client already exists.\n\
Use --force to delete the existing client and create a new one.\n\n\
Example: streamflow seed-client --force"
    )]
    pub force: bool,
}

/// Execute seed-client command
pub async fn execute(cmd: SeedClientCommand, database_url: String) -> Result<()> {
    // Load secret from file if _FILE variant is set (Docker secrets pattern)
    let client_secret = cmd
        .client_secret
        .or_else(|| load_secret("STREAMFLOW_CLIENT_SECRET"))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Client secret required (--client-secret or STREAMFLOW_CLIENT_SECRET)"
            )
        })?;

    // Connect to database
    let pool = PgPool::connect(&database_url)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to database: {}", e))?;

    // Check if client already exists
    let existing = check_client_exists(&pool, &cmd.client_id).await?;

    if existing && !cmd.force {
        println!("OAuth client '{}' already exists, skipping.", cmd.client_id);
        println!("Use --force to delete and re-create.");
        return Ok(());
    }

    // Seed the client
    seed_oauth_client(&pool, &cmd.client_id, &client_secret, cmd.force).await
}

/// Load a secret from environment, supporting Docker secrets pattern.
/// Checks for `{name}_FILE` first (reads file contents), then falls back to `{name}` direct value.
fn load_secret(name: &str) -> Option<String> {
    // First check for _FILE variant (Docker secrets pattern)
    let file_var = format!("{}_FILE", name);
    if let Ok(file_path) = std::env::var(&file_var) {
        match std::fs::read_to_string(&file_path) {
            Ok(contents) => {
                return Some(contents.trim().to_string());
            }
            Err(e) => {
                tracing::warn!("Failed to read {} from {}: {}", file_var, file_path, e);
            }
        }
    }

    // Fall back to direct environment variable
    std::env::var(name).ok()
}

/// Check if OAuth client already exists
async fn check_client_exists(pool: &PgPool, client_id: &str) -> Result<bool> {
    let result = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM oauth_clients WHERE client_id = $1",
    )
    .bind(client_id)
    .fetch_one(pool)
    .await
    .map_err(|e| anyhow::anyhow!("Failed to check for existing client: {}", e))?;

    Ok(result > 0)
}

/// Seed OAuth client in database
pub async fn seed_oauth_client(
    pool: &PgPool,
    client_id: &str,
    client_secret: &str,
    force: bool,
) -> Result<()> {
    println!("Seeding OAuth Client");
    println!("====================");
    println!("Client ID: {}", client_id);
    println!();

    // Generate bcrypt hash
    println!("Generating bcrypt hash...");
    let client_secret_hash = bcrypt::hash(client_secret, 12)
        .map_err(|e| anyhow::anyhow!("Failed to hash client secret: {}", e))?;

    if force {
        // Delete existing client if it exists
        println!("Removing existing client if present...");
        sqlx::query("DELETE FROM oauth_clients WHERE client_id = $1")
            .bind(client_id)
            .execute(pool)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to delete existing client: {}", e))?;
    }

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
    .bind(client_id)
    .bind(&client_secret_hash)
    .bind("StreamFlow Client")
    .bind("OAuth client for StreamFlow services")
    .bind(vec!["workflow:read", "workflow:write", "workflow:execute"])
    .bind(true)
    .execute(pool)
    .await
    .map_err(|e| anyhow::anyhow!("Failed to insert OAuth client: {}", e))?;

    println!();
    println!("OAuth client '{}' seeded successfully!", client_id);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_seed_client_command_defaults() {
        let cmd = SeedClientCommand {
            client_id: "test-client".to_string(),
            client_secret: Some("secret".to_string()),
            force: false,
        };
        assert_eq!(cmd.client_id, "test-client");
        assert!(!cmd.force);
    }

    #[test]
    fn test_seed_client_command_force_flag() {
        let cmd = SeedClientCommand {
            client_id: "test-client".to_string(),
            client_secret: Some("secret".to_string()),
            force: true,
        };
        assert!(cmd.force);
    }

    #[test]
    fn test_seed_client_command_optional_secret() {
        let cmd = SeedClientCommand {
            client_id: "test-client".to_string(),
            client_secret: None,
            force: false,
        };
        assert!(cmd.client_secret.is_none());
    }
}
