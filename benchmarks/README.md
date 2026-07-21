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

- **Kruxia Flow**: ~85-105 wf/sec high-concurrency; ~15-19 wf/sec on the
  latency-bound sequential/parallel scenarios (dominated by per-step
  round-trips, not engine capacity)
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
100%-success criterion above exists because of this find. Details:
`docs/architecture.md` § "Visibility-grace cursor" and CHANGELOG.

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
