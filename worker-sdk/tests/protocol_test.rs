//! Protocol conformance tests against a mock server: auth, 401 refresh,
//! 409 idempotency, usage reporting, and the full poll → execute → report
//! loop including panic/timeout containment and graceful drain.

use kruxiaflow_worker::{
    ActivityError, ActivityResult, ClientError, UsageEntry, Worker, WorkerApiClient, WorkerConfig,
};
use rust_decimal_macros::dec;
use serde_json::{Value, json};
use std::time::Duration;
use uuid::Uuid;
use wiremock::matchers::{body_partial_json, header, header_exists, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn token_mock() -> Mock {
    Mock::given(method("POST"))
        .and(path("/api/v1/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "test_token_1",
            "token_type": "Bearer",
            "expires_in": 3600
        })))
}

fn empty_poll_response() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({"activities": [], "count": 0}))
}

fn one_activity_response(activity_id: Uuid, name: &str, parameters: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "activities": [{
            "activity_id": activity_id,
            "workflow_id": Uuid::now_v7(),
            "activity_key": "step_one",
            "worker": "demo",
            "activity_name": name,
            "parameters": parameters,
            "settings": null,
            "timeout_seconds": null,
            "output_definitions": null,
            "signal_data": null
        }],
        "count": 1
    }))
}

fn test_config(server: &MockServer) -> WorkerConfig {
    WorkerConfig {
        api_url: server.uri(),
        worker: "demo".to_string(),
        worker_id: "test_worker".to_string(),
        poll_interval: Duration::from_millis(10),
        shutdown_timeout: Duration::from_secs(5),
        ..WorkerConfig::default()
    }
}

/// Wait until the mock server has seen `count` requests matching the
/// predicate, or panic after ~2s.
async fn wait_for_requests(
    server: &MockServer,
    count: usize,
    predicate: impl Fn(&Request) -> bool,
) {
    for _ in 0..200 {
        let requests = server.received_requests().await.unwrap_or_default();
        if requests.iter().filter(|r| predicate(r)).count() >= count {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("mock server never received {count} matching request(s)");
}

// =========================================================================
// Client: auth
// =========================================================================

#[tokio::test]
async fn client_sends_bearer_token_from_credentials() {
    let server = MockServer::start().await;
    token_mock().expect(1).mount(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/workers/poll"))
        .and(header("authorization", "Bearer test_token_1"))
        .respond_with(empty_poll_response())
        .expect(1)
        .mount(&server)
        .await;

    let client = WorkerApiClient::with_credentials(server.uri(), "id", "secret");
    let activities = client.poll_activities("demo", "w1", 5).await.unwrap();
    assert!(activities.is_empty());
}

#[tokio::test]
async fn client_refreshes_token_on_401_and_retries_once() {
    let server = MockServer::start().await;
    token_mock().mount(&server).await;
    // First poll: 401. Retry with refreshed token: 200.
    Mock::given(method("POST"))
        .and(path("/api/v1/workers/poll"))
        .respond_with(ResponseTemplate::new(401))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/workers/poll"))
        .respond_with(empty_poll_response())
        .mount(&server)
        .await;

    let client = WorkerApiClient::with_credentials(server.uri(), "id", "secret");
    let activities = client.poll_activities("demo", "w1", 5).await.unwrap();
    assert!(activities.is_empty());

    // Token endpoint hit twice: initial + refresh
    let token_requests = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/api/v1/oauth/token")
        .count();
    assert_eq!(token_requests, 2);
}

#[tokio::test]
async fn client_without_credentials_sends_no_auth_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/workers/poll"))
        .respond_with(empty_poll_response())
        .expect(1)
        .mount(&server)
        .await;

    let client = WorkerApiClient::new(server.uri());
    client.poll_activities("demo", "w1", 5).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    assert!(
        requests
            .iter()
            .all(|r| !r.headers.contains_key("authorization")),
        "unauthenticated client must not send an Authorization header"
    );
}

#[tokio::test]
async fn client_without_credentials_maps_401_to_auth_required() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/workers/poll"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let client = WorkerApiClient::new(server.uri());
    let err = client.poll_activities("demo", "w1", 5).await.unwrap_err();
    assert!(matches!(err, ClientError::AuthRequired));
}

// =========================================================================
// Client: completion / failure payloads
// =========================================================================

#[tokio::test]
async fn complete_sends_usage_entries_and_returns_warnings() {
    let server = MockServer::start().await;
    let activity_id = Uuid::now_v7();
    Mock::given(method("POST"))
        .and(path(format!("/api/v1/activities/{activity_id}/complete")))
        .and(body_partial_json(json!({
            "worker_id": "w1",
            "output": {"summary": "done"},
            "cost_usd": "0.002",
            "usage": [{
                "provider": "anthropic",
                "model": "claude-sonnet-5",
                "input_tokens": 1000,
                "output_tokens": 50,
                "cache_read_tokens": 800,
                "cache_creation_tokens": 0
            }]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "acknowledged": true,
            "warnings": ["unknown model recorded at cost 0"]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = WorkerApiClient::new(server.uri());
    let ack = client
        .complete_activity(
            activity_id,
            "w1",
            json!({"summary": "done"}),
            Some(dec!(0.002)),
            &[UsageEntry::new("anthropic", "claude-sonnet-5")
                .input_tokens(1000)
                .output_tokens(50)
                .cache_read_tokens(800)],
        )
        .await
        .unwrap();
    assert_eq!(ack.warnings.len(), 1);
}

#[tokio::test]
async fn complete_omits_usage_when_empty() {
    let server = MockServer::start().await;
    let activity_id = Uuid::now_v7();
    Mock::given(method("POST"))
        .and(path(format!("/api/v1/activities/{activity_id}/complete")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"acknowledged": true})))
        .expect(1)
        .mount(&server)
        .await;

    let client = WorkerApiClient::new(server.uri());
    client
        .complete_activity(activity_id, "w1", json!({}), None, &[])
        .await
        .unwrap();

    let request = &server.received_requests().await.unwrap()[0];
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    assert!(body.get("usage").is_none(), "empty usage must be omitted");
    assert!(body.get("cost_usd").is_none());
}

#[tokio::test]
async fn fail_sends_error_with_usage_and_cost() {
    let server = MockServer::start().await;
    let activity_id = Uuid::now_v7();
    Mock::given(method("POST"))
        .and(path(format!("/api/v1/activities/{activity_id}/fail")))
        .and(body_partial_json(json!({
            "worker_id": "w1",
            "error": {
                "code": "RATE_LIMITED",
                "message": "provider rate limit",
                "retryable": true
            },
            "cost_usd": "0.01",
            "usage": [{"provider": "openai", "model": "gpt-5"}]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "acknowledged": true,
            "will_retry": true
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = WorkerApiClient::new(server.uri());
    let error = ActivityError::retryable("RATE_LIMITED", "provider rate limit")
        .with_cost(dec!(0.01))
        .with_usage(vec![UsageEntry::new("openai", "gpt-5")]);
    let ack = client
        .fail_activity(activity_id, "w1", &error)
        .await
        .unwrap();
    assert!(ack.will_retry);
}

#[tokio::test]
async fn conflict_is_a_distinct_error() {
    let server = MockServer::start().await;
    let activity_id = Uuid::now_v7();
    Mock::given(method("POST"))
        .and(path(format!("/api/v1/activities/{activity_id}/heartbeat")))
        .respond_with(
            ResponseTemplate::new(409).set_body_json(json!({"error": "already completed"})),
        )
        .mount(&server)
        .await;

    let client = WorkerApiClient::new(server.uri());
    let err = client.heartbeat(activity_id, "w1").await.unwrap_err();
    assert!(matches!(err, ClientError::Conflict { .. }));
}

// =========================================================================
// Worker loop: poll → execute → report
// =========================================================================

/// Build a worker against the mock server, run it in the background, and
/// return its shutdown handle plus the join handle.
async fn spawn_worker(server: &MockServer, worker: Worker) -> tokio::task::JoinHandle<()> {
    let _ = server;
    tokio::spawn(async move { worker.run().await })
}

#[tokio::test]
async fn poll_execute_complete_roundtrip() {
    let server = MockServer::start().await;
    let activity_id = Uuid::now_v7();

    Mock::given(method("POST"))
        .and(path("/api/v1/workers/poll"))
        .respond_with(one_activity_response(
            activity_id,
            "echo",
            json!({"message": "hello"}),
        ))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/workers/poll"))
        .respond_with(empty_poll_response())
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/api/v1/activities/{activity_id}/complete")))
        .and(body_partial_json(json!({
            "worker_id": "test_worker",
            "output": {"echoed": "hello"}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"acknowledged": true})))
        .expect(1)
        .mount(&server)
        .await;

    #[derive(serde::Deserialize)]
    struct EchoParams {
        message: String,
    }

    let worker = Worker::builder()
        .config(test_config(&server))
        .register_fn("echo", |params: EchoParams, _ctx| async move {
            Ok(ActivityResult::value("echoed", json!(params.message)))
        })
        .build()
        .unwrap();
    let handle = worker.handle();
    let join = spawn_worker(&server, worker).await;

    wait_for_requests(&server, 1, |r| r.url.path().ends_with("/complete")).await;
    handle.shutdown();
    tokio::time::timeout(Duration::from_secs(5), join)
        .await
        .expect("worker should shut down")
        .unwrap();
}

#[tokio::test]
async fn panicking_handler_reports_retryable_failure() {
    let server = MockServer::start().await;
    let activity_id = Uuid::now_v7();

    Mock::given(method("POST"))
        .and(path("/api/v1/workers/poll"))
        .respond_with(one_activity_response(activity_id, "boom", json!({})))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/workers/poll"))
        .respond_with(empty_poll_response())
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/api/v1/activities/{activity_id}/fail")))
        .and(body_partial_json(json!({
            "error": {"code": "PANIC", "retryable": true}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"acknowledged": true})))
        .expect(1)
        .mount(&server)
        .await;

    let worker = Worker::builder()
        .config(test_config(&server))
        .worker("demo")
        .register_fn("boom", |_params: Value, _ctx| async move {
            panic!("intentional test panic");
            #[allow(unreachable_code)]
            Ok(ActivityResult::default())
        })
        .build()
        .unwrap();
    let handle = worker.handle();
    let join = spawn_worker(&server, worker).await;

    // The worker survives the panic and reports the failure.
    wait_for_requests(&server, 1, |r| r.url.path().ends_with("/fail")).await;
    handle.shutdown();
    tokio::time::timeout(Duration::from_secs(5), join)
        .await
        .expect("worker should survive a panicking handler")
        .unwrap();
}

#[tokio::test]
async fn terminal_error_reports_retryable_false() {
    let server = MockServer::start().await;
    let activity_id = Uuid::now_v7();

    Mock::given(method("POST"))
        .and(path("/api/v1/workers/poll"))
        .respond_with(one_activity_response(activity_id, "reject", json!({})))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/workers/poll"))
        .respond_with(empty_poll_response())
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/api/v1/activities/{activity_id}/fail")))
        .and(body_partial_json(json!({
            "error": {"code": "BAD_INPUT", "retryable": false}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"acknowledged": true})))
        .expect(1)
        .mount(&server)
        .await;

    let worker = Worker::builder()
        .config(test_config(&server))
        .worker("demo")
        .register_fn("reject", |_params: Value, _ctx| async move {
            Err::<ActivityResult, _>(ActivityError::terminal("BAD_INPUT", "no"))
        })
        .build()
        .unwrap();
    let handle = worker.handle();
    let join = spawn_worker(&server, worker).await;

    wait_for_requests(&server, 1, |r| r.url.path().ends_with("/fail")).await;
    handle.shutdown();
    tokio::time::timeout(Duration::from_secs(5), join)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn invalid_typed_params_report_invalid_parameters() {
    let server = MockServer::start().await;
    let activity_id = Uuid::now_v7();

    Mock::given(method("POST"))
        .and(path("/api/v1/workers/poll"))
        .respond_with(one_activity_response(
            activity_id,
            "echo",
            json!({"unexpected": true}),
        ))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/workers/poll"))
        .respond_with(empty_poll_response())
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/api/v1/activities/{activity_id}/fail")))
        .and(body_partial_json(json!({
            "error": {"code": "INVALID_PARAMETERS", "retryable": false}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"acknowledged": true})))
        .expect(1)
        .mount(&server)
        .await;

    #[derive(serde::Deserialize)]
    struct EchoParams {
        #[allow(dead_code)]
        message: String,
    }

    let worker = Worker::builder()
        .config(test_config(&server))
        .worker("demo")
        .register_fn("echo", |params: EchoParams, _ctx| async move {
            Ok(ActivityResult::value("echoed", json!(params.message)))
        })
        .build()
        .unwrap();
    let handle = worker.handle();
    let join = spawn_worker(&server, worker).await;

    wait_for_requests(&server, 1, |r| r.url.path().ends_with("/fail")).await;
    handle.shutdown();
    tokio::time::timeout(Duration::from_secs(5), join)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn timeout_reports_retryable_timeout_failure() {
    let server = MockServer::start().await;
    let activity_id = Uuid::now_v7();

    // timeout_seconds: 1 → slow handler must be failed with TIMEOUT
    Mock::given(method("POST"))
        .and(path("/api/v1/workers/poll"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "activities": [{
                "activity_id": activity_id,
                "workflow_id": Uuid::now_v7(),
                "activity_key": "slow_step",
                "worker": "demo",
                "activity_name": "slow",
                "parameters": {},
                "settings": null,
                "timeout_seconds": 1,
                "output_definitions": null,
                "signal_data": null
            }],
            "count": 1
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/workers/poll"))
        .respond_with(empty_poll_response())
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/api/v1/activities/{activity_id}/fail")))
        .and(body_partial_json(json!({
            "error": {"code": "TIMEOUT", "retryable": true}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"acknowledged": true})))
        .expect(1)
        .mount(&server)
        .await;

    let worker = Worker::builder()
        .config(test_config(&server))
        .worker("demo")
        .register_fn("slow", |_params: Value, _ctx| async move {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok(ActivityResult::default())
        })
        .build()
        .unwrap();
    let handle = worker.handle();
    let join = spawn_worker(&server, worker).await;

    wait_for_requests(&server, 1, |r| r.url.path().ends_with("/fail")).await;
    handle.shutdown();
    tokio::time::timeout(Duration::from_secs(5), join)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn conflict_on_complete_is_swallowed() {
    let server = MockServer::start().await;
    let activity_id = Uuid::now_v7();

    Mock::given(method("POST"))
        .and(path("/api/v1/workers/poll"))
        .respond_with(one_activity_response(activity_id, "echo", json!({})))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/workers/poll"))
        .respond_with(empty_poll_response())
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/api/v1/activities/{activity_id}/complete")))
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({"error": "already done"})))
        .expect(1)
        .mount(&server)
        .await;

    let worker = Worker::builder()
        .config(test_config(&server))
        .worker("demo")
        .register_fn("echo", |params: Value, _ctx| async move {
            Ok(ActivityResult::value("echoed", params))
        })
        .build()
        .unwrap();
    let handle = worker.handle();
    let join = spawn_worker(&server, worker).await;

    wait_for_requests(&server, 1, |r| r.url.path().ends_with("/complete")).await;
    // Worker keeps running (conflict swallowed) and shuts down cleanly.
    handle.shutdown();
    tokio::time::timeout(Duration::from_secs(5), join)
        .await
        .expect("conflict on complete must not break the worker")
        .unwrap();
}

// =========================================================================
// Graceful shutdown
// =========================================================================

#[tokio::test]
async fn shutdown_drains_in_flight_activity() {
    let server = MockServer::start().await;
    let activity_id = Uuid::now_v7();

    Mock::given(method("POST"))
        .and(path("/api/v1/workers/poll"))
        .respond_with(one_activity_response(activity_id, "slowish", json!({})))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/workers/poll"))
        .respond_with(empty_poll_response())
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/api/v1/activities/{activity_id}/complete")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"acknowledged": true})))
        .expect(1)
        .mount(&server)
        .await;

    let worker = Worker::builder()
        .config(test_config(&server))
        .worker("demo")
        .register_fn("slowish", |_params: Value, _ctx| async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            Ok(ActivityResult::value("done", json!(true)))
        })
        .build()
        .unwrap();
    let handle = worker.handle();
    let join = spawn_worker(&server, worker).await;

    // Wait until the activity is in flight, then shut down mid-execution.
    wait_for_requests(&server, 1, |r| r.url.path() == "/api/v1/workers/poll").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    handle.shutdown();

    tokio::time::timeout(Duration::from_secs(5), join)
        .await
        .expect("worker should drain and stop")
        .unwrap();

    // The in-flight activity completed during the drain.
    let completes = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path().ends_with("/complete"))
        .count();
    assert_eq!(completes, 1, "in-flight activity must complete during drain");
}

#[tokio::test]
async fn drain_deadline_fails_stuck_activity_as_retryable() {
    let server = MockServer::start().await;
    let activity_id = Uuid::now_v7();

    Mock::given(method("POST"))
        .and(path("/api/v1/workers/poll"))
        .respond_with(one_activity_response(activity_id, "stuck", json!({})))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/workers/poll"))
        .respond_with(empty_poll_response())
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/api/v1/activities/{activity_id}/fail")))
        .and(body_partial_json(json!({
            "error": {"code": "WORKER_SHUTDOWN", "retryable": true}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"acknowledged": true})))
        .expect(1)
        .mount(&server)
        .await;

    let config = WorkerConfig {
        shutdown_timeout: Duration::from_millis(200),
        ..test_config(&server)
    };
    let worker = Worker::builder()
        .config(config)
        .worker("demo")
        .register_fn("stuck", |_params: Value, _ctx| async move {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok(ActivityResult::default())
        })
        .build()
        .unwrap();
    let handle = worker.handle();
    let join = spawn_worker(&server, worker).await;

    wait_for_requests(&server, 1, |r| r.url.path() == "/api/v1/workers/poll").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    handle.shutdown();

    tokio::time::timeout(Duration::from_secs(5), join)
        .await
        .expect("worker must not hang on a stuck activity")
        .unwrap();

    wait_for_requests(&server, 1, |r| r.url.path().ends_with("/fail")).await;
}

// =========================================================================
// Heartbeats
// =========================================================================

#[tokio::test]
async fn heartbeat_conflict_cancels_execution_without_reporting() {
    let server = MockServer::start().await;
    let activity_id = Uuid::now_v7();

    // Long timeout (> 60s) so the heartbeat task spawns; short heartbeat
    // interval so the first beat arrives immediately.
    Mock::given(method("POST"))
        .and(path("/api/v1/workers/poll"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "activities": [{
                "activity_id": activity_id,
                "workflow_id": Uuid::now_v7(),
                "activity_key": "long_step",
                "worker": "demo",
                "activity_name": "long",
                "parameters": {},
                "settings": null,
                "timeout_seconds": 300,
                "output_definitions": null,
                "signal_data": null
            }],
            "count": 1
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/workers/poll"))
        .respond_with(empty_poll_response())
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/api/v1/activities/{activity_id}/heartbeat")))
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({"error": "reassigned"})))
        .mount(&server)
        .await;

    let config = WorkerConfig {
        heartbeat_interval: Duration::from_millis(50),
        ..test_config(&server)
    };
    let worker = Worker::builder()
        .config(config)
        .worker("demo")
        .register_fn("long", |_params: Value, _ctx| async move {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok(ActivityResult::default())
        })
        .build()
        .unwrap();
    let handle = worker.handle();
    let join = spawn_worker(&server, worker).await;

    wait_for_requests(&server, 1, |r| r.url.path().ends_with("/heartbeat")).await;
    // Give the cancellation a moment, then shut down; the drain must be
    // immediate because the reassigned activity was cancelled locally.
    tokio::time::sleep(Duration::from_millis(200)).await;
    handle.shutdown();
    tokio::time::timeout(Duration::from_secs(5), join)
        .await
        .expect("reassigned activity must not block shutdown")
        .unwrap();

    // Neither complete nor fail was reported for the reassigned activity.
    let requests = server.received_requests().await.unwrap();
    assert!(
        requests
            .iter()
            .all(|r| !r.url.path().ends_with("/complete") && !r.url.path().ends_with("/fail")),
        "reassigned activity must not be reported"
    );
}

#[tokio::test]
async fn heartbeats_sent_for_long_activities() {
    let server = MockServer::start().await;
    let activity_id = Uuid::now_v7();

    Mock::given(method("POST"))
        .and(path("/api/v1/workers/poll"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "activities": [{
                "activity_id": activity_id,
                "workflow_id": Uuid::now_v7(),
                "activity_key": "long_step",
                "worker": "demo",
                "activity_name": "long",
                "parameters": {},
                "settings": null,
                "timeout_seconds": 300,
                "output_definitions": null,
                "signal_data": null
            }],
            "count": 1
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/workers/poll"))
        .respond_with(empty_poll_response())
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/api/v1/activities/{activity_id}/heartbeat")))
        .and(header_exists("content-type"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "acknowledged": true,
            "next_heartbeat_seconds": 30
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/api/v1/activities/{activity_id}/complete")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"acknowledged": true})))
        .expect(1)
        .mount(&server)
        .await;

    let config = WorkerConfig {
        heartbeat_interval: Duration::from_millis(50),
        ..test_config(&server)
    };
    let worker = Worker::builder()
        .config(config)
        .worker("demo")
        .register_fn("long", |_params: Value, _ctx| async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            Ok(ActivityResult::value("done", json!(true)))
        })
        .build()
        .unwrap();
    let handle = worker.handle();
    let join = spawn_worker(&server, worker).await;

    wait_for_requests(&server, 1, |r| r.url.path().ends_with("/complete")).await;
    handle.shutdown();
    tokio::time::timeout(Duration::from_secs(5), join)
        .await
        .unwrap()
        .unwrap();

    let heartbeats = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path().ends_with("/heartbeat"))
        .count();
    assert!(
        heartbeats >= 2,
        "expected multiple heartbeats, got {heartbeats}"
    );
}
