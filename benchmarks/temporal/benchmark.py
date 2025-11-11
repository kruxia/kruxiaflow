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
        namespace: str = "default",
        task_queue: str = "benchmark-queue",
    ):
        self.host = host
        self.namespace = namespace
        self.task_queue = task_queue
        self.client: Client | None = None
        self.worker: Worker | None = None
        self.worker_task: asyncio.Task | None = None

    async def setup(self) -> None:
        """Connect to Temporal and start worker"""
        self.client = await Client.connect(self.host, namespace=self.namespace)

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
