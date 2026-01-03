use anyhow::Result;
use clap::Args;
use sqlx::PgPool;
use std::path::PathBuf;

#[derive(Args)]
pub struct SeedLlmCommand {
    /// Path to the LLM models YAML file
    #[arg(value_name = "YAML_FILE")]
    yaml_file: PathBuf,
}

pub async fn execute(cmd: SeedLlmCommand, database_url: String) -> Result<()> {
    tracing::info!("Connecting to database...");
    let pool = PgPool::connect(&database_url).await?;

    tracing::info!("Loading LLM catalog from {:?}...", cmd.yaml_file);
    crate::llm_catalog::load_catalog_from_yaml(&pool, &cmd.yaml_file).await?;

    tracing::info!("✓ LLM catalog loaded successfully");

    pool.close().await;
    Ok(())
}
