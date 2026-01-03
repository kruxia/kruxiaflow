#!/usr/bin/env python3
"""Main benchmark CLI using Click"""

import asyncio
import json
import click
from datetime import datetime, timezone
from pathlib import Path
from kruxiaflow.benchmark import StreamFlowBenchmark
from temporal.benchmark import TemporalBenchmark
from temporal.workflows import SequentialBench5, SequentialBench3, ParallelBench10
from shared.report import generate_html_report
from shared.resource_monitor import ResourceMonitor


@click.group()
def cli():
    """Kruxia Flow Benchmark Suite - Compare Kruxia Flow vs Temporal vs Airflow"""
    pass


@cli.command()
@click.option("--platform", "-p", type=click.Choice(["kruxiaflow", "temporal", "airflow"]), multiple=True, help="Platform(s) to benchmark (can specify multiple)")
@click.option("--output-dir", type=click.Path(), default="results", help="Output directory for results")
def run(platform: tuple[str, ...], output_dir: str):
    """Run benchmarks"""
    # Default to all platforms if none specified
    platforms = list(platform) if platform else ["kruxiaflow", "temporal", "airflow"]
    asyncio.run(run_benchmarks(platforms, output_dir))


async def run_benchmarks(platforms: list[str], output_dir: str):
    """Run benchmark suite"""
    results_dir = Path(output_dir)
    results_dir.mkdir(exist_ok=True)

    # Generate timestamp for this benchmark run
    run_timestamp = datetime.now(timezone.utc)
    timestamp_iso = run_timestamp.isoformat()
    timestamp_file = run_timestamp.strftime("%Y%m%d-%H%M%S")

    # Paths for incremental saves
    json_path = results_dir / f"results-{timestamp_file}.json"
    html_path = results_dir / f"comparison-{timestamp_file}.html"

    all_results = []
    errors = []

    def save_results():
        """Save results incrementally"""
        if all_results:
            with open(json_path, "w") as f:
                json.dump([vars(r) for r in all_results], f, indent=2)
            generate_html_report(all_results, html_path, timestamp_iso)

    if "kruxiaflow" in platforms:
        try:
            kruxiaflow_results = await run_kruxiaflow_benchmarks(timestamp_iso)
            all_results.extend(kruxiaflow_results)
            save_results()
            click.echo(f"  💾 Incremental save: {json_path}")
        except Exception as e:
            click.secho(f"❌ Kruxia Flow benchmarks failed: {e}", fg="red")
            errors.append(("kruxiaflow", str(e)))

    if "temporal" in platforms:
        try:
            temporal_results = await run_temporal_benchmarks(timestamp_iso)
            all_results.extend(temporal_results)
            save_results()
            click.echo(f"  💾 Incremental save: {json_path}")
        except Exception as e:
            click.secho(f"❌ Temporal benchmarks failed: {e}", fg="red")
            errors.append(("temporal", str(e)))

    if "airflow" in platforms:
        try:
            airflow_results = await run_airflow_benchmarks(timestamp_iso)
            all_results.extend(airflow_results)
            save_results()
            click.echo(f"  💾 Incremental save: {json_path}")
        except Exception as e:
            click.secho(f"❌ Airflow benchmarks failed: {e}", fg="red")
            errors.append(("airflow", str(e)))

    # Final output
    if all_results:
        click.echo(f"\n✅ Results saved to: {json_path}")
        click.echo(f"✅ HTML report saved to: {html_path}")
        print_summary(all_results)
    else:
        click.secho("\n❌ No benchmark results collected", fg="red")

    if errors:
        click.echo("\nPlatform errors:")
        for platform, error in errors:
            click.secho(f"  • {platform}: {error}", fg="red")


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
    from kruxiaflow.benchmark import BenchmarkMetrics
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

    # Check Kruxia Flow
    try:
        response = httpx.get("http://kruxiaflow:8080/health", timeout=5.0)
        if response.status_code == 200:
            click.secho("✅ Kruxia Flow is accessible", fg="green")
        else:
            click.secho(f"⚠️  Kruxia Flow returned status {response.status_code}", fg="yellow")
    except Exception as e:
        click.secho(f"❌ Kruxia Flow not accessible: {e}", fg="red")

    # Check Temporal
    click.echo("ℹ️  Temporal check requires SDK connection (run benchmarks to verify)")

    # Check Airflow
    try:
        response = httpx.get("http://airflow-api-server:8080/api/v2/monitor/health", timeout=5.0)
        if response.status_code == 200:
            click.secho("✅ Airflow is accessible", fg="green")
        else:
            click.secho(f"⚠️  Airflow returned status {response.status_code}", fg="yellow")
    except Exception as e:
        click.secho(f"❌ Airflow not accessible: {e}", fg="red")


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

    kruxiaflow_results = [r for r in all_results if r.platform == "Kruxia Flow"]
    temporal_results = [r for r in all_results if r.platform == "Temporal"]
    airflow_results = [r for r in all_results if r.platform == "Airflow"]

    if kruxiaflow_results:
        sf_avg = sum(r.throughput_wf_per_sec for r in kruxiaflow_results) / len(kruxiaflow_results)
        click.echo(f"Kruxia Flow avg throughput: {sf_avg:.2f} wf/sec")

        if sf_avg >= 1000:
            click.secho("🎉 Kruxia Flow target achieved: >1,000 wf/sec!", fg="green", bold=True)
        else:
            click.secho(f"⚠️  Kruxia Flow below target: {sf_avg:.2f}/1000 wf/sec", fg="yellow")

    if temporal_results:
        temp_avg = sum(r.throughput_wf_per_sec for r in temporal_results) / len(temporal_results)
        click.echo(f"Temporal avg throughput: {temp_avg:.2f} wf/sec")

    if kruxiaflow_results and temporal_results:
        sf_avg = sum(r.throughput_wf_per_sec for r in kruxiaflow_results) / len(kruxiaflow_results)
        temp_avg = sum(r.throughput_wf_per_sec for r in temporal_results) / len(temporal_results)
        speedup = sf_avg / temp_avg if temp_avg > 0 else 0
        click.secho(f"Speedup vs Temporal: {speedup:.1f}x", fg="green" if speedup >= 10 else "yellow", bold=True)

    if airflow_results:
        af_avg = sum(r.throughput_wf_per_sec for r in airflow_results) / len(airflow_results)
        click.echo(f"Airflow avg throughput: {af_avg:.2f} wf/sec")

    if kruxiaflow_results and airflow_results:
        sf_avg = sum(r.throughput_wf_per_sec for r in kruxiaflow_results) / len(kruxiaflow_results)
        af_avg = sum(r.throughput_wf_per_sec for r in airflow_results) / len(airflow_results)
        speedup = sf_avg / af_avg if af_avg > 0 else 0
        click.secho(f"Speedup vs Airflow: {speedup:.1f}x", fg="green" if speedup >= 10 else "yellow", bold=True)


async def run_kruxiaflow_benchmarks(timestamp: str):
    """Run Kruxia Flow benchmark scenarios"""
    print("=" * 60)
    print("Running Kruxia Flow Benchmarks")
    print("=" * 60)

    benchmark = StreamFlowBenchmark()
    await benchmark.setup()

    # Initialize resource monitor
    monitor = ResourceMonitor("Kruxia Flow")
    monitor.connect()

    results = []

    # Sequential-5
    print("\n[1/3] Sequential-5: 100 workflows, 10 concurrent...")
    await monitor.start()
    result = await benchmark.run_scenario(
        "Sequential-5",
        "sequential_bench_5",
        num_workflows=100,
        max_concurrent=10,
    )
    resource_stats = await monitor.stop()
    result.timestamp = timestamp
    result.container_count = resource_stats.container_count
    result.peak_cpu_percent = resource_stats.peak_cpu_percent
    result.avg_cpu_percent = resource_stats.avg_cpu_percent
    result.peak_memory_mb = resource_stats.peak_memory_mb
    result.avg_memory_mb = resource_stats.avg_memory_mb
    print_result(result)
    results.append(result)

    # Parallel-10
    print("\n[2/3] Parallel-10: 50 workflows, 10 concurrent...")
    await monitor.start()
    result = await benchmark.run_scenario(
        "Parallel-10",
        "parallel_bench_10",
        num_workflows=50,
        max_concurrent=10,
    )
    resource_stats = await monitor.stop()
    result.timestamp = timestamp
    result.container_count = resource_stats.container_count
    result.peak_cpu_percent = resource_stats.peak_cpu_percent
    result.avg_cpu_percent = resource_stats.avg_cpu_percent
    result.peak_memory_mb = resource_stats.peak_memory_mb
    result.avg_memory_mb = resource_stats.avg_memory_mb
    print_result(result)
    results.append(result)

    # High-Concurrency-3
    print("\n[3/3] High-Concurrency-3: 300 workflows, 100 concurrent...")
    await monitor.start()
    result = await benchmark.run_scenario(
        "High-Concurrency-3",
        "sequential_bench_3",
        num_workflows=300,
        max_concurrent=100,
    )
    resource_stats = await monitor.stop()
    result.timestamp = timestamp
    result.container_count = resource_stats.container_count
    result.peak_cpu_percent = resource_stats.peak_cpu_percent
    result.avg_cpu_percent = resource_stats.avg_cpu_percent
    result.peak_memory_mb = resource_stats.peak_memory_mb
    result.avg_memory_mb = resource_stats.avg_memory_mb
    print_result(result)
    results.append(result)

    monitor.close()
    await benchmark.cleanup()
    return results


async def run_temporal_benchmarks(timestamp: str):
    """Run Temporal benchmark scenarios"""
    print("\n" + "=" * 60)
    print("Running Temporal Benchmarks")
    print("=" * 60)

    benchmark = TemporalBenchmark()
    await benchmark.setup()

    # Initialize resource monitor
    monitor = ResourceMonitor("Temporal")
    monitor.connect()

    results = []

    # Sequential-5
    print("\n[1/3] Sequential-5: 100 workflows, 10 concurrent...")
    await monitor.start()
    result = await benchmark.run_scenario(
        "Sequential-5",
        SequentialBench5,
        num_workflows=100,
        max_concurrent=10,
    )
    resource_stats = await monitor.stop()
    result.timestamp = timestamp
    result.container_count = resource_stats.container_count
    result.peak_cpu_percent = resource_stats.peak_cpu_percent
    result.avg_cpu_percent = resource_stats.avg_cpu_percent
    result.peak_memory_mb = resource_stats.peak_memory_mb
    result.avg_memory_mb = resource_stats.avg_memory_mb
    print_result(result)
    results.append(result)

    # Parallel-10
    print("\n[2/3] Parallel-10: 50 workflows, 10 concurrent...")
    await monitor.start()
    result = await benchmark.run_scenario(
        "Parallel-10",
        ParallelBench10,
        num_workflows=50,
        max_concurrent=10,
    )
    resource_stats = await monitor.stop()
    result.timestamp = timestamp
    result.container_count = resource_stats.container_count
    result.peak_cpu_percent = resource_stats.peak_cpu_percent
    result.avg_cpu_percent = resource_stats.avg_cpu_percent
    result.peak_memory_mb = resource_stats.peak_memory_mb
    result.avg_memory_mb = resource_stats.avg_memory_mb
    print_result(result)
    results.append(result)

    # High-Concurrency-3
    print("\n[3/3] High-Concurrency-3: 300 workflows, 100 concurrent...")
    await monitor.start()
    result = await benchmark.run_scenario(
        "High-Concurrency-3",
        SequentialBench3,
        num_workflows=300,
        max_concurrent=100,
    )
    resource_stats = await monitor.stop()
    result.timestamp = timestamp
    result.container_count = resource_stats.container_count
    result.peak_cpu_percent = resource_stats.peak_cpu_percent
    result.avg_cpu_percent = resource_stats.avg_cpu_percent
    result.peak_memory_mb = resource_stats.peak_memory_mb
    result.avg_memory_mb = resource_stats.avg_memory_mb
    print_result(result)
    results.append(result)

    monitor.close()
    await benchmark.cleanup()
    return results


async def run_airflow_benchmarks(timestamp: str):
    """Run Airflow benchmark scenarios"""
    from airflow_bench.benchmark import AirflowBenchmark

    print("\n" + "=" * 60)
    print("Running Airflow Benchmarks")
    print("=" * 60)

    benchmark = AirflowBenchmark()
    await benchmark.setup()

    # Initialize resource monitor
    monitor = ResourceMonitor("Airflow")
    monitor.connect()

    results = []

    # Sequential-5
    print("\n[1/3] Sequential-5: 100 workflows, 10 concurrent...")
    await monitor.start()
    result = await benchmark.run_scenario(
        "Sequential-5",
        "sequential_bench_5",
        num_workflows=100,
        max_concurrent=10,
    )
    resource_stats = await monitor.stop()
    result.timestamp = timestamp
    result.container_count = resource_stats.container_count
    result.peak_cpu_percent = resource_stats.peak_cpu_percent
    result.avg_cpu_percent = resource_stats.avg_cpu_percent
    result.peak_memory_mb = resource_stats.peak_memory_mb
    result.avg_memory_mb = resource_stats.avg_memory_mb
    print_result(result)
    results.append(result)

    # Parallel-10
    print("\n[2/3] Parallel-10: 50 workflows, 10 concurrent...")
    await monitor.start()
    result = await benchmark.run_scenario(
        "Parallel-10",
        "parallel_bench_10",
        num_workflows=50,
        max_concurrent=10,
    )
    resource_stats = await monitor.stop()
    result.timestamp = timestamp
    result.container_count = resource_stats.container_count
    result.peak_cpu_percent = resource_stats.peak_cpu_percent
    result.avg_cpu_percent = resource_stats.avg_cpu_percent
    result.peak_memory_mb = resource_stats.peak_memory_mb
    result.avg_memory_mb = resource_stats.avg_memory_mb
    print_result(result)
    results.append(result)

    # High-Concurrency-3
    print("\n[3/3] High-Concurrency-3: 300 workflows, 100 concurrent...")
    await monitor.start()
    result = await benchmark.run_scenario(
        "High-Concurrency-3",
        "sequential_bench_3",
        num_workflows=300,
        max_concurrent=100,
    )
    resource_stats = await monitor.stop()
    result.timestamp = timestamp
    result.container_count = resource_stats.container_count
    result.peak_cpu_percent = resource_stats.peak_cpu_percent
    result.avg_cpu_percent = resource_stats.avg_cpu_percent
    result.peak_memory_mb = resource_stats.peak_memory_mb
    result.avg_memory_mb = resource_stats.avg_memory_mb
    print_result(result)
    results.append(result)

    monitor.close()
    await benchmark.cleanup()
    return results


def print_result(result):
    """Print benchmark result summary"""
    print(f"  Throughput: {result.throughput_wf_per_sec:.2f} wf/sec")
    print(f"  P50 Latency: {result.latency_p50_ms:.1f} ms")
    print(f"  P99 Latency: {result.latency_p99_ms:.1f} ms")
    print(f"  Success Rate: {result.success_rate:.1f}%")
    if result.container_count > 0:
        print(f"  Containers: {result.container_count}, Peak CPU: {result.peak_cpu_percent:.1f}%, Avg Memory: {result.avg_memory_mb:.1f} MB")


if __name__ == "__main__":
    cli()
