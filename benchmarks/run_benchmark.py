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
    """StreamFlow Benchmark Suite - Compare StreamFlow vs Temporal vs Airflow"""
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

    # if platform in ["airflow", "all"]:
    #     airflow_results = await run_airflow_benchmarks()
    #     all_results.extend(airflow_results)

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
    click.echo("ℹ️  Temporal check requires SDK connection (run benchmarks to verify)")

    # Check Airflow
    try:
        response = httpx.get("http://airflow-webserver:8081/health", timeout=5.0, auth=("admin", "admin"))
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

    streamflow_results = [r for r in all_results if r.platform == "StreamFlow"]
    temporal_results = [r for r in all_results if r.platform == "Temporal"]
    airflow_results = [r for r in all_results if r.platform == "Airflow"]

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

    if airflow_results:
        af_avg = sum(r.throughput_wf_per_sec for r in airflow_results) / len(airflow_results)
        click.echo(f"Airflow avg throughput: {af_avg:.2f} wf/sec")

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


if __name__ == "__main__":
    cli()
