# Sandboxed Code (`run_code`)

`run_code` runs Python that hasn't been reviewed, typically code an LLM step just wrote, in a [Pydantic Monty](https://github.com/pydantic/monty) sandbox. The code has no filesystem, network or environment access. It can call only the host functions ("capabilities") the step names, and it runs under hard limits on memory, time, recursion and host calls. Every run records what it was allowed to use and what it used.

This is "governed code mode": the model writes the step, Kruxia Flow runs it sandboxed and on budget.

`run_code` is an **add-on**, not part of the engine binary: the `kruxiaflow-monty` worker in [`contrib/kruxiaflow-monty`](../contrib/kruxiaflow-monty/). It runs beside the engine like any external worker, and its steps name `worker: monty`. The engine needs nothing extra to run it; Monty's toolchain and dependencies stay in the add-on.

> **`run_code` and `script` serve different code.** The py-std worker's `script` activity runs full CPython, with all builtins, inside the worker process. That is what gives it pandas, DuckDB and similar libraries, and it is built for trusted, developer-written work. Model-written and customer-written code belongs in `run_code`, which runs it sandboxed.

## When to use it

| Use `run_code` for                                            | Use something else for                                             |
|---------------------------------------------------------------|--------------------------------------------------------------------|
| Business rules, transforms and glue code written by an LLM    | Heavy data work with third-party packages (py-std `script`)        |
| Customer-supplied formulas or rules                           | Long-running jobs (minutes); `run_code` runs are capped at seconds |
| A repair loop where a model fixes its own code from the error | Code that must call arbitrary APIs (capabilities are a fixed list) |

## Running the add-on

The add-on is one binary, `kruxiaflow-monty`, plus the `monty` sandbox binary it starts. It polls the engine's API for `run_code` steps, like any worker built on the [worker SDK](https://docs.rs/kruxiaflow-worker).

**Docker.** Build the image from the repository root; it holds both binaries and sets `KRUXIAFLOW_MONTY_BINARY=/monty`:

```bash
docker build -f contrib/kruxiaflow-monty/Dockerfile -t kruxia/kruxiaflow-monty .
docker run -e KRUXIAFLOW_API_URL=http://kruxiaflow:8080 \
  -e KRUXIAFLOW_CLIENT_ID=monty-worker -e KRUXIAFLOW_CLIENT_SECRET=... \
  kruxia/kruxiaflow-monty
```

**From source** (Rust 1.98 or newer; the engine itself doesn't need it):

```bash
cargo install monty-runtime --version 1.7.0 --locked    # installs ~/.cargo/bin/monty
cargo install --path contrib/kruxiaflow-monty --locked   # installs kruxiaflow-monty

KRUXIAFLOW_API_URL=http://localhost:8080 \
KRUXIAFLOW_MONTY_BINARY=$HOME/.cargo/bin/monty \
  kruxiaflow-monty
```

The `monty` binary must be the version the add-on is built against (currently 1.7.0): at start-up the add-on checks `monty --version` and refuses to start with a message saying what to fix. Code runs in pooled `monty` subprocesses, never inside `kruxiaflow-monty`: a crash in the interpreter kills one subprocess, and the pool replaces it.

**Steps name the add-on's worker type.** Workers claim queued steps by worker type, so a `run_code` step says `worker: monty`:

```yaml
- key: run_rule
  worker: monty
  activity_name: run_code
  parameters:
    code: "{{write_rule.result.content}}"
```

**Agents learn about it through the engine's MCP server** when the server is given the add-on's contract (see [Writing `run_code` steps through MCP](#writing-run_code-steps-through-mcp)).

### Configuration

The connection to the engine comes from the worker SDK's settings; the sandbox's from `KRUXIAFLOW_MONTY_*`. Every setting has a matching CLI flag (`kruxiaflow-monty --help`).

| Env var                                 | Default                                     | Meaning                                                                                                                 |
|-----------------------------------------|---------------------------------------------|-------------------------------------------------------------------------------------------------------------------------|
| `KRUXIAFLOW_API_URL`                    | (required)                                  | The engine's API, e.g. `http://localhost:8080`                                                                          |
| `KRUXIAFLOW_CLIENT_ID`, `_SECRET`       | unset                                       | OAuth client credentials (both or neither); unset only against `serve --insecure-dev`                                   |
| `KRUXIAFLOW_WORKER`                     | `monty`                                     | The worker type `run_code` steps name                                                                                   |
| `KRUXIAFLOW_WORKER_MAX_ACTIVITIES`      | the SDK's default                           | `run_code` steps run at once                                                                                            |
| `KRUXIAFLOW_MONTY_BINARY`               | `/monty` in the image; otherwise required   | Absolute path to the `monty` worker binary. Never looked up on `PATH`                                                   |
| `KRUXIAFLOW_MONTY_MIN_PROCESSES`        | 1                                           | Worker processes kept warm                                                                                              |
| `KRUXIAFLOW_MONTY_MAX_PROCESSES`        | `KRUXIAFLOW_WORKER_MAX_ACTIVITIES`          | Most worker processes alive at once                                                                                     |
| `KRUXIAFLOW_MONTY_REQUEST_TIMEOUT_SECS` | 30                                          | Kill switch for a worker that stops answering. Must be at least twice the duration ceiling plus 2 seconds               |
| `KRUXIAFLOW_MONTY_RECYCLE_AFTER`        | 1                                           | Runs per worker process before it is replaced. 1 gives every run a fresh process (nothing carries over between tenants) |
| `KRUXIAFLOW_MONTY_MAX_MEMORY_BYTES`     | 67108864 (64 MB)                            | Ceiling on a run's memory                                                                                               |
| `KRUXIAFLOW_MONTY_MAX_DURATION_SECS`    | 10                                          | Ceiling on a run's execution time, and separately on its total sleep                                                    |
| `KRUXIAFLOW_MONTY_MAX_RECURSION_DEPTH`  | 1000                                        | Ceiling on recursion depth. Higher values let runaway recursion run until the request timeout instead of failing fast   |

The limits are **ceilings**: a workflow may ask for less, never more, and a request above a ceiling is a configuration error. The rest of the SDK's `KRUXIAFLOW_WORKER_*` settings (poll interval, heartbeat, shutdown timeout, wake) apply as for any worker.

## Parameters

```yaml
- key: run_rule
  worker: monty
  activity_name: run_code
  parameters:
    code: "{{write_rule.result.content}}"   # or literal Python
    inputs:
      orders: "{{INPUT.orders}}"            # each key becomes a global
    capabilities: [log, now]                # from the worker's registry
    type_check: true                        # default
    limits:                                 # each optional, at most the ceiling
      max_memory: 16000000
      max_feed_duration_secs: 2
      max_recursion_depth: 500
      max_suspensions: 50
      max_total_sleep_secs: 1
```

| Parameter      | Required | Meaning                                                                                                                                                                                                                                        |
|----------------|----------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `code`         | yes      | Python source. If the whole text is wrapped in one Markdown code fence (```` ```python … ``` ````), the fence is stripped, so an LLM's reply can be passed straight through. The value of the last line, if it is an expression, is the result |
| `inputs`       | no       | JSON object. Each key must be a Python identifier and becomes a global in the code                                                                                                                                                             |
| `capabilities` | no       | Names of host functions the code may call (see [Capabilities](#capabilities)). An unknown name is a configuration error                                                                                                                        |
| `type_check`   | no       | Type-check the code against the inputs and the granted capabilities' signatures before running it (default `true`). Wrong calls then fail before anything runs                                                                                 |
| `limits`       | no       | Per-run limits. A missing limit takes the worker's ceiling                                                                                                                                                                                     |

Unknown parameters, including a misspelled limit, are rejected. (Activity `settings` accept any key, so limits live in `parameters`, where a typo is caught.)

**Templates and literal code.** Parameters are templates. Literal code that contains `{{` or `{%` must be wrapped in `{% raw %}` … `{% endraw %}`. Single braces (dicts, f-strings) are fine. Code inserted with `{{write_rule.result.content}}` is not templated again.

## Result

The step's `result` output always has this shape. The step **completes** for every outcome the code itself causes, including errors and tripped limits, so later steps can branch on `result.ok`.

```json
{
  "ok": true,
  "value": {"flagged": ["A1"], "checked": 3},
  "stdout": "",
  "logs": ["flagged A1: 612.5"],
  "run": {
    "limits": {"max_memory": 16000000, "max_feed_duration_secs": 2.0, "max_recursion_depth": 1000,
               "max_suspensions": 1000, "max_total_sleep_secs": 10.0, "request_timeout_secs": 30.0},
    "consumed": {"execution_ms": 57.7, "host_calls": 2},
    "capabilities": {"granted": ["log", "now"], "called": {"log": 1, "now": 1}},
    "type_check": true,
    "worker_crashes": 0,
    "stdout_truncated": false,
    "logs_truncated": false,
    "monty_version": "monty-runtime 1.7.0",
    "code_sha256": "193c32ff…"
  }
}
```

On failure, `ok` is `false`, `value` is absent, and `error` says what happened, in a message written to be fed back to the model that wrote the code:

```json
{"ok": false, "error": {"kind": "limit", "limit": "max_memory",
 "message": "...MemoryError: memory limit exceeded: 56277178 bytes > 10000000 bytes"}, ...}
```

| `error.kind` | Cause                                                                                                                                                                                                                 |
|--------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `type`       | The type check rejected the code; nothing ran                                                                                                                                                                         |
| `runtime`    | An exception or syntax error, including a call to a capability that was not granted (`NameError`) and an unavailable module (`ModuleNotFoundError`). With type checking on, both are usually caught earlier as `type` |
| `limit`      | A limit tripped. `error.limit` is one of `max_memory`, `max_feed_duration`, `max_recursion_depth`, `max_suspensions`, `request_timeout`, `cpu`                                                                        |
| `output`     | The result has no JSON form (a set, an integer beyond 64 bits, a date, a host object), or exceeds 1 MB                                                                                                                |
| `fail`       | The code called `fail(message)`. This sticks even if the code catches the exception                                                                                                                                   |
| `crash`      | The Monty worker crashed twice in a row on this code (a crash is retried once on a fresh worker first)                                                                                                                |

**Values.** Results convert to JSON as `None` → `null`, `bool`, `int` (64-bit), finite `float`, `str`, `list`/`tuple` → array, `dict` with `str` or `int` keys → object, and `Decimal` → its exact string (so money keeps its precision). `stdout` holds `print()` output, up to 64 KB.

**The activity fails** only for problems the code didn't cause:

- **Invalid parameters** (an unknown capability, a limit above the ceiling, an input name that isn't a Python identifier, empty code, an unknown parameter) fail it with `INVALID_PARAMETERS`, without retries: the same parameters would fail again.
- **Infrastructure problems** (no worker process could be started, the pool was exhausted, the worker broke the protocol) fail it with `SANDBOX_UNAVAILABLE`, and its retry policy applies.

**Determinism.** The clock is frozen at the step's start (`datetime.now()` and `now()` return the same instant throughout), and `random` is seeded from the workflow id and activity key, so a retried step behaves the same way.

## Capabilities

Capabilities are host functions compiled into the add-on. A workflow can only choose from this list; neither a definition nor an agent writing one can add a function. Calls to anything not granted raise `NameError` in the sandbox.

| Capability                           | Returns | Meaning                                                                                                                    |
|--------------------------------------|---------|----------------------------------------------------------------------------------------------------------------------------|
| `json_path(obj, path, default=None)` | value   | Read a nested value by path, e.g. `json_path(order, "customer.orders[0].total")`. Returns `default` when a step is missing |
| `now()`                              | `str`   | The step's start time, ISO-8601 UTC (frozen for the run)                                                                   |
| `log(message)`                       | `None`  | Add a message to the result's `logs` (up to 100 entries of 1000 characters)                                                |
| `fail(message)`                      | never   | Stop with `ok: false`, `error.kind: "fail"` and this message                                                               |

These are side-effect free. Capabilities that call models (metered against the workflow budget), make HTTP requests to an allow-list, or pause for human approval are planned.

## What Python is available

Monty implements a subset of Python 3.14: functions, closures, lambdas, comprehensions, `try`/`except`, `match`, f-strings, simple classes and `@dataclass`, and `async`/`await`. Generators (`yield`), `del`, `@property`, user-defined exception classes and third-party packages are not available.

Standard-library modules available in Monty 1.7 (an integration test imports each one): `json`, `re`, `datetime`, `decimal`, `csv`, `math`, `statistics`, `collections`, `itertools`, `functools`, `hashlib`, `base64`, `random`, `tomllib`, `typing`, `dataclasses`.

Not available: `enum`, `contextlib`, `io`, `logging`, `uuid`, `urllib`, and everything else. Importing one raises `ModuleNotFoundError` with the list above, so a model can adapt.

When an LLM writes the code, tell it in the prompt which modules it may use and that the last line must be an expression with the result.

## Writing `run_code` steps through MCP

Agents using the engine's [MCP server](mcp-server.md) discover `run_code` when the server is given the add-on's contract. Write it with the settings the add-on runs with, and point the server at it:

```bash
kruxiaflow-monty contract > /etc/kruxiaflow/run-code-contract.json
kruxiaflow serve --mcp-enabled --mcp-run-code-contract /etc/kruxiaflow/run-code-contract.json
```

`list_activities` then lists `run_code` under the add-on's worker type, `get_workflow_authoring_guide` has a `sandboxed_code` section, and `get_run_code_contract` returns the capabilities with their signatures, the available modules and the add-on's limit ceilings. The contract is a snapshot: after changing the add-on's limits, write it again.

## Pattern: let the model repair its own code

Because code errors complete the step instead of failing it, a loop can send the error back to the model. See [`examples/16b-run-code-repair-loop.yaml`](../contrib/kruxiaflow-monty/examples/16b-run-code-repair-loop.yaml):

```yaml
- key: write_rule
  activity_name: llm_prompt
  iteration_limit: 3                  # at most 3 attempts
  parameters:
    prompt: |
      Write Python for: {{INPUT.rule}} ...
      {% if run_rule.status == 'completed' %}
      Your previous code failed with this error. Fix it:
      {{ run_rule.result.error.message }}
      {% endif %}
  depends_on:
    - describe
    - activity_key: run_rule           # loop back-edge
      conditions: ["{{ run_rule.result.ok == false }}"]

- key: run_rule
  worker: monty
  activity_name: run_code
  parameters:
    code: "{{write_rule.result.content}}"
  depends_on: [write_rule]
```

- The back-edge is satisfied automatically on the first pass, then taken while `run_rule.result.ok` is false.
- Guard references to the code step with `run_rule.status == 'completed'`. Every activity is in the template context with a `status`, but `result` exists only after it completes, and templates fail on undefined values. (`run_rule is defined` is always true and does not guard anything.)
- The workflow budget caps what the retries cost.

**A sandbox makes code safe, not correct.** In testing, a model wrote a median calculation that ran cleanly and returned the wrong number. For rules that matter, put a human approval step (`wait_for_signal`) after the code is written and tested, before it is used in production.

## Security model

Monty is a language-level sandbox. The code cannot express an operation that touches the host, because the interpreter implements none. Defence in depth:

- Code runs in separate worker processes started with an empty environment, so server secrets are not in their memory.
- With the default `KRUXIAFLOW_MONTY_RECYCLE_AFTER=1`, every run gets a fresh process.
- Every call from the code is checked against the granted capabilities when it arrives, and every argument is treated as untrusted.
- The worker binary path is explicit and version-checked.
- Limits always apply, with `KRUXIAFLOW_MONTY_REQUEST_TIMEOUT_SECS` as a host-side kill switch behind them.

- The add-on is a separate process from the engine. It holds only the API URL and a worker's client credentials, never database credentials, and runs nothing but `run_code`.

For stronger isolation, run the add-on in its own container with a read-only filesystem and network access only to the API server.

## Measurements

Monty 1.6.0. Latency on an M4 Pro (macOS); sizes from local Linux arm64 image builds.

| What                                              | Result                               |
|---------------------------------------------------|--------------------------------------|
| Pool start                                        | 0.6 ms                               |
| Starting a worker process when none is warm       | ~6 ms (warm checkouts: 40-120 µs)    |
| Type check of a short snippet                     | ~30 ms                               |
| 16a example end to end (`execution_ms`)           | 30-60 ms, including the type check   |
| `monty` worker binary (Linux arm64, stripped)     | 29.2 MB                              |
| `kruxiaflow-monty` image, compressed              | about 26 MB (add-on plus `monty`)    |
| The engine's binary and image                     | unchanged: the add-on adds nothing   |

## Testing

The add-on's tests live with it and run from its directory. The integration tests run real Monty workers when `KRUXIAFLOW_MONTY_BINARY` is set, and skip otherwise. `example_16_run_code_e2e_test` deploys example 16a through the engine's API and runs it with the engine's orchestrator and this worker, like the end-to-end tests for examples 03 and 10. It also needs `DATABASE_URL`:

```bash
cd contrib/kruxiaflow-monty
KRUXIAFLOW_MONTY_BINARY=$HOME/.cargo/bin/monty DATABASE_URL=postgres://... cargo nextest run
```

The end-to-end test compiles the engine crates; with `SQLX_OFFLINE=true`, set `SQLX_OFFLINE_DIR` to the repository's `.sqlx`. CI runs all of it in its `Monty add-on (run_code)` job.
