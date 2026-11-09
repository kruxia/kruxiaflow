//! The worker loop and the server's wake stream
//! (`GET /api/v1/workers/wake`), against a mock server that can hold a
//! server-sent event stream open, send wakes on demand, drop the stream, and
//! script the answers to polls.
//!
//! Every test sets `wake_poll_interval` far longer than the test runs, so a
//! poll that arrives while the stream is connected can only have been caused
//! by a wake, by the next-due hint, or by a dropped stream. Time bounds are
//! generous: they separate "at once" from "after 30 s", not 5 ms from 50 ms.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use kruxiaflow_worker::{ActivityResult, WakeMode, Worker, WorkerConfig};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;
use tokio::time::Instant;

/// What the mock answers to one poll
#[derive(Clone, Copy)]
enum PollReply {
    /// Nothing to claim
    Empty,
    /// Nothing to claim, and a delayed activity is due in this many ms
    EmptyDueIn(u64),
    /// Hold the request this long, then answer that there is nothing to claim
    EmptyAfter(Duration),
}

struct MockInner {
    /// When each poll request arrived
    polls: Mutex<Vec<Instant>>,
    /// Answers for the next polls; `Empty` once exhausted
    script: Mutex<VecDeque<PollReply>>,
    /// Status the wake endpoint answers with (200 = stream)
    wake_status: AtomicU16,
    streams_opened: AtomicUsize,
    /// Streams whose client went away or that the server ended
    streams_closed: AtomicUsize,
    /// Bump to send a wake on every open stream
    wake: watch::Sender<u64>,
    /// Bump to end every open stream
    drop_streams: watch::Sender<u64>,
}

#[derive(Clone)]
struct MockServer {
    inner: Arc<MockInner>,
    url: String,
}

impl MockServer {
    async fn start(wake_status: u16) -> Self {
        let inner = Arc::new(MockInner {
            polls: Mutex::new(Vec::new()),
            script: Mutex::new(VecDeque::new()),
            wake_status: AtomicU16::new(wake_status),
            streams_opened: AtomicUsize::new(0),
            streams_closed: AtomicUsize::new(0),
            wake: watch::channel(0).0,
            drop_streams: watch::channel(0).0,
        });

        let app = Router::new()
            .route("/api/v1/workers/poll", post(poll_handler))
            .route("/api/v1/workers/wake", get(wake_handler))
            .with_state(inner.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        Self { inner, url }
    }

    fn poll_count(&self) -> usize {
        self.inner.polls.lock().unwrap().len()
    }

    fn poll_times(&self) -> Vec<Instant> {
        self.inner.polls.lock().unwrap().clone()
    }

    fn streams_opened(&self) -> usize {
        self.inner.streams_opened.load(Ordering::SeqCst)
    }

    fn streams_closed(&self) -> usize {
        self.inner.streams_closed.load(Ordering::SeqCst)
    }

    fn script(&self, replies: &[PollReply]) {
        self.inner.script.lock().unwrap().extend(replies);
    }

    fn send_wake(&self) {
        self.inner.wake.send_modify(|n| *n += 1);
    }

    fn drop_streams(&self) {
        self.inner.drop_streams.send_modify(|n| *n += 1);
    }

    /// Wait until at least `count` polls have arrived, or panic
    async fn wait_for_polls(&self, count: usize, within: Duration, what: &str) {
        let deadline = Instant::now() + within;
        while self.poll_count() < count {
            assert!(
                Instant::now() < deadline,
                "{what}: expected {count} poll(s) within {within:?}, saw {}",
                self.poll_count()
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// Wait until a stream is open and the loop has gone quiet on its long
    /// timer: the worker is connected and idle
    async fn wait_until_connected_and_idle(&self, streams: usize) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.streams_opened() < streams {
            assert!(Instant::now() < deadline, "wake stream was never opened");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        // The wake sent on connect causes one more poll; then silence
        loop {
            let before = self.poll_count();
            tokio::time::sleep(Duration::from_millis(300)).await;
            if self.poll_count() == before {
                return;
            }
            assert!(Instant::now() < deadline, "loop never went idle");
        }
    }
}

async fn poll_handler(State(mock): State<Arc<MockInner>>) -> Json<Value> {
    mock.polls.lock().unwrap().push(Instant::now());
    let reply = mock
        .script
        .lock()
        .unwrap()
        .pop_front()
        .unwrap_or(PollReply::Empty);

    match reply {
        PollReply::Empty => Json(json!({"activities": [], "count": 0})),
        PollReply::EmptyDueIn(ms) => Json(json!({"activities": [], "count": 0, "next_due_ms": ms})),
        PollReply::EmptyAfter(hold) => {
            tokio::time::sleep(hold).await;
            Json(json!({"activities": [], "count": 0}))
        }
    }
}

/// Counts a stream as closed when its state is dropped: the stream ended, or
/// the client disconnected and the server dropped the response body
struct CountClose(Arc<MockInner>);

impl Drop for CountClose {
    fn drop(&mut self) {
        self.0.streams_closed.fetch_add(1, Ordering::SeqCst);
    }
}

async fn wake_handler(State(mock): State<Arc<MockInner>>) -> Response {
    let status = mock.wake_status.load(Ordering::SeqCst);
    if status != 200 {
        return StatusCode::from_u16(status).unwrap().into_response();
    }
    mock.streams_opened.fetch_add(1, Ordering::SeqCst);

    let wakes = mock.wake.subscribe();
    let drops = mock.drop_streams.subscribe();
    let closed = CountClose(mock.clone());
    let stream = futures::stream::unfold(
        (wakes, drops, true, closed),
        |(mut wakes, mut drops, first, closed)| async move {
            if !first {
                tokio::select! {
                    _ = drops.changed() => return None,
                    _ = wakes.changed() => {}
                }
            }
            let event = Event::default().event("wake").data("");
            Some((Ok::<_, Infallible>(event), (wakes, drops, false, closed)))
        },
    );
    Sse::new(stream).into_response()
}

/// Connected: wait 30 s between polls. Not connected: 50 ms.
fn config(server: &MockServer) -> WorkerConfig {
    WorkerConfig {
        api_url: server.url.clone(),
        worker: "demo".to_string(),
        worker_id: "wake_test_worker".to_string(),
        poll_interval: Duration::from_millis(50),
        wake_poll_interval: Duration::from_secs(30),
        shutdown_timeout: Duration::from_secs(5),
        ..WorkerConfig::default()
    }
}

struct Running {
    handle: kruxiaflow_worker::WorkerHandle,
    join: tokio::task::JoinHandle<()>,
}

impl Running {
    async fn stop(self) {
        self.handle.shutdown();
        tokio::time::timeout(Duration::from_secs(5), self.join)
            .await
            .expect("worker should shut down")
            .unwrap();
    }
}

fn start_worker(config: WorkerConfig) -> Running {
    let worker = Worker::builder()
        .config(config)
        .register_fn("noop", |_params: Value, _ctx| async move {
            Ok(ActivityResult::value("ok", json!(true)))
        })
        .build()
        .unwrap();
    let handle = worker.handle();
    let join = tokio::spawn(async move { worker.run().await });
    Running { handle, join }
}

// =========================================================================
// Without a stream: the loop polls on its ordinary timer
// =========================================================================

async fn assert_polls_on_the_timer(server: &MockServer) {
    // 50 ms interval: about 20 polls a second; ask for far fewer
    let before = server.poll_count();
    server
        .wait_for_polls(before + 8, Duration::from_secs(2), "timer polling")
        .await;
}

#[tokio::test]
async fn polls_on_the_timer_when_the_server_has_no_wake_endpoint() {
    let server = MockServer::start(404).await;
    let worker = start_worker(config(&server));

    assert_polls_on_the_timer(&server).await;
    assert_eq!(server.streams_opened(), 0);

    worker.stop().await;
}

#[tokio::test]
async fn polls_on_the_timer_when_the_server_has_no_wake_signal() {
    let server = MockServer::start(503).await;
    let worker = start_worker(config(&server));

    assert_polls_on_the_timer(&server).await;
    assert_eq!(server.streams_opened(), 0);

    worker.stop().await;
}

#[tokio::test]
async fn wake_mode_poll_never_opens_the_stream() {
    let server = MockServer::start(200).await;
    let worker = start_worker(WorkerConfig {
        wake: WakeMode::Poll,
        ..config(&server)
    });

    assert_polls_on_the_timer(&server).await;
    assert_eq!(server.streams_opened(), 0);

    worker.stop().await;
}

// =========================================================================
// With a stream
// =========================================================================

#[tokio::test]
async fn connected_stream_replaces_the_short_timer_and_a_wake_ends_the_wait() {
    let server = MockServer::start(200).await;
    let worker = start_worker(config(&server));
    server.wait_until_connected_and_idle(1).await;

    // Connected and idle: no polling on the 50 ms timer
    let idle = server.poll_count();
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(server.poll_count(), idle, "polled while connected and idle");

    // A wake: polled at once
    server.send_wake();
    server
        .wait_for_polls(idle + 1, Duration::from_secs(2), "after a wake")
        .await;

    // And then idle again
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(server.poll_count(), idle + 1);

    worker.stop().await;
}

#[tokio::test]
async fn next_due_hint_bounds_the_wait() {
    let server = MockServer::start(200).await;
    let worker = start_worker(config(&server));
    server.wait_until_connected_and_idle(1).await;
    let idle = server.poll_count();

    // The next poll is told a delayed activity is due in 400 ms
    server.script(&[PollReply::EmptyDueIn(400)]);
    server.send_wake();
    server
        .wait_for_polls(idle + 1, Duration::from_secs(2), "after a wake")
        .await;

    // The loop comes back when it is due: not at once, not after 30 s
    server
        .wait_for_polls(idle + 2, Duration::from_secs(3), "when the activity is due")
        .await;
    let times = server.poll_times();
    let waited = times[idle + 1] - times[idle];
    assert!(
        waited >= Duration::from_millis(350),
        "polled {waited:?} after being told 400 ms"
    );

    // No hint in that answer: back to waiting for a wake
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(server.poll_count(), idle + 2);

    worker.stop().await;
}

/// A wake that arrives while a poll is in flight must not be lost: the poll
/// may already have looked, so the loop has to look again
#[tokio::test]
async fn wake_during_a_poll_is_not_lost() {
    let server = MockServer::start(200).await;
    let worker = start_worker(config(&server));
    server.wait_until_connected_and_idle(1).await;
    let idle = server.poll_count();

    // The next poll takes 500 ms to answer "nothing"
    server.script(&[PollReply::EmptyAfter(Duration::from_millis(500))]);
    server.send_wake();
    server
        .wait_for_polls(idle + 1, Duration::from_secs(2), "the slow poll")
        .await;

    // A second wake lands while that poll is still being answered
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(server.poll_count(), idle + 1);
    server.send_wake();

    // The loop polls again as soon as the slow poll returns
    server
        .wait_for_polls(
            idle + 2,
            Duration::from_secs(3),
            "after the wake during a poll",
        )
        .await;

    // Exactly once more
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(server.poll_count(), idle + 2);

    worker.stop().await;
}

/// A dropped stream may have swallowed wakes, so it counts as one; and until
/// the stream is back, nothing would announce work, so the loop returns to
/// its short timer
#[tokio::test]
async fn dropped_stream_counts_as_a_wake_and_falls_back_until_reconnected() {
    let server = MockServer::start(200).await;
    let worker = start_worker(config(&server));
    server.wait_until_connected_and_idle(1).await;
    let idle = server.poll_count();

    server.drop_streams();

    // Polled at once, and repeatedly while disconnected (50 ms timer; the
    // reconnect comes after about 1 s)
    server
        .wait_for_polls(idle + 5, Duration::from_secs(2), "while disconnected")
        .await;

    // It reconnects, and goes quiet again
    server.wait_until_connected_and_idle(2).await;
    let idle = server.poll_count();
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(
        server.poll_count(),
        idle,
        "polled while reconnected and idle"
    );

    // The new stream delivers wakes
    server.send_wake();
    server
        .wait_for_polls(idle + 1, Duration::from_secs(2), "wake on the new stream")
        .await;

    worker.stop().await;
}

#[tokio::test]
async fn shutdown_interrupts_the_long_wait() {
    let server = MockServer::start(200).await;
    let worker = start_worker(config(&server));
    server.wait_until_connected_and_idle(1).await;

    // The loop is waiting up to 30 s; shutdown must not wait for that
    let started = Instant::now();
    worker.stop().await;
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "shutdown took {:?}",
        started.elapsed()
    );
}

/// The graceful path closes the stream too
#[tokio::test]
async fn shutdown_closes_the_wake_stream() {
    let server = MockServer::start(200).await;
    let worker = start_worker(config(&server));
    server.wait_until_connected_and_idle(1).await;
    assert_eq!(server.streams_closed(), 0);

    worker.stop().await;

    let deadline = Instant::now() + Duration::from_secs(3);
    while server.streams_closed() < 1 {
        assert!(
            Instant::now() < deadline,
            "stream still open after shutdown"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Aborting the task that runs the worker (how the built-in worker is
/// stopped) must not leave the stream task running with its connection open
#[tokio::test]
async fn aborting_the_worker_task_closes_the_wake_stream() {
    let server = MockServer::start(200).await;
    let worker = start_worker(config(&server));
    server.wait_until_connected_and_idle(1).await;
    assert_eq!(server.streams_closed(), 0);

    worker.join.abort();

    let deadline = Instant::now() + Duration::from_secs(3);
    while server.streams_closed() < 1 {
        assert!(
            Instant::now() < deadline,
            "stream still open after the worker task was aborted"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // And nothing reconnects
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(server.streams_opened(), 1);
}
