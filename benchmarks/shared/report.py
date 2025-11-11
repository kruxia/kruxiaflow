"""HTML report generator"""

import json
from pathlib import Path


def generate_html_report(results: list, output_path: Path) -> None:
    """Generate HTML comparison report"""
    html = """<!DOCTYPE html>
<html>
<head>
    <title>StreamFlow Benchmark Comparison</title>
    <style>
        body { font-family: Arial, sans-serif; margin: 40px; }
        h1 { color: #333; }
        table { border-collapse: collapse; width: 100%; margin: 20px 0; }
        th, td { border: 1px solid #ddd; padding: 12px; text-align: left; }
        th { background-color: #4CAF50; color: white; }
        tr:nth-child(even) { background-color: #f2f2f2; }
        .winner { background-color: #d4edda; font-weight: bold; }
        .section { margin: 30px 0; }
    </style>
</head>
<body>
    <h1>StreamFlow Benchmark Comparison</h1>
    <p>Methodology: Same echo workflows, same hardware, sequential execution</p>

    <div class="section">
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
        html += f"""            <tr class="{row_class}">
                <td>{result.platform}</td>
                <td>{result.scenario}</td>
                <td>{result.total_workflows}</td>
                <td>{result.throughput_wf_per_sec:.2f}</td>
                <td>{result.latency_p50_ms:.1f}</td>
                <td>{result.latency_p99_ms:.1f}</td>
                <td>{result.success_rate:.1f}%</td>
            </tr>
"""

    html += """        </table>
    </div>

    <div class="section">
        <h2>Key Findings</h2>
        <ul>
"""

    # Calculate comparative metrics
    sf_results = [r for r in results if r.platform == "StreamFlow"]
    temp_results = [r for r in results if r.platform == "Temporal"]
    af_results = [r for r in results if r.platform == "Airflow"]

    if sf_results:
        sf_avg = sum(r.throughput_wf_per_sec for r in sf_results) / len(sf_results)
        html += f"            <li>StreamFlow average throughput: {sf_avg:.2f} workflows/sec</li>\n"

    if temp_results:
        temp_avg = sum(r.throughput_wf_per_sec for r in temp_results) / len(temp_results)
        html += f"            <li>Temporal average throughput: {temp_avg:.2f} workflows/sec</li>\n"

        if sf_results:
            speedup = sf_avg / temp_avg if temp_avg > 0 else 0
            html += f"            <li><strong>StreamFlow is {speedup:.1f}x faster than Temporal</strong></li>\n"

    if af_results:
        af_avg = sum(r.throughput_wf_per_sec for r in af_results) / len(af_results)
        html += f"            <li>Airflow average throughput: {af_avg:.2f} workflows/sec</li>\n"

        if sf_results:
            speedup = sf_avg / af_avg if af_avg > 0 else 0
            html += f"            <li><strong>StreamFlow is {speedup:.1f}x faster than Airflow</strong></li>\n"

    html += """        </ul>
    </div>

    <div class="section">
        <h2>Raw Data (JSON)</h2>
        <pre>
"""
    html += json.dumps([vars(r) for r in results], indent=2)
    html += """
        </pre>
    </div>
</body>
</html>
"""

    output_path.write_text(html)
