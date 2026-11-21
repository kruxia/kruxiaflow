# Development Setup

This page covers local tooling beyond the basics in the README (Docker, Rust, `sqlx-cli`), the optional build features, and the settings they add. For running the test suite, see [Testing](testing.md). For contribution workflow, see [CONTRIBUTING.md](../CONTRIBUTING.md).

## Toolchain

| Tool             | Version                           | Install                                                            | Used for                                                        |
|------------------|-----------------------------------|--------------------------------------------------------------------|-----------------------------------------------------------------|
| Rust             | latest stable (CI uses stable)    | `rustup update stable`                                             | Building; the `monty` feature needs 1.98 or newer               |
| `sqlx-cli`       | 0.8.6                             | `cargo install sqlx-cli --no-default-features --features postgres` | Migrations, the offline query cache (`scripts/sqlx-prepare.sh`) |
| `cargo-nextest`  | any recent                        | `brew install cargo-nextest` or `cargo install cargo-nextest`      | Faster test runs; `scripts/test.sh` uses it when present        |
| `cargo-llvm-cov` | any recent                        | `brew install cargo-llvm-cov` or `cargo install cargo-llvm-cov`    | Coverage (`scripts/test.sh --coverage`)                         |
| `monty`          | 1.6.0, exactly the pinned version | `cargo install monty-runtime --version 1.6.0 --locked`             | The `run_code` activity (optional `monty` feature)              |

## Test database on a non-default port

`scripts/test.sh` sets up `kruxiaflow_test` on port 5432. If something else already listens there (a local PostgreSQL service, for example), start the compose database on another port and point the tests at it with `--skip-db-setup`:

```bash
POSTGRES_PORT=5433 docker compose up -d postgres
docker compose exec -T postgres psql -U kruxiaflow -c "CREATE DATABASE kruxiaflow_test;"
DATABASE_URL=postgres://kruxiaflow:kruxiaflow-local@127.0.0.1:5433/kruxiaflow_test sqlx migrate run
DATABASE_URL=postgres://kruxiaflow:kruxiaflow-local@127.0.0.1:5433/kruxiaflow_test ./scripts/test.sh --skip-db-setup
```

`kruxiaflow-local` is the compose file's default `POSTGRES_PASSWORD`.

## Optional build features

| Feature       | Crate        | Adds                                                                                    |
|---------------|--------------|-----------------------------------------------------------------------------------------|
| `mcp-server`  | `kruxiaflow` | The MCP server for AI agents ([MCP Server](mcp-server.md))                              |
| `redis-cache` | `kruxiaflow` | Redis-backed result caching                                                             |
| `monty`       | `kruxiaflow` | The sandboxed `std.run_code` activity on Pydantic Monty ([Sandboxed Code](run-code.md)) |
| `profiling`   | `kruxiaflow` | jemalloc heap profiling and extra tracing ([PROFILING.md](../PROFILING.md))             |

## The `monty` feature (`run_code`)

```bash
# 1. The Monty worker binary, exactly the version the build pins
cargo install monty-runtime --version 1.6.0 --locked      # -> ~/.cargo/bin/monty

# 2. Build with the feature
cargo build --features monty

# 3. Enable it at run time (absolute path; never looked up on PATH)
KRUXIAFLOW_MONTY_BINARY=$HOME/.cargo/bin/monty ./target/debug/kruxiaflow serve --insecure-dev
```

The worker checks the binary's version at start-up and refuses to start on a mismatch. When Monty is upgraded, bump the `=x.y.z` pins in `worker/Cargo.toml`, `MONTY_VERSION` in `worker/src/activities/run_code/config.rs`, and the install commands in these docs together, then recheck the list of available standard-library modules in [Sandboxed Code](run-code.md#what-python-is-available).

Settings (environment variables, with matching `--monty-*` flags on `serve` and `worker`):

| Env var                                 | Default                           | Meaning                                                           |
|-----------------------------------------|-----------------------------------|-------------------------------------------------------------------|
| `KRUXIAFLOW_MONTY_BINARY`               | `/monty` in the image; else unset | Absolute path to the `monty` worker binary. Empty disables it     |
| `KRUXIAFLOW_MONTY_MIN_PROCESSES`        | 1                                 | Worker processes kept warm                                        |
| `KRUXIAFLOW_MONTY_MAX_PROCESSES`        | the worker's `--max-activities`   | Most worker processes alive at once                               |
| `KRUXIAFLOW_MONTY_REQUEST_TIMEOUT_SECS` | 30                                | Kill switch for an unresponsive worker (≥ 2 × max duration + 2 s) |
| `KRUXIAFLOW_MONTY_RECYCLE_AFTER`        | 1                                 | Runs per worker process before it is replaced                     |
| `KRUXIAFLOW_MONTY_MAX_MEMORY_BYTES`     | 67108864                          | Ceiling on a run's memory                                         |
| `KRUXIAFLOW_MONTY_MAX_DURATION_SECS`    | 10                                | Ceiling on a run's execution time and, separately, its sleep      |
| `KRUXIAFLOW_MONTY_MAX_RECURSION_DEPTH`  | 1000                              | Ceiling on a run's recursion depth                                |

Notes:

- The workspace-wide CI jobs build default features. A separate `Monty (run_code)` job runs clippy and the `run_code` tests with the feature on, against a cached build of the pinned `monty` binary. Locally: `cargo clippy -p kruxiaflow -p kruxiaflow-std-worker --features kruxiaflow/monty --all-targets -- -D warnings`, and the test commands in [Sandboxed Code](run-code.md#testing).
- The Docker image builds with `redis-cache,mcp-server,monty` on `rust:1.100`, installs the pinned `monty` binary at `/monty` and sets `KRUXIAFLOW_MONTY_BINARY=/monty`. The develop container (`./docker up --develop`) builds with `--features monty` and has the binary too.
- The standalone release binaries are built without the feature.
