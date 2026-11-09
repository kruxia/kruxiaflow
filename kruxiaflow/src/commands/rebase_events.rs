use anyhow::{Context, Result};
use clap::Args;
use kruxiaflow_core::events::{find_foreign_xids, rebase_event_xids};
use sqlx::PgPool;

/// Rebase-events command - repair event cursors after the database moved to
/// a new PostgreSQL cluster
#[derive(Args)]
pub struct RebaseEventsCommand {
    /// Report whether a rebase is needed without changing anything
    #[arg(
        long,
        help = "Report whether a rebase is needed without changing anything",
        long_help = "Check whether the stored event transaction ids are consistent with this\n\
database cluster and report the result without changing anything.\n\n\
Example: kruxiaflow rebase-events --check"
    )]
    pub check: bool,
}

/// Execute rebase-events command
pub async fn execute(cmd: RebaseEventsCommand, database_url: String) -> Result<()> {
    let pool = PgPool::connect(&database_url)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to database: {}", e))?;

    let found = find_foreign_xids(&pool)
        .await
        .context("Failed to check event cursors (have migrations been applied?)")?;

    let Some(found) = found else {
        println!("Event cursors are consistent with this database cluster. Nothing to do.");
        return Ok(());
    };

    println!(
        "Event cursors were written by a different PostgreSQL cluster:\n  \
           highest stored transaction id: {}\n  \
           this cluster's next transaction id: {}",
        found.max_stored_tx(),
        found.next_xid
    );

    if cmd.check {
        anyhow::bail!(
            "A rebase is needed. Stop every Kruxia Flow process, then run `kruxiaflow rebase-events`."
        );
    }

    println!("Rebasing (workflow_events and workflow_event_consumers are locked meanwhile)...");
    let report = rebase_event_xids(&pool)
        .await
        .context("Rebase failed; nothing was changed")?;

    println!(
        "Rebased {} event(s) and {} consumer cursor(s). Kruxia Flow can be started again.",
        report.events, report.consumers
    );
    Ok(())
}
