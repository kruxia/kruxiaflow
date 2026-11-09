# Commit-order event cursor and LISTEN/NOTIFY wake

**Date:** 2026-11-08 · **Status:** implemented with changes (§0, §7), tested and benchmarked (§7.15); release pending · **Target release:** 0.10.0
**Scope:** `kruxiaflow-core` (events, orchestrator), `kruxiaflow-api` (event broadcast poller, WebSocket replay, health), one migration, docs.

This plan replaces the time-based visibility-grace cursor introduced in 0.8.1 with a cursor that is correct by construction, and adds `LISTEN`/`NOTIFY` as a wake-up signal in front of the existing poll. The poll stays the only data path. Nothing about delivery guarantees changes; latency and wasted work do.

## 0. Review after implementation

This section is feedback for the plan's author. The original text of §1 to §6 is kept as written; where the implementation departs from it, a quoted note points to the explanation in §7.

The plan's goals, structure and phasing held up, and the `NOTIFY` design (§2.2) was built as specified. The cursor design (§2.1) had a correctness hole and an unconsidered operational hazard, which between them account for most of the changes:

| #    | The plan said                                                       | What was built                                                                  | Why                                                                                                      |
|------|---------------------------------------------------------------------|---------------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------|
| 7.2  | Cursor on `position`, filtered by `tx < xmin`                       | Cursor on `(tx, position)`, ordered and stored as a pair                        | A position-only cursor still loses events: xid order and position order can differ                       |
| 7.3  | Nothing about moving the database between clusters                  | Startup guard, runtime error log, `kruxiaflow rebase-events`                    | Stored xids are meaningless in a new cluster; delivery stops silently after a dump/restore               |
| 7.4  | Existing rows take the migration's xid; "volatile default rewrites" | Existing rows take `tx = 0` through a constant default                          | Sorts below every real xid whatever is in flight; the rewrite claim was wrong (the function is `STABLE`) |
| 7.5  | "Subtransactions get ids above their parent"                        | No code change; rows carry the top-level xid                                    | `pg_current_xact_id()` returns the top-level id; the conclusion holds for a different reason             |
| 7.6  | `position` is dense                                                 | `position` is increasing, not gap-free                                          | An insert that hits `ON CONFLICT DO NOTHING` consumes an identity value                                  |
| 7.7  | Stall warning in the orchestrator, from `age(xmin)`                 | In the event source, from the oldest writing transaction's age                  | Every consumer needs it; `age()` does not apply to `xid8`                                                |
| 7.8  | Remove the post-batch sleep entirely                                | No sleep after a clean batch; `poll_interval_min` after a batch with a failure  | Without it a failing event is retried in a tight loop                                                    |
| 7.9  | `poll_interval_max` rises to 5 s with a wake                        | Two caps (1 s timer-only, 5 s with a wake); an explicit setting applies to both | An operator's explicit value should not be silently overridden by the mode                               |
| 7.10 | Listener: `Ok(Some)` and `Ok(None)` bump the counter, `Err` retries | `Err` bumps the counter too                                                     | Notifications are lost while the listener is failing                                                     |
| 7.11 | "Point `PgListener` at the database directly" behind a pooler       | The listener uses the process's own pool; no separate listener URL              | Not specified beyond that sentence; left out rather than invented                                        |
| 7.12 | Tests publish, then poll once                                       | Event-reading tests settle first (`SettledEventSource`)                         | xmin is instance-wide and `sqlx::test` runs tests in parallel on one instance                            |
| 7.13 | Document `position` "next to `from_event_id`" in `api-reference.md` | A new section for the workflow-events WebSocket                                 | The endpoint was not documented there at all                                                             |
| 7.14 | (not in the plan)                                                   | Migration fixing `uuidv7()` on PostgreSQL 17                                    | A pre-existing bug the new tests exposed; PostgreSQL 17 is still a supported target                      |

Two suggestions for the next plan of this kind:

1. **Argue ordering claims from what the database assigns and when.** §2.1's argument ("a row whose transaction is open is excluded, together with every row after it in position order") reads as obviously true, and it is false because an xid is assigned at a transaction's first write while a position is assigned at the event insert. A two-transaction counterexample test, written before the design is fixed, would have caught it. The 0.8.1 bug had the same shape (id order versus commit order).
2. **List what the design stores, and ask what gives those values meaning.** Storing transaction ids as data ties the data to one cluster's history. The rollout section (§4) covered upgrade and locking but not backup, restore, or migration between clusters, which is where that dependency shows.

## 1. The problem

### 1.1 The gap

`workflow_events.id` is a UUIDv7, assigned when the `INSERT` executes (`migrations/20251029000001_workflow_events.up.sql:22`; `core/src/workflow/service.rs:189` even generates it in Rust before the transaction commits). Consumers read `id > last_event_id` in id order and store the highest id they saw (`core/src/events/postgres_event_source.rs:63-86`, `152-170`). Rows become visible in commit order, not id order, so:

1. Transaction A inserts an event with id 100 and has not committed.
2. Transaction B inserts id 101 and commits.
3. A poll sees 101 only, processes it, and writes cursor = 101.
4. A commits. Id 100 is now below the cursor and no poll with `id > 101` ever returns it.

0.8.1 observed exactly this in the benchmark suite at 100-way concurrency (2 of 900 workflows hung with their completion events stranded; `CHANGELOG.md` 0.8.1) and fixed it with `EVENT_VISIBILITY_GRACE` (`core/src/orchestrator/orchestrator.rs:46-88`): the durable cursor never advances past `now - 500ms`, the recent tail is re-read on every poll, and an in-memory seen-set suppresses reprocessing. The same code names the proper fix: "an xid-ordered cursor column can remove the window entirely".

### 1.2 What the grace window costs

- **A throughput knee.** Above `LIMIT / grace` = 1000 / 0.5 s = 2,000 events/s the fetch window trails the pinned cursor and processing latency degrades toward 500 ms (`orchestrator.rs:64-68`).
- **Wasted work on every poll.** At any event rate, every poll re-reads and re-filters the last 500 ms of events. Idle polls on a busy system are never empty, so the adaptive backoff never settles.
- **Three places, one fix.** The grace window protects only the orchestrator. The WebSocket broadcast poller (`api/src/workflow_events/poller.rs:56-73`) advances its cursor to the highest id seen with no grace, so a late-committing event is never broadcast live. The reconnection replay (`get_events_from_id`, `postgres_event_source.rs:96-148`) and the health check (`api/src/health/checks.rs:105-121`) read `id > x` with the same hazard: a client that reconnects with `from_event_id` set to an id above a late-committing event never receives that event.
- **A clock dependency.** The horizon compares database-assigned ids against the orchestrator host's wall clock. A host clock ahead of the database by more than 500 ms removes the protection silently.

### 1.3 The latency floor

Independently of the gap, the poll interval is the latency floor: 50 ms busy to 1 s idle for the orchestrator (`core/src/orchestrator/config.rs:22-23`) and the same for the broadcast poller (`poller.rs:42`). A workflow of N dependent activities pays the idle interval up to N times when the system is quiet, and the first event after an idle period waits up to a full second. The docs' "~10 ms under load" figure (`docs/architecture.md`, "Performance Characteristics") is below the configured minimum.

## 2. The design

### 2.1 A commit-safe cursor

Two columns are added to `workflow_events`:

| Column | Type | Set by | Purpose |
|---|---|---|---|
| `position` | `bigint GENERATED ALWAYS AS IDENTITY` | Postgres at insert | Dense, monotonic read order. The cursor value. |
| `tx` | `xid8 NOT NULL DEFAULT pg_current_xact_id()` | Postgres at insert | The inserting transaction. Lets a reader know whether anything older could still be in flight. |

The poll becomes:

```sql
SELECT id, position, workflow_id, event_type, activity_key, payload, timestamp, iteration
FROM workflow_events
WHERE position > COALESCE(
        (SELECT last_position FROM workflow_event_consumers WHERE consumer_id = $1), 0)
  AND tx < pg_snapshot_xmin(pg_current_snapshot())
ORDER BY position
LIMIT 1000
```

> **Changed (§7.2):** a cursor on `position` alone is not commit-safe even with the `tx < xmin` filter. The implementation orders by `(tx, position)` and stores both.

`pg_snapshot_xmin` is the lowest transaction id still in progress anywhere in the instance when the statement takes its snapshot. Every transaction below it has committed or aborted. An aborted row is never visible. A committed row with `position > cursor` is visible to this very statement. A row whose transaction is still open has `tx >= xmin` and is excluded, together with every row after it in position order on the next poll, until that transaction finishes. So the cursor can be set to the last position returned, with nothing left behind it. No grace window, no seen-set, no clock.

The cost is head-of-line blocking by long transactions: transaction ids are instance-wide, so one long-running *writing* transaction (read-only transactions never take an id) in any database on the instance holds `xmin` down and delays every consumer until it ends. Kruxia Flow's own transactions are short. The orchestrator logs a warning when `age(xmin)` exceeds a threshold (§3.4) so an operator can see a stall for what it is.

> **Changed (§7.7):** the warning lives in the event source and is based on the oldest writing transaction's age. **Added (§7.3):** a second cost this section does not mention: stored transaction ids are only meaningful within one cluster.

`xid8` never wraps, so `tx < xmin` is a plain comparison with no epoch handling. Subtransactions get ids above their parent, so a savepoint-wrapped insert is still excluded while its parent is open.

> **Corrected (§7.5):** the row carries the top-level transaction's id, not a subtransaction id. The conclusion stands.

### 2.2 `NOTIFY` as a wake, never as data

An `AFTER INSERT ... FOR EACH STATEMENT` trigger on `workflow_events` runs `pg_notify('workflow_events', '')`. Postgres queues the notification inside the inserting transaction and delivers it at commit, to every connection listening at that moment. Rows that never commit never notify. An `INSERT ... ON CONFLICT DO NOTHING` that inserts nothing still fires the statement trigger; the consumer wakes, polls, finds nothing, and sleeps. Identical notifications within one transaction collapse into one.

Each process holds one `sqlx::postgres::PgListener` on channel `workflow_events` and fans it out to its consumers through a `tokio::sync::watch<u64>` counter. A consumer waits on `changed()` with the backoff interval as a timeout. `watch` keeps a change that happened between the consumer's last poll and its wait, so a notification cannot be lost in that window.

Three rules keep the data path honest:

1. The payload is empty and consumers never read it. A wake means "poll from your cursor until empty".
2. Every wake and every timer tick runs the same catch-up query. A missed notification only delays an event until the next tick.
3. A listener reconnect (connection dropped, `PgListener::try_recv` returning `Ok(None)`) counts as a wake, because notifications sent during the outage are gone.

> **Extended (§7.10):** a listener error counts as a wake as well.

If `LISTEN` fails (no permission, a transaction-mode connection pooler), the process logs once and runs on the timer alone. `KRUXIAFLOW_EVENT_WAKE=poll` forces that mode.

The poll loop changes from "sleep between every batch" to "drain, then wait": after a non-empty batch the consumer polls again immediately; after an empty batch it waits for a wake or the backoff timer, whichever comes first. The idle maximum can rise (default 1 s → 5 s) because the timer is a safety net, not the delivery path.

> **Changed (§7.8, §7.9):** a short wait remains after a batch that contains a failed event, and an explicitly configured maximum applies in both modes.

### 2.3 What `architecture.md` has wrong

The "Why Polling (Not LISTEN/NOTIFY)" section states that `NOTIFY` is "not transactional" and "happens regardless of transaction commit/rollback". That is false: `NOTIFY` is delivered only when the issuing transaction commits, and not at all if it rolls back. The other points (lost on connection drop, no backlog, connection-bound) are true and are the reasons it stays a wake-up. §3.5 corrects the section.

### 2.4 Out of scope

- The activity queue (`claim_next` on `activities` with `FOR UPDATE SKIP LOCKED`) keeps its own polling. The same wake pattern would fit it later; workers poll over HTTP, so that is a separate design.
- Logical replication, Kafka, NATS: unchanged as post-MVP options.
- The worker SDKs and the Python SDK: no API change reaches them.

## 3. Phases

Each phase ships with its tests in the same change. `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `./scripts/test.sh`, and `cargo sqlx prepare --workspace` (the `.sqlx` offline data, `SQLX_OFFLINE=true` in CI) before a phase is called done.

### Phase 1 · Commit-order cursor

**Migration `migrations/20261108000001_event_commit_order.{up,down}.sql`**

```sql
-- up
ALTER TABLE workflow_events ADD COLUMN position bigint;
UPDATE workflow_events SET position = n
  FROM (SELECT id, row_number() OVER (ORDER BY id) AS n FROM workflow_events) r
  WHERE workflow_events.id = r.id;
ALTER TABLE workflow_events ALTER COLUMN position SET NOT NULL;
ALTER TABLE workflow_events ALTER COLUMN position ADD GENERATED ALWAYS AS IDENTITY;
SELECT setval(pg_get_serial_sequence('workflow_events', 'position'),
              COALESCE((SELECT max(position) FROM workflow_events), 0) + 1, false);
ALTER TABLE workflow_events ADD CONSTRAINT workflow_events_position_key UNIQUE (position);

-- existing rows take the migration's own xid, which is below every future xmin
ALTER TABLE workflow_events ADD COLUMN tx xid8 NOT NULL DEFAULT pg_current_xact_id();

DROP INDEX idx_events_consumer_poll;
CREATE INDEX idx_events_consumer_poll ON workflow_events (position)
  INCLUDE (tx, id, workflow_id, event_type, activity_key, timestamp, iteration);

ALTER TABLE workflow_event_consumers ADD COLUMN last_position bigint NOT NULL DEFAULT 0;
UPDATE workflow_event_consumers c
  SET last_position = COALESCE((SELECT e.position FROM workflow_events e WHERE e.id = c.last_event_id), 0);
ALTER TABLE workflow_event_consumers DROP COLUMN last_event_id;
```

> **Changed (§7.2, §7.4):** the migration as built stores `(last_tx, last_position)` on consumers, indexes `(tx, position)`, and gives existing rows `tx = 0` through a constant default (`ADD COLUMN tx xid8 NOT NULL DEFAULT '0'`, then `SET DEFAULT pg_current_xact_id()`).

The down migration restores `last_event_id` from `position`, drops the two columns and the unique constraint, and recreates the old index. Backfilling `position` in id order keeps the existing history in the order consumers already saw it. Adding `tx` with a volatile default rewrites the table under an `ACCESS EXCLUSIVE` lock; §4 covers the operational consequence.

**`kruxiaflow-core` events**

- `WorkflowEvent` gains `pub position: i64` (`core/src/events/models.rs:18`). Serialized as `position` so the WebSocket message carries it; clients keep deduplicating by `id`.
- `EventSource::update_position(&self, consumer_id, last_position: i64)` (`core/src/events/mod.rs:22`). *(Changed, §7.2: takes an `EventCursor { tx, position }`, from `WorkflowEvent::cursor()`; `WorkflowEvent` gains `tx` as well as `position`; the replay query below compares `(tx, position)` rows.)* `poll` is unchanged in signature and uses the §2.1 query. `get_events_from_id(from_event_id, ...)` keeps its name and parameter and resolves the id to a position inside the query:

  ```sql
  WHERE position > (SELECT position FROM workflow_events WHERE id = $1)
    AND tx < pg_snapshot_xmin(pg_current_snapshot())
  ```

  An unknown `from_event_id` makes the subquery `NULL`, the predicate false, and the replay empty; the handler logs it at `warn` and continues live, as it already does on a replay error.
- Delete the doc comments that describe the grace window (`postgres_event_source.rs:47-62`) and the nil-UUID fallback.

**Orchestrator (`core/src/orchestrator/orchestrator.rs`)**

- Delete `EVENT_VISIBILITY_GRACE`, `uuidv7_lower_bound`, `cursor_horizon`, `processed_recent`, and their unit tests.
- Keep the contiguous-resolved-prefix rule (a failed event pins the cursor so the next poll retries it) and poison handling, keyed on `position`.
- Keep every handler replay-idempotent. Delivery is still at-least-once: a crash between processing and the cursor write replays the batch, and two orchestrators sharing the `orchestrator` consumer id still race. Nothing here changes that; the 0.8.1 duplicate guards stay.
- `update_position(CONSUMER_ID, resolved_prefix_position)` once per batch, as now.

**API**

- Broadcast poller (`api/src/workflow_events/poller.rs:67`): `update_position(..., event.position)`.
- Health check (`api/src/health/checks.rs:105-121`): `position > COALESCE((SELECT last_position ...), 0)`. The lag figure needs no `tx` bound; an in-flight event is not "unprocessed".
- `WorkflowEventMessage` (`api/src/workflow_events/messages.rs`) gains `position`. Document it in `docs/api-reference.md` next to `from_event_id`.
- `kruxiaflow profile` (`kruxiaflow/src/commands/profile.rs:415-427`) explains the new poll query.
- Mocks: `MockEventSource` (`api/src/state.rs:329`), `MockPollerEventSource` and friends (`poller.rs:144-220`), the `mockall` `EventSrc` (`orchestrator.rs:3371`).

**Tests** (all `#[sqlx::test(migrations = "../migrations")]` unless noted)

- `core/tests/event_tests.rs`: existing tests move from `update_position(.., events[i].id)` to `.position`; a new test asserts `position` is dense and increasing across three publishes. *(§7.6: increasing; consecutive only without conflicting inserts.)*
- **The gap test**, the one that justifies the change: open transaction A on its own connection and insert an event without committing; publish event B on the pool and commit; `poll` returns nothing (A pins `xmin`); commit A; `poll` returns A then B in position order; `update_position` to B's position; `poll` returns nothing. A second variant asserts the *old* predicate (`id > B.id`, run as a raw query) would have returned B alone, so the test documents the hazard it guards.
- **Head-of-line test**: an unrelated open transaction that has taken an xid (an insert into `workflow_definitions`, uncommitted) delays delivery of a committed event; delivery resumes when it commits. This pins down the trade-off in §2.1 so a future change cannot remove it unknowingly.
- **Replay test** (`api` WebSocket tests): `from_event_id` set to an event committed late (same two-transaction setup) still yields the events after it; an unknown id yields an empty replay and a live stream.
- **Health test**: lag computed over `position`.
- `core/tests/orchestrator_loop_tests.rs`, `retry_integration_tests.rs`, `scheduling_integration_tests.rs`: mechanical `id` → `position` changes in cursor updates.
- **Migration round trip**: a test that runs the migrator, publishes events, runs `Migrator::undo` to the previous version and `run` again, and checks `position` order matches `id` order for the backfilled rows.
- **Benchmark**: re-run `benchmarks/run_benchmark.py` at the 100-way concurrency that found the original bug. Exit criterion: zero hung workflows across three runs, and a poll under sustained load returns only new rows (verified by the orchestrator's existing `event_count` trace).

### Phase 2 · `LISTEN`/`NOTIFY` wake

**Migration `migrations/20261108000002_event_notify.{up,down}.sql`**

```sql
CREATE FUNCTION workflow_events_notify() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  PERFORM pg_notify('workflow_events', '');
  RETURN NULL;
END $$;
CREATE TRIGGER workflow_events_notify
  AFTER INSERT ON workflow_events
  FOR EACH STATEMENT EXECUTE FUNCTION workflow_events_notify();
```

**`kruxiaflow-core` events**

- New `core/src/events/wake.rs`: `EventWake` owns a `PgListener` task and a `watch::Sender<u64>`; `EventWake::subscribe() -> WakeReceiver` wraps `watch::Receiver<u64>` with `async fn wait(&mut self, timeout: Duration) -> Woken | TimedOut`. The listener task loops on `try_recv`: `Ok(Some(_))` and `Ok(None)` (reconnect) both bump the counter; `Err` logs, sleeps with backoff, and retries. If the initial `LISTEN` fails, `EventWake::connect` returns `Err` and callers fall back to the timer.
- `EventSource` gains `fn wake(&self) -> Option<WakeReceiver>` with a default of `None`, so mocks and non-Postgres sources need nothing. `PostgresEventSource::with_wake(pool, wake)` attaches one.
- `KRUXIAFLOW_EVENT_WAKE` = `notify` (default) | `poll`, read where the event source is built (`kruxiaflow/src/commands/{api,orchestrator,serve}.rs`). Configuration stays environment and CLI only, per `CLAUDE.md`.

**Loops**

- Orchestrator: drain while batches are non-empty; on empty, `wake.wait(backoff.current())`; on `Woken` reset the backoff and poll at once. `poll_interval_max` default rises to 5 s when a wake is attached; stays 1 s under `poll`. Remove the unconditional post-batch sleep (`orchestrator.rs:320-322`) in favour of drain-then-wait; the `LIMIT` bounds each iteration, so there is no spin.
- Broadcast poller (`poller.rs`): the same shape, so live WebSocket events arrive within milliseconds of commit instead of up to 1 s.
- Log once at startup which mode is active.

**Stall warning (§2.1 cost)**: once per minute, when a poll returns nothing and the oldest in-progress transaction is older than 30 s, log at `warn` with `age(pg_snapshot_xmin(pg_current_snapshot()))` and the `pid` of the oldest backend from `pg_stat_activity` (`backend_xid IS NOT NULL ORDER BY xact_start LIMIT 1`). This is the only way an operator can tell a stall from idleness.

**Tests**

- `core/tests/event_wake_tests.rs`: a publish wakes a `WakeReceiver` within 200 ms; a wake issued before `wait` is called is not lost; two receivers both wake from one publish; a plain SQL `INSERT` (not through `publish`) wakes, proving the trigger and not the client is the source; an insert that hits `ON CONFLICT DO NOTHING` still wakes and the following poll is empty.
- Reconnect path: unit test the listener task's handling of `Ok(None)` with a stub that yields it; no network fault injection needed.
- `KRUXIAFLOW_EVENT_WAKE=poll`: the orchestrator still processes an event within `poll_interval_max`.
- Latency measurement, not a test gate: time from `publish` to `process_workflow_event` for a single quiet workflow, before and after. Expected: from roughly 0.5 s average (idle poll at 1 s) to under 10 ms. Record the numbers in the changelog entry.

### Phase 3 · Docs and release

- `docs/architecture.md`: replace "Why Polling (Not LISTEN/NOTIFY)" with "Delivery: commit-order cursor with a NOTIFY wake", keeping the true points about `NOTIFY` (lost on disconnect, no backlog, connection-bound) and dropping the false one (§2.3); delete the "Visibility-grace cursor" paragraph; correct the "Performance Characteristics" figures; update the illustrative `EventSource` code, which still shows a `last_event_id: Option<Uuid>` signature and `ORDER BY timestamp` that the real code never had.
- `CLAUDE.md` "Never Use": change "LISTEN/NOTIFY for workflow events" to "LISTEN/NOTIFY as the delivery path for workflow events; it is a wake-up only, and every consumer must be correct with it disabled".
- `docs/api-reference.md`: `position` on WebSocket messages; `from_event_id` semantics unchanged.
- `scripts/analyze_monitoring.py:154` already treats `last_event_id` as an integer and is stale; point it at `last_position` or delete it.
- `CHANGELOG.md` under `[Unreleased]`, then `cargo release 0.10.0`. The minor bump covers: a migration that rewrites `workflow_events` and changes `workflow_event_consumers`; `EventSource::update_position` taking `i64`; `WorkflowEvent.position`; the new environment variable.

## 4. Rollout

- **Coordinated upgrade.** Old binaries read `last_event_id`, which the migration drops, and new binaries read `last_position`. Stop the orchestrator and API, run `kruxiaflow migrate`, start the new binaries. Mixed versions are not supported for this release.
- **Lock duration.** *(Corrected, §7.4: the lock is real, the stated cause is not what the implementation does.)* Adding `tx` with a volatile default rewrites `workflow_events` under `ACCESS EXCLUSIVE`. On a table of a few million rows this is seconds; on a large deployment, run it in the maintenance window the coordinated upgrade already needs. The down migration is a second rewrite.
- **Cursor continuity.** `last_position` is backfilled from `last_event_id`, so no consumer replays history. If a consumer's `last_event_id` points at a row that no longer exists, it restarts from 0; the handlers are idempotent, so that is slow rather than wrong.
- **Connection poolers.** `LISTEN` needs a session; under PgBouncer in transaction mode the listener fails and the process falls back to the timer with a single log line. Point `PgListener` at the database directly where a pooler sits in front. *(Not built, §7.11.)*
- **Moving the database to another cluster** *(added, §7.3)*. After a `pg_dump`/restore or a logical-replication migration, run `kruxiaflow rebase-events` before starting the services; they refuse to start until then. `pg_upgrade`, replica promotion and physical point-in-time recovery need nothing.

## 5. Estimate

Roughly three working days: a day and a half for Phase 1 (the migration round trip and the three concurrency tests are the bulk), a day for Phase 2, half a day for Phase 3 and the benchmark runs. Phase 1 is independently shippable and removes the knee on its own; Phase 2 is where the latency goes.

## 6. Origin

This design was worked out for Kruxia Writer's event store (`writer/docs/design.md` §7.5 and §7.6), which was written against Kruxia Flow's event source and adopted an `xid8` column precisely to avoid the gap described in §1.1. Applying it here gives both systems the same delivery semantics.

## 7. Implementation record

### 7.1 Status

| Part                                                      | Status  | Notes                                                                                        |
|-----------------------------------------------------------|---------|----------------------------------------------------------------------------------------------|
| Phase 1 migration `20261108000001_event_commit_order`     | Done    | `(tx, position)` cursor (§7.2); existing rows at `tx = 0` (§7.4); up/down round trip tested  |
| Phase 1 core events, orchestrator, API, profile, mocks    | Done    | Grace window, horizon, seen-set and nil-UUID fallback removed                                |
| Phase 1 tests                                             | Done    | `core/tests/event_commit_order_tests.rs`, WebSocket replay and health tests                  |
| Phase 2 migration `20261108000002_event_notify`           | Done    | Statement trigger, empty payload                                                             |
| Phase 2 `EventWake`, `EventSource::wake`, drain-then-wait | Done    | `core/src/events/wake.rs`; orchestrator and broadcast poller                                 |
| Phase 2 `KRUXIAFLOW_EVENT_WAKE` / `--event-wake`          | Done    | `serve`, `api`, `orchestrator`; a failed `LISTEN` falls back to poll with a warning          |
| Phase 2 stall warning                                     | Done    | In `PostgresEventSource::poll` on an empty batch, once a minute (§7.7)                       |
| Phase 2 tests                                             | Done    | `core/tests/event_wake_tests.rs`, stub-listener unit tests, orchestrator wake test           |
| Cluster guard and `kruxiaflow rebase-events` (§7.3)       | Done    | `core/src/events/rebase.rs`, `core/tests/event_rebase_tests.rs`                              |
| Migration `20261108000003_uuidv7_rollback_safe` (§7.14)   | Done    | `core/tests/uuidv7_tests.rs`                                                                 |
| Phase 3 docs                                              | Done    | architecture.md, CLAUDE.md, api-reference.md, raspberry-pi-deployment.md, CHANGELOG, scripts |
| Full workspace test run on the final code                 | Done    | 2191 tests, all passing                                                                      |
| Benchmark at 100-way concurrency (3 runs)                 | Done    | 2,700 workflows per configuration, all completed (§7.15)                                     |
| Latency measurement for the changelog                     | Done    | First event after idle: 456 ms → 6 ms mean (§7.15)                                           |
| `cargo sqlx prepare --workspace`                          | Done    | 5 query files replaced; offline `cargo check --workspace --all-targets` passes               |
| `cargo release 0.10.0`                                    | Pending | Not run; the decision to release is the maintainer's                                         |

Verification, on the final code against PostgreSQL 17 (`postgres:17-bookworm`, the version CI uses):

- `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` are clean; the offline build (`SQLX_OFFLINE=true`) passes with the regenerated `.sqlx` cache.
- `cargo nextest run --workspace`: all 2191 tests pass. An earlier run had five failures in `worker/tests/email_activity_integration_test.rs`, unrelated to this work: those tests share one Mailpit inbox and are marked `#[serial]`, which only serializes within a process, while nextest runs each test in its own process. `.config/nextest.toml` now puts that file and `example_10_e2e_test` (the other Mailpit reader) in a single-threaded test group.
- The guard and repair were also exercised through the binary: on a database whose stored ids were shifted above the cluster's counter, `kruxiaflow orchestrator` refused to start with the `ForeignCluster` message, `rebase-events --check` exited 1, `rebase-events` renumbered the rows, and a second `--check` reported nothing to do.

The benchmark (§7.15) ran on PostgreSQL 18, so all three migrations have also been applied there, where `20261108000003` finds the native `uuidv7()` and does nothing. The Rust test suite itself has only run against PostgreSQL 17.

### 7.2 The cursor is `(tx, position)`, not `position`

§2.1 argues that a row whose transaction is open is excluded "together with every row after it in position order". That holds only if xid order matches position order, and it does not: a transaction takes its xid at its first write, but its event's position is assigned when the event `INSERT` executes.

Counterexample:

1. T1 takes xid 10 with an earlier write.
2. T2 takes xid 11 by inserting event E2 at position 4, and stays open.
3. T1 inserts E1 at position 5 and commits.
4. xmin is 11, so a poll returns E1 (10 < 11) and the cursor moves to position 5.
5. T2 commits. E2 at position 4 is behind the cursor forever.

In Kruxia Flow this order is routine: orchestrator transactions write workflow state and queue rows before they publish events.

The fix is to order by `(tx, position)` and store both. Every row admitted by `tx < xmin` belongs to a finished transaction, and every row that becomes visible later has `tx >= xmin`, so it sorts after everything already returned. Stopping at the `LIMIT` in the middle of a transaction's rows is safe because that transaction is finished.

What follows from it:

- `workflow_event_consumers` stores `last_tx xid8` and `last_position bigint`; the poll index is `(tx, position)`.
- `EventSource::update_position` takes an `EventCursor { tx, position }`; `WorkflowEvent` carries `tx` (as `i64`, read with `tx::text::bigint`) and has a `cursor()` method.
- Replay (`get_events_from_id`) and the health check compare `(tx, position)` rows.
- **Delivery order is transaction-id order**, then insert order within a transaction. It is no longer id order, and `position` is not monotonic along the stream; the WebSocket documentation says so. An event published in reaction to a *delivered* event always sorts after it (the reacting transaction either was open at delivery, so its xid is at or above that xmin, or started later), so cause precedes effect along the delivery chain.

`test_late_commit_is_not_skipped` is the plan's gap test; `test_lower_xid_with_higher_position_is_not_skipped` reproduces the counterexample above and shows that a position-only cursor would have lost the event.

### 7.3 Stored transaction ids are bound to one cluster (new: guard and repair)

`tx` values and consumer cursors are data; the transaction-id counter belongs to the cluster. `pg_upgrade`, streaming-replica promotion and physical point-in-time recovery carry the counter along. A `pg_dump`/restore or a logical-replication migration (which includes the blue/green upgrades of some managed services) does not. In the new cluster:

- restored events have `tx` far above xmin and are held back, and
- new events get small `tx` values that sort behind every restored cursor and are never delivered.

Both are silent, and the orchestrator health check reports "caught up". This is the failure the plan set out to remove, reached by a different road, so it is handled rather than documented:

- **Detection** (`find_foreign_xids`): any stored `tx` or cursor at or above the cluster's next transaction id (`pg_snapshot_xmax`). A committed row or a delivered cursor is never there in a healthy cluster, so there are no false positives. A restore into a cluster whose counter is already past every stored id is not detected and does not need to be: the stored ids are then all below xmin and all below future ids.
- **Startup guard**: `PostgresEventSource::connect`, used by `serve`, `api` and `orchestrator`, fails with `EventError::ForeignCluster`, whose message names the repair command.
- **Runtime signals**, for a database swapped under a running process without a restart (a startup-only check would miss a managed blue/green switchover): the once-a-minute idle check (§7.7) repeats the detection and logs at `error`, and `check_event_source_health` runs it too, so `/health/ready` returns 503 with the reason. The event-source component gates readiness; the orchestrator component, which is where the plan's health work went, is reported but does not.
- **Repair** (`kruxiaflow rebase-events`, `rebase_event_xids`): in one transaction, under `ACCESS EXCLUSIVE` on both tables, every event gets `tx = 0` and `position` = its rank in the stream, and every cursor becomes `(0, rank of the last event at or before it)`. `--check` reports without changing anything.

The rank order has three groups: rows already at `tx = 0`, then rows with a foreign `tx`, then rows with a `tx` this cluster could have assigned. The third group is events published after the restore, which sorted behind the restored cursors and were never delivered; placing them last delivers them. A restored event whose old `tx` is below the new counter cannot be told apart from that group and may be delivered again, which the at-least-once contract allows.

### 7.4 Existing rows take `tx = 0`; the "volatile default" claim

The plan gives existing rows the migration's own xid and says that adding `tx` with a volatile default rewrites the table. As built, the column is added with the constant default `'0'` and the default is then changed to `pg_current_xact_id()`. Reasons:

- `0` is below every transaction id a cluster assigns, so backfilled rows are deliverable at once and sort before every new event **whatever else is in flight**. The migration's own xid is not below a transaction that started before the migration and inserts an event after it. The coordinated upgrade makes that unlikely, but the constant makes it impossible.
- A constant default is stored in the catalog and does not rewrite the table.
- It is the same convention the rebase uses (§7.3): `tx = 0` means "history from before this cluster's cursor", ordered by position.

The rewrite claim is wrong. `pg_current_xact_id()` is declared `STABLE`, not `VOLATILE` (`pg_proc.provolatile = 's'` on PostgreSQL 17), so the plan's `ADD COLUMN ... DEFAULT pg_current_xact_id()` would also have evaluated the default once and stored it in the catalog without rewriting the table. With the constant default, `pg_attribute.attmissingval` for `tx` is `{0}` after the migration, confirming no rewrite. The operational advice in §4 still holds for a different reason: the migration holds `ACCESS EXCLUSIVE` on `workflow_events` while it updates every row to backfill `position` and builds two indexes.

### 7.5 Subtransactions

`pg_current_xact_id()` returns the top-level transaction's id, so a row inserted inside a savepoint carries its parent's xid, not a higher subtransaction id. It is excluded while the parent is open because the parent's xid is at or above xmin. `test_savepoint_insert_carries_top_level_xid` checks both.

### 7.6 `position` is not dense

An insert that hits `ON CONFLICT DO NOTHING` (every duplicate publish) still consumes an identity value, so positions have gaps. Nothing depends on density; the test asserts increasing order and that the cursor order is strict.

### 7.7 Stall warning

Built in `PostgresEventSource::poll` rather than the orchestrator, so the broadcast poller reports stalls too. On an empty poll, at most once a minute, it looks in `pg_stat_activity` for a backend with `backend_xid IS NOT NULL` and `xact_start` older than 30 s and logs its `pid`, database and age. `age()` takes a 32-bit `xid` and measures distance in transactions, not time, so the plan's `age(pg_snapshot_xmin(...))` does not accept an `xid8` and would not express "older than 30 s" if it did.

### 7.8 Retry spacing

The plan removes the post-batch sleep in favour of drain-then-wait. With no sleep at all, a batch whose first event fails is polled again at once, and the same event is retried in a tight loop until it becomes a poison message. The loop therefore polls again immediately after a clean batch, and waits `poll_interval_min` after a batch that left an event unresolved.

### 7.9 Poll interval caps

`OrchestratorConfig` has `poll_interval_max` (1 s, timer only) and `wake_poll_interval_max` (5 s, wake attached). `KRUXIAFLOW_ORCHESTRATOR_POLL_INTERVAL_MAX_MS` / `--poll-interval-max`, when set, applies to both; `--poll-interval-max` lost its default so "unset" can be told from "1000". The broadcast poller uses the same two caps as constants.

### 7.10 Listener errors

`EventWake`'s listener task bumps the wake counter on `Err` as well as on a notification or a reconnect, then retries with backoff (100 ms to 5 s). Notifications sent while the listener is failing are as lost as those sent during a reconnect.

### 7.11 No separate listener connection target

`EventWake::connect` builds the `PgListener` from the process's pool (`PgListener::connect_with`), so it listens through the same address as everything else and holds one pool connection for the life of the process. Behind a transaction-mode pooler the `LISTEN` fails and the process falls back to the timer, as §2.2 specifies. The plan's "point `PgListener` at the database directly" would need a second connection setting, which the plan does not name; it was left out. If it is wanted, a `KRUXIAFLOW_EVENT_LISTEN_URL` is the obvious shape.

### 7.12 Tests and the instance-wide xmin

`sqlx::test` runs many tests at once against one PostgreSQL instance, each in its own database and each applying migrations in transactions. Because xmin is instance-wide, a test that publishes and immediately polls can find its event held back by another test's open transaction: 33 existing tests failed this way on the first run. It is the §2.1 head-of-line trade-off appearing in the test harness, and worth knowing about for any future test that reads events.

Integration tests that read events go through `core/tests/common::SettledEventSource`, which waits before each read until every committed event in the test's database is below xmin. Tests of the delivery predicate use `PostgresEventSource` directly and settle explicitly, asserting "nothing delivered" only where one of the test's own open transactions is what blocks delivery. `test_run_orchestrator_loop` had fixed 500 ms sleeps; it now waits on the condition.

### 7.13 Documentation found stale

- `docs/api-reference.md` had no section for `GET /api/v1/workflow_events/ws`; one was added (parameters, message shape, replay semantics, `position`).
- `docs/architecture.md` named the replay parameter `event_id`; it is `from_event_id`.
- `scripts/monitor_performance.sh` read consumer positions from a `consumer_positions` table that does not exist; it and `scripts/analyze_monitoring.py` now use `workflow_event_consumers.last_position`.

### 7.14 `uuidv7()` on PostgreSQL 17 (pre-existing bug, fixed)

The PL/pgSQL `uuidv7()` created for PostgreSQL < 18 keeps its state in two custom session variables set with `set_config(..., false)` and reads them inside a `BEGIN ... EXCEPTION WHEN undefined_object` block. If a connection's first `uuidv7()` call is in a transaction that rolls back, the variables survive as empty strings, the cast to `bigint` fails with `invalid input syntax for type bigint: ""`, and every later call on that pooled connection fails until the connection closes. The new rollback test for `NOTIFY` hit it.

Migration `20261108000003_uuidv7_rollback_safe` replaces the function body where the custom function exists, reading the variables with `NULLIF(current_setting(name, true), '')`. `CREATE OR REPLACE` keeps the function's identity, so column defaults are untouched; on PostgreSQL 18+ the migration does nothing.

Removing the custom function by requiring PostgreSQL 18 was considered and rejected: PostgreSQL 17 is what Raspberry Pi OS (Trixie) ships and what `docs/raspberry-pi-deployment.md` installs, the published `docker-compose.yml` and the quickstart both use `postgres:17-bookworm` (existing volumes would need a major-version upgrade), and CI and `scripts/sqlx-prepare.sh` run on 17.

### 7.15 Benchmark and latency results

Full tables and method are in `benchmarks/README.md` (2026-11-08). Three configurations were compared on fresh stacks, with images built from source: released 0.9.1, the new code with the `NOTIFY` wake, and the new code on the timer alone. Each got one warm-up pass and three measured passes of the standard suite, then a quiet-system test (one workflow at a time after a random 4 to 8 s idle gap). Orchestrator reaction time was read from event timestamps in the database.

**Exit criterion (Phase 1):** met. All 2,700 measured workflows completed in each configuration, including nine passes at 100-way concurrency, with no errors or warnings in the server logs.

| Measurement                                      | 0.9.1 | New, notify | New, poll |
|--------------------------------------------------|-------|-------------|-----------|
| Quiet: first event after idle, mean (ms)         | 456   | 5.8         | 486       |
| Quiet: follow-up events, mean (ms)               | 24.7  | 2.2         | 42.0      |
| 10 concurrent: orchestrator reaction, mean (ms)  | 41.5  | 7.4         | 49.1      |
| 100 concurrent: orchestrator reaction, mean (ms) | 139.5 | 76.5        | 71.0      |
| Sequential-5 throughput (wf/s)                   | 15.4  | 21.3        | 15.6      |
| Sequential-5 P50 latency (ms)                    | 642   | 439         | 601       |
| High-Concurrency-3 throughput (wf/s)             | 110.3 | 149.8       | 134.1     |
| High-Concurrency-3 P99 latency (ms)              | 1161  | 864         | 963       |

How this compares with what the plan predicted:

- **"From roughly 0.5 s average to under 10 ms" for a single quiet workflow (Phase 2): confirmed.** 456 ms to 5.8 ms for the first event, with a maximum of 11.8 ms over 40 workflows.
- **"Phase 1 removes the knee on its own; Phase 2 is where the latency goes": mostly right.** Timer-only mode (Phase 1 alone, plus the drain-then-wait loop) gains throughput at high concurrency (110 to 134 wf/s) because the fixed 50 ms sleep after every batch is gone, and gains nothing on a quiet system. The 2,000 events/s knee itself was not approached by these runs (peak was roughly 600 events/s), so its removal is established by the design and the tests, not by this benchmark.
- **Not anticipated: at 100 concurrent workflows the orchestrator is processing-bound.** It handles events one at a time, and reaction time (about 75 ms in both new modes) is queueing behind earlier events. The wake cannot help there; parallel event processing would.
- **Not anticipated: the activity round trip is now the dominant per-step cost** (65 to 100 ms from `ActivityScheduled` to `ActivityCompleted`, mostly the worker's 100 ms activity-queue poll). §2.4 left the activity queue out of scope and noted the same wake pattern would fit; these numbers make it the obvious next step.
- **The head-of-line cost (§2.1) did not show up.** Writing transactions were sampled every 10 ms during a pass, and none stayed open longer than about 10 ms.
- **One small regression, in the fallback mode only.** In timer-only mode, follow-up events on a quiet system take about 42 ms instead of 25 ms. The new loop drains the orchestrator's own `ActivityScheduled` event immediately and then starts backing off, where the old loop's fixed 50 ms sleep happened to keep the timer at its minimum for one more tick. Not tuned; it does not affect the default mode.

### 7.16 A woken consumer held back behind the horizon

Raised from Kruxia Writer, whose event store uses the same horizon. Its first runtime waited for its timer when a consumer was woken by an event's commit while another transaction still held the horizon: the event is not deliverable yet, and the blocking transaction's end notifies nobody. Writer now detects "held back" on an empty catch-up and polls again after 50 ms, doubling up to its tick.

**As first built, Kruxia Flow's loop avoided that wait by a side effect**, which neither the plan nor this record mentioned. A wake reset the backoff to its minimum, so the empty poll that followed a wake was retried after `poll_interval_min` (50 ms), then on the growing backoff, up to the idle maximum. That had three weaknesses:

- The retry gap grew towards the idle maximum, which the wake had raised to 5 s. A transaction open for 10 s or more could cost up to 5 s beyond its own lifetime.
- The loop made the same run of about a dozen empty polls after any wake that brought nothing.
- The stall warning fired on an empty poll whenever a writing transaction was older than 30 s, whether or not an event was waiting behind it.

**Now built: the loop asks.** A long transaction on a shared instance is always possible, so the side effect was replaced by an explicit check.

| Part                                      | What it does                                                                                                                                                                                      |
|-------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `EventSource::has_pending_events`         | Whether committed events lie past the consumer's stored cursor. In PostgreSQL, the poll's cursor comparison without the transaction-id bound: one index probe. Default `false` for other sources. |
| `ConsumerPacing` (`orchestrator/backoff`) | The wait after an empty poll, shared by the orchestrator and the broadcast poller so the two cannot drift apart.                                                                                  |
| Stall warning                             | Moved into the pending check: logged only while events are waiting, at most once a minute. The moved-database check stays on every empty poll, on its own once-a-minute timer.                    |

The wait after an empty poll:

| Empty poll                                  | Wait                                                                                  |
|---------------------------------------------|---------------------------------------------------------------------------------------|
| The first after a batch or a wake           | The minimum (50 ms). The event source is not asked.                                   |
| A later one, nothing pending, wake attached | The idle maximum (5 s). An event that commits later sends a wake.                     |
| A later one, nothing pending, timer only    | The backoff, continuing to the timer maximum (1 s). Unchanged.                        |
| A later one, events pending                 | From 50 ms on first sight, growing to the timer maximum (1 s), with or without a wake |

**The first version asked after every empty poll, and that cost 8 to 10% of throughput.** Run back to back against the commit before it (same host, two passes each, medians of three): Sequential-5 90.0 and 90.3 wf/s before, 78.8 and 85.1 after; Parallel-10 44.0 and 44.0 before, 40.0 and 40.5 after; High-Concurrency-3 141.5 and 143.0 before, 118.6 and 132.7 after. A busy orchestrator reaches an empty poll after almost every event and is woken again within milliseconds, so the probe added one database round trip to each turn of a serial loop that is already the limit. The unit and integration tests could not see this; the benchmark did. The fix is the first row of the table: the first empty poll after a batch or a wake waits the minimum interval without asking, exactly as the loop did before this section's change, and the probe runs only when that wait ends with nothing new. That is also the only time its answer matters, since no wait longer than the minimum is chosen without it. Results with the fix are in `benchmarks/README.md`.

The check is safe against the race where the blocker ends between the empty poll and the check: it has no transaction-id bound, so it also answers yes for events that have just become deliverable, and the loop polls again within 50 ms. An event that commits after the check's snapshot is not seen by it, but its commit sends a notification after that snapshot, and the wake receiver does not lose a notification that arrives before the wait starts.

Bounds, measured from the end of a blocker that was open for time H after the event committed:

- H under 50 ms (every case seen in the benchmark, where no writing transaction lasted longer than about 10 ms): at most 50 ms.
- Longer: at most about H/2 (orchestrator) or H (poller).
- Never more than the timer maximum, 1 s by default. It was 5 s.

Costs: one index probe per empty poll that follows a timer expiry, which means one about 50 ms after each burst of activity ends and one per idle timer tick. A wake that brought nothing costs two polls and a probe where it used to cost about a dozen polls. On a moved database (§7.9) the probe answers yes for as long as nothing is delivered, so a consumer polls once a second there instead of once every 5 s, next to the error it logs each minute.

Tests: `ConsumerPacing` unit tests for each row of the table; `test_pending_events_are_reported_while_held_back` and `test_uncommitted_events_are_not_pending` for the PostgreSQL check; poller tests on a paused clock (3 polls in 9 s with nothing pending, 14 while events are pending, and no probe on the first empty poll after a batch); and `test_run_orchestrator_delivers_a_held_back_event_soon_after_its_blocker_ends`, with a 30 s idle maximum, a 100 ms timer maximum and a multiplier of 100 (so the old backoff would retry at 50 ms and then 5 s later), which requires delivery within 2 s of the blocker's end. With the check disabled that test fails ("never delivered"). The stall warning's 30 s threshold is not tested: a test holding a writing transaction that long would hold back event delivery for every test on the instance, which is also why the orchestrator test keeps its blocker to 300 ms.

**Also fixed while writing that test:** the orchestrator's background timeout checker slept its full interval (30 s) without watching the shutdown token, so after a shutdown it kept the event source, and now its `LISTEN` connection, for up to 30 s. It stops at once.

