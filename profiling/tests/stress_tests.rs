//! Stress tests for Kruxia Flow.
//!
//! These tests run ramping stress tests to identify system breaking points
//! and validate graceful degradation behavior.
//!
//! Run with:
//!   cargo test --package kruxiaflow-profiling --test stress_tests -- --ignored --nocapture

use serial_test::serial;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use kruxiaflow_profiling::bottleneck::BottleneckAnalyzer;
use kruxiaflow_profiling::client::StreamFlowClient;
use kruxiaflow_profiling::monitor::{ResourceAnalysis, ResourceMonitor};
use kruxiaflow_profiling::stress::{StressTestConfig, run_stress_test};

/// Create authenticated client from environment variables
fn create_client() -> StreamFlowClient {
    let base_url =
        env::var("KRUXIAFLOW_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());

    let client_id = env::var("KRUXIAFLOW_CLIENT_ID")
        .expect("KRUXIAFLOW_CLIENT_ID environment variable must be set");

    let client_secret = env::var("KRUXIAFLOW_CLIENT_SECRET")
        .expect("KRUXIAFLOW_CLIENT_SECRET environment variable must be set");

    StreamFlowClient::new(base_url, client_id, client_secret)
}

/// Get output directory for stress test results
fn get_output_dir() -> PathBuf {
    env::var("PROFILING_OUTPUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
            PathBuf::from(format!("var/stress-test-{}", timestamp))
        })
}

/// Save stress test results to the output directory
fn save_results(
    results: &kruxiaflow_profiling::stress::StressTestResults,
    report: &kruxiaflow_profiling::bottleneck::BottleneckReport,
    output_dir: &PathBuf,
) {
    if let Err(e) = fs::create_dir_all(output_dir) {
        eprintln!("Failed to create output directory: {}", e);
        return;
    }

    // Save stress test results
    let results_path = output_dir.join("stress-test-results.json");
    if let Err(e) = results.save_to_file(&results_path) {
        eprintln!("Failed to save results: {}", e);
    } else {
        println!("Results saved to: {}", results_path.display());
    }

    // Save bottleneck report
    let report_path = output_dir.join("bottleneck-report.md");
    if let Err(e) = fs::write(&report_path, report.to_markdown()) {
        eprintln!("Failed to save bottleneck report: {}", e);
    } else {
        println!("Bottleneck report saved to: {}", report_path.display());
    }
}

/// Quick stress test: 100 -> 1,000 concurrent workflows
#[tokio::test]
#[serial]
#[ignore] // Run explicitly with: cargo test --package kruxiaflow-profiling --test stress_tests -- --ignored
async fn test_stress_quick() {
    let client = create_client();
    let config = StressTestConfig::quick();

    println!("\n=== Quick Stress Test ===");
    println!(
        "Ramping from {} to {} concurrent workflows",
        config.initial_concurrent, config.peak_concurrent
    );

    // Start resource monitoring
    let mut monitor = ResourceMonitor::new(Duration::from_secs(1), None);
    monitor.start();

    let results = run_stress_test(&client, config, Some(monitor.samples())).await;

    // Stop monitoring and analyze
    let samples = monitor.stop().await;
    let resource_analysis = ResourceAnalysis::from_samples(&samples);

    println!("\n{}", resource_analysis.summary());

    // Analyze bottlenecks
    let analyzer = BottleneckAnalyzer::default();
    let report = analyzer.analyze(&results, Some(&resource_analysis), None);

    if let Some(ref bn) = report.primary_bottleneck {
        println!("\nPrimary bottleneck: {} - {}", bn.category, bn.description);
    }

    // Save results
    let output_dir = get_output_dir();
    save_results(&results, &report, &output_dir);

    // Verify we got meaningful results
    assert!(!results.steps.is_empty(), "Should have at least one step");
    assert!(
        results.peak_throughput_wf_per_sec > 0.0,
        "Should have positive throughput"
    );
}

/// Standard stress test: 100 -> 5,000 concurrent workflows
#[tokio::test]
#[serial]
#[ignore]
async fn test_stress_standard() {
    let client = create_client();
    let config = StressTestConfig::standard();

    println!("\n=== Standard Stress Test ===");
    println!(
        "Ramping from {} to {} concurrent workflows",
        config.initial_concurrent, config.peak_concurrent
    );

    let mut monitor = ResourceMonitor::new(Duration::from_secs(1), None);
    monitor.start();

    let results = run_stress_test(&client, config, Some(monitor.samples())).await;

    let samples = monitor.stop().await;
    let resource_analysis = ResourceAnalysis::from_samples(&samples);

    println!("\n{}", resource_analysis.summary());

    let analyzer = BottleneckAnalyzer::default();
    let report = analyzer.analyze(&results, Some(&resource_analysis), None);

    let output_dir = get_output_dir();
    save_results(&results, &report, &output_dir);

    assert!(!results.steps.is_empty());
}

/// Full stress test: 100 -> 10,000 concurrent workflows
#[tokio::test]
#[serial]
#[ignore]
async fn test_stress_full() {
    let client = create_client();
    let config = StressTestConfig::full();

    println!("\n=== Full Stress Test ===");
    println!(
        "Ramping from {} to {} concurrent workflows",
        config.initial_concurrent, config.peak_concurrent
    );

    let mut monitor = ResourceMonitor::new(Duration::from_secs(1), None);
    monitor.start();

    let results = run_stress_test(&client, config, Some(monitor.samples())).await;

    let samples = monitor.stop().await;
    let resource_analysis = ResourceAnalysis::from_samples(&samples);

    println!("\n{}", resource_analysis.summary());

    let analyzer = BottleneckAnalyzer::default();
    let report = analyzer.analyze(&results, Some(&resource_analysis), None);

    let output_dir = get_output_dir();
    save_results(&results, &report, &output_dir);

    assert!(!results.steps.is_empty());
}

/// Graceful degradation test: Verify system errors gracefully under overload
#[tokio::test]
#[serial]
#[ignore]
async fn test_graceful_degradation() {
    let client = create_client();

    // Configure a test that will likely hit limits
    let config = StressTestConfig {
        initial_concurrent: 500,
        peak_concurrent: 5000,
        step_size: 500,
        step_duration: Duration::from_secs(20),
        cooldown: Duration::from_secs(3),
        workflow_definition: "sequential_bench_3".to_string(),
        error_rate_threshold: 0.10, // Allow 10% errors before breaking point
        latency_threshold_ms: 10000, // 10 second timeout
        stop_on_failure: false,     // Continue even after breaking point
        workflow_timeout: Duration::from_secs(30),
    };

    println!("\n=== Graceful Degradation Test ===");
    println!("Testing system behavior under overload conditions");

    let mut monitor = ResourceMonitor::new(Duration::from_secs(1), None);
    monitor.start();

    let results = run_stress_test(&client, config, Some(monitor.samples())).await;

    let samples = monitor.stop().await;
    let resource_analysis = ResourceAnalysis::from_samples(&samples);

    println!("\n{}", resource_analysis.summary());

    // Verify graceful degradation criteria:
    // 1. Test completed without crash
    // 2. At least some workflows succeeded
    // 3. Errors were handled (not panics)

    let total_successful: usize = results.steps.iter().map(|s| s.successful_workflows).sum();
    let total_failed: usize = results.steps.iter().map(|s| s.failed_workflows).sum();

    println!("\nGraceful Degradation Results:");
    println!("  Total Successful: {}", total_successful);
    println!("  Total Failed: {}", total_failed);
    println!(
        "  Overall Success Rate: {:.1}%",
        (total_successful as f64 / (total_successful + total_failed) as f64) * 100.0
    );

    // Verify we had some success even under load
    assert!(
        total_successful > 0,
        "Should have at least some successful workflows"
    );

    // Verify errors were tracked (not crashes)
    let has_error_info = results
        .steps
        .iter()
        .any(|s| s.failed_workflows > 0 && s.errors.len() > 0);
    if total_failed > 0 {
        println!(
            "  Error tracking: {}",
            if has_error_info {
                "Working"
            } else {
                "Errors not tracked"
            }
        );
    }

    let output_dir = get_output_dir();
    let analyzer = BottleneckAnalyzer::default();
    let report = analyzer.analyze(&results, Some(&resource_analysis), None);
    save_results(&results, &report, &output_dir);
}

/// Recovery test: Verify system recovers after overload
#[tokio::test]
#[serial]
#[ignore]
async fn test_recovery_after_overload() {
    let client = create_client();

    println!("\n=== Recovery After Overload Test ===");
    println!("Phase 1: Establish baseline...");

    // Phase 1: Establish baseline at low load
    let baseline_config = StressTestConfig {
        initial_concurrent: 50,
        peak_concurrent: 50,
        step_size: 100, // Won't increase
        step_duration: Duration::from_secs(15),
        cooldown: Duration::from_secs(1),
        ..Default::default()
    };

    let baseline_results = run_stress_test(&client, baseline_config, None).await;
    let baseline_throughput = baseline_results.peak_throughput_wf_per_sec;
    println!("  Baseline throughput: {:.2} wf/sec", baseline_throughput);

    // Phase 2: Apply heavy load
    println!("\nPhase 2: Applying heavy load...");
    let overload_config = StressTestConfig {
        initial_concurrent: 500,
        peak_concurrent: 2000,
        step_size: 500,
        step_duration: Duration::from_secs(10),
        cooldown: Duration::from_secs(2),
        stop_on_failure: false,
        ..Default::default()
    };

    let _ = run_stress_test(&client, overload_config, None).await;
    println!("  Heavy load completed");

    // Phase 3: Wait for cooldown
    println!("\nPhase 3: Cooldown period...");
    tokio::time::sleep(Duration::from_secs(10)).await;

    // Phase 4: Check recovery at baseline load
    println!("\nPhase 4: Checking recovery...");
    let recovery_results = run_stress_test(
        &client,
        StressTestConfig {
            initial_concurrent: 50,
            peak_concurrent: 50,
            step_size: 100,
            step_duration: Duration::from_secs(15),
            cooldown: Duration::from_secs(1),
            ..Default::default()
        },
        None,
    )
    .await;

    let recovery_throughput = recovery_results.peak_throughput_wf_per_sec;
    println!("  Recovery throughput: {:.2} wf/sec", recovery_throughput);

    // Allow 20% degradation from baseline
    let recovery_ratio = recovery_throughput / baseline_throughput;
    println!("  Recovery ratio: {:.1}%", recovery_ratio * 100.0);

    assert!(
        recovery_ratio > 0.8,
        "System should recover to at least 80% of baseline throughput, got {:.1}%",
        recovery_ratio * 100.0
    );

    println!("\n  Recovery test PASSED");
}
