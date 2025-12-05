//! StreamFlow Stress Test CLI
//!
//! Runs ramping stress tests to identify system breaking points and capacity limits.
//!
//! Usage:
//!   stress-test [OPTIONS]
//!
//! Examples:
//!   stress-test --quick                    # Quick test: 100 -> 1,000 concurrent
//!   stress-test --standard                 # Standard test: 100 -> 5,000 concurrent
//!   stress-test --full                     # Full test: 100 -> 10,000 concurrent
//!   stress-test --peak 2000 --step-size 200

use clap::Parser;
use std::env;
use std::path::PathBuf;
use std::time::Duration;

use streamflow_profiling::client::StreamFlowClient;
use streamflow_profiling::monitor::{ResourceAnalysis, ResourceMonitor};
use streamflow_profiling::stress::{StressTestConfig, run_stress_test};

#[derive(Parser, Debug)]
#[command(name = "stress-test")]
#[command(about = "StreamFlow Stress Test - Identify system breaking points and capacity limits")]
#[command(version)]
struct Args {
    /// Run quick stress test (100 -> 1,000 concurrent)
    #[arg(long, conflicts_with_all = ["standard", "full", "peak_concurrent"])]
    quick: bool,

    /// Run standard stress test (100 -> 5,000 concurrent)
    #[arg(long, conflicts_with_all = ["quick", "full", "peak_concurrent"])]
    standard: bool,

    /// Run full stress test (100 -> 10,000 concurrent)
    #[arg(long, conflicts_with_all = ["quick", "standard", "peak_concurrent"])]
    full: bool,

    /// Initial number of concurrent workflows
    #[arg(long, default_value = "100")]
    initial_concurrent: usize,

    /// Peak number of concurrent workflows to ramp up to
    #[arg(long)]
    peak_concurrent: Option<usize>,

    /// Number of concurrent workflows to add per step
    #[arg(long, default_value = "500")]
    step_size: usize,

    /// Duration of each step in seconds
    #[arg(long, default_value = "30")]
    step_duration: u64,

    /// Cooldown period between steps in seconds
    #[arg(long, default_value = "5")]
    cooldown: u64,

    /// Workflow definition to use for testing
    #[arg(long, default_value = "sequential_bench_5")]
    workflow: String,

    /// Error rate threshold (0.0-1.0) that triggers breaking point
    #[arg(long, default_value = "0.05")]
    error_threshold: f64,

    /// P99 latency threshold in milliseconds
    #[arg(long, default_value = "5000")]
    latency_threshold: u64,

    /// Stop test when breaking point is detected
    #[arg(long, default_value = "true")]
    stop_on_failure: bool,

    /// Workflow completion timeout in seconds
    #[arg(long, default_value = "60")]
    workflow_timeout: u64,

    /// Output directory for results
    #[arg(long)]
    output_dir: Option<PathBuf>,

    /// Enable resource monitoring
    #[arg(long, default_value = "true")]
    monitor_resources: bool,

    /// Resource monitoring interval in milliseconds
    #[arg(long, default_value = "1000")]
    monitor_interval: u64,
}

fn create_client() -> StreamFlowClient {
    let base_url =
        env::var("STREAMFLOW_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());

    let client_id = env::var("STREAMFLOW_CLIENT_ID")
        .expect("STREAMFLOW_CLIENT_ID environment variable must be set");

    let client_secret = env::var("STREAMFLOW_CLIENT_SECRET")
        .expect("STREAMFLOW_CLIENT_SECRET environment variable must be set");

    StreamFlowClient::new(base_url, client_id, client_secret)
}

fn build_config(args: &Args) -> StressTestConfig {
    if args.quick {
        return StressTestConfig::quick();
    }
    if args.standard {
        return StressTestConfig::standard();
    }
    if args.full {
        return StressTestConfig::full();
    }

    // Custom configuration
    StressTestConfig {
        initial_concurrent: args.initial_concurrent,
        peak_concurrent: args.peak_concurrent.unwrap_or(10_000),
        step_size: args.step_size,
        step_duration: Duration::from_secs(args.step_duration),
        cooldown: Duration::from_secs(args.cooldown),
        workflow_definition: args.workflow.clone(),
        error_rate_threshold: args.error_threshold,
        latency_threshold_ms: args.latency_threshold,
        stop_on_failure: args.stop_on_failure,
        workflow_timeout: Duration::from_secs(args.workflow_timeout),
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    println!("\n{}", "=".repeat(70));
    println!("StreamFlow Stress Test");
    println!("{}\n", "=".repeat(70));

    // Verify server is accessible
    let base_url =
        env::var("STREAMFLOW_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
    println!("Checking server at {}...", base_url);

    let health_url = format!("{}/health", base_url);
    match reqwest::get(&health_url).await {
        Ok(response) if response.status().is_success() => {
            println!("Server is accessible\n");
        }
        Ok(response) => {
            eprintln!("Server returned status: {}", response.status());
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("Failed to connect to server: {}", e);
            eprintln!("\nMake sure the StreamFlow server is running:");
            eprintln!("  streamflow serve --port 8080");
            std::process::exit(1);
        }
    }

    let client = create_client();
    let config = build_config(&args);

    println!("Configuration:");
    println!("  Initial Concurrent: {}", config.initial_concurrent);
    println!("  Peak Concurrent:    {}", config.peak_concurrent);
    println!("  Step Size:          {}", config.step_size);
    println!("  Step Duration:      {}s", config.step_duration.as_secs());
    println!("  Workflow:           {}", config.workflow_definition);
    println!(
        "  Error Threshold:    {:.1}%",
        config.error_rate_threshold * 100.0
    );
    println!("  Latency Threshold:  {}ms", config.latency_threshold_ms);
    println!(
        "  Estimated Duration: {:.0}s ({} steps)",
        config.estimated_duration().as_secs_f64(),
        config.num_steps()
    );

    // Start resource monitoring if enabled
    let resource_samples = if args.monitor_resources {
        let mut monitor = ResourceMonitor::new(
            Duration::from_millis(args.monitor_interval),
            None, // Monitor system-wide resources
        );
        monitor.start();
        println!(
            "\nResource monitoring enabled ({}ms interval)",
            args.monitor_interval
        );
        Some(monitor.samples())
    } else {
        None
    };

    // Run stress test
    let results = run_stress_test(&client, config, resource_samples.clone()).await;

    // Analyze resources if monitoring was enabled
    if let Some(samples) = resource_samples {
        let samples = samples.read().await;
        if !samples.is_empty() {
            let analysis = ResourceAnalysis::from_samples(&samples);
            println!("\n{}", analysis.summary());
        }
    }

    // Save results if output directory specified
    let output_dir = args.output_dir.unwrap_or_else(|| {
        let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
        PathBuf::from(format!("var/stress-test-{}", timestamp))
    });

    if let Err(e) = std::fs::create_dir_all(&output_dir) {
        eprintln!("Failed to create output directory: {}", e);
    } else {
        let results_path = output_dir.join("stress-test-results.json");
        if let Err(e) = results.save_to_file(&results_path) {
            eprintln!("Failed to save results: {}", e);
        } else {
            println!("\nResults saved to: {}", results_path.display());
        }

        // Save summary as markdown
        let summary_path = output_dir.join("stress-test-summary.md");
        let summary_content = generate_markdown_summary(&results);
        if let Err(e) = std::fs::write(&summary_path, summary_content) {
            eprintln!("Failed to save summary: {}", e);
        } else {
            println!("Summary saved to: {}", summary_path.display());
        }
    }

    // Exit with appropriate code
    if results.breaking_point.is_some() {
        println!("\nBreaking point was detected. Review results for capacity planning.");
        std::process::exit(0); // Breaking point detection is expected behavior
    } else {
        println!("\nTest completed successfully without hitting breaking point.");
        std::process::exit(0);
    }
}

fn generate_markdown_summary(results: &streamflow_profiling::stress::StressTestResults) -> String {
    let mut md = String::new();

    md.push_str("# Stress Test Results\n\n");
    md.push_str(&format!(
        "**Date**: {}\n\n",
        results.started_at.format("%Y-%m-%d %H:%M:%S UTC")
    ));

    md.push_str("## Summary\n\n");
    md.push_str(&format!("| Metric | Value |\n|--------|-------|\n"));
    md.push_str(&format!(
        "| Duration | {:.1}s |\n",
        results.total_duration.as_secs_f64()
    ));
    md.push_str(&format!(
        "| Steps Completed | {} / {} |\n",
        results.steps.len(),
        results.config.num_steps()
    ));
    md.push_str(&format!(
        "| Peak Throughput | {:.2} wf/sec |\n",
        results.peak_throughput_wf_per_sec
    ));
    md.push_str(&format!(
        "| Max Concurrent | {} |\n",
        results.max_concurrent_achieved
    ));

    if let Some(ref bp) = results.breaking_point {
        md.push_str("\n## Breaking Point\n\n");
        md.push_str(&format!(
            "**Concurrent Workflows**: {}\n\n",
            bp.concurrent_workflows
        ));
        md.push_str(&format!("**Reason**: {}\n\n", bp.failure_mode));
        md.push_str("### Metrics at Breaking Point\n\n");
        md.push_str(&format!("| Metric | Value |\n|--------|-------|\n"));
        md.push_str(&format!(
            "| Throughput | {:.2} wf/sec |\n",
            bp.metrics.throughput_wf_per_sec
        ));
        md.push_str(&format!(
            "| Success Rate | {:.1}% |\n",
            bp.metrics.success_rate * 100.0
        ));
        md.push_str(&format!(
            "| P99 Latency | {}ms |\n",
            bp.metrics.p99_latency_ms
        ));
    } else {
        md.push_str("\n## No Breaking Point Detected\n\n");
        md.push_str(&format!(
            "System successfully handled {} concurrent workflows.\n",
            results.config.peak_concurrent
        ));
    }

    md.push_str("\n## Step Details\n\n");
    md.push_str("| Step | Concurrent | Throughput | Success | P99 |\n");
    md.push_str("|------|------------|------------|---------|-----|\n");
    for step in &results.steps {
        md.push_str(&format!(
            "| {} | {} | {:.2} wf/sec | {:.1}% | {}ms |\n",
            step.step_number + 1,
            step.target_concurrent,
            step.throughput_wf_per_sec,
            step.success_rate * 100.0,
            step.p99_latency_ms
        ));
    }

    md.push_str("\n## Configuration\n\n");
    md.push_str(&format!(
        "- Initial Concurrent: {}\n",
        results.config.initial_concurrent
    ));
    md.push_str(&format!(
        "- Peak Concurrent: {}\n",
        results.config.peak_concurrent
    ));
    md.push_str(&format!("- Step Size: {}\n", results.config.step_size));
    md.push_str(&format!(
        "- Step Duration: {}s\n",
        results.config.step_duration.as_secs()
    ));
    md.push_str(&format!(
        "- Workflow: {}\n",
        results.config.workflow_definition
    ));

    md
}
