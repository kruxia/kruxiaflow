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
            f"{self.base_url}/api/v1/workflow_definitions",
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
