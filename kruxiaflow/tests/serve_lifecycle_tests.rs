//! Boots real service stacks as subprocesses against the test database and
//! exercises startup, health surfaces, and the graceful-shutdown paths that
//! unit tests cannot reach (`serve::execute` + spawn helpers, the
//! `orchestrator` launcher's run-until-SIGTERM path).
//!
//! Requires DATABASE_URL (provided by scripts/test.sh and CI); tests skip
//! with a notice when it is absent. Unix-only: shutdown is driven by SIGTERM.

#![cfg(unix)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

// Committed test-only RSA keypair, shared with the api crate's auth tests
const PRIVATE_KEY: &str = include_str!("../../oauth/tests/private.pem");
const PUBLIC_KEY: &str = include_str!("../../oauth/tests/public.pem");

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

/// Minimal HTTP/1.0 GET, avoiding any new dev-dependencies
fn http_get(port: u16, path: &str) -> Option<(u16, String)> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    write!(
        stream,
        "GET {path} HTTP/1.0\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    let status: u16 = response.split_whitespace().nth(1)?.parse().ok()?;
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default();
    Some((status, body))
}

/// Collect whatever the child wrote to stderr (for failure diagnostics)
fn drain_stderr(child: &mut Child) -> String {
    let _ = child.kill();
    let _ = child.wait();
    let mut out = String::new();
    if let Some(stderr) = child.stderr.as_mut() {
        let _ = stderr.read_to_string(&mut out);
    }
    out
}

fn sigterm(child: &Child) {
    let status = Command::new("kill")
        .arg(child.id().to_string())
        .status()
        .expect("run kill");
    assert!(status.success(), "kill failed");
}

/// Wait for the child to exit within `timeout`, asserting success
fn await_graceful_exit(child: &mut Child, timeout: Duration, what: &str) {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            assert!(
                status.success(),
                "{what} exited non-zero after SIGTERM: {status:?}\nstderr:\n{}",
                drain_stderr(child)
            );
            return;
        }
        if Instant::now() > deadline {
            let stderr = drain_stderr(child);
            panic!("{what} did not shut down within {timeout:?}\nstderr:\n{stderr}");
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
#[serial_test::serial]
fn serve_boots_serves_health_and_shuts_down_gracefully() {
    let Ok(db_url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping serve lifecycle test: DATABASE_URL not set");
        return;
    };

    let port = free_port();
    let mut child = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .args([
            "serve",
            "--insecure-dev",
            "--no-worker",
            "--bind",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--shutdown-timeout",
            "5",
        ])
        .env("DATABASE_URL", &db_url)
        .env("KRUXIAFLOW_OAUTH_RSA_PRIVATE_KEY_PEM", PRIVATE_KEY)
        .env("KRUXIAFLOW_OAUTH_RSA_PUBLIC_KEY_PEM", PUBLIC_KEY)
        .env("KRUXIAFLOW_CLIENT_SECRET", "serve-lifecycle-test")
        .env("KRUXIAFLOW_MCP_ENABLED", "false")
        .env_remove("KRUXIAFLOW_CACHE_PROVIDER") // default: noop
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn serve");

    // Wait for liveness (debug binaries under coverage start slowly)
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut live = false;
    while Instant::now() < deadline {
        if let Some((200, _)) = http_get(port, "/health") {
            live = true;
            break;
        }
        if child.try_wait().expect("try_wait").is_some() {
            panic!(
                "serve exited before becoming live\nstderr:\n{}",
                drain_stderr(&mut child)
            );
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    if !live {
        let stderr = drain_stderr(&mut child);
        panic!("serve did not become live within 60s\nstderr:\n{stderr}");
    }

    // Readiness carries the component-object contract the health CLI parses
    let (status, body) = http_get(port, "/health/ready").expect("readiness reachable");
    assert_eq!(status, 200, "readiness body: {body}");
    for component in [
        "\"database\"",
        "\"event_source\"",
        "\"queue\"",
        "\"orchestrator\"",
    ] {
        assert!(
            body.contains(component),
            "readiness missing {component}: {body}"
        );
    }

    // Service info is served without auth
    let (status, body) = http_get(port, "/api/v1/info").expect("info reachable");
    assert_eq!(status, 200, "info body: {body}");
    assert!(body.contains("\"insecure_dev\":true"), "info body: {body}");

    // Boot a standalone worker against the running dev-mode server (its poll
    // loop authenticates lazily; dev mode accepts credential-less polls) and
    // drain it gracefully — this covers the `worker` launcher's execute +
    // shutdown paths, which need a live API server.
    let mut worker = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .args([
            "worker",
            "--api-url",
            &format!("http://127.0.0.1:{port}"),
            "--client-secret",
            "serve-lifecycle-test",
            "--shutdown-timeout",
            "5",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn worker");

    // Let it run a few poll cycles; an early exit is a failure
    let worker_deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < worker_deadline {
        if worker.try_wait().expect("try_wait").is_some() {
            panic!(
                "worker exited early\nstderr:\n{}",
                drain_stderr(&mut worker)
            );
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    sigterm(&worker);
    await_graceful_exit(&mut worker, Duration::from_secs(30), "worker");

    // Graceful shutdown: SIGTERM must drain and exit zero
    sigterm(&child);
    await_graceful_exit(&mut child, Duration::from_secs(30), "serve");
}

#[test]
#[serial_test::serial]
fn orchestrator_launcher_boots_and_shuts_down_gracefully() {
    let Ok(db_url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping orchestrator lifecycle test: DATABASE_URL not set");
        return;
    };

    let mut child = Command::new(env!("CARGO_BIN_EXE_kruxiaflow"))
        .args(["orchestrator", "--shutdown-timeout", "5"])
        .env("DATABASE_URL", &db_url)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn orchestrator");

    // Give it time to connect and enter the poll loop (it prints readiness to
    // logs only; absence of an early exit is the observable signal here)
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if child.try_wait().expect("try_wait").is_some() {
            panic!(
                "orchestrator exited early\nstderr:\n{}",
                drain_stderr(&mut child)
            );
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    sigterm(&child);
    await_graceful_exit(&mut child, Duration::from_secs(30), "orchestrator");
}
