# Kruxia Flow Benchmark Suite

Reproducible benchmarks comparing Kruxia Flow against Temporal and Airflow workflow engines.

## Methodology

- **Workflows**: Identical echo activities (sequential, parallel, high-concurrency)
- **Execution**: Sequential (one platform at a time, no cross-contamination)
- **Metrics**: Throughput (wf/sec), Latency (P50/P95/P99), Success Rate
- **Environment**: Docker Compose (controlled, reproducible)
- **Client approach**: All platforms benchmarked via external clients (HTTP/SDK)

## Quick Start

```bash
# First time: generate RSA keys and .env (from repo root)
./scripts/init.sh

# Build and run all benchmarks
cd benchmarks
docker-compose up --build

# Results will be in results/
# - results.json (raw data)
# - comparison.html (visual report)
```

## Prerequisites

Before running benchmarks, generate RSA keys (required for JWT signing) from the repo root:

```bash
./scripts/init.sh
```

This creates `docker-keys/private.pem` and `docker-keys/public.pem` (skips if they already exist).

## Manual Execution

```bash
# Install dependencies
pip install -e .

# Start platforms
docker-compose up -d kruxiaflow temporal airflow-webserver airflow-scheduler airflow-worker

# Check platforms are accessible
python run_benchmark.py check

# Run benchmarks (all platforms)
python run_benchmark.py run

# Run specific platform only
python run_benchmark.py run --platform kruxiaflow
python run_benchmark.py run --platform temporal
python run_benchmark.py run --platform airflow

# List available scenarios
python run_benchmark.py list-scenarios

# Generate report from existing results
python run_benchmark.py report results/

# Stop platforms
docker-compose down
```

## Benchmark Scenarios

1. **Sequential-5**: 5 echo activities in sequence (100 workflows, 10 concurrent)
2. **Parallel-10**: 10 echo activities in parallel with fan-out/fan-in (50 workflows, 10 concurrent)
3. **High-Concurrency-3**: 3 echo activities (300 workflows, 100 concurrent)

## Expected Results

Measured on the reference hardware (Apple Silicon, Docker Desktop; July 2026):

- **Kruxia Flow**: ~135-150 wf/sec high-concurrency (bound by the
  orchestrator processing events one at a time); ~80-90 wf/sec Sequential-5
  and ~42-44 wf/sec Parallel-10. From 0.10.0; through 0.9.x it was ~85-110
  high-concurrency and ~15-20 on the other two, which were bound by poll
  timers (see the two 2026-11-08 result sets)
- **Temporal**: ~12-66 wf/sec (high run-to-run variance on this host)
- **Airflow**: ~2-11 wf/sec (batch-oriented, not optimized for throughput)
- **Speedup**: from 0.10.0, ~5x vs Temporal on Sequential-5 and ~2.5x on the
  other two scenarios (it was ~1.2-2x through 0.9.x); Airflow has not been
  rerun since 0.8.0, when Kruxia Flow was ~7-12x faster
- **Success rate**: 100% on every scenario is part of the pass criteria — a
  single hung workflow is an engine bug, not noise (see 2026-07-20 notes)

## Architecture

### Kruxia Flow
- **Client**: Python httpx HTTP client
- **API**: REST API at :8080
- **Components**: API Server, Orchestrator, Built-in Worker
- **Database**: PostgreSQL

### Temporal
- **Client**: Python SDK (temporalio)
- **API**: gRPC at :7233
- **Components**: Server, Python Worker
- **Database**: PostgreSQL

### Airflow
- **Client**: REST API client
- **API**: Webserver at :8081
- **Components**: Webserver, Scheduler, Celery Executor
- **Database**: PostgreSQL + Redis

## System Requirements

- Docker and Docker Compose
- 4+ CPU cores
- 8+ GB RAM
- Linux or macOS (Windows via WSL2)

## Directory Structure

```
benchmarks/
├── README.md                    # This file
├── pyproject.toml               # Python package configuration
├── run_benchmark.py             # Main CLI entry point
├── docker-compose.yml           # All platforms in one file
├── Dockerfile.benchmark         # Benchmark runner container
├── kruxiaflow/
│   ├── benchmark.py            # Kruxia Flow HTTP client benchmark (std + py-std)
│   └── workflows.py            # Workflow definitions (Python dicts)
├── temporal/
│   ├── benchmark.py            # Temporal SDK benchmark
│   ├── workflows.py            # Temporal workflow classes
│   └── activities.py           # Temporal echo activity
├── airflow_bench/
│   ├── benchmark.py            # Airflow API client benchmark
│   └── dags.py                 # Airflow DAG definitions (dir mounted as the DAGs folder)
├── shared/
│   ├── report.py               # HTML report generator
│   └── resource_monitor.py     # Per-platform container CPU/memory sampling
└── results/                     # Output directory (gitignored)
```

Note for Apple Silicon: `kruxia/kruxiaflow-py-std` on Docker Hub is
amd64-only — build it locally before running
(`docker build -t kruxia/kruxiaflow-py-std:latest ../../kruxiaflow-python`)
or the py-std services fail to start.

## Results

### 2026-11-08 (0.10.0 against Temporal)

Kruxia Flow 0.10.0 and Temporal, each alone on the same host in one session
(Apple Silicon, Docker Desktop, the host doing other work throughout). Two
runs of the harness per platform, each a discarded warm-up and three measured
passes on a fresh stack: six passes per figure. Kruxia Flow is the released
build from the section below (PostgreSQL 18, built-in Rust worker). Temporal
is `temporalio/auto-setup:1.24.2` on PostgreSQL 13 with its default settings,
as in the earlier comparisons, with a Python SDK 1.35.0 worker in the
benchmark process.

**Every workflow completed on both platforms** (2,700 each).

Standard suite: median of six passes, with the range of throughput (wf/s) and
latency in ms:

| Scenario           | Metric     | Kruxia Flow 0.10.0  | Temporal 1.24.2  | Kruxia Flow vs Temporal |
|--------------------|------------|---------------------|------------------|-------------------------|
| Sequential-5       | Throughput | 89.9 (86.2-91.1)    | 17.6 (13.3-18.0) | 5.1x                    |
| Sequential-5       | P50        | 108                 | 553              | 5.1x lower              |
| Sequential-5       | P95        | 139                 | 696              | 5.0x lower              |
| Parallel-10        | Throughput | 44.1 (39.9-45.0)    | 18.5 (12.1-25.9) | 2.4x                    |
| Parallel-10        | P50        | 216                 | 380              | 1.8x lower              |
| Parallel-10        | P95        | 272                 | 512              | 1.9x lower              |
| High-Concurrency-3 | Throughput | 139.1 (125.1-144.6) | 55.9 (36.7-65.6) | 2.5x                    |
| High-Concurrency-3 | P50        | 642                 | 802              | 1.2x lower              |
| High-Concurrency-3 | P95        | 919                 | 2589             | 2.8x lower              |

One workflow at a time on an otherwise idle system, after a random 4-8 s idle
gap (30 one-activity and 10 five-activity workflows per run, two runs; mean
end-to-end latency in ms as the client sees it):

| Workflow     | Kruxia Flow 0.10.0 | Temporal 1.24.2 |
|--------------|--------------------|-----------------|
| 1 activity   | 18.4, 19.0         | 35.9, 36.9      |
| 5 activities | 33.6, 35.2         | 72.2, 74.9      |

How the comparison has moved (throughput, wf/s; Temporal's figures are from
each session's own run):

| Session    | Engine  | Sequential-5: KF / Temporal | Parallel-10: KF / Temporal | High-Concurrency-3: KF / Temporal |
|------------|---------|-----------------------------|----------------------------|-----------------------------------|
| 2026-03-01 | 86e9ac7 | 15.0 / 13.1                 | 17.5 / 26.1                | 74.0 / 47.7                       |
| 2026-07-20 | 0.8.0   | 15.4 / 13.4                 | 18.7 / 12.7                | 86.6 / 46.0                       |
| 2026-11-08 | 0.10.0  | 89.9 / 17.6                 | 44.1 / 18.5                | 139.1 / 55.9                      |

What to keep in mind when reading these:

- **Through 0.9.x Kruxia Flow's low-concurrency results were set by its
  poll timers, not by work.** 0.10.0 removed those waits (event wake and
  worker wake). Temporal's figures are about where they were in the earlier
  sessions.
- **This is each platform out of the box on one machine, not either at its
  best.** Temporal runs all its services in one container with default
  settings, and its
  worker is the Python SDK inside the load generator's process. Kruxia Flow's
  worker is compiled Rust inside the server. Temporal also records more per
  step than Kruxia Flow does (a full replayable history). A tuned,
  multi-node Temporal would do better than this; so would Kruxia Flow with
  more than one worker.
- **Temporal 1.24.2 is not current.** It is pinned for continuity with the
  earlier sessions.
- **The client measures the two differently, in Temporal's favour.** The
  Kruxia Flow client learns of completion by polling the status endpoint
  every 50 ms (every 5 ms in the idle test); the Temporal client is told at
  once. Kruxia Flow's standard-suite latencies therefore include up to 50 ms
  of waiting that is the client's, not the engine's.
- **Temporal varies more from pass to pass on this host**, as before:
  Parallel-10 passes fell into two groups, around 12-13 and 23-26 wf/s.
- **At 100 concurrent workflows Kruxia Flow is limited by its orchestrator**,
  which processes events one at a time; its median latency there (642 ms) is
  queueing, and it is the smallest margin in the table.

The Temporal stack and idle test used here are kept with the raw results
(`results/2026-11-08-temporal/`, not in version control).

### 2026-11-08 (held-back check, for 0.10.0)

A regression check on the last change before 0.10.0: after an empty poll the
event consumers now ask whether committed events are waiting behind an open
transaction (`docs/implementation/2026-11-08-commit-order-cursor-and-notify-wake.md`,
§7.16). Three builds from source, run alternately in one session on the same
host as the runs below: the worker-wake commit (before), the first version of
the check, which asked after every empty poll, and the released version,
which does not ask on the first empty poll after a batch or a wake. Each
figure is the median of three passes after a discarded warm-up; each column
is one run of the harness. Throughput in wf/s.

| Scenario           | Before (3 runs)     | Check on every empty poll (3 runs) | Released (2 runs) |
|--------------------|---------------------|------------------------------------|-------------------|
| Sequential-5       | 90.0, 90.3, 90.7    | 75.4, 78.8, 85.1                   | 89.6, 90.8        |
| Parallel-10        | 44.0, 44.0, 43.9    | 36.7, 40.0, 40.5                   | 42.0, 44.2        |
| High-Concurrency-3 | 141.5, 143.0, 138.8 | 117.8, 118.6, 132.7                | 143.4, 137.7      |

**Every workflow completed in every run** (1,840 per run), with 0 errors and
0 warnings in the server logs.

- **Asking after every empty poll cost 8-10% of throughput.** A busy
  orchestrator reaches an empty poll after almost every event, so the probe
  added a database round trip to each turn of a serial loop that is already
  the limit. Orchestrator reaction time at 10 concurrent workflows rose from
  11.3-12.2 ms to 12.3-15.1 ms.
- **The released version costs nothing measurable**: it is within run-to-run
  variation of the build before it on all three scenarios (reaction time at
  10 concurrent: 11.4 and 11.9 ms).
- These runs are several percent faster than the worker-wake table below for
  the same code ("Before" here is the "+ worker wake" build there). The host
  was doing other work in both sessions; compare within a session, not across.

### 2026-11-08 (worker wake over SSE, for 0.10.0)

Same harness, host and day as the event-wake runs below. Two configurations
built from source: the event-wake commit (the baseline here) and the worker
wake on top of it (`docs/implementation/2026-11-08-activity-wake-sse.md`).
Workers now learn that an activity was scheduled from a server-sent event
stream and poll at once, instead of polling every 100 ms.

**All 2,700 measured workflows completed in both configurations**, with 0
errors and 0 warnings in the server logs.

Standard suite, mean of three passes (throughput in wf/s, latency in ms). The
0.9.1 column is from the event-wake runs below, for the cumulative picture:

| Scenario           | Metric     | 0.9.1 | event wake | + worker wake | change vs event wake |
|--------------------|------------|-------|------------|---------------|----------------------|
| Sequential-5       | Throughput | 15.4  | 21.1       | 82.4          | 3.9x                 |
| Sequential-5       | P50        | 642   | 436        | 111           | -75%                 |
| Sequential-5       | P95        | 776   | 522        | 169           | -68%                 |
| Sequential-5       | P99        | 798   | 575        | 176           | -69%                 |
| Parallel-10        | Throughput | 20.3  | 26.8       | 42.1          | +57%                 |
| Parallel-10        | P50        | 478   | 341        | 234           | -31%                 |
| Parallel-10        | P95        | 570   | 427        | 275           | -35%                 |
| Parallel-10        | P99        | 609   | 446        | 279           | -37%                 |
| High-Concurrency-3 | Throughput | 110.3 | 135.4      | 134.9         | none                 |
| High-Concurrency-3 | P50        | 805   | 653        | 682           | +4%                  |
| High-Concurrency-3 | P95        | 1020  | 924        | 922           | none                 |
| High-Concurrency-3 | P99        | 1161  | 1043       | 1015          | -3%                  |

Where a step's time goes, from event timestamps in the database (mean, ms).
"Activity round trip" is `ActivityScheduled` to `ActivityCompleted`: the
worker noticing, claiming, running an echo, and reporting. "Orchestrator
reaction" is an event being published to the orchestrator acting on it:

| Load                     | Step                  | event wake | + worker wake |
|--------------------------|-----------------------|------------|---------------|
| Quiet system             | Activity round trip   | 58 to 92   | 2.0           |
| Quiet system             | Orchestrator reaction | 3.0        | 2.5           |
| 10 concurrent workflows  | Activity round trip   | 74.7       | 1.9           |
| 10 concurrent workflows  | Orchestrator reaction | 7.5        | 13.8          |
| 100 concurrent workflows | Activity round trip   | 69.6       | 5.8           |
| 100 concurrent workflows | Orchestrator reaction | 81.2       | 132.3         |

Quiet system, submit to completed as the client sees it (one workflow at a
time after a random 4 to 8 s idle gap; mean, ms):

| Workflow     | 0.9.1 | event wake | + worker wake |
|--------------|-------|------------|---------------|
| 1 activity   | 556   | 73.6       | 14.7          |
| Sequential-5 | 914   | 483        | 31.3          |

Other checks on an idle stack:

| Measurement                                       | event wake | + worker wake |
|---------------------------------------------------|------------|---------------|
| Database transactions per second, nothing running | 11         | 2             |
| A 2 s delayed activity completes late by (ms)     | 86 to 105  | 24 to 32      |

Reading the numbers:

- **The worker's poll timer is gone from the path.** An activity is picked
  up, run and reported in about 2 ms, where it used to wait up to 100 ms to be
  noticed. A quiet five-step workflow finishes in 31 ms instead of 483 ms
  (914 ms in 0.9.1).
- **Latency-bound scenarios gain the most.** Sequential-5 runs 3.9 times
  faster (5.4 times faster than 0.9.1), because each workflow was five timer
  waits and is now five round trips.
- **At 100 concurrent workflows nothing changes, because the orchestrator is
  the limit.** It processes events one at a time. Activities now complete in
  6 ms instead of 70 ms, so their events simply reach the orchestrator's queue
  sooner and wait there longer (reaction time 81 ms to 132 ms); throughput
  and end-to-end latency are unchanged within run-to-run variance (the same
  baseline code measured 135 wf/s here and 150 wf/s in the runs below).
  Processing events for different workflows in parallel is what would move
  this scenario.
- **Idle cost drops.** An idle worker no longer polls ten times a second:
  database activity with nothing running falls from 11 to 2 transactions a
  second.
- **Delayed activities are more punctual, not less**, even though the
  worker's idle timer went from 100 ms to 5 s: an empty poll tells the worker
  when the next one is due.

### 2026-11-08 (commit-order event cursor + NOTIFY wake, for 0.10.0)

Kruxia Flow only (std worker); Temporal and Airflow were not rerun. Three
configurations, each on a fresh stack with the service settings of
`docker-compose.yml` (PostgreSQL 18), one discarded warm-up pass and three
measured passes of the standard suite:

- **0.9.1**: the released code (visibility-grace cursor, 50 ms to 1 s poll timer)
- **new, notify**: commit-order cursor, consumers woken by `NOTIFY` (the default)
- **new, poll**: commit-order cursor on the timer alone (`KRUXIAFLOW_EVENT_WAKE=poll`)

Both server images were built locally from source with the same Dockerfile.
Host: Apple Silicon, Docker Desktop with 8 CPUs and 23 GiB.

**All 2,700 measured workflows completed in every configuration** (the
100%-success criterion; 0 errors and 0 warnings in the server logs).

Standard suite, mean of the three passes (throughput in wf/s, latency in ms):

| Scenario           | Metric     | 0.9.1 | new, notify | change | new, poll |
|--------------------|------------|-------|-------------|--------|-----------|
| Sequential-5       | Throughput | 15.4  | 21.3        | +38%   | 15.6      |
| Sequential-5       | P50        | 642   | 439         | -32%   | 601       |
| Sequential-5       | P95        | 776   | 508         | -35%   | 1087      |
| Sequential-5       | P99        | 798   | 549         | -31%   | 1097      |
| Parallel-10        | Throughput | 20.3  | 26.6        | +31%   | 23.5      |
| Parallel-10        | P50        | 478   | 345         | -28%   | 402       |
| Parallel-10        | P95        | 570   | 421         | -26%   | 492       |
| Parallel-10        | P99        | 609   | 432         | -29%   | 504       |
| High-Concurrency-3 | Throughput | 110.3 | 149.8       | +36%   | 134.1     |
| High-Concurrency-3 | P50        | 805   | 578         | -28%   | 629       |
| High-Concurrency-3 | P95        | 1020  | 773         | -24%   | 843       |
| High-Concurrency-3 | P99        | 1161  | 864         | -26%   | 963       |

Orchestrator reaction time: from an event being published to the orchestrator
acting on it, read from event timestamps in the database (`WorkflowCreated` or
`ActivityCompleted` to the `ActivityScheduled` / `WorkflowCompleted` that
follows it in the same workflow). One pass per configuration; ms:

| Load                                | Metric | 0.9.1 | new, notify | new, poll |
|-------------------------------------|--------|-------|-------------|-----------|
| Sequential-5 (10 concurrent)        | mean   | 41.5  | 7.4         | 49.1      |
| Sequential-5 (10 concurrent)        | P50    | 37.5  | 7.9         | 39.7      |
| Sequential-5 (10 concurrent)        | P95    | 69.8  | 13.5        | 78.8      |
| High-Concurrency-3 (100 concurrent) | mean   | 139.5 | 76.5        | 71.0      |
| High-Concurrency-3 (100 concurrent) | P50    | 148.0 | 82.8        | 70.3      |
| High-Concurrency-3 (100 concurrent) | P95    | 197.1 | 102.1       | 121.8     |

Quiet system: one workflow at a time, with a random 4 to 8 s idle gap before
each (30 single-activity workflows, then 10 Sequential-5); ms:

| Measurement                                   | Metric | 0.9.1 | new, notify | new, poll |
|-----------------------------------------------|--------|-------|-------------|-----------|
| Orchestrator reaction, first event after idle | mean   | 456   | 5.8         | 486       |
| Orchestrator reaction, first event after idle | P95    | 936   | 8.7         | 986       |
| Orchestrator reaction, follow-up events       | mean   | 24.7  | 2.2         | 42.0      |
| Orchestrator reaction, follow-up events       | P95    | 52.6  | 3.2         | 77.1      |
| Activity round trip (queue + worker)          | mean   | 64.6  | 80.9        | 55.1      |
| 1-activity workflow, submit to completed      | mean   | 556   | 73          | 579       |
| 1-activity workflow, submit to completed      | P95    | 1054  | 115         | 1128      |
| Sequential-5 workflow, submit to completed    | mean   | 914   | 509         | 1030      |

Reading the numbers:

- **The wake removes the poll timer from the path.** On a quiet system the
  first event used to wait out whatever was left of the idle timer (up to
  1 s, 456 ms on average); it is now picked up in about 6 ms. Follow-up
  events go from about 25 ms to about 2 ms.
- **At moderate concurrency the orchestrator reacts in under 10 ms** (7.4 ms
  mean at 10 concurrent, from 41.5 ms).
- **At 100 concurrent the orchestrator is processing-bound, not
  waiting-bound**: it handles events one at a time, and reaction time
  (about 75 ms) is queueing behind the events ahead. The gain there comes
  from dropping the fixed 50 ms sleep after every batch, and timer-only mode
  gets it too. Writing transactions were sampled every 10 ms during a pass
  and none stayed open longer than about 10 ms, so the commit-order cursor's
  hold-back (it delivers only below the oldest open transaction) is not a
  factor in these runs.
- **The activity round trip is now the dominant term.** A step spends about
  65 to 100 ms between `ActivityScheduled` and `ActivityCompleted`, almost all
  of it the built-in worker's 100 ms activity-queue poll, which this work did
  not touch. Sequential-5 on a quiet system is five of those. The same wake
  pattern applied to the activity queue was the next step, and is measured in
  the section above.
- **Timer-only mode is a fallback, and behaves like one.** Its throughput is
  equal to or better than 0.9.1, and the first event after idle waits for the
  timer exactly as before. Two caveats: follow-up events on a quiet system
  are about 17 ms slower than 0.9.1 (the loop now drains and backs off sooner,
  where the old loop's fixed sleep happened to keep the timer short), and its
  Sequential-5 P95 above is inflated by where the suite's start happened to
  land in the 1 s idle timer (the first ten workflows of a pass wait for it;
  0.9.1 has the same exposure and landed early in these passes, as the
  matching first-event figures in the quiet table show).

### 2026-07-20 (engine 0.8.0 + event-delivery fix)

Run `results-20260720-185807.json`. **All 900 workflows across all platforms
completed, 100% success on every scenario.**

| Platform             | Sequential-5 | Parallel-10 | High-Concurrency-3 |
|----------------------|--------------|-------------|---------------------|
| Kruxia Flow          | 15.4 wf/s    | 18.7 wf/s   | 86.6 wf/s           |
| Kruxia Flow (py-std) | 15.2 wf/s    | 16.4 wf/s   | 102.9 wf/s          |
| Temporal             | 13.4 wf/s    | 12.7 wf/s   | 46.0 wf/s           |
| Airflow              | 2.2 wf/s     | 1.9 wf/s    | 7.1 wf/s            |

Compared to 2026-03-01 (five engine releases earlier: worker SDK rewrite,
budget integrity, per-attempt event dedup, retry authority, scheduler),
Kruxia Flow throughput is flat-to-better with tighter tail latencies
(Sequential-5 P95 1239→832 ms, Parallel-10 P95 800→584 ms). Temporal
shows large run-to-run variance on this host (46-68 wf/s high-concurrency
across runs); compare within a run, not across runs.

**The first 2026-07-20 run caught a real durability bug**: at 100-way
concurrency, 2 of 900 workflows hung — their activities completed, but the
completion events committed milliseconds after the consumer cursor had
already read past their UUIDv7 ids, stranding them forever (id order ≠
commit order). The fix (visibility-grace cursor + fully replay-idempotent
event handlers) shipped before these numbers were recorded; the
100%-success criterion above exists because of this find. The grace window
was later replaced by a commit-safe `(tx, position)` cursor. Details:
`docs/architecture.md` § "Event Source Interface" and CHANGELOG (0.8.1,
Unreleased).

### 2026-03-01 (commit 86e9ac7)

Compared to the previous run (2026-02-02), Kruxia Flow performance improved:

| Scenario           | Metric         | Feb 2   | Mar 1   | Change |
|--------------------|----------------|---------|---------|--------|
| Sequential-5       | Throughput     | 15.6    | 15.0    | -3%    |
| Parallel-10        | Throughput     | 14.2    | 17.5    | +23%   |
| High-Concurrency-3 | Throughput    | 70.7    | 74.0    | +5%    |
| Parallel-10        | P95 Latency   | 1406 ms | 687 ms  | -51%   |
| High-Concurrency-3 | P95 Latency   | 2019 ms | 1503 ms | -26%   |
| High-Concurrency-3 | Peak Memory   | 328 MB  | 343 MB  | +5%    |

Throughput in wf/sec (higher is better). Latency in ms (lower is better).

Key improvements: Parallel-10 throughput up 23% and P95 latency cut in half. High-Concurrency-3 throughput up 5% with P95 latency down 26%.

New platforms added in March: **Kruxia Flow (py-std)** and **Airflow**.

| Platform              | Sequential-5 | Parallel-10 | High-Concurrency-3 |
|-----------------------|--------------|-------------|---------------------|
| Kruxia Flow           | 15.0 wf/s    | 17.5 wf/s   | 74.0 wf/s           |
| Kruxia Flow (py-std)  | 15.2 wf/s    | 17.1 wf/s   | 103.4 wf/s          |
| Temporal              | 13.1 wf/s    | 26.1 wf/s   | 47.7 wf/s           |
| Airflow               | 2.5 wf/s     | 2.1 wf/s    | 7.1 wf/s            |

## Caveats

- Docker overhead may affect absolute performance
- Results should be compared relatively (apples-to-apples)
- For production benchmarks, use native deployments
- All benchmarks use echo activities for MVP (realistic workflows post-MVP)

## Contributing

To add new scenarios or platforms:

1. Add workflow definitions in the respective platform directory
2. Update benchmark runner to include new scenarios
3. Update `run_benchmark.py` to execute new scenarios
4. Document methodology and expected results

## License

Apache-2.0 (matches Kruxia Flow license)
