# kruxiaflow-worker

[![crates.io](https://img.shields.io/crates/v/kruxiaflow-worker.svg)](https://crates.io/crates/kruxiaflow-worker)
[![docs.rs](https://img.shields.io/docsrs/kruxiaflow-worker)](https://docs.rs/kruxiaflow-worker)

Worker SDK for [Kruxia Flow](https://github.com/kruxia/kruxiaflow) — run
Rust activities in **budgeted workflows** with hard cost limits enforced in
the engine.

A Kruxia Flow worker polls the server for queued activities, executes your
registered handlers, and reports results — including per-LLM-call usage, so
activities running in your own process count against workflow budgets with
the same fidelity as built-in activities. This crate is the complete worker
loop: registration, polling, bounded concurrency, heartbeats, timeout and
panic containment, OAuth2 client-credentials auth, and graceful drain on
shutdown. The same crate powers Kruxia Flow's own built-in `std` worker.

## Quickstart

Start a server locally ([5-minute quickstart](https://github.com/kruxia/kruxiaflow#quickstart)),
or in dev mode with no auth:

```sh
kruxiaflow serve --insecure-dev
```

Add the SDK to a binary crate:

```sh
cargo add kruxiaflow-worker tokio serde serde_json anyhow
```

Register a handler and run:

```rust,no_run
use kruxiaflow_worker::{ActivityContext, ActivityResult, Worker};
use serde_json::json;

#[derive(serde::Deserialize)]
struct EchoParams {
    message: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let worker = Worker::builder()
        .worker("demo") // the workflow definition's `worker:` field
        .register_fn("echo", |params: EchoParams, _ctx: ActivityContext| async move {
            Ok(ActivityResult::value("echoed", json!(params.message)))
        })
        .build()?; // config from KRUXIAFLOW_* environment variables

    worker.run_until_shutdown().await;
    Ok(())
}
```

```sh
KRUXIAFLOW_API_URL=http://localhost:8080 cargo run
```

Any workflow activity declaring `worker: demo` / `name: echo` now executes
on your worker:

```yaml
name: hello-sdk
activities:
  greet:
    worker: demo
    name: echo
    parameters:
      message: "hello from Rust"
```

## Reporting cost and usage

Cost governance is the point: budgets live in the engine, and what your
activity reports is what the engine enforces. An activity that calls an LLM
itself attaches one `UsageEntry` per call — the server prices it from its
model catalog, records it exactly like a built-in LLM activity (visible in
`/cost/history` and `/cost/analytics`), and counts it against the workflow's
budget:

```rust
use kruxiaflow_worker::{ActivityResult, UsageEntry};
use serde_json::json;

# fn f() -> ActivityResult {
ActivityResult::value("summary", json!("..."))
    .push_usage(
        UsageEntry::new("anthropic", "claude-sonnet-5")
            .input_tokens(12034)
            .output_tokens(512)
            .cache_read_tokens(9800),
    )
# }
```

- `UsageEntry::cost_usd(...)` overrides catalog pricing for a single call —
  use it for costs the catalog can't model (e.g., time-based cache-storage
  billing).
- `ActivityResult::with_cost(...)` reports spend *not* covered by usage
  entries (a paid non-LLM API). Never repeat entry costs there.
- Failures spend money too: `ActivityError::with_usage(...)` /
  `.with_cost(...)` make failed attempts budget-counted.

## Failure semantics

Handlers return `Result<ActivityResult, ActivityError>`:

| Failure                                       | Reported as                        |
|-----------------------------------------------|------------------------------------|
| `ActivityError::retryable(code, msg)`         | re-queued up to the retry limit    |
| `ActivityError::terminal(code, msg)`          | no retry                           |
| parameter deserialization failure             | terminal `INVALID_PARAMETERS`      |
| handler panic                                 | retryable `PANIC` (worker survives)|
| timeout (`settings.timeout` or worker default)| retryable `TIMEOUT`                |

## Configuration

Environment variables (matching the Python SDK), overridable via
`WorkerConfig::builder()`:

| Variable                                | Required | Default        |
|-----------------------------------------|----------|----------------|
| `KRUXIAFLOW_API_URL`                    | yes      | —              |
| `KRUXIAFLOW_CLIENT_ID`                  | yes*     | —              |
| `KRUXIAFLOW_CLIENT_SECRET`              | yes*     | —              |
| `KRUXIAFLOW_WORKER`                     | no       | inferred from registered activities |
| `KRUXIAFLOW_WORKER_ID`                  | no       | auto-generated |
| `KRUXIAFLOW_WORKER_POLL_MAX_ACTIVITIES` | no       | 10             |
| `KRUXIAFLOW_WORKER_POLL_INTERVAL`       | no       | 0.1 (seconds)  |
| `KRUXIAFLOW_WORKER_MAX_ACTIVITIES`      | no       | 16             |
| `KRUXIAFLOW_WORKER_ACTIVITY_TIMEOUT`    | no       | 300 (seconds)  |
| `KRUXIAFLOW_WORKER_HEARTBEAT_INTERVAL`  | no       | 30 (seconds)   |
| `KRUXIAFLOW_WORKER_SHUTDOWN_TIMEOUT`    | no       | 30 (seconds)   |

\* not required against a dev-mode server (`kruxiaflow serve --insecure-dev`).

## Graceful shutdown

`run_until_shutdown()` listens for SIGINT/SIGTERM; `worker.handle()` gives a
programmatic trigger (k8s preStop, tests). On shutdown the worker stops
polling, drains in-flight activities up to `shutdown_timeout`, then fails
whatever remains as retryable so it re-queues on another worker — nothing is
lost or double-completed.

## Examples

- [`echo_worker`](examples/echo_worker.rs) — minimal worker, one typed handler
- [`llm_usage_worker`](examples/llm_usage_worker.rs) — reporting per-call LLM usage and extra costs
- [`graceful_drain`](examples/graceful_drain.rs) — SIGTERM drain behavior with a slow activity

Run one against a local dev-mode server:

```sh
KRUXIAFLOW_API_URL=http://localhost:8080 cargo run --example echo_worker
```

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
