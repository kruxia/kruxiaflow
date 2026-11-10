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

Measured on the reference hardware (Apple Silicon, Docker Desktop; November 2026):

- **Kruxia Flow**: ~240-315 wf/sec high-concurrency, ~158-170 Sequential-5
  and ~77-88 Parallel-10, from 0.11.0 (parallel orchestrator, 8 partitions
  by default). 0.10.0 gave ~135-150, ~80-90 and ~42-44 (bound by the
  orchestrator processing events one at a time); through 0.9.x it was
  ~85-110 high-concurrency and ~15-20 on the other two, which were bound by
  poll timers (see the two 2026-11-08 result sets)
- **Temporal**: 1.32.2 gives ~35 wf/sec Sequential-5, ~34 Parallel-10 and
  ~118 high-concurrency, steady from pass to pass; 1.24.2 gave about half
  that with high run-to-run variance (~12-66 wf/sec)
- **Airflow**: 3.4.0 gives ~1-3 wf/sec (batch-oriented, not optimized for
  throughput); 3.0.1 gave ~2-7 in July
- **Speedup**: 0.11.0 against Temporal 1.32.2 is ~4.7x on Sequential-5,
  ~2.4x on Parallel-10 and ~2.3x on High-Concurrency-3, with lower latency
  at every percentile; a single workflow on an idle system completes in
  a little over half the time. 0.10.0 was ~2.5x, ~1.3x and ~1.2x; through
  0.9.x Kruxia Flow was ~1.2-2x against Temporal 1.24.2. Against Airflow
  3.4.0 it is ~80-90x
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

### 2026-11-10 (Airflow 3.4.0, footprint and memory)

Airflow brought up to date and rerun, and the size and memory figures
re-measured for all three platforms. Kruxia Flow 0.11.0 (published image,
default settings), Temporal 1.32.2 and Airflow 3.4.0 (was 3.0.1; CeleryExecutor,
two workers at concurrency 16, Redis 7, PostgreSQL 18 like the other two), each
alone on the host, two rounds in the order Kruxia Flow, Temporal, Airflow, a
fresh stack per round with a discarded warm-up and three measured passes: six
passes per figure. Load average 2-4 at the start of each round.

**Every workflow completed on all three platforms** (2,700 each over the
measured passes).

Throughput in wf/s (median with range) and latency in ms (median of the
passes):

| Scenario           | Metric     | Kruxia Flow 0.11.0  | Temporal 1.32.2     | Airflow 3.4.0 |
|--------------------|------------|---------------------|---------------------|---------------|
| Sequential-5       | Throughput | 155.7 (145.4-158.5) | 34.9 (32.6-34.9)    | 1.7 (1.6-2.1) |
| Sequential-5       | P50 / P95  | 57 / 82             | 298 / 330           | 6108 / 7338   |
| Parallel-10        | Throughput | 79.8 (78.5-81.4)    | 38.5 (36.6-39.9)    | 1.0 (0.9-1.0) |
| Parallel-10        | P50 / P95  | 111 / 176           | 248 / 325           | 10086 / 11826 |
| High-Concurrency-3 | Throughput | 250.0 (212.1-324.9) | 120.0 (114.1-124.1) | 3.2 (3.0-3.5) |
| High-Concurrency-3 | P50 / P95  | 361 / 554           | 524 / 1928          | 27006 / 43899 |

Kruxia Flow and Temporal agree with the twelve-pass session in the next
section, which remains the one to quote for those two. Kruxia Flow is about
90x, 80x and 80x Airflow's throughput. Airflow 3.4.0 is slower here than
3.0.1 was in July (2.2, 1.9 and 7.1 wf/s); the two were not run side by side,
so whether that is the version, PostgreSQL 18 or the host is not known.

Sizes, from the v0.11.0 release assets and the registry manifests (arm64
unless noted):

| Measure                     | Kruxia Flow 0.11.0                  | Temporal 1.32.2             | Airflow 3.4.0 |
|-----------------------------|-------------------------------------|-----------------------------|---------------|
| Server binary               | 12.9 MB (Linux arm64)               | 108 MB (`temporal-server`)  | n/a           |
|                             | 14.7 MB (Linux x86_64)              |                             |               |
|                             | 10.4 MB (macOS arm64)               |                             |               |
| Release download (tar.gz)   | 5.1-5.7 MB                          | n/a                         | n/a           |
| Image, compressed download  | 15.8 MB                             | 36.1 MB                     | 648 MB        |
| Image, unpacked             | 50 MB                               | 118 MB                      | 2.4 GB        |
| Other images needed         | PostgreSQL                          | PostgreSQL, `admin-tools`   | PostgreSQL,   |
|                             |                                     | (93 MB, schema setup)       | Redis         |

The 7.5 MB binary size quoted in earlier documentation does not match any
0.11.0 release binary and has been corrected.

Memory, in MB. "Suite monitor" is the sum over the platform's containers as
sampled by `shared/resource_monitor.py` (peak per scenario, range over the
measured passes). The per-container figures are from `docker stats` sampled
on the host over three passes on a fresh stack:

| Measure                           | Kruxia Flow 0.11.0 | Temporal 1.32.2 | Airflow 3.4.0 |
|-----------------------------------|--------------------|-----------------|---------------|
| At rest: server                   | 11                 | 72              | not measured  |
| At rest: database                 | 52                 | 130             | not measured  |
| Peak over three passes: server    | 549                | 261             | not measured  |
| Peak over three passes: database  | 142                | 260             | not measured  |
| Suite monitor, peak, all services | 700-926            | 699-979         | 4141-4970     |

Reading the memory figures:

- **At rest Kruxia Flow is small** (63 MB with its database against 202 MB),
  **but under this suite the 0.11.0 server grows to several hundred MB and
  does not give the memory back.** It rose pass by pass (64, 240, 480, 549 MB
  over the first three passes) and 0.10.0 behaves the same way (488 MB after
  three passes), so it is not the parallel orchestrator. Under load 0.11.0 is
  level with Temporal on memory, not ahead of it. July's 370 MB figure was a
  single pass. The cause was found and fixed the same day: see the next
  subsection.
- The suite monitor finds containers by compose project name and recorded
  nothing in the earlier sessions of this month, which ran under other
  project names; `BENCH_COMPOSE_PROJECT` now overrides the name.
- Kruxia Flow's scenarios last under two seconds, so the monitor's
  half-second sampling gives few samples per scenario and its CPU figures
  for Kruxia Flow are not usable.

#### The server's memory growth: cause and fix (fixed in 0.11.1)

What the growth is, from the server's own `/proc` figures (resident anonymous
memory, read through a sidecar in its pid namespace):

- **It is not a leak in the usual sense.** Over 30 consecutive passes (13,500
  workflows) 0.11.0 moved between 530 and 700 MB without a steady climb.
- **It does not come from workflows, orchestration or the database pool.**
  The `/health` endpoint, which touches none of them, reproduces it, and pool
  sizes of 20 and 100 give the same memory.
- **It is about 1.5 MB per simultaneously open client connection**, kept
  after the connections close. `serve` and `api` passed `axum::serve` a bare
  `Router`; axum 0.7 then copies the whole router (every route and handler,
  one of which holds the OpenAPI document) for each accepted connection.
  glibc's allocator does not return the peak to the system, and its
  per-thread arenas multiply it (about 4x under the suite: 154 MB with
  `MALLOC_ARENA_MAX=1`, 550-700 MB by default).

So a 0.11.0 server's memory is set by the largest number of simultaneous
connections it has ever served and stays there until restart; it does not
grow with workflows run or with uptime at a steady connection count. The
benchmark client opens up to 100 connections, which is the whole of the
figure above.

The fix is to hand `axum::serve` the router's make-service, which shares one
router between connections. Server memory in MB after each step, steps run in
order on one fresh stack (3,000-6,000 `/health` requests per step; default
allocator settings unless noted):

| Simultaneous connections | 0.11.0 | 0.11.0, one arena | With the fix | With the fix, one arena |
|--------------------------|--------|-------------------|--------------|-------------------------|
| 100                      | 155    | 153               | 8            | 7                       |
| 300                      | 1216   | 459               | 19           | 11                      |
| 300, repeated            | 1405   | 494               | 22           | 13                      |
| 1,000                    | 2418   | 1494              | 34           | 25                      |

Under workflow load, with the fix:

| Measure                                             | 0.11.0  | With the fix |
|-----------------------------------------------------|---------|--------------|
| Server after 30 passes of the suite (13,500 wf), MB | 619     | 27           |
| Server, range over passes 3-30, MB                  | 534-697 | 25-27        |
| Server after 3,000 workflows at 300 concurrent, MB  | 731     | 18           |
| Peak over three passes, server (`docker stats`), MB | 549     | 17           |
| Peak over three passes, server + database, MB       | 690     | 236          |

With the fix the server is flat from the fourth pass on, and the database is
the larger part of the footprint. Throughput is unchanged (one round:
159.1 / 81.2 / 293.5 wf/s, every workflow completed, no server warnings).
The footprint table in the main README still shows the released 0.11.0.

On 32-bit ARM (the Raspberry Pi Zero build, `arm-unknown-linux-gnueabihf`)
the copy is about 1 MB per connection. Server resident memory in MB after
each step:

| Simultaneous connections | 0.8.3 on a Pi Zero W | Fix on the same Pi Zero W | 0.8.3, emulated ARM1176 | Fix, emulated ARM1176 |
|--------------------------|----------------------|---------------------------|-------------------------|-----------------------|
| start                    | 9                    | 13                        | 45                      | 44                    |
| 10                       | 18                   | 14                        | 57                      | 47                    |
| 30                       | 37                   | 14                        | 86                      | 48                    |
| 50                       | 57                   | 15                        | 134                     | 48                    |
| 100                      | not run              | 15                        | 221                     | 51                    |
| 200                      | not run              | 17                        | not run                 | not run               |
| 300                      | not run              | not run                   | 506                     | 51                    |
| 300, repeated            | not run              | not run                   | 789                     | 54                    |

The Pi is a Zero W (ARMv6, one core, 427 MB) whose service runs with
`MemoryMax=150M`; 0.8.3 is the version it was running, and it was not taken
past 50 connections so as not to reach that limit. The fixed build ran
beside it on another port with a scratch database, and completed 20
three-activity and 10 ten-activity-parallel workflows there with no warnings.
The emulated figures (QEMU user mode, Raspbian userland in Docker) include
the emulator's own memory, which is the 44 MB at start.

Raw results are in `results/2026-11-10-airflow-3.4-footprint/` (not in
version control).

### 2026-11-09 (0.11.0 against Temporal 1.32.2)

The released parallel orchestrator against current Temporal, in one session:
the published `kruxia/kruxiaflow:0.11.0` image with default settings (8
partitions) and Temporal server 1.32.2 with the stack described in the
0.10.0 comparison below (Python SDK 1.35.0, PostgreSQL 18 for both). Each
platform alone, four alternating rounds, a fresh stack per round with a
discarded warm-up and three measured passes: twelve passes per figure. Other
projects' containers were up but idle (load average 3-6); no round was
discarded.

**Every workflow completed on both platforms** (5,400 each over the measured
passes); Kruxia Flow's server log had no warnings, Temporal's the usual
handful of transient errors (0-9 per round).

Standard suite: median of twelve passes, with the range of throughput (wf/s)
and latency in ms:

| Scenario           | Metric     | Kruxia Flow 0.11.0  | Temporal 1.32.2    | Kruxia Flow vs Temporal |
|--------------------|------------|---------------------|--------------------|-------------------------|
| Sequential-5       | Throughput | 163.0 (157.8-169.6) | 34.9 (33.7-36.8)   | 4.7x                    |
| Sequential-5       | P50        | 56                  | 298                | 5.3x lower              |
| Sequential-5       | P95        | 72                  | 331                | 4.6x lower              |
| Parallel-10        | Throughput | 81.1 (77.4-88.2)    | 34.1 (31.4-38.7)   | 2.4x                    |
| Parallel-10        | P50        | 111                 | 273                | 2.5x lower              |
| Parallel-10        | P95        | 168                 | 347                | 2.1x lower              |
| High-Concurrency-3 | Throughput | 267.1 (239.1-316.5) | 118.0 (91.1-124.3) | 2.3x                    |
| High-Concurrency-3 | P50        | 324                 | 544                | 1.7x lower              |
| High-Concurrency-3 | P95        | 499                 | 1985               | 4.0x lower              |
| High-Concurrency-3 | P99        | 593                 | 2242               | 3.8x lower              |

One workflow at a time on an otherwise idle system, after a random 4-8 s idle
gap (30 one-activity and 10 five-activity workflows per round; mean
end-to-end latency in ms as the client sees it, one figure per round):

| Workflow     | Kruxia Flow 0.11.0     | Temporal 1.32.2        |
|--------------|------------------------|------------------------|
| 1 activity   | 20.4, 20.3, 21.5, 20.0 | 34.8, 31.3, 34.7, 34.8 |
| 5 activities | 38.6, 32.5, 32.7, 34.4 | 65.2, 63.8, 63.3, 67.4 |

Reading these:

- **This confirms the cross-session estimate in the next section** (4.6x,
  2.4x and 2.3x) with both platforms measured side by side: 4.7x, 2.4x and
  2.3x. Temporal's figures match its earlier session on every scenario, so
  the change against the 0.10.0 comparison (2.5x, 1.3x, 1.2x) is all on
  Kruxia Flow's side.
- **At 100 concurrent workflows Kruxia Flow now leads on every latency
  figure.** With 0.10.0 Temporal's median was lower (554 vs 654 ms); it is
  now 544 against 324 ms, and the tail is a quarter of Temporal's. The
  orchestrator's mean reaction time in that scenario was 15-22 ms per round
  (about 130 ms in 0.10.0), and 3.5-3.8 ms at 10 concurrent.
- **Idle latency is unchanged** on both sides: about 20 vs 34 ms for one
  activity, 35 vs 65 ms for five.
- The caveats of the 0.10.0 comparison still apply: each platform out of the
  box on one machine, Temporal's worker is the Python SDK in the load
  generator's process, and the Kruxia Flow client polls for completion every
  50 ms, so Sequential-5's 56 ms P50 is close to the least this client can
  report.

Raw results are in `results/2026-11-09-v0.11.0-temporal-1.32/` (not in
version control).

### 2026-11-09 (parallel orchestrator, for 0.11.0)

The orchestrator handling events of different workflows concurrently
(`docs/implementation/2026-11-09-parallel-orchestrator.md`), against 0.10.0,
across partition counts. One session on a quiet host (load average 1-3,
nothing else running in Docker), configurations alternated, each run a fresh
stack with a discarded warm-up and three measured passes. 0.10.0, 4 and 8
partitions were run twice (six passes per figure), the others once (three).

**Every workflow completed in every run** (1,840 per run, no server warnings).

Throughput in wf/s, median with range:

| Configuration | Sequential-5        | Parallel-10      | High-Concurrency-3  |
|---------------|---------------------|------------------|---------------------|
| 0.10.0        | 90.2 (84.7-94.8)    | 44.9 (41.6-46.1) | 140.3 (106.2-144.4) |
| 1 partition   | 89.2 (83.2-89.8)    | 42.1 (41.4-44.1) | 132.2 (127.8-134.9) |
| 2 partitions  | 113.4 (111.1-118.4) | 56.9 (54.4-60.3) | 181.3 (170.0-188.5) |
| 4 partitions  | 148.1 (135.8-154.5) | 71.5 (65.4-76.4) | 233.7 (184.4-272.4) |
| 8 partitions  | 159.7 (153.7-166.7) | 80.7 (80.0-87.4) | 268.6 (250.1-313.2) |
| 16 partitions | 167.3 (161.6-167.4) | 81.2 (75.7-86.9) | 254.4 (211.6-264.1) |

Client latency in ms, P50 / P95 (median of the passes):

| Configuration | Sequential-5 | Parallel-10 | High-Concurrency-3 |
|---------------|--------------|-------------|--------------------|
| 0.10.0        | 108 / 122    | 216 / 264   | 650 / 915          |
| 1 partition   | 109 / 123    | 218 / 274   | 687 / 958          |
| 2 partitions  | 67 / 120     | 173 / 222   | 489 / 830          |
| 4 partitions  | 57 / 109     | 112 / 215   | 403 / 610          |
| 8 partitions  | 57 / 90      | 112 / 193   | 310 / 544          |
| 16 partitions | 57 / 69      | 113 / 168   | 348 / 546          |

Orchestrator reaction time (an event's commit to the next event of its
workflow, from event timestamps in the database; mean in ms, one figure per
run):

| Configuration | 10 concurrent (Sequential-5) | 100 concurrent (High-Concurrency-3) |
|---------------|------------------------------|-------------------------------------|
| 0.10.0        | 12.0, 11.3                   | 139.6, 122.7                        |
| 1 partition   | 12.7                         | 133.9                               |
| 2 partitions  | 7.2                          | 75.0                                |
| 4 partitions  | 5.1, 4.8                     | 39.2, 45.3                          |
| 8 partitions  | 3.9, 3.7                     | 20.8, 23.6                          |
| 16 partitions | 3.2                          | 10.4                                |

Reading these:

- **One partition is 0.10.0.** The restructured loop costs nothing when it
  runs serially.
- **Eight partitions about doubles throughput at 100 concurrent workflows and
  cuts the orchestrator's reaction time there from about 130 ms to about
  22 ms.** Eight is the default.
- **The low-concurrency scenarios gain as much**, which the plan did not
  expect: at 10 concurrent workflows the serial orchestrator was already
  queueing (12 ms reaction time against 3-4 ms with eight partitions).
  Sequential-5 P50 latency of 57 ms is close to the floor this client can
  report: it polls for completion every 50 ms.
- **Past eight partitions throughput stops rising** although reaction time
  keeps falling, so something other than the orchestrator is the limit at
  that point. Which part (the built-in worker claiming one activity per poll,
  the load generator, PostgreSQL) was not measured.
- **A single workflow on an idle system is unchanged** (20-23 ms for one
  activity, 34-39 ms for five, in every configuration).
- **Against Temporal 1.32.2** (figures from the section below, taken earlier
  the same day on the same host, not in this session): 159.7 vs 34.9,
  80.7 vs 33.0 and 268.6 vs 117.3 wf/s, that is 4.6x, 2.4x and 2.3x, where
  0.10.0 was 2.5x, 1.3x and 1.2x. At 100 concurrent workflows the P50 is now
  310 ms against Temporal's 554 ms.
- **Not measured:** the effect of handlers waking the poller when they finish
  (plan §2.4). The with/without comparison needs a second build and was not
  run.

An earlier session on a busy host showed the same ordering with lower
absolute numbers and is kept with the raw results
(`results/2026-11-09-parallel-orchestrator/`, not in version control).

### 2026-11-09 (0.10.0 against Temporal 1.32.2)

The same comparison as the next section, with Temporal brought up to date:
server 1.32.2 (the current release; the all-in-one `auto-setup` image was
deprecated with 1.29, so the stack now follows Temporal's reference
docker-compose: `temporalio/admin-tools` creates the schema and the
namespace, `temporalio/server` serves, default settings and the reference
`development-sql.yaml` dynamic config), Python SDK 1.35.0, and PostgreSQL 18
for both platforms. Kruxia Flow is the published `kruxia/kruxiaflow:0.10.0`
image. Each platform alone on the host, alternating, a fresh stack per run
with a discarded warm-up and three measured passes. Six runs each were made;
the first two rounds of each platform coincided with heavy host load (load
average near 6, another project's stack starting) and show it on both sides
(Temporal passes at 5-8 wf/s, Kruxia Flow at 37-40), so the figures below
are the four clean rounds: twelve passes per figure.

**Every workflow completed on both platforms** (5,400 each over the four
rounds; 8,100 counting the discarded ones).

Standard suite: median of twelve passes, with the range of throughput (wf/s)
and latency in ms:

| Scenario           | Metric     | Kruxia Flow 0.10.0  | Temporal 1.32.2     | Kruxia Flow vs Temporal |
|--------------------|------------|---------------------|---------------------|-------------------------|
| Sequential-5       | Throughput | 87.5 (81.8-93.4)    | 34.9 (33.7-36.2)    | 2.5x                    |
| Sequential-5       | P50        | 109                 | 297                 | 2.7x lower              |
| Sequential-5       | P95        | 143                 | 314                 | 2.2x lower              |
| Parallel-10        | Throughput | 42.0 (38.5-46.1)    | 33.0 (28.7-37.3)    | 1.3x                    |
| Parallel-10        | P50        | 218                 | 282                 | 1.3x lower              |
| Parallel-10        | P95        | 278                 | 358                 | 1.3x lower              |
| High-Concurrency-3 | Throughput | 138.7 (103.3-142.3) | 117.3 (109.5-127.8) | 1.2x                    |
| High-Concurrency-3 | P50        | 654                 | 554                 | 1.2x higher             |
| High-Concurrency-3 | P95        | 906                 | 1769                | 2.0x lower              |
| High-Concurrency-3 | P99        | 1036                | 2146                | 2.1x lower              |

One workflow at a time on an otherwise idle system, after a random 4-8 s idle
gap (30 one-activity and 10 five-activity workflows per run; mean end-to-end
latency in ms as the client sees it, one figure per clean round):

| Workflow     | Kruxia Flow 0.10.0     | Temporal 1.32.2        |
|--------------|------------------------|------------------------|
| 1 activity   | 20.4, 20.6, 17.9, 19.7 | 36.4, 34.7, 35.7, 36.7 |
| 5 activities | 37.5, 35.4, 36.2, 30.8 | 69.5, 67.1, 73.6, 66.4 |

What changed against the 1.24.2 comparison below, and what to keep in mind:

- **Temporal 1.32.2 is about twice as fast as 1.24.2 on this suite** (34.9
  vs 17.6, 33.0 vs 18.5 and 117.3 vs 55.9 wf/s), and far steadier from pass
  to pass: the bimodal Parallel-10 results are gone. Kruxia Flow's figures
  are the same as the day before. The margins are therefore 2.5x, 1.3x and
  1.2x rather than 5.1x, 2.4x and 2.5x. Quote these, not the older ones.
- **At 100 concurrent workflows Temporal's median latency is now lower than
  Kruxia Flow's** (554 vs 654 ms), while its tail is still twice as long
  (P95 1769 vs 906 ms). Kruxia Flow's orchestrator processes events one at a
  time, so its latency there is queueing and is spread evenly across
  workflows; on Temporal most workflows finish sooner and some wait much
  longer.
- **Idle latency did not move**: a single workflow still completes in about
  half the time on Kruxia Flow (about 20 vs 36 ms for one activity, 35 vs
  70 ms for five).
- **This is each platform out of the box on one machine, not either at its
  best.** Temporal runs all its services in one process with the reference
  compose's default settings, and its worker is the
  Python SDK inside the load generator's process. Kruxia Flow's worker is
  compiled Rust inside the server. Temporal also records more per step (a
  full replayable history).
- **The client measures the two differently, in Temporal's favour.** The
  Kruxia Flow client learns of completion by polling the status endpoint
  every 50 ms (every 5 ms in the idle test); the Temporal client is told at
  once. Kruxia Flow's standard-suite latencies include up to 50 ms of waiting
  that is the client's, not the engine's. At Parallel-10 and High-Concurrency
  that is a small share; at Sequential-5 (109 ms) it is not.
- Temporal's server log carried a handful of transient errors per run
  ("Queue reader unable to retrieve tasks", "Update workflow execution
  operation failed"; 1-13 per run) with no failed workflows. Kruxia Flow's
  log had none.

The Temporal stack and idle test used here are kept with the raw results
(`results/2026-11-09-temporal-1.32/`, not in version control); the stack
itself is in `docker-compose.yml`.

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

| Session    | Engine  | Temporal | Sequential-5: KF / Temporal | Parallel-10: KF / Temporal | High-Concurrency-3: KF / Temporal |
|------------|---------|----------|-----------------------------|----------------------------|-----------------------------------|
| 2026-03-01 | 86e9ac7 | 1.24.2   | 15.0 / 13.1                 | 17.5 / 26.1                | 74.0 / 47.7                       |
| 2026-07-20 | 0.8.0   | 1.24.2   | 15.4 / 13.4                 | 18.7 / 12.7                | 86.6 / 46.0                       |
| 2026-11-08 | 0.10.0  | 1.24.2   | 89.9 / 17.6                 | 44.1 / 18.5                | 139.1 / 55.9                      |
| 2026-11-09 | 0.10.0  | 1.32.2   | 87.5 / 34.9                 | 42.0 / 33.0                | 138.7 / 117.3                     |
| 2026-11-09 | 0.11.0  | 1.32.2   | 163.0 / 34.9                | 81.1 / 34.1                | 267.1 / 118.0                     |

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
- **Temporal 1.24.2 is not current.** It was pinned for continuity with the
  earlier sessions; the section above repeats the comparison against 1.32.2,
  which is about twice as fast on this suite.
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
