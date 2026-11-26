# kruxiaflow-monty

Sandboxed Python for [Kruxia Flow](https://kruxiaflow.com): the `run_code` activity on [Pydantic Monty](https://github.com/pydantic/monty), as an add-on worker.

An LLM writes a step as Python, and `run_code` runs it with no filesystem, network or environment, only the host functions ("capabilities") the step grants, under hard limits on memory, time, recursion and host calls. Errors complete the step with a message for the model, so a budgeted loop can let the model repair its own code. Every run records what it was allowed to use and what it used.

The add-on runs beside the engine like any external worker, on the [worker SDK](https://docs.rs/kruxiaflow-worker). It is its own Cargo workspace, so Monty's toolchain (Rust 1.98+) and exact version pins never reach the engine: the engine's binary and image are the same with or without it.

The full guide, covering parameters, the result shape, capabilities, the Python subset, the repair-loop pattern and the security model, is [docs/run-code.md](../../docs/run-code.md).

## Run it

**Docker**, built from the repository root:

```bash
docker build -f contrib/kruxiaflow-monty/Dockerfile -t kruxia/kruxiaflow-monty .
docker run -e KRUXIAFLOW_API_URL=http://kruxiaflow:8080 \
  -e KRUXIAFLOW_CLIENT_ID=monty-worker -e KRUXIAFLOW_CLIENT_SECRET=... \
  kruxia/kruxiaflow-monty
```

**From source**:

```bash
cargo install monty-runtime --version 1.7.0 --locked    # the sandbox: ~/.cargo/bin/monty
cargo install --path contrib/kruxiaflow-monty --locked   # this worker

KRUXIAFLOW_API_URL=http://localhost:8080 \
KRUXIAFLOW_MONTY_BINARY=$HOME/.cargo/bin/monty \
  kruxiaflow-monty
```

`run_code` steps name the add-on's worker type, `monty` by default (`KRUXIAFLOW_WORKER`):

```yaml
- key: run_rule
  worker: monty
  activity_name: run_code
  parameters:
    code: "{{write_rule.result.content}}"
```

Settings: the worker SDK's `KRUXIAFLOW_API_URL`, `KRUXIAFLOW_CLIENT_ID` / `_SECRET` and `KRUXIAFLOW_WORKER_*`, and the sandbox's `KRUXIAFLOW_MONTY_*` (binary, pool size, limit ceilings). They're listed in [docs/run-code.md](../../docs/run-code.md#configuration) and in `kruxiaflow-monty --help`.

## Tell agents about it

The engine's MCP server lists `run_code` when it is given this worker's contract:

```bash
kruxiaflow-monty contract > /etc/kruxiaflow/run-code-contract.json
kruxiaflow serve --mcp-enabled --mcp-run-code-contract /etc/kruxiaflow/run-code-contract.json
```

The contract names the worker type and lists the capabilities with their Python signatures, the importable modules and this worker's limit ceilings. Write it again after changing the limits.

## Examples

- **[16a](examples/16a-run-code-order-rules.yaml)** runs a fixed business rule ("flag orders over $500 from first-time customers on a weekend") with `Decimal` money handling, `log()` and `now()`. `check_orders.result` is `{"ok": true, "value": {"flagged": ["A1"], "checked": 3, ...}, "logs": ["flagged A1: 612.5"], "run": {...}}`, where `run` records the limits the code ran under and what it used.
- **[16b](examples/16b-run-code-repair-loop.yaml)** has a model write the rule and repairs it in a loop. `run_code` completes with `result.ok == false` and a model-facing error, and `write_rule` runs again with that error, up to `iteration_limit: 3`. It uses a local Ollama model (`ollama/qwen2.5-coder:7b`); any `llm_prompt` model works.

```text
describe → write_rule ⇄ run_rule → done
  (echo)  (llm_prompt)  (run_code)  (echo, when result.ok)
```

Deploy and run 16a with the engine's API, with this worker running:

```bash
curl -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Authorization: Bearer $TOKEN" \
  --data-binary @contrib/kruxiaflow-monty/examples/16a-run-code-order-rules.yaml

curl -X POST http://localhost:8080/api/v1/workflows \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"definition_name": "run_code_order_rules", "input": {"orders": [
        {"id": "A1", "total": 612.50, "first_order": true, "weekday": "Sat"},
        {"id": "A2", "total": 80.00, "first_order": true, "weekday": "Sun"},
        {"id": "A3", "total": 900.00, "first_order": false, "weekday": "Sat"}]}}'
```

## Develop

From this directory:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
KRUXIAFLOW_MONTY_BINARY=$HOME/.cargo/bin/monty DATABASE_URL=postgres://... cargo nextest run
```

- The sandbox integration tests run real `monty` processes, and skip without `KRUXIAFLOW_MONTY_BINARY`.
- `example_16_run_code_e2e_test` runs 16a with the engine's API and orchestrator in-process and this worker, and needs `DATABASE_URL`. It compiles the engine crates: with `SQLX_OFFLINE=true`, set `SQLX_OFFLINE_DIR` to the repository's `.sqlx`.
- CI runs all of it in the `Monty add-on (run_code)` job.

Keep `MONTY_VERSION` in step in three places: `src/run_code/config.rs`, this directory's `Dockerfile`, and the CI job. The `monty-pool` and `monty-types` pins in `Cargo.toml` must match it.
