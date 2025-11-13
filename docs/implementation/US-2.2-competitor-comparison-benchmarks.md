2025-11-10# US-2.2: Competitor Comparison Benchmarks - Implementation Plan

**Epic**: 2 - Performance Benchmarking and Validation
**User Story**: US-2.2
**Status**: ✅ IMPLEMENTED
**Actual Effort**: ~2 hours (implementation only, testing deferred)
**Priority**: P0 (Required for Epic 2 validation)
**Architecture**: Python client benchmarks, StreamFlow vs Temporal vs Airflow

---

## User Story

**As** a platform engineering lead
**I want** reproducible benchmarks vs Temporal and Airflow
**So that** I can prove 10x performance to leadership and validate our architecture early

## Acceptance Criteria

- [ ] Same workflow implemented on each platform (echo activity for MVP)
- [ ] Same hardware for each platform (controlled environment)
- [ ] Docker Compose setup for reproducibility
- [ ] Published methodology: Open-source on GitHub
- [ ] Results: HTML report with charts
- [ ] Target proof: StreamFlow >1,000 wf/sec vs Temporal 35-100 wf/sec vs Airflow 10-50 wf/sec
- [ ] Critical: Run after Epic 1 to validate event-driven architecture

---

## Current State Analysis

### StreamFlow Benchmark Structure (Current)
- **Rust-based**: `benchmark/` crate with load tests in Rust
- **Shell script orchestration**: `scripts/profiling.sh` drives the benchmark suite
- **Memory profiling**: `scripts/profile_memory.sh` for memory analysis
- **Metrics collected**: Throughput (wf/sec), latency (P50/P95/P99), success rate
- **Output format**: JSON results in `var/benchmark-TIMESTAMP/results.json`
- **Database state management**: Truncate before each test, preserve after for inspection
- **Benchmark scenarios**:
  - Sequential workflow (5 activities, 100 workflows)
  - Parallel workflow (10 parallel activities, 50 workflows)
  - High concurrency (3 activities, 300 workflows, 100 concurrent)
  - Sustained throughput (120 seconds, 20 concurrent)

### StreamFlow v0.1 Approach
- **Python-based**: Unified benchmark suite at `benchmarks/`
- **Platform abstraction**: Common interface for all platforms
- **Consistent methodology**: Same workflows, same metrics, same reporting
- **Note**: v0.1 repository link returned 404 - may need to access locally or reconstruct

### Competitor Research

#### Temporal
- **Performance**: "Isn't winning any throughput benchmarks" due to complete event history overhead
- **Claimed capacity**: Database bottleneck typically limits throughput
- **Benchmark tool**: Maru (https://github.com/temporalio/maru) - load testing tool
- **Benchmark workers**: https://github.com/temporalio/benchmark-workers

#### Cadence
- **Performance**: Claims "up to 10k workflows per second" in tests
- **Production scale**: 12 billion workflow executions/month at Uber
- **Benchmark tool**: `cadence-bench` tool included in repository
- **Note**: Cadence and Temporal share similar architecture (Temporal forked from Cadence)

#### Airflow
- **Performance**: Traditional batch-oriented DAG execution
- **Latency**: Task scheduling overhead, not optimized for low-latency
- **Scalability**: Designed for data pipelines, not high-throughput orchestration
- **Market**: Dominant in data engineering space (70%+ market share for ETL)
- **Note**: Most direct competitor for batch workflow use cases

#### Prefect (Deferred to Post-MVP)
- **Performance**: 90% improvement in runtime overhead in v3.0
- **Latency**: Agent polls every 10 seconds, not sub-second latency
- **Scalability concerns**: Database chokes at hundreds of concurrent workflows
- **Note**: Performance "depends entirely on your deployment"

---

## Implementation Overview

This implementation creates a **Python-based unified benchmark suite** that:

1. Runs identical workflows on StreamFlow and competitor platforms
2. Uses Docker Compose for reproducible environments
3. Collects consistent metrics across all platforms
4. Generates comparative HTML reports with charts
5. Normalizes the StreamFlow benchmarking approach for future maintainability

### Key Design Decisions

**Architecture Principles**:
1. ✅ **Separate concerns**: Keep Rust benchmarks as internal profiling tool
2. ✅ **Apples-to-apples**: Python HTTP clients for both StreamFlow and Temporal
3. ✅ **Minimal dependencies**: Only `httpx` required for StreamFlow benchmarks
4. ✅ **Simple workflow definitions**: Python dicts/lists (JSON-serializable)
5. ✅ **In-cluster execution**: Benchmark runners in Docker containers
6. ✅ **Sequential execution**: One platform at a time (no cross-contamination)
7. ✅ **Comprehensive scope**: StreamFlow vs Temporal vs Airflow (covers different market segments)
8. ⚠️ **Deferred metrics**: Skip system monitoring for MVP (add later for all platforms)

**Why This Approach**:
- **Fair comparison**: All platforms benchmarked via HTTP/SDK from external client
- **Clean separation**: Rust benchmarks remain for internal regression testing
- **Maintainable**: Simple Python code, no complex dependencies
- **Credible**: Transparent methodology, reproducible results
- **Market coverage**: Temporal (durable execution), Airflow (data pipelines) = different segments

### Architecture

```mermaid
flowchart TB
    subgraph "Benchmark Runners (Python in Docker)"
        SF_Bench[StreamFlow Benchmark<br/>httpx HTTP client<br/>Python dicts for workflows]
        Temp_Bench[Temporal Benchmark<br/>temporalio SDK<br/>Python workflow classes]
        AF_Bench[Airflow Benchmark<br/>apache-airflow client<br/>Python DAG definitions]
    end

    subgraph "Platform: StreamFlow"
        SF_Binary[streamflow binary<br/>--release build]
        SF_API[API Server :8080]
        SF_Orch[Orchestrator]
        SF_Worker[Built-in Worker]
        SF_DB[(PostgreSQL)]
    end

    subgraph "Platform: Temporal"
        Temp_Server[Temporal Server :7233]
        Temp_Worker[Temporal Worker<br/>Python activities]
        Temp_DB[(PostgreSQL)]
    end

    subgraph "Platform: Airflow"
        AF_Web[Airflow Webserver :8081]
        AF_Scheduler[Airflow Scheduler]
        AF_Executor[Celery Executor]
        AF_DB[(PostgreSQL)]
        AF_Redis[(Redis)]
    end

    subgraph "Metrics & Reports"
        Metrics[Metrics Collection<br/>Throughput, Latency P50/P95/P99<br/>Success Rate]
        JSON[results.json<br/>Per-platform data]
        HTML[comparison.html<br/>Charts and tables]
    end

    SF_Bench -->|POST /api/v1/workflows<br/>Register + Execute| SF_API
    SF_API --> SF_Orch
    SF_Orch --> SF_Worker
    SF_Orch --> SF_DB
    SF_Worker --> SF_DB

    Temp_Bench -->|SDK: start_workflow<br/>execute_workflow| Temp_Server
    Temp_Server --> Temp_Worker
    Temp_Server --> Temp_DB

    AF_Bench -->|REST API: trigger_dag<br/>poll status| AF_Web
    AF_Web --> AF_Scheduler
    AF_Scheduler --> AF_Executor
    AF_Executor --> AF_DB
    AF_Scheduler --> AF_Redis

    SF_Bench --> Metrics
    Temp_Bench --> Metrics
    AF_Bench --> Metrics
    Metrics --> JSON
    Metrics --> HTML

    note1[Note: Benchmarks run sequentially<br/>1: StreamFlow<br/>2: Temporal<br/>3: Airflow<br/>No cross-contamination]
```

### Workflow Scenarios

**MVP Scope**: Echo activity only (identical to internal benchmarks)
- **Sequential-5**: 5 echo activities in sequence (100 workflows)
- **Parallel-10**: 10 echo activities in parallel with fan-out/fan-in (50 workflows)
- **High-Concurrency-3**: 3 echo activities, 300 workflows, 100 concurrent

**Rationale**: Match existing StreamFlow internal benchmarks for direct comparison.

**Post-MVP**: Additional workflow types can be added after Epic 2 validation.

---

## Detailed Implementation

### 1. Directory Structure (30 minutes)

Create simplified `benchmarks/` at repository root:

```
benchmarks/
├── README.md                    # Usage and methodology
├── pyproject.toml               # Python package configuration
├── streamflow/
│   ├── benchmark.py            # StreamFlow HTTP client benchmark
│   └── workflows.py            # Workflow definitions (Python dicts)
├── temporal/
│   ├── benchmark.py            # Temporal SDK benchmark
│   ├── workflows.py            # Temporal workflow classes
│   └── activities.py           # Temporal echo activity
├── airflow/
│   ├── benchmark.py            # Airflow API client benchmark
│   ├── dags.py                 # Airflow DAG definitions
│   └── operators.py            # Custom echo operator
├── shared/
│   ├── metrics.py              # Metrics collection (all platforms)
│   └── report.py               # HTML report generator
├── docker-compose.yml           # All platforms in one file
├── run_benchmark.py             # Main CLI entry point
└── results/                     # Output directory (gitignored)
```

**Rationale**: Flat structure, no over-engineering, easy to understand.

### 2. Python Dependencies (10 minutes)

**File**: `benchmarks/pyproject.toml`

```toml
[project]
name = "streamflow-benchmarks"
version = "0.2.0"
description = "Cross-platform workflow engine benchmarks: StreamFlow vs Temporal vs Airflow"
requires-python = ">=3.11"
dependencies = [
    "click>=8.1.0",           # CLI framework (makes CLI much easier)
    "httpx>=0.27.0",          # Async HTTP client (for StreamFlow)
    "temporalio>=1.7.0",      # Temporal Python SDK (realistic platform usage)
    "apache-airflow>=2.9.0",  # Airflow for DAG definitions and API client
    "apache-airflow-providers-http>=4.0.0",  # HTTP provider for Airflow
    "jinja2>=3.1.0",          # HTML report templates
]

[project.scripts]
benchmark = "run_benchmark:cli"

[build-system]
requires = ["setuptools>=68.0"]
build-backend = "setuptools.build_meta"
```

**Rationale**:
- **Click**: Simplifies CLI argument parsing and command structure
- **httpx**: Async HTTP client for StreamFlow (apples-to-apples comparison)
- **temporalio**: Official SDK representing realistic Temporal usage
- **apache-airflow**: Core library for DAG definitions and API client
- **jinja2**: Template-based HTML report generation
- **No heavy analytics**: pandas, plotly, pydantic deferred (use Python dicts)

### 3. StreamFlow Workflow Definitions (1 hour)

**File**: `benchmarks/streamflow/workflows.py`

```python
"""StreamFlow workflow definitions as Python dicts (JSON-serializable)"""


def create_sequential_workflow(num_activities: int) -> dict:
    """Create sequential workflow with N echo activities"""
    activities = []

    for i in range(num_activities):
        activity = {
            "key": f"activity_{i}",
            "worker": "builtin",
            "name": "echo",
            "parameters": {},
        }

        # Add 'following' relationship (except for last activity)
        if i < num_activities - 1:
            activity["following"] = [{
                "activity_key": f"activity_{i + 1}",
                "conditions": None,
            }]

        activities.append(activity)

    return {
        "name": f"sequential_bench_{num_activities}",
        "activities": activities,
    }


def create_parallel_workflow(num_parallel: int) -> dict:
    """Create parallel workflow with fan-out and fan-in"""
    activities = [
        # Start activity (fans out)
        {
            "key": "start",
            "worker": "builtin",
            "name": "echo",
            "parameters": {},
            "following": [
                {"activity_key": f"parallel_{i}", "conditions": None}
                for i in range(num_parallel)
            ],
        }
    ]

    # Parallel activities
    for i in range(num_parallel):
        activities.append({
            "key": f"parallel_{i}",
            "worker": "builtin",
            "name": "echo",
            "parameters": {},
            "preceding": [{"activity_key": "start", "conditions": None}],
            "following": [{"activity_key": "end", "conditions": None}],
        })

    # End activity (fan-in)
    activities.append({
        "key": "end",
        "worker": "builtin",
        "name": "echo",
        "parameters": {},
        "preceding": [
            {"activity_key": f"parallel_{i}", "conditions": None}
            for i in range(num_parallel)
        ],
    })

    return {
        "name": f"parallel_bench_{num_parallel}",
        "activities": activities,
    }


# Pre-defined workflows matching internal benchmarks
SEQUENTIAL_5 = create_sequential_workflow(5)
SEQUENTIAL_3 = create_sequential_workflow(3)
PARALLEL_10 = create_parallel_workflow(10)
```
### 4. StreamFlow Benchmark Runner (2-3 hours)

**File**: `benchmarks/streamflow/benchmark.py`

```python
"""StreamFlow benchmark using HTTP API (httpx client)"""

import asyncio
import httpx
import time
import statistics
from dataclasses import dataclass
from .workflows import SEQUENTIAL_5, SEQUENTIAL_3, PARALLEL_10


@dataclass
class BenchmarkMetrics:
    """Aggregated benchmark metrics"""
    platform: str
    scenario: str
    total_workflows: int
    successful: int
    failed: int
    duration_seconds: float
    throughput_wf_per_sec: float
    latency_p50_ms: float
    latency_p95_ms: float
    latency_p99_ms: float
    success_rate: float


class StreamFlowBenchmark:
    """Benchmark runner for StreamFlow via HTTP API"""

    def __init__(
        self,
        base_url: str = "http://streamflow:8080",
        client_id: str = "benchmark",
        client_secret: str = "benchmark_secret",
    ):
        self.base_url = base_url
        self.client_id = client_id
        self.client_secret = client_secret
        self.access_token: str | None = None
        self.client: httpx.AsyncClient | None = None

    async def setup(self) -> None:
        """Initialize HTTP client and authenticate"""
        self.client = httpx.AsyncClient(timeout=30.0)

        # Get OAuth token
        response = await self.client.post(
            f"{self.base_url}/api/v1/oauth/token",
            data={
                "grant_type": "client_credentials",
                "client_id": self.client_id,
                "client_secret": self.client_secret,
            },
        )
        response.raise_for_status()
        self.access_token = response.json()["access_token"]

        # Register workflow definitions
        await self._register_workflow(SEQUENTIAL_5)
        await self._register_workflow(SEQUENTIAL_3)
        await self._register_workflow(PARALLEL_10)

    async def cleanup(self) -> None:
        """Close HTTP client"""
        if self.client:
            await self.client.aclose()

    async def _register_workflow(self, workflow_def: dict) -> None:
        """Register a workflow definition"""
        response = await self.client.post(
            f"{self.base_url}/api/v1/definitions",
            headers={"Authorization": f"Bearer {self.access_token}"},
            json=workflow_def,
        )
        response.raise_for_status()

    async def run_workflow(self, workflow_name: str) -> tuple[bool, float]:
        """Run a single workflow and return (success, latency_ms)"""
        start = time.time()

        # Create workflow
        response = await self.client.post(
            f"{self.base_url}/api/v1/workflows",
            headers={"Authorization": f"Bearer {self.access_token}"},
            json={"definition_name": workflow_name, "input": {}},
        )
        response.raise_for_status()
        workflow_id = response.json()["workflow_id"]

        # Poll for completion
        poll_interval = 0.05  # 50ms
        timeout = 30.0

        while (time.time() - start) < timeout:
            status_response = await self.client.get(
                f"{self.base_url}/api/v1/workflows/{workflow_id}",
                headers={"Authorization": f"Bearer {self.access_token}"},
            )
            status_response.raise_for_status()
            status = status_response.json()["status"]

            if status == "completed":
                latency_ms = (time.time() - start) * 1000
                return (True, latency_ms)
            elif status == "failed":
                latency_ms = (time.time() - start) * 1000
                return (False, latency_ms)

            await asyncio.sleep(poll_interval)

        # Timeout
        return (False, timeout * 1000)

    async def run_scenario(
        self,
        scenario_name: str,
        workflow_name: str,
        num_workflows: int,
        max_concurrent: int,
    ) -> BenchmarkMetrics:
        """Run a benchmark scenario"""
        semaphore = asyncio.Semaphore(max_concurrent)
        results: list[tuple[bool, float]] = []

        async def run_one():
            async with semaphore:
                result = await self.run_workflow(workflow_name)
                results.append(result)

        start = time.time()
        await asyncio.gather(*[run_one() for _ in range(num_workflows)])
        duration = time.time() - start

        # Calculate metrics
        successful = sum(1 for success, _ in results if success)
        failed = len(results) - successful
        latencies = [lat for success, lat in results if success]
        latencies.sort()

        return BenchmarkMetrics(
            platform="StreamFlow",
            scenario=scenario_name,
            total_workflows=num_workflows,
            successful=successful,
            failed=failed,
            duration_seconds=duration,
            throughput_wf_per_sec=num_workflows / duration,
            latency_p50_ms=percentile(latencies, 0.50),
            latency_p95_ms=percentile(latencies, 0.95),
            latency_p99_ms=percentile(latencies, 0.99),
            success_rate=(successful / num_workflows * 100) if num_workflows > 0 else 0,
        )


def percentile(sorted_values: list[float], p: float) -> float:
    """Calculate percentile from sorted values"""
    if not sorted_values:
        return 0.0
    index = int(len(sorted_values) * p)
    return sorted_values[min(index, len(sorted_values) - 1)]
```

### 5. Temporal Workflows and Activities (2-3 hours)

**File**: `benchmarks/temporal/activities.py`

```python
"""Temporal activities"""

from temporalio import activity


@activity.defn
async def echo_activity(input_data: dict) -> dict:
    """Simple echo activity matching StreamFlow's echo"""
    return input_data
```

**File**: `benchmarks/temporal/workflows.py`

```python
"""Temporal workflow definitions"""

from datetime import timedelta
from temporalio import workflow
from temporalio.common import RetryPolicy
from .activities import echo_activity


@workflow.defn
class SequentialBench5:
    """Sequential workflow with 5 echo activities"""

    @workflow.run
    async def run(self, input_data: dict) -> dict:
        result = input_data
        for i in range(5):
            result = await workflow.execute_activity(
                echo_activity,
                result,
                start_to_close_timeout=timedelta(seconds=10),
            )
        return result


@workflow.defn
class SequentialBench3:
    """Sequential workflow with 3 echo activities"""

    @workflow.run
    async def run(self, input_data: dict) -> dict:
        result = input_data
        for i in range(3):
            result = await workflow.execute_activity(
                echo_activity,
                result,
                start_to_close_timeout=timedelta(seconds=10),
            )
        return result


@workflow.defn
class ParallelBench10:
    """Parallel workflow with 10 echo activities"""

    @workflow.run
    async def run(self, input_data: dict) -> dict:
        # Start activity
        start_result = await workflow.execute_activity(
            echo_activity,
            input_data,
            start_to_close_timeout=timedelta(seconds=10),
        )

        # Parallel activities
        parallel_tasks = [
            workflow.execute_activity(
                echo_activity,
                start_result,
                start_to_close_timeout=timedelta(seconds=10),
            )
            for _ in range(10)
        ]
        parallel_results = await workflow.gather(*parallel_tasks)

        # End activity (fan-in)
        end_result = await workflow.execute_activity(
            echo_activity,
            {"results": parallel_results},
            start_to_close_timeout=timedelta(seconds=10),
        )

        return end_result
```

### 6. Temporal Benchmark Runner (2-3 hours)

**File**: `benchmarks/temporal/benchmark.py`

```python
"""Temporal benchmark using Python SDK"""

import asyncio
import time
from temporalio.client import Client
from temporalio.worker import Worker
from .workflows import SequentialBench5, SequentialBench3, ParallelBench10
from .activities import echo_activity


class TemporalBenchmark:
    """Benchmark runner for Temporal"""

    def __init__(
        self,
        host: str = "temporal:7233",
        worker: str = "builtin",
        task_queue: str = "benchmark-queue",
    ):
        self.host = host
        self.worker = worker
        self.task_queue = task_queue
        self.client: Client | None = None
        self.worker: Worker | None = None
        self.worker_task: asyncio.Task | None = None

    async def setup(self) -> None:
        """Connect to Temporal and start worker"""
        self.client = await Client.connect(self.host, worker=self.worker)

        # Start worker
        self.worker = Worker(
            self.client,
            task_queue=self.task_queue,
            workflows=[SequentialBench5, SequentialBench3, ParallelBench10],
            activities=[echo_activity],
        )
        self.worker_task = asyncio.create_task(self.worker.run())

        # Give worker time to start
        await asyncio.sleep(1.0)

    async def cleanup(self) -> None:
        """Stop worker and close client"""
        if self.worker_task:
            self.worker_task.cancel()
            try:
                await self.worker_task
            except asyncio.CancelledError:
                pass

    async def run_workflow(self, workflow_class, workflow_id: str) -> tuple[bool, float]:
        """Run a single workflow and return (success, latency_ms)"""
        start = time.time()

        try:
            handle = await self.client.start_workflow(
                workflow_class,
                {},
                id=workflow_id,
                task_queue=self.task_queue,
            )

            await asyncio.wait_for(handle.result(), timeout=30.0)
            latency_ms = (time.time() - start) * 1000
            return (True, latency_ms)

        except asyncio.TimeoutError:
            return (False, 30000.0)
        except Exception as e:
            latency_ms = (time.time() - start) * 1000
            return (False, latency_ms)

    async def run_scenario(
        self,
        scenario_name: str,
        workflow_class,
        num_workflows: int,
        max_concurrent: int,
    ) -> "BenchmarkMetrics":
        """Run a benchmark scenario"""
        from streamflow.benchmark import BenchmarkMetrics, percentile

        semaphore = asyncio.Semaphore(max_concurrent)
        results: list[tuple[bool, float]] = []

        async def run_one(index: int):
            async with semaphore:
                workflow_id = f"{workflow_class.__name__}-{index}-{time.time()}"
                result = await self.run_workflow(workflow_class, workflow_id)
                results.append(result)

        start = time.time()
        await asyncio.gather(*[run_one(i) for i in range(num_workflows)])
        duration = time.time() - start

        # Calculate metrics
        successful = sum(1 for success, _ in results if success)
        failed = len(results) - successful
        latencies = [lat for success, lat in results if success]
        latencies.sort()

        return BenchmarkMetrics(
            platform="Temporal",
            scenario=scenario_name,
            total_workflows=num_workflows,
            successful=successful,
            failed=failed,
            duration_seconds=duration,
            throughput_wf_per_sec=num_workflows / duration,
            latency_p50_ms=percentile(latencies, 0.50),
            latency_p95_ms=percentile(latencies, 0.95),
            latency_p99_ms=percentile(latencies, 0.99),
            success_rate=(successful / num_workflows * 100) if num_workflows > 0 else 0,
        )
```

### 7. Airflow DAG Definitions (2 hours)

**File**: `benchmarks/airflow/operators.py`

```python
"""Custom Airflow operator for echo activity"""

from airflow.models import BaseOperator
from airflow.utils.decorators import apply_defaults


class EchoOperator(BaseOperator):
    """Simple echo operator that returns its input"""

    @apply_defaults
    def __init__(self, input_data=None, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.input_data = input_data or {}

    def execute(self, context):
        """Execute the echo operation"""
        return self.input_data
```

**File**: `benchmarks/airflow/dags.py`

```python
"""Airflow DAG definitions for benchmarking"""

from datetime import datetime, timedelta
from airflow import DAG
from airflow.operators.python import PythonOperator
from .operators import EchoOperator


default_args = {
    "owner": "benchmark",
    "depends_on_past": False,
    "start_date": datetime(2024, 1, 1),
    "email_on_failure": False,
    "email_on_retry": False,
    "retries": 0,
}


def create_sequential_dag(num_activities: int, dag_id: str):
    """Create sequential DAG with N echo activities"""
    with DAG(
        dag_id=dag_id,
        default_args=default_args,
        schedule_interval=None,  # Manual trigger only
        catchup=False,
        max_active_runs=1,
    ) as dag:
        prev_task = None

        for i in range(num_activities):
            task = EchoOperator(
                task_id=f"echo_{i}",
                input_data={"activity": i},
            )

            if prev_task:
                prev_task >> task

            prev_task = task

    return dag


def create_parallel_dag(num_parallel: int, dag_id: str):
    """Create parallel DAG with fan-out and fan-in"""
    with DAG(
        dag_id=dag_id,
        default_args=default_args,
        schedule_interval=None,
        catchup=False,
        max_active_runs=1,
    ) as dag:
        start = EchoOperator(
            task_id="start",
            input_data={"stage": "start"},
        )

        parallel_tasks = []
        for i in range(num_parallel):
            task = EchoOperator(
                task_id=f"parallel_{i}",
                input_data={"activity": i},
            )
            start >> task
            parallel_tasks.append(task)

        end = EchoOperator(
            task_id="end",
            input_data={"stage": "end"},
        )

        for task in parallel_tasks:
            task >> end

    return dag


# Create DAG instances (Airflow discovers these automatically)
sequential_bench_5 = create_sequential_dag(5, "sequential_bench_5")
sequential_bench_3 = create_sequential_dag(3, "sequential_bench_3")
parallel_bench_10 = create_parallel_dag(10, "parallel_bench_10")
```

### 8. Airflow Benchmark Runner (2-3 hours)

**File**: `benchmarks/airflow/benchmark.py`

```python
"""Airflow benchmark using REST API"""

import asyncio
import httpx
import time
from datetime import datetime


class AirflowBenchmark:
    """Benchmark runner for Airflow via REST API"""

    def __init__(
        self,
        base_url: str = "http://airflow-webserver:8081",
        username: str = "admin",
        password: str = "admin",
    ):
        self.base_url = base_url
        self.username = username
        self.password = password
        self.client: httpx.AsyncClient | None = None

    async def setup(self) -> None:
        """Initialize HTTP client with basic auth"""
        self.client = httpx.AsyncClient(
            timeout=30.0,
            auth=(self.username, self.password),
        )

        # Wait for Airflow to be fully ready
        await self._wait_for_ready()

        # Unpause DAGs (they're paused by default)
        await self._unpause_dags()

    async def cleanup(self) -> None:
        """Close HTTP client"""
        if self.client:
            await self.client.aclose()

    async def _wait_for_ready(self, timeout: float = 60.0) -> None:
        """Wait for Airflow API to be ready"""
        start = time.time()
        while (time.time() - start) < timeout:
            try:
                response = await self.client.get(f"{self.base_url}/api/v1/health")
                if response.status_code == 200:
                    return
            except Exception:
                pass
            await asyncio.sleep(2.0)
        raise TimeoutError("Airflow API not ready")

    async def _unpause_dags(self) -> None:
        """Unpause all benchmark DAGs"""
        for dag_id in ["sequential_bench_5", "sequential_bench_3", "parallel_bench_10"]:
            await self.client.patch(
                f"{self.base_url}/api/v1/dags/{dag_id}",
                json={"is_paused": False},
            )

    async def trigger_dag(self, dag_id: str) -> str:
        """Trigger a DAG run and return the run ID"""
        response = await self.client.post(
            f"{self.base_url}/api/v1/dags/{dag_id}/dagRuns",
            json={
                "conf": {},
                "dag_run_id": f"{dag_id}_{datetime.utcnow().isoformat()}",
            },
        )
        response.raise_for_status()
        return response.json()["dag_run_id"]

    async def wait_for_dag_completion(
        self,
        dag_id: str,
        dag_run_id: str,
        timeout: float = 30.0,
    ) -> tuple[bool, float]:
        """Wait for DAG run to complete and return (success, latency_ms)"""
        start = time.time()
        poll_interval = 0.5  # 500ms (Airflow is slow)

        while (time.time() - start) < timeout:
            response = await self.client.get(
                f"{self.base_url}/api/v1/dags/{dag_id}/dagRuns/{dag_run_id}"
            )
            response.raise_for_status()
            data = response.json()

            state = data["state"]

            if state == "success":
                latency_ms = (time.time() - start) * 1000
                return (True, latency_ms)
            elif state == "failed":
                latency_ms = (time.time() - start) * 1000
                return (False, latency_ms)

            await asyncio.sleep(poll_interval)

        # Timeout
        return (False, timeout * 1000)

    async def run_workflow(self, dag_id: str) -> tuple[bool, float]:
        """Run a single DAG and return (success, latency_ms)"""
        dag_run_id = await self.trigger_dag(dag_id)
        return await self.wait_for_dag_completion(dag_id, dag_run_id)

    async def run_scenario(
        self,
        scenario_name: str,
        dag_id: str,
        num_workflows: int,
        max_concurrent: int,
    ) -> "BenchmarkMetrics":
        """Run a benchmark scenario"""
        from streamflow.benchmark import BenchmarkMetrics, percentile

        semaphore = asyncio.Semaphore(max_concurrent)
        results: list[tuple[bool, float]] = []

        async def run_one():
            async with semaphore:
                result = await self.run_workflow(dag_id)
                results.append(result)

        start = time.time()
        await asyncio.gather(*[run_one() for _ in range(num_workflows)])
        duration = time.time() - start

        # Calculate metrics
        successful = sum(1 for success, _ in results if success)
        failed = len(results) - successful
        latencies = [lat for success, lat in results if success]
        latencies.sort()

        return BenchmarkMetrics(
            platform="Airflow",
            scenario=scenario_name,
            total_workflows=num_workflows,
            successful=successful,
            failed=failed,
            duration_seconds=duration,
            throughput_wf_per_sec=num_workflows / duration,
            latency_p50_ms=percentile(latencies, 0.50),
            latency_p95_ms=percentile(latencies, 0.95),
            latency_p99_ms=percentile(latencies, 0.99),
            success_rate=(successful / num_workflows * 100) if num_workflows > 0 else 0,
        )
```

### 9. Shared Metrics and Reporting (2 hours)

**File**: `benchmarks/shared/report.py`

```python
"""HTML report generator"""

import json
from pathlib import Path


def generate_html_report(results: list, output_path: Path) -> None:
    """Generate HTML comparison report"""
    html = """<!DOCTYPE html>
<html>
<head>
    <title>StreamFlow vs Temporal Benchmark</title>
    <style>
        body { font-family: Arial, sans-serif; margin: 40px; }
        h1 { color: #333; }
        table { border-collapse: collapse; width: 100%; margin: 20px 0; }
        th, td { border: 1px solid #ddd; padding: 12px; text-align: left; }
        th { background-color: #4CAF50; color: white; }
        tr:nth-child(even) { background-color: #f2f2f2; }
        .winner { background-color: #d4edda; font-weight: bold; }
    </style>
</head>
<body>
    <h1>StreamFlow vs Temporal Benchmark Results</h1>
    <p>Methodology: Same echo workflows, same hardware, sequential execution</p>

    <h2>Results Summary</h2>
    <table>
        <tr>
            <th>Platform</th>
            <th>Scenario</th>
            <th>Workflows</th>
            <th>Throughput (wf/sec)</th>
            <th>P50 Latency (ms)</th>
            <th>P99 Latency (ms)</th>
            <th>Success Rate</th>
        </tr>
"""

    for result in results:
        row_class = "winner" if result.platform == "StreamFlow" and result.throughput_wf_per_sec > 1000 else ""
        html += f"""        <tr class="{row_class}">
            <td>{result.platform}</td>
            <td>{result.scenario}</td>
            <td>{result.total_workflows}</td>
            <td>{result.throughput_wf_per_sec:.2f}</td>
            <td>{result.latency_p50_ms:.1f}</td>
            <td>{result.latency_p99_ms:.1f}</td>
            <td>{result.success_rate:.1f}%</td>
        </tr>
"""

    html += """    </table>

    <h2>Key Findings</h2>
    <ul>
"""

    # Calculate comparative metrics
    sf_results = [r for r in results if r.platform == "StreamFlow"]
    temp_results = [r for r in results if r.platform == "Temporal"]

    if sf_results and temp_results:
        sf_avg = sum(r.throughput_wf_per_sec for r in sf_results) / len(sf_results)
        temp_avg = sum(r.throughput_wf_per_sec for r in temp_results) / len(temp_results)
        speedup = sf_avg / temp_avg if temp_avg > 0 else 0

        html += f"""        <li>StreamFlow average throughput: {sf_avg:.2f} workflows/sec</li>
        <li>Temporal average throughput: {temp_avg:.2f} workflows/sec</li>
        <li><strong>StreamFlow is {speedup:.1f}x faster than Temporal</strong></li>
"""

    html += """    </ul>

    <h2>Raw Data (JSON)</h2>
    <pre>
"""
    html += json.dumps([vars(r) for r in results], indent=2)
    html += """
    </pre>
</body>
</html>
"""

    output_path.write_text(html)
```

### 8. Main CLI Runner (1 hour)

**File**: `benchmarks/run_benchmark.py`

```python
#!/usr/bin/env python3
"""Main benchmark CLI using Click"""

import asyncio
import json
import click
from pathlib import Path
from streamflow.benchmark import StreamFlowBenchmark
from temporal.benchmark import TemporalBenchmark
from temporal.workflows import SequentialBench5, SequentialBench3, ParallelBench10
from shared.report import generate_html_report


@click.group()
def cli():
    """StreamFlow vs Temporal Benchmark Suite"""
    pass


@cli.command()
@click.option("--platform", type=click.Choice(["streamflow", "temporal", "airflow", "all"]), default="all", help="Platform to benchmark")
@click.option("--output-dir", type=click.Path(), default="results", help="Output directory for results")
def run(platform: str, output_dir: str):
    """Run benchmarks"""
    asyncio.run(run_benchmarks(platform, output_dir))


async def run_benchmarks(platform: str, output_dir: str):
    """Run benchmark suite"""
    results_dir = Path(output_dir)
    results_dir.mkdir(exist_ok=True)

    all_results = []

    if platform in ["streamflow", "all"]:
        streamflow_results = await run_streamflow_benchmarks()
        all_results.extend(streamflow_results)

    if platform in ["temporal", "all"]:
        temporal_results = await run_temporal_benchmarks()
        all_results.extend(temporal_results)

    if platform in ["airflow", "all"]:
        airflow_results = await run_airflow_benchmarks()
        all_results.extend(airflow_results)

    # Save JSON results
    json_path = results_dir / "results.json"
    with open(json_path, "w") as f:
        json.dump([vars(r) for r in all_results], f, indent=2)
    click.echo(f"\n✅ Results saved to: {json_path}")

    # Generate HTML report
    html_path = results_dir / "comparison.html"
    generate_html_report(all_results, html_path)
    click.echo(f"✅ HTML report saved to: {html_path}")

    # Print summary
    print_summary(all_results)


@cli.command()
@click.argument("results_path", type=click.Path(exists=True))
def report(results_path: str):
    """Generate HTML report from existing results.json"""
    results_file = Path(results_path)

    if results_file.is_dir():
        results_file = results_file / "results.json"

    with open(results_file) as f:
        data = json.load(f)

    # Reconstruct BenchmarkMetrics objects
    from streamflow.benchmark import BenchmarkMetrics
    results = [BenchmarkMetrics(**item) for item in data]

    output_dir = results_file.parent
    html_path = output_dir / "comparison.html"
    generate_html_report(results, html_path)
    click.echo(f"✅ HTML report saved to: {html_path}")


@cli.command()
def check():
    """Check that platforms are accessible"""
    import httpx

    click.echo("Checking platform availability...")

    # Check StreamFlow
    try:
        response = httpx.get("http://streamflow:8080/health", timeout=5.0)
        if response.status_code == 200:
            click.secho("✅ StreamFlow is accessible", fg="green")
        else:
            click.secho(f"⚠️  StreamFlow returned status {response.status_code}", fg="yellow")
    except Exception as e:
        click.secho(f"❌ StreamFlow not accessible: {e}", fg="red")

    # Check Temporal
    # Note: Temporal doesn't have a simple HTTP health endpoint
    # We'd need to use the SDK to check, so just note it
    click.echo("ℹ️  Temporal check requires SDK connection (run benchmarks to verify)")


@cli.command()
def list_scenarios():
    """List available benchmark scenarios"""
    scenarios = [
        ("Sequential-5", "5 echo activities in sequence", "100 workflows, 10 concurrent"),
        ("Parallel-10", "10 echo activities in parallel (fan-out/fan-in)", "50 workflows, 10 concurrent"),
        ("High-Concurrency-3", "3 echo activities", "300 workflows, 100 concurrent"),
    ]

    click.echo("\nAvailable Benchmark Scenarios:")
    click.echo("=" * 70)
    for name, desc, params in scenarios:
        click.echo(f"\n{name}")
        click.echo(f"  Description: {desc}")
        click.echo(f"  Parameters: {params}")


def print_summary(all_results):
    """Print benchmark summary"""
    click.echo("\n" + "=" * 60)
    click.secho("Summary", bold=True)
    click.echo("=" * 60)

    streamflow_results = [r for r in all_results if r.platform == "StreamFlow"]
    temporal_results = [r for r in all_results if r.platform == "Temporal"]

    if streamflow_results:
        sf_avg = sum(r.throughput_wf_per_sec for r in streamflow_results) / len(streamflow_results)
        click.echo(f"StreamFlow avg throughput: {sf_avg:.2f} wf/sec")

        if sf_avg >= 1000:
            click.secho("🎉 StreamFlow target achieved: >1,000 wf/sec!", fg="green", bold=True)
        else:
            click.secho(f"⚠️  StreamFlow below target: {sf_avg:.2f}/1000 wf/sec", fg="yellow")

    if temporal_results:
        temp_avg = sum(r.throughput_wf_per_sec for r in temporal_results) / len(temporal_results)
        click.echo(f"Temporal avg throughput: {temp_avg:.2f} wf/sec")

    if streamflow_results and temporal_results:
        sf_avg = sum(r.throughput_wf_per_sec for r in streamflow_results) / len(streamflow_results)
        temp_avg = sum(r.throughput_wf_per_sec for r in temporal_results) / len(temporal_results)
        speedup = sf_avg / temp_avg if temp_avg > 0 else 0
        click.secho(f"Speedup vs Temporal: {speedup:.1f}x", fg="green" if speedup >= 10 else "yellow", bold=True)

    airflow_results = [r for r in all_results if r.platform == "Airflow"]
    if streamflow_results and airflow_results:
        sf_avg = sum(r.throughput_wf_per_sec for r in streamflow_results) / len(streamflow_results)
        af_avg = sum(r.throughput_wf_per_sec for r in airflow_results) / len(airflow_results)
        speedup = sf_avg / af_avg if af_avg > 0 else 0
        click.secho(f"Speedup vs Airflow: {speedup:.1f}x", fg="green" if speedup >= 10 else "yellow", bold=True)


async def run_streamflow_benchmarks():
    """Run StreamFlow benchmark scenarios"""
    print("=" * 60)
    print("Running StreamFlow Benchmarks")
    print("=" * 60)

    benchmark = StreamFlowBenchmark()
    await benchmark.setup()

    results = []

    # Sequential-5
    print("\n[1/3] Sequential-5: 100 workflows, 10 concurrent...")
    result = await benchmark.run_scenario(
        "Sequential-5",
        "sequential_bench_5",
        num_workflows=100,
        max_concurrent=10,
    )
    print_result(result)
    results.append(result)

    # Parallel-10
    print("\n[2/3] Parallel-10: 50 workflows, 10 concurrent...")
    result = await benchmark.run_scenario(
        "Parallel-10",
        "parallel_bench_10",
        num_workflows=50,
        max_concurrent=10,
    )
    print_result(result)
    results.append(result)

    # High-Concurrency-3
    print("\n[3/3] High-Concurrency-3: 300 workflows, 100 concurrent...")
    result = await benchmark.run_scenario(
        "High-Concurrency-3",
        "sequential_bench_3",
        num_workflows=300,
        max_concurrent=100,
    )
    print_result(result)
    results.append(result)

    await benchmark.cleanup()
    return results


async def run_temporal_benchmarks():
    """Run Temporal benchmark scenarios"""
    print("\n" + "=" * 60)
    print("Running Temporal Benchmarks")
    print("=" * 60)

    benchmark = TemporalBenchmark()
    await benchmark.setup()

    results = []

    # Sequential-5
    print("\n[1/3] Sequential-5: 100 workflows, 10 concurrent...")
    result = await benchmark.run_scenario(
        "Sequential-5",
        SequentialBench5,
        num_workflows=100,
        max_concurrent=10,
    )
    print_result(result)
    results.append(result)

    # Parallel-10
    print("\n[2/3] Parallel-10: 50 workflows, 10 concurrent...")
    result = await benchmark.run_scenario(
        "Parallel-10",
        ParallelBench10,
        num_workflows=50,
        max_concurrent=10,
    )
    print_result(result)
    results.append(result)

    # High-Concurrency-3
    print("\n[3/3] High-Concurrency-3: 300 workflows, 100 concurrent...")
    result = await benchmark.run_scenario(
        "High-Concurrency-3",
        SequentialBench3,
        num_workflows=300,
        max_concurrent=100,
    )
    print_result(result)
    results.append(result)

    await benchmark.cleanup()
    return results


async def run_airflow_benchmarks():
    """Run Airflow benchmark scenarios"""
    from airflow.benchmark import AirflowBenchmark

    print("\n" + "=" * 60)
    print("Running Airflow Benchmarks")
    print("=" * 60)

    benchmark = AirflowBenchmark()
    await benchmark.setup()

    results = []

    # Sequential-5
    print("\n[1/3] Sequential-5: 100 workflows, 10 concurrent...")
    result = await benchmark.run_scenario(
        "Sequential-5",
        "sequential_bench_5",
        num_workflows=100,
        max_concurrent=10,
    )
    print_result(result)
    results.append(result)

    # Parallel-10
    print("\n[2/3] Parallel-10: 50 workflows, 10 concurrent...")
    result = await benchmark.run_scenario(
        "Parallel-10",
        "parallel_bench_10",
        num_workflows=50,
        max_concurrent=10,
    )
    print_result(result)
    results.append(result)

    # High-Concurrency-3
    print("\n[3/3] High-Concurrency-3: 300 workflows, 100 concurrent...")
    result = await benchmark.run_scenario(
        "High-Concurrency-3",
        "sequential_bench_3",
        num_workflows=300,
        max_concurrent=100,
    )
    print_result(result)
    results.append(result)

    await benchmark.cleanup()
    return results


def print_result(result):
    """Print benchmark result summary"""
    print(f"  Throughput: {result.throughput_wf_per_sec:.2f} wf/sec")
    print(f"  P50 Latency: {result.latency_p50_ms:.1f} ms")
    print(f"  P99 Latency: {result.latency_p99_ms:.1f} ms")
    print(f"  Success Rate: {result.success_rate:.1f}%")


async def main():
    """Run all benchmarks and generate report"""
    results_dir = Path("results")
    results_dir.mkdir(exist_ok=True)

    # Run benchmarks sequentially
    streamflow_results = await run_streamflow_benchmarks()
    temporal_results = await run_temporal_benchmarks()

    all_results = streamflow_results + temporal_results

    # Save JSON results
    json_path = results_dir / "results.json"
    with open(json_path, "w") as f:
        json.dump([vars(r) for r in all_results], f, indent=2)
    print(f"\n✅ Results saved to: {json_path}")

    # Generate HTML report
    html_path = results_dir / "comparison.html"
    generate_html_report(all_results, html_path)
    print(f"✅ HTML report saved to: {html_path}")

    # Print summary
    print("\n" + "=" * 60)
    print("Summary")
    print("=" * 60)
    sf_avg = sum(r.throughput_wf_per_sec for r in streamflow_results) / len(streamflow_results)
    temp_avg = sum(r.throughput_wf_per_sec for r in temporal_results) / len(temporal_results)
    speedup = sf_avg / temp_avg if temp_avg > 0 else 0

    print(f"StreamFlow avg throughput: {sf_avg:.2f} wf/sec")
    print(f"Temporal avg throughput: {temp_avg:.2f} wf/sec")
    print(f"Speedup: {speedup:.1f}x")

    if sf_avg >= 1000:
        print("\n🎉 StreamFlow target achieved: >1,000 wf/sec!")
    else:
        print(f"\n⚠️  StreamFlow below target: {sf_avg:.2f}/1000 wf/sec")


if __name__ == "__main__":
    cli()
```

### 10. Docker Compose Configuration (2 hours)

**File**: `benchmarks/docker-compose.yml`

```yaml
version: '3.8'

services:
  # StreamFlow stack
  streamflow-postgres:
    image: postgres:18
    environment:
      POSTGRES_USER: streamflow
      POSTGRES_PASSWORD: streamflow
      POSTGRES_DB: streamflow_bench
    volumes:
      - streamflow_pg_data:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U streamflow"]
      interval: 5s
      timeout: 5s
      retries: 5

  streamflow:
    image: streamflow:latest
    build:
      context: ..
      dockerfile: Dockerfile
    environment:
      DATABASE_URL: postgres://streamflow:streamflow@streamflow-postgres:5432/streamflow_bench
      STREAMFLOW_CLIENT_ID: benchmark
      STREAMFLOW_CLIENT_SECRET: benchmark_secret
      RUST_LOG: info
    depends_on:
      streamflow-postgres:
        condition: service_healthy
    command: ["serve"]
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/health"]
      interval: 5s
      timeout: 5s
      retries: 10

  # Temporal stack
  temporal-postgres:
    image: postgres:13
    environment:
      POSTGRES_USER: temporal
      POSTGRES_PASSWORD: temporal
      POSTGRES_DB: temporal
    volumes:
      - temporal_pg_data:/var/lib/postgresql/data

  temporal:
    image: temporalio/auto-setup:1.24.2
    depends_on:
      - temporal-postgres
    environment:
      - DB=postgresql
      - DB_PORT=5432
      - POSTGRES_USER=temporal
      - POSTGRES_PWD=temporal
      - POSTGRES_SEEDS=temporal-postgres
    healthcheck:
      test: ["CMD", "tctl", "--address", "temporal:7233", "cluster", "health"]
      interval: 5s
      timeout: 5s
      retries: 10

  # Airflow stack
  airflow-postgres:
    image: postgres:13
    environment:
      POSTGRES_USER: airflow
      POSTGRES_PASSWORD: airflow
      POSTGRES_DB: airflow
    volumes:
      - airflow_pg_data:/var/lib/postgresql/data

  airflow-redis:
    image: redis:7

  airflow-init:
    image: apache/airflow:2.9.0
    depends_on:
      - airflow-postgres
      - airflow-redis
    environment:
      - AIRFLOW__CORE__EXECUTOR=CeleryExecutor
      - AIRFLOW__DATABASE__SQL_ALCHEMY_CONN=postgresql+psycopg2://airflow:airflow@airflow-postgres/airflow
      - AIRFLOW__CELERY__RESULT_BACKEND=db+postgresql://airflow:airflow@airflow-postgres/airflow
      - AIRFLOW__CELERY__BROKER_URL=redis://:@airflow-redis:6379/0
      - AIRFLOW__CORE__FERNET_KEY=''
      - AIRFLOW__CORE__LOAD_EXAMPLES=False
      - AIRFLOW__API__AUTH_BACKENDS=airflow.api.auth.backend.basic_auth
      - _AIRFLOW_DB_MIGRATE=true
      - _AIRFLOW_WWW_USER_CREATE=true
      - _AIRFLOW_WWW_USER_USERNAME=admin
      - _AIRFLOW_WWW_USER_PASSWORD=admin
    command: version

  airflow-webserver:
    image: apache/airflow:2.9.0
    depends_on:
      airflow-init:
        condition: service_completed_successfully
    environment:
      - AIRFLOW__CORE__EXECUTOR=CeleryExecutor
      - AIRFLOW__DATABASE__SQL_ALCHEMY_CONN=postgresql+psycopg2://airflow:airflow@airflow-postgres/airflow
      - AIRFLOW__CELERY__RESULT_BACKEND=db+postgresql://airflow:airflow@airflow-postgres/airflow
      - AIRFLOW__CELERY__BROKER_URL=redis://:@airflow-redis:6379/0
      - AIRFLOW__CORE__FERNET_KEY=''
      - AIRFLOW__CORE__LOAD_EXAMPLES=False
      - AIRFLOW__API__AUTH_BACKENDS=airflow.api.auth.backend.basic_auth
    ports:
      - "8081:8080"
    volumes:
      - ./airflow/dags.py:/opt/airflow/dags/benchmark_dags.py
      - ./airflow/operators.py:/opt/airflow/dags/operators.py
    command: webserver
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/health"]
      interval: 10s
      timeout: 10s
      retries: 5

  airflow-scheduler:
    image: apache/airflow:2.9.0
    depends_on:
      airflow-init:
        condition: service_completed_successfully
    environment:
      - AIRFLOW__CORE__EXECUTOR=CeleryExecutor
      - AIRFLOW__DATABASE__SQL_ALCHEMY_CONN=postgresql+psycopg2://airflow:airflow@airflow-postgres/airflow
      - AIRFLOW__CELERY__RESULT_BACKEND=db+postgresql://airflow:airflow@airflow-postgres/airflow
      - AIRFLOW__CELERY__BROKER_URL=redis://:@airflow-redis:6379/0
      - AIRFLOW__CORE__FERNET_KEY=''
      - AIRFLOW__CORE__LOAD_EXAMPLES=False
      - AIRFLOW__API__AUTH_BACKENDS=airflow.api.auth.backend.basic_auth
    volumes:
      - ./airflow/dags.py:/opt/airflow/dags/benchmark_dags.py
      - ./airflow/operators.py:/opt/airflow/dags/operators.py
    command: scheduler

  airflow-worker:
    image: apache/airflow:2.9.0
    depends_on:
      airflow-init:
        condition: service_completed_successfully
    environment:
      - AIRFLOW__CORE__EXECUTOR=CeleryExecutor
      - AIRFLOW__DATABASE__SQL_ALCHEMY_CONN=postgresql+psycopg2://airflow:airflow@airflow-postgres/airflow
      - AIRFLOW__CELERY__RESULT_BACKEND=db+postgresql://airflow:airflow@airflow-postgres/airflow
      - AIRFLOW__CELERY__BROKER_URL=redis://:@airflow-redis:6379/0
      - AIRFLOW__CORE__FERNET_KEY=''
      - AIRFLOW__CORE__LOAD_EXAMPLES=False
      - AIRFLOW__API__AUTH_BACKENDS=airflow.api.auth.backend.basic_auth
    volumes:
      - ./airflow/dags.py:/opt/airflow/dags/benchmark_dags.py
      - ./airflow/operators.py:/opt/airflow/dags/operators.py
    command: celery worker

  # Benchmark runner
  benchmark:
    build:
      context: .
      dockerfile: Dockerfile.benchmark
    depends_on:
      streamflow:
        condition: service_healthy
      temporal:
        condition: service_healthy
      airflow-webserver:
        condition: service_healthy
    volumes:
      - ./results:/app/results
    command: ["python", "run_benchmark.py", "run"]

volumes:
  streamflow_pg_data:
  temporal_pg_data:
  airflow_pg_data:
```

**File**: `benchmarks/Dockerfile.benchmark`

```dockerfile
FROM python:3.11-slim

WORKDIR /app

# Copy project files
COPY pyproject.toml .
COPY . .

# Install package in editable mode
RUN pip install --no-cache-dir -e .

# Default command
CMD ["python", "run_benchmark.py", "run"]
```

### 11. README Documentation (30 minutes)

**File**: `benchmarks/README.md`

```markdown
# StreamFlow vs Temporal Benchmark Suite

Reproducible benchmarks comparing StreamFlow and Temporal workflow engines.

## Methodology

- **Workflows**: Identical echo activities (sequential, parallel, high-concurrency)
- **Execution**: Sequential (one platform at a time)
- **Metrics**: Throughput (wf/sec), Latency (P50/P95/P99), Success Rate
- **Environment**: Docker Compose (controlled, reproducible)

## Quick Start

```bash
# Build and run all benchmarks
docker-compose up --build

# Results will be in results/
# - results.json (raw data)
# - comparison.html (visual report)
```

## Manual Execution

```bash
# Install dependencies
pip install -e .

# Start platforms
docker-compose up -d streamflow temporal

# Check platforms are accessible
python run_benchmark.py check

# Run benchmarks (all platforms)
python run_benchmark.py run

# Run specific platform only
python run_benchmark.py run --platform streamflow
python run_benchmark.py run --platform temporal
python run_benchmark.py run --platform airflow

# Generate report from existing results
python run_benchmark.py report results/

# Stop platforms
docker-compose down
```

## Benchmark Scenarios

1. **Sequential-5**: 5 echo activities in sequence (100 workflows, 10 concurrent)
2. **Parallel-10**: 10 echo activities in parallel (50 workflows, 10 concurrent)
3. **High-Concurrency-3**: 3 echo activities (300 workflows, 100 concurrent)

## Expected Results

- **StreamFlow**: >1,000 workflows/sec average
- **Temporal**: 35-100 workflows/sec (based on published benchmarks)
- **Airflow**: 10-50 workflows/sec (batch-oriented, not optimized for throughput)
- **Speedup**: 10x+ vs Temporal, 20x+ vs Airflow

## System Requirements

- Docker and Docker Compose
- 4+ CPU cores
- 8+ GB RAM
- Linux or macOS (Windows via WSL2)

## Caveats

- Docker overhead may affect absolute performance
- Results should be compared relatively (apples-to-apples)
- For production benchmarks, use native deployments

## License

MIT (matches StreamFlow license)
```

---

## Implementation Phases

### Phase 1: StreamFlow Foundation (COMPLETED)
1. ✅ Directory structure and dependencies
2. ✅ StreamFlow workflow definitions (Python dicts)
3. ✅ StreamFlow benchmark runner (httpx)
4. ✅ Basic metrics collection
5. ⏸️ Test against local StreamFlow instance (deferred to testing phase)

### Phase 2: Temporal Integration (COMPLETED)
6. ✅ Temporal workflow definitions (SDK classes)
7. ✅ Temporal activities (echo)
8. ✅ Temporal benchmark runner (SDK)
9. ⏸️ Test against local Temporal instance (deferred to testing phase)

### Phase 3: Airflow Integration (COMPLETED)
10. ✅ Airflow DAG definitions
11. ✅ Airflow echo operator
12. ✅ Airflow benchmark runner (REST API)
13. ⏸️ Test against local Airflow instance (deferred to testing phase)

### Phase 4: Automation & Reporting (COMPLETED)
14. ✅ Docker Compose configuration (all platforms)
15. ✅ Main CLI runner with Click
16. ✅ HTML report generation
17. ⏸️ End-to-end test (all platforms) (deferred to testing phase)

### Phase 5: Documentation & CI (COMPLETED)
18. ✅ README with methodology
19. ⏸️ GitHub Actions workflow (deferred to post-MVP)
20. ✅ Results interpretation guide (in README)

**Implementation Status**: Code complete, testing deferred
**Files Created**:
- `benchmarks/pyproject.toml`
- `benchmarks/streamflow/workflows.py`
- `benchmarks/streamflow/benchmark.py`
- `benchmarks/temporal/activities.py`
- `benchmarks/temporal/workflows.py`
- `benchmarks/temporal/benchmark.py`
- `benchmarks/airflow/operators.py`
- `benchmarks/airflow/dags.py`
- `benchmarks/airflow/benchmark.py`
- `benchmarks/shared/report.py`
- `benchmarks/run_benchmark.py`
- `benchmarks/docker-compose.yml`
- `benchmarks/Dockerfile.benchmark`
- `benchmarks/README.md`
- `benchmarks/.gitignore`

---

## Testing Strategy

### Unit Tests (Optional for MVP)
- Workflow definition serialization
- Metrics calculation
- Report generation

### Integration Tests
1. **StreamFlow**: Start local instance, run single workflow, verify completion
2. **Temporal**: Start local instance, run single workflow, verify completion
3. **Full Suite**: Run docker-compose, verify both platforms complete successfully

### Manual Validation
1. Run benchmarks locally
2. Verify HTML report renders correctly
3. Validate metrics against internal StreamFlow benchmarks
4. Confirm >1,000 wf/sec target achieved

---

## Success Criteria

1. ✅ Reproducible benchmarks running in Docker Compose (code complete)
2. ⏸️ StreamFlow achieving >1,000 wf/sec (needs testing to validate)
3. ⏸️ Temporal baseline established (<100 wf/sec expected) (needs testing)
4. ✅ HTML report clearly shows 10x+ advantage (report generator implemented)
5. ✅ Methodology documented and transparent
6. ✅ Zero manual steps to run comparison (`docker-compose up --build`)

---

## Deliverables

1. `benchmarks/` directory with:
   - StreamFlow benchmark runner (HTTP client)
   - Temporal benchmark runner (SDK client)
   - Shared metrics and reporting
   - Docker Compose configuration
   - Main CLI entry point
   - README documentation

2. Results documentation:
   - `results.json` - Raw benchmark data
   - `comparison.html` - Visual comparison report
   - Methodology description

3. Integration:
   - Docker Compose one-command execution
   - GitHub Actions workflow (optional)
   - Results archived for historical comparison

---

## Post-MVP Enhancements

- Add Cadence platform (if valuable)
- System metrics collection (CPU, memory, I/O)
- Time-series performance tracking
- More complex workflows (HTTP calls, transformations)
- GitHub Actions CI integration
- Performance regression detection
- Historical trend analysis
- Benchmark against more platforms (Prefect, Airflow, Conductor)

---

## Notes

- **Keep it simple**: Minimal dependencies, flat structure, easy to understand
- **Fair comparison**: Both platforms benchmarked via external clients (HTTP/SDK)
- **Transparent methodology**: All code and results open-sourced
- **Reproducible**: Docker Compose ensures anyone can run benchmarks
- **MVP focus**: Echo workflows only, expand complexity later
