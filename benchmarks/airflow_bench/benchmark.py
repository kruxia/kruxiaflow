"""Airflow 3 benchmark using REST API"""

import asyncio
import httpx
import time
from datetime import datetime, timezone


class AirflowBenchmark:
    """Benchmark runner for Airflow 3 via REST API"""

    def __init__(
        self,
        base_url: str = "http://airflow-api-server:8080",
        username: str = "airflow",
        password: str = "airflow",
    ):
        self.base_url = base_url
        self.username = username
        self.password = password
        self.client: httpx.AsyncClient | None = None
        self.access_token: str | None = None

    async def setup(self) -> None:
        """Initialize HTTP client and authenticate"""
        self.client = httpx.AsyncClient(timeout=60.0)

        # Wait for Airflow to be fully ready
        await self._wait_for_ready()

        # Authenticate and get JWT token
        await self._authenticate()

        # Unpause DAGs (they're paused by default)
        await self._unpause_dags()

    async def cleanup(self) -> None:
        """Close HTTP client"""
        if self.client:
            await self.client.aclose()

    async def _authenticate(self) -> None:
        """Authenticate with Airflow 3 and obtain JWT token"""
        response = await self.client.post(
            f"{self.base_url}/auth/token",
            json={
                "username": self.username,
                "password": self.password,
            },
        )
        response.raise_for_status()
        self.access_token = response.json()["access_token"]

    def _auth_headers(self) -> dict:
        """Return authorization headers"""
        return {"Authorization": f"Bearer {self.access_token}"}

    async def _wait_for_ready(self, timeout: float = 180.0) -> None:
        """Wait for Airflow API to be ready and DAGs to be loaded"""
        start = time.time()
        dags_loaded = False

        while (time.time() - start) < timeout:
            try:
                # Check health endpoint (Airflow 3 uses /api/v2/monitor/health)
                response = await self.client.get(f"{self.base_url}/api/v2/monitor/health")
                if response.status_code != 200:
                    await asyncio.sleep(2.0)
                    continue

                # Need to authenticate first to check DAGs
                if not self.access_token:
                    try:
                        await self._authenticate()
                    except Exception as e:
                        print(f"Auth not ready yet: {e}")
                        await asyncio.sleep(2.0)
                        continue

                # Check if DAGs are loaded
                if not dags_loaded:
                    # Check for import errors first
                    errors_response = await self.client.get(
                        f"{self.base_url}/api/v2/importErrors",
                        headers=self._auth_headers(),
                    )
                    if errors_response.status_code == 200:
                        errors = errors_response.json()
                        if errors.get("import_errors"):
                            print("DAG import errors found:")
                            for error in errors["import_errors"]:
                                print(f"  File: {error.get('filename')}")
                                print(f"  Error: {error.get('stack_trace')}")

                    # Query all DAGs (no filter - paused DAGs are included by default)
                    dags_response = await self.client.get(
                        f"{self.base_url}/api/v2/dags",
                        headers=self._auth_headers(),
                    )
                    if dags_response.status_code == 200:
                        dags = dags_response.json()
                        dag_ids = {dag["dag_id"] for dag in dags.get("dags", [])}
                        required_dags = {"sequential_bench_5", "sequential_bench_3", "parallel_bench_10"}

                        if required_dags.issubset(dag_ids):
                            dags_loaded = True
                            print(f"All required DAGs loaded: {required_dags}")
                            return
                        else:
                            missing = required_dags - dag_ids
                            print(f"Waiting for DAGs to load. Found: {dag_ids}, Missing: {missing}")
                            print(f"  Total DAGs in response: {dags.get('total_entries', 0)}")
                    else:
                        print(f"Failed to fetch DAGs: {dags_response.status_code} - {dags_response.text}")

            except Exception as e:
                print(f"Error checking Airflow readiness: {e}")

            await asyncio.sleep(5.0)

        raise TimeoutError("Airflow API not ready or DAGs not loaded")

    async def _unpause_dags(self) -> None:
        """Unpause all benchmark DAGs"""
        for dag_id in ["sequential_bench_5", "sequential_bench_3", "parallel_bench_10"]:
            response = await self.client.patch(
                f"{self.base_url}/api/v2/dags/{dag_id}",
                headers=self._auth_headers(),
                json={"is_paused": False},
            )
            response.raise_for_status()
            print(f"Unpaused DAG: {dag_id}")

    async def trigger_dag(self, dag_id: str) -> str:
        """Trigger a DAG run and return the run ID"""
        # Airflow 3 uses logical_date instead of execution_date
        # Setting logical_date to None triggers with current timestamp
        dag_run_id = f"{dag_id}_{datetime.now(timezone.utc).strftime('%Y%m%d%H%M%S%f')}"
        response = await self.client.post(
            f"{self.base_url}/api/v2/dags/{dag_id}/dagRuns",
            headers=self._auth_headers(),
            json={
                "dag_run_id": dag_run_id,
                "conf": {},
                "logical_date": None,
            },
        )
        response.raise_for_status()
        return response.json()["dag_run_id"]

    async def wait_for_dag_completion(
        self,
        dag_id: str,
        dag_run_id: str,
        timeout: float = 300.0,
    ) -> tuple[bool, float]:
        """Wait for DAG run to complete and return (success, latency_ms)"""
        start = time.time()
        poll_interval = 0.5  # 500ms (Airflow is slow)

        while (time.time() - start) < timeout:
            response = await self.client.get(
                f"{self.base_url}/api/v2/dags/{dag_id}/dagRuns/{dag_run_id}",
                headers=self._auth_headers(),
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
        from kruxiaflow.benchmark import BenchmarkMetrics, percentile

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
