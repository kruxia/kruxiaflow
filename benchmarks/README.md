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

- **Kruxia Flow**: ~150 wf/sec high-concurrency; ~21-27 wf/sec on the
  latency-bound sequential/parallel scenarios (dominated by per-step
  round-trips, not engine capacity). From 0.10.0; through 0.9.x it was
  ~85-110 and ~15-20 (see the 2026-11-08 results)
- **Temporal**: ~13-66 wf/sec (high run-to-run variance on this host)
- **Airflow**: ~2-11 wf/sec (batch-oriented, not optimized for throughput)
- **Speedup**: ~1.5-2x vs Temporal average, ~10x vs Airflow average
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
  pattern applied to the activity queue is the next step.
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
