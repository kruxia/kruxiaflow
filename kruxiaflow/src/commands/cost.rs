// Client CLI: cost reporting commands (wraps the REST cost API).
//
// Design: kruxiaflow-internal/docs/features/2026-02-07-client-cli.md §cost and
// kruxiaflow-internal/docs/implementation/2026-07-12-cost-visibility.md Phase 1.
// Auth is OAuth2 client-credentials (KRUXIAFLOW_CLIENT_ID/SECRET); against a
// server running with --insecure-dev no credentials are needed.

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, NaiveDate, Utc};
use clap::{Args, Subcommand};
use reqwest::{Client, StatusCode};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::io::Write;
use std::time::Duration;
use uuid::Uuid;

#[derive(Args)]
pub struct CostCommand {
    #[command(subcommand)]
    pub command: CostSubcommand,

    /// API server URL
    #[arg(
        long,
        env = "KRUXIAFLOW_API_URL",
        default_value = "http://127.0.0.1:8080",
        global = true
    )]
    pub api_url: String,

    /// OAuth2 client ID (not needed against --insecure-dev servers)
    #[arg(long, env = "KRUXIAFLOW_CLIENT_ID", global = true)]
    pub client_id: Option<String>,

    /// OAuth2 client secret (not needed against --insecure-dev servers)
    #[arg(
        long,
        env = "KRUXIAFLOW_CLIENT_SECRET",
        global = true,
        hide_env_values = true
    )]
    pub client_secret: Option<String>,

    /// Output format (table, json, csv)
    #[arg(
        long,
        env = "KRUXIAFLOW_OUTPUT_FORMAT",
        default_value = "table",
        value_parser = ["table", "json", "csv"],
        global = true
    )]
    pub format: String,

    /// Request timeout in seconds
    #[arg(long, default_value = "10", global = true)]
    pub timeout: u64,
}

#[derive(Subcommand)]
pub enum CostSubcommand {
    /// Cost summary for a workflow
    #[command(
        about = "Cost summary for a workflow",
        long_about = "Cost summary for a workflow: total cost, tokens, budget status.\n\n\
EXAMPLES:\n  \
  kruxiaflow cost workflow 0197a2b0-...\n  \
  kruxiaflow cost workflow 0197a2b0-... --detailed\n  \
  kruxiaflow cost workflow 0197a2b0-... --format json"
    )]
    Workflow(WorkflowArgs),

    /// Aggregated cost analytics for a period
    #[command(
        about = "Aggregated cost analytics for a period",
        long_about = "Aggregated cost analytics: totals, cache hit rate, budget events,\n\
optionally grouped by provider, model, definition, or day.\n\n\
EXAMPLES:\n  \
  kruxiaflow cost analytics\n  \
  kruxiaflow cost analytics --since 7d\n  \
  kruxiaflow cost analytics --group-by provider\n  \
  kruxiaflow cost analytics --start-date 2026-07-01 --end-date 2026-07-18"
    )]
    Analytics(AnalyticsArgs),

    /// Most expensive workflows or definitions
    #[command(
        about = "Most expensive workflows or definitions",
        long_about = "Most expensive workflows (default) or definitions in a period.\n\n\
EXAMPLES:\n  \
  kruxiaflow cost top --limit 10 --since 30d\n  \
  kruxiaflow cost top --by definitions"
    )]
    Top(TopArgs),

    /// Export cost data as CSV
    #[command(
        about = "Export cost data as CSV",
        long_about = "Export per-workflow costs (default) or grouped costs as CSV.\n\n\
EXAMPLES:\n  \
  kruxiaflow cost export --since 30d --output costs.csv\n  \
  kruxiaflow cost export --group-by day"
    )]
    Export(ExportArgs),
}

#[derive(Args)]
pub struct WorkflowArgs {
    /// Workflow ID (UUID)
    pub workflow_id: Uuid,

    /// Include per-activity/attempt breakdown
    #[arg(long, short = 'd')]
    pub detailed: bool,
}

#[derive(Args)]
pub struct PeriodArgs {
    /// Relative period like 7d, 24h, 4w (conflicts with explicit dates)
    #[arg(long, conflicts_with_all = ["start_date", "end_date"])]
    pub since: Option<String>,

    /// Start date, YYYY-MM-DD or RFC 3339 (default: 30 days ago)
    #[arg(long)]
    pub start_date: Option<String>,

    /// End date, YYYY-MM-DD or RFC 3339 (default: now)
    #[arg(long)]
    pub end_date: Option<String>,
}

#[derive(Args)]
pub struct AnalyticsArgs {
    #[command(flatten)]
    pub period: PeriodArgs,

    /// Group results by dimension
    #[arg(long, value_parser = ["provider", "model", "definition", "day"])]
    pub group_by: Option<String>,
}

#[derive(Args)]
pub struct TopArgs {
    #[command(flatten)]
    pub period: PeriodArgs,

    /// Number of rows to show
    #[arg(long, default_value = "10")]
    pub limit: i64,

    /// Rank workflows or definitions
    #[arg(long, value_parser = ["workflows", "definitions"], default_value = "workflows")]
    pub by: String,
}

#[derive(Args)]
pub struct ExportArgs {
    #[command(flatten)]
    pub period: PeriodArgs,

    /// Output file (default: stdout)
    #[arg(long, short = 'o')]
    pub output: Option<std::path::PathBuf>,

    /// Export grouped costs instead of per-workflow rows
    #[arg(long, value_parser = ["provider", "model", "definition", "day"])]
    pub group_by: Option<String>,
}

// ---------------------------------------------------------------------------
// API response types (mirror api/src/handlers/cost.rs; Decimal fields arrive
// as JSON strings)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct WorkflowCostSummary {
    workflow_id: Uuid,
    workflow_name: String,
    total_cost_usd: Decimal,
    budget_limit_usd: Option<Decimal>,
    budget_remaining_usd: Option<Decimal>,
    total_activities: i64,
}

#[derive(Debug, Deserialize)]
struct ActivityCostDetail {
    activity_key: String,
    attempt: i32,
    cost_usd: Decimal,
    prompt_tokens: Option<i32>,
    output_tokens: Option<i32>,
    total_tokens: Option<i32>,
    cached_tokens: Option<i32>,
    provider: Option<String>,
    model: Option<String>,
    budget_exceeded: Option<bool>,
    budget_event: Option<String>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct CostGroup {
    key: Option<String>,
    total_cost_usd: Decimal,
    activities: i64,
    workflows: i64,
    total_tokens: i64,
}

#[derive(Debug, Deserialize)]
struct TopWorkflow {
    workflow_id: Uuid,
    definition_name: String,
    status: String,
    created_at: DateTime<Utc>,
    total_cost_usd: Decimal,
    activities: i64,
    budget_limit_usd: Option<Decimal>,
}

#[derive(Debug, Deserialize)]
struct TopDefinition {
    definition_name: String,
    workflows: i64,
    total_cost_usd: Decimal,
    avg_cost_per_workflow: Decimal,
}

#[derive(Debug, Deserialize)]
struct BudgetEvent {
    definition_name: String,
    activity_key: String,
    event: String,
    estimated_cost_usd: Option<Decimal>,
    budget_limit_usd: Option<Decimal>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct CostAnalytics {
    total_workflows: i64,
    total_cost_usd: Decimal,
    avg_cost_per_activity: Decimal,
    start_date: DateTime<Utc>,
    end_date: DateTime<Utc>,
    total_activities: i64,
    avg_cost_per_workflow: Decimal,
    total_tokens: i64,
    cached_tokens: i64,
    cache_hit_rate: Option<f64>,
    budget_aborts: i64,
    budget_downgrades: i64,
    #[serde(default)]
    groups: Option<Vec<CostGroup>>,
    top_workflows: Vec<TopWorkflow>,
    top_definitions: Vec<TopDefinition>,
    budget_events: Vec<BudgetEvent>,
}

// ---------------------------------------------------------------------------
// Authenticated API client
// ---------------------------------------------------------------------------

struct ApiClient {
    http: Client,
    api_url: String,
    token: Option<String>,
}

impl ApiClient {
    async fn connect(cmd: &CostCommand) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(cmd.timeout))
            .build()
            .context("Failed to build HTTP client")?;
        let api_url = cmd.api_url.trim_end_matches('/').to_string();

        let token = match (&cmd.client_id, &cmd.client_secret) {
            (Some(id), Some(secret)) => Some(obtain_token(&http, &api_url, id, secret).await?),
            (None, None) => None,
            _ => {
                return Err(anyhow!(
                    "--client-id and --client-secret must be provided together \
                     (or neither, against a --insecure-dev server)"
                ));
            }
        };

        Ok(Self {
            http,
            api_url,
            token,
        })
    }

    /// GET a path (with query params) and return the raw JSON body.
    async fn get_raw(&self, path: &str, query: &[(&str, String)]) -> Result<String> {
        let mut request = self.http.get(format!("{}{}", self.api_url, path));
        if !query.is_empty() {
            request = request.query(query);
        }
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("Failed to reach API server at {}", self.api_url))?;

        match response.status() {
            StatusCode::UNAUTHORIZED => Err(anyhow!(
                "Authentication required. Set KRUXIAFLOW_CLIENT_ID and \
                 KRUXIAFLOW_CLIENT_SECRET (or run the server with --insecure-dev)."
            )),
            StatusCode::NOT_FOUND => Err(anyhow!("Not found: {}", path)),
            status if !status.is_success() => {
                let body = response.text().await.unwrap_or_default();
                Err(anyhow!("API request failed ({}): {}", status, body))
            }
            _ => response.text().await.context("Failed to read API response"),
        }
    }

    async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T> {
        let body = self.get_raw(path, query).await?;
        serde_json::from_str(&body)
            .with_context(|| format!("Unexpected response shape from {}", path))
    }
}

async fn obtain_token(http: &Client, api_url: &str, id: &str, secret: &str) -> Result<String> {
    #[derive(serde::Serialize)]
    struct TokenRequest<'a> {
        grant_type: &'a str,
        client_id: &'a str,
        client_secret: &'a str,
    }
    #[derive(Deserialize)]
    struct TokenResponse {
        access_token: String,
    }

    let response = http
        .post(format!("{}/api/v1/oauth/token", api_url))
        .json(&TokenRequest {
            grant_type: "client_credentials",
            client_id: id,
            client_secret: secret,
        })
        .send()
        .await
        .with_context(|| format!("Failed to reach API server at {}", api_url))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("Token request failed ({}): {}", status, body));
    }

    Ok(response.json::<TokenResponse>().await?.access_token)
}

// ---------------------------------------------------------------------------
// Period handling
// ---------------------------------------------------------------------------

/// Resolve --since / --start-date / --end-date into query parameters. Only
/// explicitly-given bounds are sent; the server defaults to the last 30 days.
fn period_query(period: &PeriodArgs) -> Result<Vec<(&'static str, String)>> {
    let mut query = Vec::new();

    if let Some(since) = &period.since {
        let duration = parse_since(since)?;
        query.push(("start_date", (Utc::now() - duration).to_rfc3339()));
        return Ok(query);
    }

    if let Some(start) = &period.start_date {
        query.push(("start_date", parse_date(start, false)?.to_rfc3339()));
    }
    if let Some(end) = &period.end_date {
        query.push(("end_date", parse_date(end, true)?.to_rfc3339()));
    }
    Ok(query)
}

fn parse_since(s: &str) -> Result<chrono::Duration> {
    let (num, unit) = s.split_at(s.len().saturating_sub(1));
    let n: i64 = num.parse().map_err(|_| {
        anyhow!(
            "Invalid --since value '{}': expected forms like 7d, 24h, 4w",
            s
        )
    })?;
    match unit {
        "d" => Ok(chrono::Duration::days(n)),
        "h" => Ok(chrono::Duration::hours(n)),
        "w" => Ok(chrono::Duration::weeks(n)),
        _ => Err(anyhow!(
            "Invalid --since unit '{}': use d (days), h (hours), or w (weeks)",
            unit
        )),
    }
}

/// Parse YYYY-MM-DD (midnight UTC; end dates roll to end of day) or RFC 3339.
fn parse_date(s: &str, end_of_day: bool) -> Result<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    let date = NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| anyhow!("Invalid date '{}': expected YYYY-MM-DD or RFC 3339", s))?;
    let time = if end_of_day {
        date.and_hms_opt(23, 59, 59).unwrap()
    } else {
        date.and_hms_opt(0, 0, 0).unwrap()
    };
    Ok(DateTime::from_naive_utc_and_offset(time, Utc))
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

fn fmt_usd(value: Decimal) -> String {
    let rounded = value.round_dp(6).normalize();
    format!("${}", rounded)
}

fn fmt_opt_usd(value: Option<Decimal>) -> String {
    value.map(fmt_usd).unwrap_or_else(|| "—".to_string())
}

fn fmt_time(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%d %H:%M").to_string()
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{}…", cut)
    }
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn print_json_pretty(body: &str) -> Result<()> {
    let value: serde_json::Value = serde_json::from_str(body)?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub async fn execute(cmd: CostCommand) -> Result<()> {
    let client = ApiClient::connect(&cmd).await?;

    match &cmd.command {
        CostSubcommand::Workflow(args) => workflow_report(&client, &cmd, args).await,
        CostSubcommand::Analytics(args) => analytics_report(&client, &cmd, args).await,
        CostSubcommand::Top(args) => top_report(&client, &cmd, args).await,
        CostSubcommand::Export(args) => export_csv(&client, args).await,
    }
}

// ---------------------------------------------------------------------------
// cost workflow
// ---------------------------------------------------------------------------

async fn workflow_report(client: &ApiClient, cmd: &CostCommand, args: &WorkflowArgs) -> Result<()> {
    let cost_path = format!("/api/v1/workflows/{}/cost", args.workflow_id);
    let history_path = format!("/api/v1/workflows/{}/cost/history", args.workflow_id);
    let workflow_path = format!("/api/v1/workflows/{}", args.workflow_id);

    let cost_body = client.get_raw(&cost_path, &[]).await?;
    let history_body = client.get_raw(&history_path, &[]).await?;
    let summary: WorkflowCostSummary =
        serde_json::from_str(&cost_body).context("Unexpected cost summary response shape")?;
    let history: Vec<ActivityCostDetail> =
        serde_json::from_str(&history_body).context("Unexpected cost history response shape")?;

    // Workflow status is informational — don't fail the report over it
    let status = client
        .get_json::<serde_json::Value>(&workflow_path, &[])
        .await
        .ok()
        .and_then(|w| w.get("status").and_then(|s| s.as_str()).map(String::from))
        .unwrap_or_else(|| "unknown".to_string());

    match cmd.format.as_str() {
        "json" => {
            let combined = serde_json::json!({
                "cost": serde_json::from_str::<serde_json::Value>(&cost_body)?,
                "history": serde_json::from_str::<serde_json::Value>(&history_body)?,
                "status": status,
            });
            println!("{}", serde_json::to_string_pretty(&combined)?);
            return Ok(());
        }
        "csv" => {
            let mut out = String::from(
                "activity_key,attempt,provider,model,prompt_tokens,output_tokens,total_tokens,cached_tokens,cost_usd,budget_event,created_at\n",
            );
            for row in &history {
                out.push_str(&format!(
                    "{},{},{},{},{},{},{},{},{},{},{}\n",
                    csv_field(&row.activity_key),
                    row.attempt,
                    csv_field(row.provider.as_deref().unwrap_or("")),
                    csv_field(row.model.as_deref().unwrap_or("")),
                    row.prompt_tokens.unwrap_or(0),
                    row.output_tokens.unwrap_or(0),
                    row.total_tokens.unwrap_or(0),
                    row.cached_tokens.unwrap_or(0),
                    row.cost_usd,
                    csv_field(row.budget_event.as_deref().unwrap_or("")),
                    row.created_at.to_rfc3339(),
                ));
            }
            print!("{}", out);
            return Ok(());
        }
        _ => {}
    }

    let prompt_tokens: i64 = history
        .iter()
        .filter_map(|h| h.prompt_tokens)
        .map(i64::from)
        .sum();
    let output_tokens: i64 = history
        .iter()
        .filter_map(|h| h.output_tokens)
        .map(i64::from)
        .sum();
    let cached_tokens: i64 = history
        .iter()
        .filter_map(|h| h.cached_tokens)
        .map(i64::from)
        .sum();
    let total_tokens: i64 = history
        .iter()
        .filter_map(|h| h.total_tokens)
        .map(i64::from)
        .sum();
    let aborts = history
        .iter()
        .filter(|h| h.budget_event.as_deref() == Some("abort"))
        .count();
    let downgrades = history
        .iter()
        .filter(|h| h.budget_event.as_deref() == Some("downgrade"))
        .count();

    println!();
    println!("  Workflow Cost Report");
    println!("  {:-<56}", "");
    println!("  {:<16}{}", "Workflow ID:", summary.workflow_id);
    println!("  {:<16}{}", "Definition:", summary.workflow_name);
    println!("  {:<16}{}", "Status:", status);
    println!("  {:-<56}", "");
    println!("  {:<16}{}", "Total Cost:", fmt_usd(summary.total_cost_usd));
    println!(
        "  {:<16}{} (in: {}, out: {}, cached: {})",
        "Total Tokens:", total_tokens, prompt_tokens, output_tokens, cached_tokens
    );
    println!("  {:<16}{}", "Activities:", summary.total_activities);
    if let Some(limit) = summary.budget_limit_usd {
        let pct = if limit > Decimal::ZERO {
            (summary.total_cost_usd / limit * Decimal::from(100)).round_dp(1)
        } else {
            Decimal::ZERO
        };
        println!(
            "  {:<16}{} ({}% used, {} remaining)",
            "Budget:",
            fmt_usd(limit),
            pct,
            fmt_opt_usd(summary.budget_remaining_usd)
        );
    } else {
        println!("  {:<16}none", "Budget:");
    }
    if aborts > 0 || downgrades > 0 {
        println!(
            "  {:<16}{} abort(s), {} downgrade(s)",
            "Budget Events:", aborts, downgrades
        );
    }
    println!("  {:-<56}", "");

    if args.detailed {
        println!();
        println!("  Activity Breakdown:");
        println!(
            "  {:<22} {:>3}  {:<34} {:>8} {:>12}  {:<10} CREATED",
            "ACTIVITY", "TRY", "MODEL", "TOKENS", "COST", "EVENT"
        );
        println!("  {:-<110}", "");
        for row in &history {
            let model = match (&row.provider, &row.model) {
                (Some(p), Some(m)) => format!("{}/{}", p, m),
                (None, Some(m)) => m.clone(),
                _ => "—".to_string(),
            };
            let mut event = row.budget_event.clone().unwrap_or_default();
            if row.budget_exceeded == Some(true) && event.is_empty() {
                event = "exceeded".to_string();
            }
            println!(
                "  {:<22} {:>3}  {:<34} {:>8} {:>12}  {:<10} {}",
                truncate(&row.activity_key, 22),
                row.attempt,
                truncate(&model, 34),
                row.total_tokens.unwrap_or(0),
                fmt_usd(row.cost_usd),
                event,
                fmt_time(row.created_at),
            );
        }
    }
    println!();
    Ok(())
}

// ---------------------------------------------------------------------------
// cost analytics
// ---------------------------------------------------------------------------

async fn analytics_report(
    client: &ApiClient,
    cmd: &CostCommand,
    args: &AnalyticsArgs,
) -> Result<()> {
    let mut query = period_query(&args.period)?;
    if let Some(group_by) = &args.group_by {
        query.push(("group_by", group_by.clone()));
    }

    let body = client.get_raw("/api/v1/cost/analytics", &query).await?;
    if cmd.format == "json" {
        return print_json_pretty(&body);
    }
    let analytics: CostAnalytics =
        serde_json::from_str(&body).context("Unexpected analytics response shape")?;

    if cmd.format == "csv" {
        if let Some(groups) = &analytics.groups {
            print_groups_csv(args.group_by.as_deref().unwrap_or("group"), groups);
        } else {
            println!(
                "start_date,end_date,total_cost_usd,total_workflows,total_activities,avg_cost_per_workflow,avg_cost_per_activity,total_tokens,cached_tokens,cache_hit_rate,budget_aborts,budget_downgrades"
            );
            println!(
                "{},{},{},{},{},{},{},{},{},{},{},{}",
                analytics.start_date.to_rfc3339(),
                analytics.end_date.to_rfc3339(),
                analytics.total_cost_usd,
                analytics.total_workflows,
                analytics.total_activities,
                analytics.avg_cost_per_workflow,
                analytics.avg_cost_per_activity,
                analytics.total_tokens,
                analytics.cached_tokens,
                analytics
                    .cache_hit_rate
                    .map(|r| format!("{:.4}", r))
                    .unwrap_or_default(),
                analytics.budget_aborts,
                analytics.budget_downgrades,
            );
        }
        return Ok(());
    }

    println!();
    println!(
        "  Cost Analytics ({} → {})",
        analytics.start_date.format("%Y-%m-%d"),
        analytics.end_date.format("%Y-%m-%d")
    );
    println!("  {:-<56}", "");
    println!(
        "  {:<20}{}",
        "Total Cost:",
        fmt_usd(analytics.total_cost_usd)
    );
    println!("  {:<20}{}", "Workflows:", analytics.total_workflows);
    println!("  {:<20}{}", "Activities:", analytics.total_activities);
    println!(
        "  {:<20}{}",
        "Avg Cost/Workflow:",
        fmt_usd(analytics.avg_cost_per_workflow)
    );
    println!(
        "  {:<20}{}",
        "Avg Cost/Activity:",
        fmt_usd(analytics.avg_cost_per_activity)
    );
    let cache_rate = analytics
        .cache_hit_rate
        .map(|r| format!("{:.1}%", r * 100.0))
        .unwrap_or_else(|| "—".to_string());
    println!(
        "  {:<20}{} (cached: {}, cache hit rate: {})",
        "Tokens:", analytics.total_tokens, analytics.cached_tokens, cache_rate
    );
    println!(
        "  {:<20}{} abort(s), {} downgrade(s)",
        "Budget Events:", analytics.budget_aborts, analytics.budget_downgrades
    );
    println!("  {:-<56}", "");

    if let Some(groups) = &analytics.groups {
        let label = args.group_by.as_deref().unwrap_or("group").to_uppercase();
        println!();
        println!(
            "  {:<34} {:>12} {:>10} {:>10} {:>12}",
            label, "COST", "ACTIVITIES", "WORKFLOWS", "TOKENS"
        );
        println!("  {:-<84}", "");
        for group in groups {
            println!(
                "  {:<34} {:>12} {:>10} {:>10} {:>12}",
                truncate(group.key.as_deref().unwrap_or("(none)"), 34),
                fmt_usd(group.total_cost_usd),
                group.activities,
                group.workflows,
                group.total_tokens,
            );
        }
    }

    if !analytics.budget_events.is_empty() {
        println!();
        println!("  Recent Budget Events:");
        println!(
            "  {:<10} {:<24} {:<22} {:>12} {:>12}  WHEN",
            "EVENT", "DEFINITION", "ACTIVITY", "EST.COST", "BUDGET"
        );
        println!("  {:-<100}", "");
        for event in &analytics.budget_events {
            println!(
                "  {:<10} {:<24} {:<22} {:>12} {:>12}  {}",
                event.event,
                truncate(&event.definition_name, 24),
                truncate(&event.activity_key, 22),
                fmt_opt_usd(event.estimated_cost_usd),
                fmt_opt_usd(event.budget_limit_usd),
                fmt_time(event.created_at),
            );
        }
    }
    println!();
    Ok(())
}

fn print_groups_csv(dimension: &str, groups: &[CostGroup]) {
    println!(
        "{},total_cost_usd,activities,workflows,total_tokens",
        dimension
    );
    for group in groups {
        println!(
            "{},{},{},{},{}",
            csv_field(group.key.as_deref().unwrap_or("")),
            group.total_cost_usd,
            group.activities,
            group.workflows,
            group.total_tokens,
        );
    }
}

// ---------------------------------------------------------------------------
// cost top
// ---------------------------------------------------------------------------

async fn top_report(client: &ApiClient, cmd: &CostCommand, args: &TopArgs) -> Result<()> {
    let mut query = period_query(&args.period)?;
    query.push(("limit", args.limit.to_string()));

    let body = client.get_raw("/api/v1/cost/analytics", &query).await?;
    if cmd.format == "json" {
        return print_json_pretty(&body);
    }
    let analytics: CostAnalytics =
        serde_json::from_str(&body).context("Unexpected analytics response shape")?;

    if args.by == "definitions" {
        if cmd.format == "csv" {
            println!("definition_name,workflows,total_cost_usd,avg_cost_per_workflow");
            for def in &analytics.top_definitions {
                println!(
                    "{},{},{},{}",
                    csv_field(&def.definition_name),
                    def.workflows,
                    def.total_cost_usd,
                    def.avg_cost_per_workflow
                );
            }
            return Ok(());
        }
        println!();
        println!(
            "  Top Definitions by Cost ({} → {})",
            analytics.start_date.format("%Y-%m-%d"),
            analytics.end_date.format("%Y-%m-%d")
        );
        println!();
        println!(
            "  {:<34} {:>10} {:>14} {:>14}",
            "DEFINITION", "WORKFLOWS", "TOTAL", "AVG/WORKFLOW"
        );
        println!("  {:-<78}", "");
        for def in &analytics.top_definitions {
            println!(
                "  {:<34} {:>10} {:>14} {:>14}",
                truncate(&def.definition_name, 34),
                def.workflows,
                fmt_usd(def.total_cost_usd),
                fmt_usd(def.avg_cost_per_workflow),
            );
        }
    } else {
        if cmd.format == "csv" {
            print_workflows_csv(&analytics.top_workflows);
            return Ok(());
        }
        println!();
        println!(
            "  Top Workflows by Cost ({} → {})",
            analytics.start_date.format("%Y-%m-%d"),
            analytics.end_date.format("%Y-%m-%d")
        );
        println!();
        println!(
            "  {:<38} {:<24} {:<10} {:>12} {:>10} {:>12}",
            "WORKFLOW ID", "DEFINITION", "STATUS", "COST", "ACTIVITIES", "BUDGET"
        );
        println!("  {:-<112}", "");
        for wf in &analytics.top_workflows {
            println!(
                "  {:<38} {:<24} {:<10} {:>12} {:>10} {:>12}",
                wf.workflow_id,
                truncate(&wf.definition_name, 24),
                wf.status,
                fmt_usd(wf.total_cost_usd),
                wf.activities,
                fmt_opt_usd(wf.budget_limit_usd),
            );
        }
    }
    println!();
    Ok(())
}

fn print_workflows_csv(workflows: &[TopWorkflow]) {
    println!(
        "workflow_id,definition_name,status,created_at,activities,total_cost_usd,budget_limit_usd"
    );
    for wf in workflows {
        println!(
            "{},{},{},{},{},{},{}",
            wf.workflow_id,
            csv_field(&wf.definition_name),
            wf.status,
            wf.created_at.to_rfc3339(),
            wf.activities,
            wf.total_cost_usd,
            wf.budget_limit_usd
                .map(|d| d.to_string())
                .unwrap_or_default(),
        );
    }
}

// ---------------------------------------------------------------------------
// cost export
// ---------------------------------------------------------------------------

async fn export_csv(client: &ApiClient, args: &ExportArgs) -> Result<()> {
    let mut query = period_query(&args.period)?;
    // Per-workflow export wants all rows, not the display default
    query.push(("limit", "10000".to_string()));
    if let Some(group_by) = &args.group_by {
        query.push(("group_by", group_by.clone()));
    }

    let analytics: CostAnalytics = client.get_json("/api/v1/cost/analytics", &query).await?;

    let mut csv = String::new();
    if let Some(groups) = &analytics.groups {
        let dimension = args.group_by.as_deref().unwrap_or("group");
        csv.push_str(&format!(
            "{},total_cost_usd,activities,workflows,total_tokens\n",
            dimension
        ));
        for group in groups {
            csv.push_str(&format!(
                "{},{},{},{},{}\n",
                csv_field(group.key.as_deref().unwrap_or("")),
                group.total_cost_usd,
                group.activities,
                group.workflows,
                group.total_tokens,
            ));
        }
    } else {
        csv.push_str(
            "workflow_id,definition_name,status,created_at,activities,total_cost_usd,budget_limit_usd\n",
        );
        for wf in &analytics.top_workflows {
            csv.push_str(&format!(
                "{},{},{},{},{},{},{}\n",
                wf.workflow_id,
                csv_field(&wf.definition_name),
                wf.status,
                wf.created_at.to_rfc3339(),
                wf.activities,
                wf.total_cost_usd,
                wf.budget_limit_usd
                    .map(|d| d.to_string())
                    .unwrap_or_default(),
            ));
        }
    }

    match &args.output {
        Some(path) => {
            let mut file = std::fs::File::create(path)
                .with_context(|| format!("Failed to create {}", path.display()))?;
            file.write_all(csv.as_bytes())?;
            eprintln!(
                "Exported {} rows to {}",
                csv.lines().count().saturating_sub(1),
                path.display()
            );
        }
        None => print!("{}", csv),
    }
    Ok(())
}
