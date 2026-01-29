"""Resource monitoring using Docker Stats API"""

import asyncio
import re
import docker
from dataclasses import dataclass, field
from typing import Optional


@dataclass
class ResourceStats:
    """Aggregated resource statistics for a platform"""
    platform: str
    container_count: int = 0
    peak_cpu_percent: float = 0.0
    avg_cpu_percent: float = 0.0
    peak_memory_mb: float = 0.0
    avg_memory_mb: float = 0.0
    total_memory_limit_mb: float = 0.0
    samples_collected: int = 0

    def to_dict(self) -> dict:
        return {
            "platform": self.platform,
            "container_count": self.container_count,
            "peak_cpu_percent": round(self.peak_cpu_percent, 2),
            "avg_cpu_percent": round(self.avg_cpu_percent, 2),
            "peak_memory_mb": round(self.peak_memory_mb, 2),
            "avg_memory_mb": round(self.avg_memory_mb, 2),
            "total_memory_limit_mb": round(self.total_memory_limit_mb, 2),
            "samples_collected": self.samples_collected,
        }


# Container name patterns for each platform
PLATFORM_CONTAINERS = {
    "Kruxia Flow": [
        "kruxiaflow-1",
        "kruxiaflow_kruxiaflow_1",
        "kruxiaflow",
        "kruxiaflow-postgres-1",
        "kruxiaflow_kruxiaflow-postgres_1",
        "kruxiaflow-postgres",
    ],
    "Kruxia Flow (py-std)": [
        "kruxiaflow-py-std-1",
        "kruxiaflow_kruxiaflow-py-std_1",
        "kruxiaflow-py-std",
    ],
    "Temporal": [
        "temporal-1",
        "kruxiaflow_temporal_1",
        "temporal",
        "temporal-postgres-1",
        "kruxiaflow_temporal-postgres_1",
        "temporal-postgres",
    ],
    "Airflow": [
        "airflow-api-server-1",
        "kruxiaflow_airflow-api-server_1",
        "airflow-api-server",
        "airflow-scheduler-1",
        "kruxiaflow_airflow-scheduler_1",
        "airflow-scheduler",
        "airflow-dag-processor-1",
        "kruxiaflow_airflow-dag-processor_1",
        "airflow-dag-processor",
        "airflow-worker-1",
        "kruxiaflow_airflow-worker_1",
        "airflow-worker",
        "airflow-worker-2",
        "kruxiaflow_airflow-worker_2",
        "airflow-postgres-1",
        "kruxiaflow_airflow-postgres_1",
        "airflow-postgres",
        "airflow-redis-1",
        "kruxiaflow_airflow-redis_1",
        "airflow-redis",
    ],
}


class ResourceMonitor:
    """Monitor Docker container resource usage during benchmarks"""

    def __init__(self, platform: str, sample_interval: float = 0.5):
        """
        Initialize resource monitor.

        Args:
            platform: Platform name ("Kruxia Flow", "Temporal", "Airflow")
            sample_interval: Seconds between samples (default 0.5s)
        """
        self.platform = platform
        self.sample_interval = sample_interval
        self.client: Optional[docker.DockerClient] = None
        self.containers: list = []
        self._monitoring = False
        self._monitor_task: Optional[asyncio.Task] = None

        # Collected samples
        self._cpu_samples: list[float] = []
        self._memory_samples: list[float] = []
        self._peak_cpu: float = 0.0
        self._peak_memory: float = 0.0
        self._memory_limit: float = 0.0

    def connect(self) -> bool:
        """Connect to Docker and find platform containers"""
        try:
            self.client = docker.from_env()
            self._find_containers()
            return len(self.containers) > 0
        except Exception as e:
            print(f"Failed to connect to Docker: {e}")
            return False

    def _find_containers(self) -> None:
        """Find running containers for this platform"""
        if not self.client:
            return

        container_patterns = PLATFORM_CONTAINERS.get(self.platform, [])
        all_containers = self.client.containers.list()

        self.containers = []
        for container in all_containers:
            container_name = container.name
            # Check if container name matches any pattern for this platform.
            # Match pattern as a complete service name segment — the pattern must
            # be followed by end-of-string, or a separator + replica number (e.g.
            # "-1", "_1"), NOT by more name segments like "-py-std".
            for pattern in container_patterns:
                escaped = re.escape(pattern)
                if re.search(rf'(^|[-_]){escaped}([-_]\d+)?$', container_name):
                    self.containers.append(container)
                    break

        if self.containers:
            print(f"  Monitoring {len(self.containers)} containers for {self.platform}: "
                  f"{[c.name for c in self.containers]}")

    def _calculate_cpu_percent(self, stats: dict) -> float:
        """Calculate CPU percentage from Docker stats"""
        try:
            cpu_delta = (
                stats["cpu_stats"]["cpu_usage"]["total_usage"]
                - stats["precpu_stats"]["cpu_usage"]["total_usage"]
            )
            system_delta = (
                stats["cpu_stats"]["system_cpu_usage"]
                - stats["precpu_stats"]["system_cpu_usage"]
            )

            if system_delta > 0 and cpu_delta > 0:
                # Get number of CPUs
                num_cpus = stats["cpu_stats"].get("online_cpus")
                if not num_cpus:
                    num_cpus = len(stats["cpu_stats"]["cpu_usage"].get("percpu_usage", [1]))

                cpu_percent = (cpu_delta / system_delta) * num_cpus * 100.0
                return cpu_percent
        except (KeyError, TypeError, ZeroDivisionError):
            pass
        return 0.0

    def _get_memory_usage_mb(self, stats: dict) -> tuple[float, float]:
        """Get memory usage and limit in MB from Docker stats"""
        try:
            memory_usage = stats["memory_stats"]["usage"]
            # Subtract cache if available for more accurate "active" memory
            cache = stats["memory_stats"].get("stats", {}).get("cache", 0)
            active_memory = memory_usage - cache

            memory_limit = stats["memory_stats"].get("limit", 0)

            return active_memory / (1024 * 1024), memory_limit / (1024 * 1024)
        except (KeyError, TypeError):
            return 0.0, 0.0

    def _get_container_stats(self, container) -> tuple[float, float, float]:
        """Get stats for a single container. Returns (cpu, memory, limit)."""
        try:
            stats = container.stats(stream=False)
            cpu = self._calculate_cpu_percent(stats)
            memory, limit = self._get_memory_usage_mb(stats)
            return cpu, memory, limit
        except Exception:
            return 0.0, 0.0, 0.0

    def _collect_sample(self) -> tuple[float, float]:
        """Collect a single sample of CPU and memory usage across all containers"""
        total_cpu = 0.0
        total_memory = 0.0
        total_limit = 0.0

        for container in self.containers:
            cpu, memory, limit = self._get_container_stats(container)
            total_cpu += cpu
            total_memory += memory
            total_limit += limit

        self._memory_limit = max(self._memory_limit, total_limit)
        return total_cpu, total_memory

    async def _collect_sample_async(self) -> tuple[float, float]:
        """Collect stats from all containers in parallel."""
        import concurrent.futures

        total_cpu = 0.0
        total_memory = 0.0
        total_limit = 0.0

        if not self.containers:
            return 0.0, 0.0

        loop = asyncio.get_event_loop()
        # Collect stats from all containers in parallel using thread pool
        with concurrent.futures.ThreadPoolExecutor(max_workers=len(self.containers)) as executor:
            futures = [
                loop.run_in_executor(executor, self._get_container_stats, container)
                for container in self.containers
            ]
            results = await asyncio.gather(*futures, return_exceptions=True)

        for result in results:
            if isinstance(result, tuple):
                cpu, memory, limit = result
                total_cpu += cpu
                total_memory += memory
                total_limit += limit

        self._memory_limit = max(self._memory_limit, total_limit)
        return total_cpu, total_memory

    async def _monitor_loop(self) -> None:
        """Background monitoring loop"""
        while self._monitoring:
            # Collect stats from all containers in parallel (much faster)
            cpu, memory = await self._collect_sample_async()

            self._cpu_samples.append(cpu)
            self._memory_samples.append(memory)
            self._peak_cpu = max(self._peak_cpu, cpu)
            self._peak_memory = max(self._peak_memory, memory)

            await asyncio.sleep(self.sample_interval)

    async def start(self) -> None:
        """Start monitoring in background"""
        # Always refresh container list to ensure we have valid references
        # (container objects can become stale between benchmark scenarios)
        if self.client:
            self._find_containers()
        if not self.containers:
            self.connect()

        self._monitoring = True
        self._cpu_samples = []
        self._memory_samples = []
        self._peak_cpu = 0.0
        self._peak_memory = 0.0

        # Collect an initial sample immediately to ensure we have data
        # even for fast benchmarks
        if self.containers:
            cpu, memory = await self._collect_sample_async()
            self._cpu_samples.append(cpu)
            self._memory_samples.append(memory)
            self._peak_cpu = max(self._peak_cpu, cpu)
            self._peak_memory = max(self._peak_memory, memory)

        self._monitor_task = asyncio.create_task(self._monitor_loop())

    async def stop(self) -> ResourceStats:
        """Stop monitoring and return aggregated stats"""
        self._monitoring = False

        if self._monitor_task:
            self._monitor_task.cancel()
            try:
                await self._monitor_task
            except asyncio.CancelledError:
                pass

        # Calculate averages
        avg_cpu = sum(self._cpu_samples) / len(self._cpu_samples) if self._cpu_samples else 0.0
        avg_memory = sum(self._memory_samples) / len(self._memory_samples) if self._memory_samples else 0.0

        return ResourceStats(
            platform=self.platform,
            container_count=len(self.containers),
            peak_cpu_percent=self._peak_cpu,
            avg_cpu_percent=avg_cpu,
            peak_memory_mb=self._peak_memory,
            avg_memory_mb=avg_memory,
            total_memory_limit_mb=self._memory_limit,
            samples_collected=len(self._cpu_samples),
        )

    def close(self) -> None:
        """Close Docker client connection"""
        if self.client:
            self.client.close()
            self.client = None


async def collect_baseline_resources(platforms: list[str], duration: float = 5.0) -> dict[str, ResourceStats]:
    """
    Collect baseline resource usage for platforms at idle.

    Args:
        platforms: List of platform names
        duration: How long to collect samples (seconds)

    Returns:
        Dict mapping platform name to ResourceStats
    """
    monitors = {}
    for platform in platforms:
        monitor = ResourceMonitor(platform)
        if monitor.connect():
            monitors[platform] = monitor

    # Start all monitors
    for monitor in monitors.values():
        await monitor.start()

    # Wait for collection period
    await asyncio.sleep(duration)

    # Stop and collect results
    results = {}
    for platform, monitor in monitors.items():
        results[platform] = await monitor.stop()
        monitor.close()

    return results
