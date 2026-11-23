// ============================================================================
// run_code integration tests: real Monty worker processes
// Guide: docs/run-code.md
//
// Drives run_code end to end through the public RunCodeActivity: the Monty
// subprocess pool, capabilities, limits, imports, error mapping and the run
// record.
//
// Requires the `monty` worker binary (from contrib/kruxiaflow-monty):
//
//   cargo install monty-runtime --version 1.6.0 --locked
//   KRUXIAFLOW_MONTY_BINARY=$HOME/.cargo/bin/monty \
//     cargo nextest run --test run_code_integration_tests
//
// Without KRUXIAFLOW_MONTY_BINARY each test prints a notice and returns.
// ============================================================================

use kruxiaflow_monty::{MontyConfig, RunCodeActivity, WORKER};
use kruxiaflow_worker::{ActivityContext, ActivityImpl};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// The activity under test, or `None` (with a notice) when no binary is set
async fn activity() -> Option<RunCodeActivity> {
    let binary = std::env::var_os("KRUXIAFLOW_MONTY_BINARY").filter(|b| !b.is_empty())?;
    let config = MontyConfig {
        binary: PathBuf::from(binary),
        min_processes: 1,
        max_processes: 4,
        request_timeout: Duration::from_secs(8),
        recycle_after: 1,
        max_memory_bytes: 32 * 1024 * 1024,
        max_duration: Duration::from_secs(2),
        max_recursion_depth: 1000,
    };
    Some(
        RunCodeActivity::new(config, WORKER)
            .await
            .expect("valid Monty configuration and binary"),
    )
}

macro_rules! activity_or_skip {
    () => {
        match activity().await {
            Some(activity) => activity,
            None => {
                eprintln!("skipped: set KRUXIAFLOW_MONTY_BINARY to run run_code integration tests");
                return;
            }
        }
    };
}

fn context(key: &str) -> ActivityContext {
    ActivityContext::new(Uuid::now_v7(), Uuid::now_v7(), key)
}

/// Run with a given context and return the `result` output
async fn run_in(activity: &RunCodeActivity, ctx: &ActivityContext, params: Value) -> Value {
    let result = activity
        .execute(params, ctx)
        .await
        .expect("run_code completes for code-caused outcomes");
    assert_eq!(result.outputs.len(), 1);
    assert_eq!(result.outputs[0].name, "result");
    result.outputs[0].value.clone()
}

async fn run(activity: &RunCodeActivity, params: Value) -> Value {
    run_in(activity, &context("step"), params).await
}

fn assert_error(result: &Value, kind: &str, needle: &str) {
    assert_eq!(result["ok"], json!(false), "expected failure: {result:#}");
    assert_eq!(result["error"]["kind"], json!(kind), "{result:#}");
    let message = result["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains(needle),
        "message should contain {needle:?}: {message}"
    );
}

// ---------------------------------------------------------------------------
// Success paths
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_rule_with_inputs_capabilities_and_run_record() {
    let activity = activity_or_skip!();
    let result = run(
        &activity,
        json!({
            "code": "from decimal import Decimal\n\
                     flagged = []\n\
                     for order in orders:\n\
                     \x20   if Decimal(str(order['total'])) > Decimal('500'):\n\
                     \x20       flagged.append(order['id'])\n\
                     \x20       log(f\"flagged {order['id']}\")\n\
                     {'flagged': flagged, 'at': now()}",
            "inputs": {"orders": [{"id": "A1", "total": 612.5}, {"id": "A2", "total": 80}]},
            "capabilities": ["log", "now"],
            "limits": {"max_feed_duration_secs": 1, "max_memory": 8000000},
        }),
    )
    .await;

    assert_eq!(result["ok"], json!(true), "{result:#}");
    assert_eq!(result["value"]["flagged"], json!(["A1"]));
    assert_eq!(result["logs"], json!(["flagged A1"]));

    let run = &result["run"];
    assert_eq!(run["limits"]["max_feed_duration_secs"], json!(1.0));
    assert_eq!(run["limits"]["max_memory"], json!(8000000));
    assert_eq!(run["capabilities"]["granted"], json!(["log", "now"]));
    assert_eq!(run["capabilities"]["called"], json!({"log": 1, "now": 1}));
    assert_eq!(run["consumed"]["host_calls"], json!(2));
    assert_eq!(run["type_check"], json!(true));
    assert_eq!(run["worker_crashes"], json!(0));
    assert!(
        run["monty_version"]
            .as_str()
            .unwrap()
            .starts_with("monty-runtime ")
    );
    assert_eq!(run["code_sha256"].as_str().unwrap().len(), 64);
}

#[tokio::test]
async fn test_stdout_decimal_and_fenced_code() {
    let activity = activity_or_skip!();
    let result = run(
        &activity,
        json!({"code": "```python\nfrom decimal import Decimal\nprint('total:', Decimal('1.10') + Decimal('2.20'))\nDecimal('1.10') + Decimal('2.20')\n```"}),
    )
    .await;
    assert_eq!(result["ok"], json!(true), "{result:#}");
    assert_eq!(result["value"], json!("3.30"));
    assert_eq!(result["stdout"], json!("total: 3.30\n"));
}

#[tokio::test]
async fn test_json_path_capability() {
    let activity = activity_or_skip!();
    let result = run(
        &activity,
        json!({
            "code": "(json_path(order, 'customer.tags[1]'), json_path(order, 'missing', 'none'))",
            "inputs": {"order": {"customer": {"tags": ["new", "vip"]}}},
            "capabilities": ["json_path"],
        }),
    )
    .await;
    assert_eq!(result["value"], json!(["vip", "none"]), "{result:#}");
}

#[tokio::test]
async fn test_clock_is_frozen_and_random_is_seeded_per_step() {
    let activity = activity_or_skip!();
    let code = "import random, datetime\n(str(datetime.datetime.now()), random.random(), now())";
    let ctx = context("seeded");
    let first = run_in(
        &activity,
        &ctx,
        json!({"code": code, "capabilities": ["now"]}),
    )
    .await;
    let second = run_in(
        &activity,
        &ctx,
        json!({"code": code, "capabilities": ["now"]}),
    )
    .await;
    // Same workflow and activity key: same random draw, on different workers
    assert_eq!(first["value"][1], second["value"][1]);
    // datetime.now() is frozen: within one run it matches now() to the second
    let in_code = first["value"][0].as_str().unwrap();
    let from_now = first["value"][2].as_str().unwrap();
    assert_eq!(&in_code[..19], &from_now[..19].replace('T', " "));

    let other = run_in(
        &activity,
        &context("other_step"),
        json!({"code": code, "capabilities": ["now"]}),
    )
    .await;
    assert_ne!(first["value"][1], other["value"][1]);
}

#[tokio::test]
async fn test_concurrent_runs_share_the_pool() {
    let activity = Arc::new(activity_or_skip!());
    let mut handles = Vec::new();
    for i in 0..12 {
        let activity = activity.clone();
        handles.push(tokio::spawn(async move {
            run(&activity, json!({"code": "n * n", "inputs": {"n": i}})).await
        }));
    }
    for (i, handle) in handles.into_iter().enumerate() {
        let result = handle.await.unwrap();
        assert_eq!(result["value"], json!(i * i), "{result:#}");
    }
}

// ---------------------------------------------------------------------------
// Code errors complete the step with ok: false
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_type_error_is_caught_before_running() {
    let activity = activity_or_skip!();
    let result = run(
        &activity,
        json!({"code": "log(42)", "capabilities": ["log"]}),
    )
    .await;
    assert_error(&result, "type", "invalid-argument-type");
    assert_eq!(result["run"]["consumed"]["host_calls"], json!(0));
}

#[tokio::test]
async fn test_runtime_exception() {
    let activity = activity_or_skip!();
    let result = run(&activity, json!({"code": "x = {'a': 1}\nx['b']"})).await;
    assert_error(&result, "runtime", "KeyError");
}

#[tokio::test]
async fn test_syntax_error() {
    let activity = activity_or_skip!();
    let result = run(&activity, json!({"code": "def (", "type_check": false})).await;
    assert_error(&result, "runtime", "SyntaxError");
}

#[tokio::test]
async fn test_ungranted_capability_is_refused() {
    let activity = activity_or_skip!();
    // Without type checking the call reaches the host and is refused there
    let result = run(&activity, json!({"code": "log('hi')", "type_check": false})).await;
    assert_error(&result, "runtime", "granted no capabilities");

    let result = run(
        &activity,
        json!({"code": "fail('x')", "capabilities": ["log"], "type_check": false}),
    )
    .await;
    assert_error(&result, "runtime", "may call only: log");

    // With type checking it never runs
    let result = run(&activity, json!({"code": "price_of('sku')"})).await;
    assert_error(&result, "type", "price_of");
}

#[tokio::test]
async fn test_unavailable_module() {
    let activity = activity_or_skip!();
    let result = run(
        &activity,
        json!({"code": "import statistics\nstatistics.mean([1, 2])", "type_check": false}),
    )
    .await;
    assert_error(&result, "runtime", "No module named 'statistics'");
    assert_error(&result, "runtime", "json, re, datetime");
}

#[tokio::test]
async fn test_files_and_environment_are_unavailable() {
    let activity = activity_or_skip!();
    let result = run(
        &activity,
        json!({"code": "import pathlib\npathlib.Path('/etc/passwd').read_text()", "type_check": false}),
    )
    .await;
    assert_eq!(result["ok"], json!(false), "{result:#}");

    let result = run(
        &activity,
        json!({"code": "import os\nos.getenv('HOME')", "type_check": false}),
    )
    .await;
    // Either refused, or None: the sandbox never sees the host environment
    assert!(
        result["ok"] == json!(false) || result["value"] == json!(null),
        "{result:#}"
    );
}

#[tokio::test]
async fn test_fail_is_sticky() {
    let activity = activity_or_skip!();
    let result = run(
        &activity,
        json!({
            "code": "try:\n    fail('total must be positive')\nexcept Exception:\n    pass\n'carried on'",
            "capabilities": ["fail"],
        }),
    )
    .await;
    assert_error(&result, "fail", "total must be positive");
}

#[tokio::test]
async fn test_values_without_json_form() {
    let activity = activity_or_skip!();
    let result = run(&activity, json!({"code": "{1, 2, 3}"})).await;
    assert_error(&result, "output", "sorted(list(...))");
    let result = run(&activity, json!({"code": "2 ** 100"})).await;
    assert_error(&result, "output", "64 bits");
}

// ---------------------------------------------------------------------------
// Limits
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_memory_limit() {
    let activity = activity_or_skip!();
    let result = run(
        &activity,
        json!({"code": "x = 'a' * 50_000_000\nlen(x)", "limits": {"max_memory": 4000000}}),
    )
    .await;
    assert_error(&result, "limit", "memory limit exceeded");
    assert_eq!(result["error"]["limit"], json!("max_memory"));
}

#[tokio::test]
async fn test_time_limit_stops_an_infinite_loop() {
    let activity = activity_or_skip!();
    let result = run(
        &activity,
        json!({"code": "while True:\n    pass", "limits": {"max_feed_duration_secs": 0.5}}),
    )
    .await;
    assert_error(&result, "limit", "time limit");
    assert_eq!(result["error"]["limit"], json!("max_feed_duration"));
}

#[tokio::test]
async fn test_recursion_limit() {
    let activity = activity_or_skip!();
    let result = run(
        &activity,
        json!({"code": "def f(n):\n    return f(n + 1)\nf(0)", "limits": {"max_recursion_depth": 50}}),
    )
    .await;
    assert_error(&result, "limit", "Recursion");
    assert_eq!(result["error"]["limit"], json!("max_recursion_depth"));
}

#[tokio::test]
async fn test_host_call_limit() {
    let activity = activity_or_skip!();
    let result = run(
        &activity,
        json!({
            "code": "for i in range(20):\n    log(str(i))",
            "capabilities": ["log"],
            "limits": {"max_suspensions": 5},
        }),
    )
    .await;
    assert_eq!(result["ok"], json!(false), "{result:#}");
    assert_eq!(
        result["error"]["limit"],
        json!("max_suspensions"),
        "{result:#}"
    );
}

#[tokio::test]
async fn test_pool_recovers_after_limits() {
    let activity = activity_or_skip!();
    let _ = run(
        &activity,
        json!({"code": "x = 'a' * 50_000_000", "limits": {"max_memory": 4000000}}),
    )
    .await;
    let result = run(&activity, json!({"code": "1 + 1"})).await;
    assert_eq!(result["value"], json!(2), "{result:#}");
}

// ---------------------------------------------------------------------------
// Configuration errors fail the activity
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_configuration_errors_fail_the_activity() {
    let activity = activity_or_skip!();
    let cases = [
        (
            json!({"code": "1", "bogus": true}),
            "invalid run_code parameters",
        ),
        (
            json!({"code": "1", "capabilities": ["shell"]}),
            "unknown capability",
        ),
        (
            json!({"code": "1", "limits": {"max_memory": 999999999999u64}}),
            "exceeds this worker's ceiling",
        ),
        (
            json!({"code": "1", "inputs": {"not-valid": 1}}),
            "not a valid Python identifier",
        ),
        (
            json!({"code": "1", "inputs": {"log": 1}, "capabilities": ["log"]}),
            "same name as a granted capability",
        ),
        (json!({"code": "   "}), "is empty"),
    ];
    for (params, needle) in cases {
        let err = activity
            .execute(params.clone(), &context("bad"))
            .await
            .expect_err(&format!("should fail: {params}"));
        assert!(
            !err.retryable,
            "{params}: a workflow's mistake is not retried"
        );
        assert_eq!(err.code, "INVALID_PARAMETERS", "{params}");
        let err = err.to_string();
        assert!(err.contains(needle), "{params}: {err}");
    }
}
