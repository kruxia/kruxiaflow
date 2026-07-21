# Changelog

All notable changes to Kruxia Flow are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- **Dead-letters are now always self-explaining**: workflows failed by the
  engine itself (workflow timeout — e.g. queued work that no worker ever
  claimed because the worker was down) previously exposed
  `error_message: null` on the workflow list/detail APIs; the only reason
  lived in the unpersisted `WorkflowFailed` event payload, and no activity
  had failed for the API's activity-error extraction to find. The
  `WorkflowFailed` reason is now persisted to a new `workflows.error_message`
  column, the timeout reason diagnoses the cause ("N queued activities were
  never claimed by any worker — is a worker for this queue running?"), and
  the APIs return the most specific reason available: a failed activity's
  error first, the workflow-level reason as fallback.
  (nukumori-support-needs item 12, from the prod cutover day.)
- **Dead workflows no longer leave claimable work behind**: a workflow
  failed from outside the activity flow (workflow timeout) left its
  unclaimed queue rows `pending` — a worker coming back later would execute
  (and pay for) work whose results the terminal-state guards then discarded.
  Processing a `WorkflowFailed` event now cancels the workflow's unclaimed
  rows (pending and waiting); in-flight claimed work still drains normally.

## [0.8.1] - 2026-07-20

### Fixed

- **Event delivery could silently drop events under concurrent load — the
  exact failure the polling EventSource exists to prevent** (caught by the
  benchmark suite at 100-way concurrency: 2 of 900 workflows hung with
  their work completed but the completion events stranded). Event ids are
  UUIDv7 assigned when the INSERT executes, but readers see commit order:
  an event committing milliseconds late could land behind an
  already-advanced consumer cursor and never be read. The orchestrator's
  durable cursor now trails a 500ms visibility-grace horizon (an in-memory
  seen-set keeps re-polled tail events from being reprocessed; the poll
  batch limit rose 100 → 1000 so the trailing window doesn't throttle
  high event rates). Event processing is now fully replay-idempotent, as
  at-least-once delivery requires — closing latent bugs that crash
  recovery or multiple orchestrators sharing a consumer could always
  trigger: a replayed `WorkflowCreated` re-initialized workflow state,
  WIPING activity progress and wedging the workflow; a replayed
  `ActivityCompleted` re-ran iteration management and cost recording (it
  now has the same duplicate guard as `ActivityFailed`); a replayed
  `ActivityWaiting` dragged a finished activity back to waiting; and a
  failed event's promised retry-on-next-poll never happened if a later
  event in the same batch succeeded (the cursor now only advances through
  the contiguous prefix of resolved events).

## [0.8.0] - 2026-07-20

### Added

- **First-class recurring schedules**: a schedule is a new resource that
  submits a workflow definition on a cadence — standard 5-field crontab
  (or 6-field with leading seconds) with IANA timezone support, or a fixed
  interval in seconds. CRUD at `POST/GET /api/v1/schedules` and
  `GET/PATCH/DELETE /api/v1/schedules/{id}`. The scheduler loop runs inside
  `serve` and the distributed `orchestrator` command (env-gated:
  `KRUXIAFLOW_SCHEDULER_ENABLED`, `KRUXIAFLOW_SCHEDULER_TICK_INTERVAL_MS`,
  `KRUXIAFLOW_SCHEDULER_BATCH_LIMIT`) and submits server-side — no client
  credentials ride the recurrence. Fire-once misfire policy (downtime
  collapses into at most one catch-up run), `overlap_policy: skip|allow`,
  and idempotent runs via `unique_key = schedule:<name>:<occurrence epoch>`;
  multiple engine instances are safe (SKIP LOCKED + unique_key).
- **Gemini cache-storage cost modeling**: new nullable
  `llm_models.cache_storage_price_per_million_token_hours` catalog column
  (seeded for Google Gemini models from the published storage prices),
  optional `cache_storage_token_hours` field on usage entries
  (completion/fail; additive under the worker-API contract-freeze
  discipline), priced server-side alongside cache writes. Token-hours
  reported for a model with no storage price record that component at 0
  with a warning. Exposed in `POST /api/v1/llm/models/search` and the
  `kruxiaflow-worker` SDK (`UsageEntry::cache_storage_token_hours`).
- **Dead-letter visibility over the API**: `GET /api/v1/workflows` rows and
  `GET /api/v1/workflows/{id}` now carry `error_message` (a failed
  activity's error), and each activity in the detail response exposes its
  `error`. Combined with the existing `status`/`definition_name` filters,
  failed workflows are fully diagnosable without SQL.
- **Worker SDK `Worker::run_once()`**: poll once, execute whatever was
  claimed to completion, and return the count — for smoke tests and
  `--once`-style worker binaries.
- **Cost CLI (`kruxiaflow cost`)**: terminal cost reporting against the REST
  API — `cost workflow <id> [--detailed]` (summary, tokens, budget %,
  per-activity/attempt breakdown with the model actually used after
  fallback), `cost analytics [--since 7d] [--group-by
  provider|model|definition|day]`, `cost top [--by
  workflows|definitions]`, and `cost export` (CSV). All commands support
  `--format table|json|csv`, authenticate via OAuth2 client credentials
  (`KRUXIAFLOW_CLIENT_ID`/`KRUXIAFLOW_CLIENT_SECRET`), and work
  credential-free against a `--insecure-dev` server.
- **Built-in cost analytics dashboard** at `/dashboard`: a self-contained
  page embedded in the binary (no Node build, no CDN assets, nothing added
  to the distroless image) served by the API server — spend over time,
  spend by provider and model, top workflows/definitions, cache hit rate,
  and budget enforcement events, with light/dark themes and 10s polling
  (differential: the page re-renders only when the data changed, so a
  refresh never disturbs scroll position or tooltips).
  The page is public; its data calls carry API auth (credential-free under
  dev mode). Sign-in exchanges client credentials for a token kept in
  `sessionStorage`.
- **Cost analytics API extension**: `GET /api/v1/cost/analytics` now accepts
  `group_by=provider|model|definition|day` and `limit`, and additionally
  returns `total_activities`, `avg_cost_per_workflow`, token totals and
  `cache_hit_rate`, `top_workflows`, `top_definitions`, budget-event counts,
  and recent `budget_events` — one server-side aggregation shared by the
  CLI, dashboard, and MCP tools. Existing response fields are unchanged
  (additive).
- **Budget enforcement events are now recorded, not just logged**: a new
  `activity_costs.budget_event` column ('abort' | 'downgrade'). The
  orchestrator's pre-execution abort writes a zero-cost line item
  (`budget_exceeded = true`, with the estimate that tripped the limit), and
  an `llm_prompt` fallback chain that skips models for budget reasons marks
  the completed row as a downgrade (the worker reports
  `budget_skipped_models` in the activity output). `/cost/history` rows now
  include `budget_event`.

### Fixed

- **Worker-activity retry backoff** (load-bearing for production LLM
  workloads): a failed activity's requeue now honors the definition's
  `retry` policy — exponential/fixed backoff computed by the queue
  (`scheduled_for = NOW() + backoff`) instead of the previous immediate
  requeue that burned all retries in milliseconds. Activities without a
  `retry` block get the default shape (exponential, base 2s, factor 2,
  cap 300s).
- **`kruxiaflow health` no longer reports DEGRADED against a healthy
  server**: the CLI has always parsed readiness checks as component objects
  (`{"status": ..., "message": ...}`) with an `orchestrator` entry, but the
  server returned flat strings and never reported the orchestrator — so
  `database` parsed as unknown, `orchestrator` was missing, and the overall
  verdict was DEGRADED (exit 1) on every version, failing container
  healthchecks while the engine served fine. `/health/ready` now returns the
  component-object shape the CLI expects and adds a real `orchestrator`
  check: event-consumption freshness from the orchestrator's durable
  consumer position (unhealthy when events sit unprocessed past a 30s grace
  period — the dead-orchestrator state). The orchestrator component is
  informational for the HTTP status (a distributed API server is not taken
  out of rotation by a separate orchestrator deployment), while the CLI
  folds it into its exit code — the right verdict for the all-in-one
  container, whose bundled-compose healthcheck already runs this CLI (and
  therefore reported the engine unhealthy on every prior version).
- **Event dedup is now attempt-aware — terminal failures can no longer be
  silently dropped** (the root cause of workflows stuck `running` after
  retries exhausted): `workflow_events`' unique constraint allowed only ONE
  `ActivityFailed` event per activity ever, so the first retryable failure
  occupied the slot and publish's `ON CONFLICT DO NOTHING` discarded every
  later failure event — including the terminal one. The dedup index now
  includes the payload's `attempt` (set by worker-reported failures, retry
  scheduling, and timeout failures); all other event types carry no attempt
  and keep their exact one-slot idempotency semantics.
- **Single retry authority — no more queue/orchestrator disagreement**: the
  queue decides retry-vs-terminal (it owns `retry_count`/`max_retries`) and
  the orchestrator now follows the `will_retry` in the `ActivityFailed`
  event instead of re-deciding from a different counter. This fixes both
  halves of the old mismatch: workflows stuck `running` after a terminal
  worker failure until a restart/timeout (dead-letters invisible to the
  status API), and zombie re-executions whose completions were discarded.
  Terminal failures now propagate to `WorkflowFailed` live, within one
  orchestrator poll interval. A retry that would exceed an LLM budget is
  cancelled in the queue (new `ActivityQueue::cancel_pending`) and fails
  permanently, preserving budget enforcement on the retry path.
- **`max_attempts` off-by-one**: `retry.max_attempts` now means total
  attempts. Previously it was projected into the queue's `max_retries`
  column unchanged, granting one extra attempt.
- **Failed workflows no longer hold their `unique_key` forever**: dedup now
  excludes `failed` workflows (partial unique index replaces the
  unconditional constraint), so a dead-lettered submission can be
  resubmitted under the same key. Concurrent duplicate submissions of the
  same key now both surface as 409 (the loser previously hit the raw
  constraint and got a 500).
- **Worker-reported failure messages were dropped**: the orchestrator read
  only the `error` payload key, but worker-reported failures publish
  `error_message` — failed activities recorded "Unknown error" (or
  nothing). Both keys are read now, so real error text lands in workflow
  state and the API.
- **Docker `latest` tag policy**: `latest` now moves only on release tags —
  rolling `main` builds publish only the date-sha tag, so `latest` always
  equals the newest release. `docker-compose.yml` and the quickstart
  document version pinning for production.
- The `activity_costs` total-cost trigger no longer fires for zero-cost
  rows, which would deadlock orchestration when a budget-abort marker was
  recorded while the event-processing transaction held the workflow row
  lock.

- **Automated crates.io releases**: the tag pipeline (`main-ci.yml`) now
  publishes `kruxiaflow-worker` via crates.io Trusted Publishing (OIDC, no
  token secret) after checks pass, alongside binaries and Docker images; the
  GitHub Release is created only when all three channels succeed. Re-running
  a tag run is idempotent (already-published versions are skipped).
  Server-internal crates are marked `publish = false`.
- **Rust worker SDK (`kruxiaflow-worker` on crates.io)**: new `worker-sdk`
  workspace crate — the complete external-worker loop for Rust applications:
  activity registration (trait, typed-params trait, and closure forms),
  semaphore-bounded polling, heartbeats with cancel-on-conflict, timeout and
  panic containment, retryable/terminal failure semantics, per-LLM-call usage
  reporting (`UsageEntry` on results *and* failures), OAuth2
  client-credentials auth with automatic refresh (credentials optional
  against a dev-mode server), and graceful drain on shutdown (in-flight
  activities finish or are re-queued as retryable — nothing lost or
  double-completed). Dual-licensed MIT OR Apache-2.0. The built-in std worker
  now runs on this SDK's poll loop (internal crate renamed
  `kruxiaflow-std-worker`; `kruxiaflow serve` / `kruxiaflow worker` behavior
  unchanged), so every deployment exercises the published code path.
- **External activity usage reporting**: `POST /api/v1/activities/{id}/complete`
  and `/fail` accept an optional `usage` list — one entry per LLM call made
  inside the activity (`provider`, `model`, `input_tokens`, `output_tokens`,
  `cache_read_tokens`, `cache_creation_tokens`, optional per-entry `cost_usd`).
  Each entry becomes an `activity_costs` row shaped like a built-in `llm_prompt`
  row; entries without an explicit cost are priced server-side from the
  `llm_models` catalog. External spend now counts against budgets and appears in
  `/cost`, `/cost/history`, `/cost/analytics`, and `workflows.total_cost_usd`.
  An unknown provider/model never fails the completion: the entry is recorded at
  cost 0 with a warning in the response body (`warnings`) and a WARN log.
- **Lump-sum cost recording**: a completion reporting only top-level `cost_usd`
  (today's wire shape, unchanged) now lands one cost row that counts against
  budgets — previously the value was display-only and external spend was
  invisible to budget enforcement. When `usage` entries are present, top-level
  `cost_usd` means cost *not covered by the entries* (e.g., a paid non-LLM API)
  and records its own row; never repeat entry costs there.
- **Failure-path cost reporting**: `/fail` accepts the same `cost_usd` and
  `usage` fields — a failed attempt that made LLM calls still spent the money.
  Spend is recorded under the failing attempt before retry scheduling, so the
  retry's budget check already sees it.
- **Workflow-level budgets enforced**: the top-level `settings.budget` block in
  a workflow definition (previously parsed and silently dropped) is now
  persisted with the definition, copied to `workflows.budget_limit_usd` at
  submission, surfaced by `GET /workflows/{id}/cost`, and enforced by the
  scheduling-time budget check alongside activity-level budgets.
- **Cache-write pricing in the model catalog**: new nullable
  `llm_models.cache_write_price_per_million` column (seeded via
  `cache_write_price_per_million` in `config/llm_models.yaml`, set to 1.25x
  input for Anthropic models). Reported `cache_creation_tokens` are billed at
  this price; models without one fall back to the input-token price. Exposed
  in `POST /api/v1/llm/models/search` responses and the `model_pricing`
  parameter enrichment. Google's Gemini explicit caching bills cache storage
  per token-hour instead of a write premium — modeled by the cache-storage
  price + `cache_storage_token_hours` entry field (see Added above).

### Changed

- `activity_costs.provider` and `.model` are now nullable (lump-sum and
  non-LLM cost line items have no provider/model); `/cost/history` returns
  `null` for those fields on such rows.

### Fixed

- Quickstart `catalog` init container failed with curl exit 23 on a truly
  fresh machine: the `curlimages/curl` default user (uid 100) cannot write to
  a newly created root-owned volume. It now runs as root; the keygen one-shot
  also chmods the generated dev keys to 644 so the non-root distroless server
  can always read them.
- README, docs, and landing-page budget snippets used `limit_usd`, which the
  schema has never accepted (`BudgetSettings` field is `limit`) — every
  copy-pasted first workflow failed validation. All launch assets corrected
  (the `budget_limit_usd` cost-API *response* field is unchanged and was
  already correct).

- **LLM model catalog refreshed to July 2026** (`config/llm_models.yaml`),
  verified against provider pricing pages. Added: Claude Fable 5, Opus 4.8/4.7,
  Sonnet 5 (introductory pricing through 2026-08-31 — bump to $3/$15 on
  2026-09-01), GPT-5.6/5.5/5.4 families, Gemini 3.5/3.1. Corrected: removed
  nonexistent `claude-haiku-4-6` and wrong Opus 4.1 ID; 4.6+ Claude models now
  show 1M context/128K output with no long-context premium; removed retired
  models (Claude 3.x, Gemini 2.0/1.5 — Gemini 2.0 shut down 2026-06-01).
  Examples, README, and landing updated off dead model IDs (including the
  never-valid `claude-haiku-4-20250415`).

- Docker builds broke when unpinned `cargo install sqlx-cli` began resolving
  to 0.9.0, which requires a newer rustc than the pinned base image. sqlx-cli
  is now pinned to 0.8.6 (matching the workspace's sqlx) and the base images
  bumped from `rust:1.90-bookworm` to `rust:1.97-bookworm`.

### Changed

- Release binaries now build with `--features redis-cache`, matching the
  Docker image's feature set.
- CI and releases unified into one pipeline (`main-ci.yml`): tag pushes run
  checks → binaries + versioned multi-arch Docker → GitHub Release, so nothing
  publishes unless clippy/fmt/tests pass; the redundant branch-push run for
  release commits is skipped (same SHA runs once, on the tag). Releases are
  cut with `cargo release <version>` (config under
  `[workspace.metadata.release]`); a preflight job fails fast if a tag doesn't
  match the workspace version.
- CI caching: prebuilt tool binaries (cargo-llvm-cov, sqlx-cli, cross) instead
  of source installs, cache-on-failure for cargo caches, Docker layer caching.
- **Compose unification**: the root `docker-compose.yml` is now the published
  quickstart file — pull-only (the `build:` section moved to
  `docker-compose.override.yml`, which Compose auto-loads for plain
  `docker compose up` in a checkout; `./docker` passes explicit `-f` lists and
  is unaffected), self-initializing (keygen + LLM-catalog one-shot containers
  with named volumes replace the bind-mounted `docker-keys/` and `config/`
  prerequisite), secure by default (`KRUXIAFLOW_INSECURE_DEV=true` opts in;
  fixed local OAuth client defaults otherwise, guarded by the new default
  127.0.0.1 API port binding). Redis now sits behind the `cache` compose
  profile with `KRUXIAFLOW_CACHE_PROVIDER` defaulting to `noop` (`./docker`
  enables both automatically; standalone users opt in via `COMPOSE_PROFILES`).
  Postgres/redis host ports moved to the override file and bind loopback only;
  the `platform: linux/amd64` pins are gone (images are multi-arch);
  `docker-compose.develop.yml` was renamed to `docker-compose.override.yml`
  and `quickstart/docker-compose.yml` removed.

- **License changed from AGPL-3.0 to Apache-2.0** (2026-07-15). All current and
  future releases are Apache-2.0; releases prior to this change remain AGPL-3.0.
  Client SDKs are Apache-2.0 as well (previously MIT); forthcoming Rust crates
  will be dual-licensed MIT OR Apache-2.0. Copyright holder: Kruxia Corp.
  See `docs/licensing-faq.md`.
- README and positioning rewritten around budgeted workflows (hard cost limits
  enforced in the engine); comparison table re-cut, human-in-the-loop approval
  gates documented as a first-class feature.

### Added

- **Insecure dev mode** (`--insecure-dev` / `KRUXIAFLOW_INSECURE_DEV`): local
  development without the OAuth token dance. Unauthenticated requests execute
  as a synthetic dev principal; presented tokens are still validated normally.
  Startup refuses non-loopback binds without a second explicit override, warns
  loudly, and surfaces the flag in `/health` and `/api/v1/info`. Production
  behavior with the flag absent is unchanged.
- **Quickstart** (the root `docker-compose.yml`, fetched straight from the
  repo's raw GitHub URL — no separate published copy): one curl +
  `KRUXIAFLOW_INSECURE_DEV=true docker compose up -d` runs Kruxia Flow +
  PostgreSQL on 127.0.0.1, with generated local keys and the LLM pricing
  catalog seeded — first budgeted workflow with zero auth steps. README
  Getting Started rewritten around this flow.
- Release CI (`.github/workflows/release.yml`): tagged releases build binaries
  (x86_64-linux-gnu, aarch64-linux-gnu, aarch64-macos) with SHA-256 checksums,
  publish multi-arch Docker images (`kruxia/kruxiaflow`), and create a GitHub
  Release. Fully static musl builds are deferred (musl does not currently build).
- `scripts/sqlx-prepare.sh`: regenerates the sqlx offline cache for the whole
  workspace including test targets, against a throwaway postgres — the `.sqlx`
  cache now covers `--all-targets` as CI requires.
- `NOTICE` file and `CHANGELOG.md` (this file).
