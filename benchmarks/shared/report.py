"""HTML report generator"""

import json
from datetime import datetime
from pathlib import Path
from collections import defaultdict


def generate_html_report(results: list, output_path: Path, timestamp: str = "") -> None:
    """Generate HTML comparison report with side-by-side scenario comparisons"""
    # Format timestamp for display
    if timestamp:
        try:
            dt = datetime.fromisoformat(timestamp)
            display_timestamp = dt.strftime("%Y-%m-%d %H:%M:%S UTC")
        except ValueError:
            display_timestamp = timestamp
    else:
        display_timestamp = "N/A"

    # Group results by scenario
    scenarios = defaultdict(dict)
    platforms = set()
    for result in results:
        scenarios[result.scenario][result.platform] = result
        platforms.add(result.platform)

    # Sort platforms with Kruxia Flow first
    platform_order = ["Kruxia Flow", "Kruxia Flow (py-std)", "Temporal", "Airflow"]
    platforms = [p for p in platform_order if p in platforms]

    html = f"""<!DOCTYPE html>
<html>
<head>
    <title>Kruxia Flow Benchmark Comparison</title>
    <style>
        body {{ font-family: Arial, sans-serif; margin: 40px; background-color: #f8f9fa; }}
        h1 {{ color: #333; margin-bottom: 5px; }}
        h2 {{ color: #444; border-bottom: 2px solid #4CAF50; padding-bottom: 10px; }}
        h3 {{ color: #555; margin-top: 30px; }}
        .timestamp {{ color: #666; font-size: 0.9em; margin-bottom: 20px; }}
        .methodology {{ color: #666; font-size: 0.95em; margin-bottom: 30px; }}
        table {{ border-collapse: collapse; width: 100%; margin: 15px 0; background: white; box-shadow: 0 1px 3px rgba(0,0,0,0.1); }}
        th, td {{ border: 1px solid #ddd; padding: 12px; text-align: right; }}
        th {{ background-color: #4CAF50; color: white; font-weight: 600; }}
        th:first-child, td:first-child {{ text-align: left; }}
        tr:nth-child(even) {{ background-color: #f8f8f8; }}
        tr:hover {{ background-color: #f0f0f0; }}
        .winner {{ background-color: #d4edda !important; font-weight: bold; }}
        .section {{ margin: 40px 0; }}
        .scenario-section {{ background: white; padding: 20px; margin: 20px 0; border-radius: 8px; box-shadow: 0 2px 4px rgba(0,0,0,0.1); }}
        .summary-box {{ background: linear-gradient(135deg, #667eea 0%, #764ba2 100%); color: white; padding: 20px; border-radius: 8px; margin: 20px 0; }}
        .summary-box h3 {{ color: white; margin-top: 0; border: none; }}
        .summary-grid {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 15px; margin-top: 15px; }}
        .summary-item {{ background: rgba(255,255,255,0.15); padding: 15px; border-radius: 6px; }}
        .summary-item .label {{ font-size: 0.85em; opacity: 0.9; }}
        .summary-item .value {{ font-size: 1.5em; font-weight: bold; margin-top: 5px; }}
        .speedup {{ color: #28a745; font-weight: bold; }}
        .metric-header {{ font-weight: 600; background-color: #e9ecef !important; }}
        pre {{ background: #2d2d2d; color: #f8f8f2; padding: 20px; border-radius: 8px; overflow-x: auto; font-size: 0.85em; }}
        .comparison-arrow {{ color: #6c757d; font-size: 0.9em; }}
    </style>
</head>
<body>
    <h1>Kruxia Flow Benchmark Comparison</h1>
    <p class="timestamp">Run timestamp: {display_timestamp}</p>
    <p class="methodology">Methodology: Identical echo workflows executed on the same hardware with sequential benchmark runs.</p>
"""

    # Summary section
    html += _generate_summary_section(results, platforms)

    # Per-scenario comparison tables
    html += """
    <div class="section">
        <h2>Detailed Scenario Comparisons</h2>
"""

    for scenario_name in sorted(scenarios.keys()):
        scenario_results = scenarios[scenario_name]
        html += _generate_scenario_table(scenario_name, scenario_results, platforms)

    html += """
    </div>
"""

    # Averages table
    html += _generate_averages_table(results, platforms)

    # Resource usage section
    html += _generate_resource_section(results, platforms)

    # Raw data section
    html += """
    <div class="section">
        <h2>Raw Data (JSON)</h2>
        <pre>"""
    html += json.dumps([vars(r) for r in results], indent=2)
    html += """</pre>
    </div>
</body>
</html>
"""

    output_path.write_text(html)


def _generate_summary_section(results: list, platforms: list) -> str:
    """Generate the summary box with key findings"""
    html = """
    <div class="summary-box">
        <h3>Key Findings</h3>
        <div class="summary-grid">
"""

    # Calculate averages per platform
    platform_stats = {}
    for platform in platforms:
        platform_results = [r for r in results if r.platform == platform]
        if platform_results:
            platform_stats[platform] = {
                "avg_throughput": sum(r.throughput_wf_per_sec for r in platform_results) / len(platform_results),
                "avg_p50": sum(r.latency_p50_ms for r in platform_results) / len(platform_results),
                "avg_p99": sum(r.latency_p99_ms for r in platform_results) / len(platform_results),
                "total_workflows": sum(r.total_workflows for r in platform_results),
                "total_successful": sum(r.successful for r in platform_results),
            }

    # Show throughput for each platform
    for platform in platforms:
        if platform in platform_stats:
            stats = platform_stats[platform]
            html += f"""
            <div class="summary-item">
                <div class="label">{platform} Avg Throughput</div>
                <div class="value">{stats['avg_throughput']:.1f} wf/sec</div>
            </div>
"""

    # Show speedup comparisons
    if "Kruxia Flow" in platform_stats:
        sf_throughput = platform_stats["Kruxia Flow"]["avg_throughput"]

        for platform in ["Temporal", "Airflow"]:
            if platform in platform_stats and platform_stats[platform]["avg_throughput"] > 0:
                speedup = sf_throughput / platform_stats[platform]["avg_throughput"]
                html += f"""
            <div class="summary-item">
                <div class="label">Kruxia Flow vs {platform}</div>
                <div class="value">{speedup:.1f}x faster</div>
            </div>
"""

    html += """
        </div>
    </div>
"""
    return html


def _generate_scenario_table(scenario_name: str, scenario_results: dict, platforms: list) -> str:
    """Generate a comparison table for a single scenario"""
    html = f"""
        <div class="scenario-section">
            <h3>{scenario_name}</h3>
            <table>
                <tr>
                    <th>Metric</th>
"""

    # Header row with platform names
    for platform in platforms:
        html += f"                    <th>{platform}</th>\n"

    # Find winner for throughput (highest)
    throughputs = {p: scenario_results[p].throughput_wf_per_sec for p in platforms if p in scenario_results}
    throughput_winner = max(throughputs, key=throughputs.get) if throughputs else None

    # Find winner for latency (lowest)
    p50s = {p: scenario_results[p].latency_p50_ms for p in platforms if p in scenario_results}
    p50_winner = min(p50s, key=p50s.get) if p50s else None

    p99s = {p: scenario_results[p].latency_p99_ms for p in platforms if p in scenario_results}
    p99_winner = min(p99s, key=p99s.get) if p99s else None

    html += "                </tr>\n"

    # Workflows row
    html += "                <tr>\n                    <td>Total Workflows</td>\n"
    for platform in platforms:
        if platform in scenario_results:
            html += f"                    <td>{scenario_results[platform].total_workflows}</td>\n"
        else:
            html += "                    <td>-</td>\n"
    html += "                </tr>\n"

    # Throughput row
    html += "                <tr>\n                    <td>Throughput (wf/sec)</td>\n"
    for platform in platforms:
        if platform in scenario_results:
            value = scenario_results[platform].throughput_wf_per_sec
            winner_class = ' class="winner"' if platform == throughput_winner else ""
            html += f"                    <td{winner_class}>{value:.2f}</td>\n"
        else:
            html += "                    <td>-</td>\n"
    html += "                </tr>\n"

    # P50 Latency row
    html += "                <tr>\n                    <td>P50 Latency (ms)</td>\n"
    for platform in platforms:
        if platform in scenario_results:
            value = scenario_results[platform].latency_p50_ms
            winner_class = ' class="winner"' if platform == p50_winner else ""
            html += f"                    <td{winner_class}>{value:.1f}</td>\n"
        else:
            html += "                    <td>-</td>\n"
    html += "                </tr>\n"

    # P95 Latency row
    html += "                <tr>\n                    <td>P95 Latency (ms)</td>\n"
    for platform in platforms:
        if platform in scenario_results:
            value = scenario_results[platform].latency_p95_ms
            html += f"                    <td>{value:.1f}</td>\n"
        else:
            html += "                    <td>-</td>\n"
    html += "                </tr>\n"

    # P99 Latency row
    html += "                <tr>\n                    <td>P99 Latency (ms)</td>\n"
    for platform in platforms:
        if platform in scenario_results:
            value = scenario_results[platform].latency_p99_ms
            winner_class = ' class="winner"' if platform == p99_winner else ""
            html += f"                    <td{winner_class}>{value:.1f}</td>\n"
        else:
            html += "                    <td>-</td>\n"
    html += "                </tr>\n"

    # Success rate row
    html += "                <tr>\n                    <td>Success Rate</td>\n"
    for platform in platforms:
        if platform in scenario_results:
            value = scenario_results[platform].success_rate
            html += f"                    <td>{value:.1f}%</td>\n"
        else:
            html += "                    <td>-</td>\n"
    html += "                </tr>\n"

    # Speedup comparison row (relative to Kruxia Flow)
    if "Kruxia Flow" in scenario_results and len(scenario_results) > 1:
        sf_throughput = scenario_results["Kruxia Flow"].throughput_wf_per_sec
        html += '                <tr class="metric-header">\n                    <td>Speedup vs Others</td>\n'
        for platform in platforms:
            if platform == "Kruxia Flow":
                html += "                    <td>baseline</td>\n"
            elif platform in scenario_results and scenario_results[platform].throughput_wf_per_sec > 0:
                speedup = sf_throughput / scenario_results[platform].throughput_wf_per_sec
                html += f'                    <td class="speedup">{speedup:.2f}x</td>\n'
            else:
                html += "                    <td>-</td>\n"
        html += "                </tr>\n"

    html += """            </table>
        </div>
"""
    return html


def _generate_averages_table(results: list, platforms: list) -> str:
    """Generate the averages/totals summary table"""
    html = """
    <div class="section">
        <h2>Overall Averages &amp; Totals</h2>
        <table>
            <tr>
                <th>Metric</th>
"""

    for platform in platforms:
        html += f"                <th>{platform}</th>\n"
    html += "            </tr>\n"

    # Calculate stats per platform
    platform_stats = {}
    for platform in platforms:
        platform_results = [r for r in results if r.platform == platform]
        if platform_results:
            platform_stats[platform] = {
                "avg_throughput": sum(r.throughput_wf_per_sec for r in platform_results) / len(platform_results),
                "avg_p50": sum(r.latency_p50_ms for r in platform_results) / len(platform_results),
                "avg_p95": sum(r.latency_p95_ms for r in platform_results) / len(platform_results),
                "avg_p99": sum(r.latency_p99_ms for r in platform_results) / len(platform_results),
                "total_workflows": sum(r.total_workflows for r in platform_results),
                "total_successful": sum(r.successful for r in platform_results),
                "total_failed": sum(r.failed for r in platform_results),
                "total_duration": sum(r.duration_seconds for r in platform_results),
            }

    # Find winners
    throughputs = {p: platform_stats[p]["avg_throughput"] for p in platforms if p in platform_stats}
    throughput_winner = max(throughputs, key=throughputs.get) if throughputs else None

    p50s = {p: platform_stats[p]["avg_p50"] for p in platforms if p in platform_stats}
    p50_winner = min(p50s, key=p50s.get) if p50s else None

    p99s = {p: platform_stats[p]["avg_p99"] for p in platforms if p in platform_stats}
    p99_winner = min(p99s, key=p99s.get) if p99s else None

    # Scenarios count
    html += "            <tr>\n                <td>Scenarios Run</td>\n"
    for platform in platforms:
        count = len([r for r in results if r.platform == platform])
        html += f"                <td>{count}</td>\n"
    html += "            </tr>\n"

    # Total workflows
    html += "            <tr>\n                <td>Total Workflows</td>\n"
    for platform in platforms:
        if platform in platform_stats:
            html += f"                <td>{platform_stats[platform]['total_workflows']}</td>\n"
        else:
            html += "                <td>-</td>\n"
    html += "            </tr>\n"

    # Total successful
    html += "            <tr>\n                <td>Total Successful</td>\n"
    for platform in platforms:
        if platform in platform_stats:
            html += f"                <td>{platform_stats[platform]['total_successful']}</td>\n"
        else:
            html += "                <td>-</td>\n"
    html += "            </tr>\n"

    # Total failed
    html += "            <tr>\n                <td>Total Failed</td>\n"
    for platform in platforms:
        if platform in platform_stats:
            html += f"                <td>{platform_stats[platform]['total_failed']}</td>\n"
        else:
            html += "                <td>-</td>\n"
    html += "            </tr>\n"

    # Total duration
    html += "            <tr>\n                <td>Total Duration (sec)</td>\n"
    for platform in platforms:
        if platform in platform_stats:
            html += f"                <td>{platform_stats[platform]['total_duration']:.1f}</td>\n"
        else:
            html += "                <td>-</td>\n"
    html += "            </tr>\n"

    # Average throughput
    html += "            <tr>\n                <td>Avg Throughput (wf/sec)</td>\n"
    for platform in platforms:
        if platform in platform_stats:
            value = platform_stats[platform]["avg_throughput"]
            winner_class = ' class="winner"' if platform == throughput_winner else ""
            html += f"                <td{winner_class}>{value:.2f}</td>\n"
        else:
            html += "                <td>-</td>\n"
    html += "            </tr>\n"

    # Average P50 latency
    html += "            <tr>\n                <td>Avg P50 Latency (ms)</td>\n"
    for platform in platforms:
        if platform in platform_stats:
            value = platform_stats[platform]["avg_p50"]
            winner_class = ' class="winner"' if platform == p50_winner else ""
            html += f"                <td{winner_class}>{value:.1f}</td>\n"
        else:
            html += "                <td>-</td>\n"
    html += "            </tr>\n"

    # Average P95 latency
    html += "            <tr>\n                <td>Avg P95 Latency (ms)</td>\n"
    for platform in platforms:
        if platform in platform_stats:
            value = platform_stats[platform]["avg_p95"]
            html += f"                <td>{value:.1f}</td>\n"
        else:
            html += "                <td>-</td>\n"
    html += "            </tr>\n"

    # Average P99 latency
    html += "            <tr>\n                <td>Avg P99 Latency (ms)</td>\n"
    for platform in platforms:
        if platform in platform_stats:
            value = platform_stats[platform]["avg_p99"]
            winner_class = ' class="winner"' if platform == p99_winner else ""
            html += f"                <td{winner_class}>{value:.1f}</td>\n"
        else:
            html += "                <td>-</td>\n"
    html += "            </tr>\n"

    # Overall success rate
    html += "            <tr>\n                <td>Overall Success Rate</td>\n"
    for platform in platforms:
        if platform in platform_stats:
            total = platform_stats[platform]["total_workflows"]
            successful = platform_stats[platform]["total_successful"]
            rate = (successful / total * 100) if total > 0 else 0
            html += f"                <td>{rate:.1f}%</td>\n"
        else:
            html += "                <td>-</td>\n"
    html += "            </tr>\n"

    # Speedup comparison row
    if "Kruxia Flow" in platform_stats and len(platform_stats) > 1:
        sf_throughput = platform_stats["Kruxia Flow"]["avg_throughput"]
        html += '            <tr class="metric-header">\n                <td>Avg Speedup vs Others</td>\n'
        for platform in platforms:
            if platform == "Kruxia Flow":
                html += "                <td>baseline</td>\n"
            elif platform in platform_stats and platform_stats[platform]["avg_throughput"] > 0:
                speedup = sf_throughput / platform_stats[platform]["avg_throughput"]
                html += f'                <td class="speedup">{speedup:.2f}x</td>\n'
            else:
                html += "                <td>-</td>\n"
        html += "            </tr>\n"

    html += """        </table>
    </div>
"""
    return html


def _generate_resource_section(results: list, platforms: list) -> str:
    """Generate resource usage comparison section"""
    # Check if any results have resource data
    has_resource_data = any(
        getattr(r, 'container_count', 0) > 0 for r in results
    )

    if not has_resource_data:
        return ""

    html = """
    <div class="section">
        <h2>Resource Usage Comparison</h2>
        <p style="color: #666; font-size: 0.9em;">CPU and memory usage measured across all containers for each platform during benchmark execution.</p>
        <table>
            <tr>
                <th>Metric</th>
"""

    for platform in platforms:
        html += f"                <th>{platform}</th>\n"
    html += "            </tr>\n"

    # Calculate resource stats per platform
    platform_resources = {}
    for platform in platforms:
        platform_results = [r for r in results if r.platform == platform]
        if platform_results:
            container_counts = [getattr(r, 'container_count', 0) for r in platform_results]
            peak_cpus = [getattr(r, 'peak_cpu_percent', 0) for r in platform_results]
            avg_cpus = [getattr(r, 'avg_cpu_percent', 0) for r in platform_results]
            peak_mems = [getattr(r, 'peak_memory_mb', 0) for r in platform_results]
            avg_mems = [getattr(r, 'avg_memory_mb', 0) for r in platform_results]

            platform_resources[platform] = {
                "container_count": max(container_counts) if container_counts else 0,
                "peak_cpu": max(peak_cpus) if peak_cpus else 0,
                "avg_cpu": sum(avg_cpus) / len(avg_cpus) if avg_cpus else 0,
                "peak_memory": max(peak_mems) if peak_mems else 0,
                "avg_memory": sum(avg_mems) / len(avg_mems) if avg_mems else 0,
            }

    # Find winners (lowest resource usage)
    container_counts = {p: platform_resources[p]["container_count"] for p in platforms if p in platform_resources and platform_resources[p]["container_count"] > 0}
    container_winner = min(container_counts, key=container_counts.get) if container_counts else None

    peak_cpus = {p: platform_resources[p]["peak_cpu"] for p in platforms if p in platform_resources and platform_resources[p]["peak_cpu"] > 0}
    cpu_winner = min(peak_cpus, key=peak_cpus.get) if peak_cpus else None

    peak_mems = {p: platform_resources[p]["peak_memory"] for p in platforms if p in platform_resources and platform_resources[p]["peak_memory"] > 0}
    mem_winner = min(peak_mems, key=peak_mems.get) if peak_mems else None

    # Container count row
    html += "            <tr>\n                <td>Container Count</td>\n"
    for platform in platforms:
        if platform in platform_resources and platform_resources[platform]["container_count"] > 0:
            value = platform_resources[platform]["container_count"]
            winner_class = ' class="winner"' if platform == container_winner else ""
            html += f"                <td{winner_class}>{value}</td>\n"
        else:
            html += "                <td>-</td>\n"
    html += "            </tr>\n"

    # Peak CPU row
    html += "            <tr>\n                <td>Peak CPU (%)</td>\n"
    for platform in platforms:
        if platform in platform_resources and platform_resources[platform]["peak_cpu"] > 0:
            value = platform_resources[platform]["peak_cpu"]
            winner_class = ' class="winner"' if platform == cpu_winner else ""
            html += f"                <td{winner_class}>{value:.1f}%</td>\n"
        else:
            html += "                <td>-</td>\n"
    html += "            </tr>\n"

    # Avg CPU row
    html += "            <tr>\n                <td>Avg CPU (%)</td>\n"
    for platform in platforms:
        if platform in platform_resources and platform_resources[platform]["avg_cpu"] > 0:
            value = platform_resources[platform]["avg_cpu"]
            html += f"                <td>{value:.1f}%</td>\n"
        else:
            html += "                <td>-</td>\n"
    html += "            </tr>\n"

    # Peak Memory row
    html += "            <tr>\n                <td>Peak Memory (MB)</td>\n"
    for platform in platforms:
        if platform in platform_resources and platform_resources[platform]["peak_memory"] > 0:
            value = platform_resources[platform]["peak_memory"]
            winner_class = ' class="winner"' if platform == mem_winner else ""
            html += f"                <td{winner_class}>{value:.1f}</td>\n"
        else:
            html += "                <td>-</td>\n"
    html += "            </tr>\n"

    # Avg Memory row
    html += "            <tr>\n                <td>Avg Memory (MB)</td>\n"
    for platform in platforms:
        if platform in platform_resources and platform_resources[platform]["avg_memory"] > 0:
            value = platform_resources[platform]["avg_memory"]
            html += f"                <td>{value:.1f}</td>\n"
        else:
            html += "                <td>-</td>\n"
    html += "            </tr>\n"

    # Efficiency metric: throughput per CPU% and per MB
    html += '            <tr class="metric-header">\n                <td>CPU Efficiency (wf/sec per CPU%)</td>\n'
    for platform in platforms:
        platform_results = [r for r in results if r.platform == platform]
        if platform_results and platform in platform_resources:
            avg_throughput = sum(r.throughput_wf_per_sec for r in platform_results) / len(platform_results)
            avg_cpu = platform_resources[platform]["avg_cpu"]
            if avg_cpu > 0:
                efficiency = avg_throughput / avg_cpu
                html += f'                <td>{efficiency:.2f}</td>\n'
            else:
                html += "                <td>-</td>\n"
        else:
            html += "                <td>-</td>\n"
    html += "            </tr>\n"

    html += '            <tr class="metric-header">\n                <td>Memory Efficiency (wf/sec per 100MB)</td>\n'
    for platform in platforms:
        platform_results = [r for r in results if r.platform == platform]
        if platform_results and platform in platform_resources:
            avg_throughput = sum(r.throughput_wf_per_sec for r in platform_results) / len(platform_results)
            avg_memory = platform_resources[platform]["avg_memory"]
            if avg_memory > 0:
                efficiency = avg_throughput / (avg_memory / 100)
                html += f'                <td>{efficiency:.2f}</td>\n'
            else:
                html += "                <td>-</td>\n"
        else:
            html += "                <td>-</td>\n"
    html += "            </tr>\n"

    html += """        </table>
    </div>
"""
    return html
