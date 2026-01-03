use anyhow::Result;
use clap::Args;
use sqlx::PgPool;
use sqlx::migrate::Migrator;

/// Embedded migrations from ../migrations directory
static MIGRATOR: Migrator = sqlx::migrate!("../migrations");

/// Migrate command - Database migration management
#[derive(Args)]
pub struct MigrateCommand {
    /// Show migration status without running
    #[arg(
        long,
        help = "Show migration status without running migrations",
        long_help = "Display the status of all migrations (applied/pending) without executing any.\n\n\
Example: kruxiaflow migrate --status"
    )]
    pub status: bool,

    /// Preview migrations without applying
    #[arg(
        long,
        help = "Preview migrations that would be applied",
        long_help = "Show which migrations would be applied without actually running them.\n\n\
Example: kruxiaflow migrate --dry-run"
    )]
    pub dry_run: bool,
}

/// Execute migrate command
pub async fn execute(cmd: MigrateCommand, database_url: String) -> Result<()> {
    // Connect to database
    let pool = PgPool::connect(&database_url)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to database: {}", e))?;

    if cmd.status {
        return show_status(&pool).await;
    }

    if cmd.dry_run {
        return show_dry_run(&pool).await;
    }

    // Run migrations
    run_migrations(&pool).await
}

/// Show migration status
async fn show_status(pool: &PgPool) -> Result<()> {
    println!("Migration Status");
    println!("================");
    println!();

    // Get applied migrations from database
    let applied =
        sqlx::query_scalar::<_, i64>("SELECT version FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await
            .unwrap_or_default();

    let applied_set: std::collections::HashSet<i64> = applied.into_iter().collect();

    // List all migrations
    let mut pending_count = 0;
    let mut applied_count = 0;

    for migration in MIGRATOR.iter() {
        let status = if applied_set.contains(&migration.version) {
            applied_count += 1;
            "✓ Applied"
        } else {
            pending_count += 1;
            "○ Pending"
        };

        println!(
            "{:12} {:>14} - {}",
            status, migration.version, migration.description
        );
    }

    println!();
    println!(
        "Total: {} applied, {} pending",
        applied_count, pending_count
    );

    if pending_count > 0 {
        println!();
        println!("Run 'kruxiaflow migrate' to apply pending migrations.");
    }

    Ok(())
}

/// Show what would be run (dry-run)
async fn show_dry_run(pool: &PgPool) -> Result<()> {
    println!("Dry Run - Migrations that would be applied");
    println!("===========================================");
    println!();

    // Get applied migrations from database
    let applied =
        sqlx::query_scalar::<_, i64>("SELECT version FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await
            .unwrap_or_default();

    let applied_set: std::collections::HashSet<i64> = applied.into_iter().collect();

    // List pending migrations
    let mut pending_count = 0;

    for migration in MIGRATOR.iter() {
        if !applied_set.contains(&migration.version) {
            pending_count += 1;
            println!(
                "  Would apply: {} - {}",
                migration.version, migration.description
            );
        }
    }

    if pending_count == 0 {
        println!("  No pending migrations.");
    } else {
        println!();
        println!("{} migration(s) would be applied.", pending_count);
        println!();
        println!("Run 'kruxiaflow migrate' to apply these migrations.");
    }

    Ok(())
}

/// Run pending migrations
pub async fn run_migrations(pool: &PgPool) -> Result<()> {
    tracing::info!("Running database migrations...");

    MIGRATOR
        .run(pool)
        .await
        .map_err(|e| anyhow::anyhow!("Migration failed: {}", e))?;

    tracing::info!("Migrations completed successfully");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Get the embedded migrator (for testing)
    fn migrator() -> &'static Migrator {
        &MIGRATOR
    }

    #[test]
    fn test_migrate_command_defaults() {
        let cmd = MigrateCommand {
            status: false,
            dry_run: false,
        };
        assert!(!cmd.status);
        assert!(!cmd.dry_run);
    }

    #[test]
    fn test_migrate_command_status_flag() {
        let cmd = MigrateCommand {
            status: true,
            dry_run: false,
        };
        assert!(cmd.status);
        assert!(!cmd.dry_run);
    }

    #[test]
    fn test_migrate_command_dry_run_flag() {
        let cmd = MigrateCommand {
            status: false,
            dry_run: true,
        };
        assert!(!cmd.status);
        assert!(cmd.dry_run);
    }

    #[test]
    fn test_migrator_has_migrations() {
        // Verify that migrations are embedded at compile time
        let migrator = migrator();
        assert!(
            migrator.iter().count() > 0,
            "Should have embedded migrations"
        );
    }

    #[test]
    fn test_migrations_are_sorted() {
        // Verify migrations are in ascending version order
        let migrator = migrator();
        let versions: Vec<i64> = migrator.iter().map(|m| m.version).collect();

        let mut sorted = versions.clone();
        sorted.sort();

        assert_eq!(versions, sorted, "Migrations should be in version order");
    }

    #[test]
    fn test_migrations_have_descriptions() {
        // Verify all migrations have non-empty descriptions
        let migrator = migrator();
        for migration in migrator.iter() {
            assert!(
                !migration.description.is_empty(),
                "Migration {} should have a description",
                migration.version
            );
        }
    }
}
