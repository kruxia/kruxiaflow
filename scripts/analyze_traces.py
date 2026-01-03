#!/usr/bin/env python3
"""Analyze tracing spans from Kruxia Flow server logs."""
import re
import sys
from collections import defaultdict
from statistics import mean, median, stdev

def parse_log_line(line):
    """Parse a log line and extract timing information if present."""
    # Match lines with tracing span close events
    # Format: \x1b[1mspan_name\x1b[0m{...}: module: close \x1b[3mtime.busy\x1b[0m\x1b[2m=\x1b[0m123µs
    timing_pattern = r'\x1b\[1m([a-z_]+)\x1b\[0m.*?close.*?\x1b\[3mtime\.busy\x1b\[0m\x1b\[2m=\x1b\[0m(\d+(?:\.\d+)?)(µs|ms|s)'
    match = re.search(timing_pattern, line)
    if match:
        span_name = match.group(1)
        duration = float(match.group(2))
        unit = match.group(3)

        # Convert to milliseconds
        if unit == 'µs':
            duration_ms = duration / 1000.0
        elif unit == 'ms':
            duration_ms = duration
        elif unit == 's':
            duration_ms = duration * 1000.0
        else:
            return None, None

        return span_name, duration_ms
    return None, None

def analyze_traces(log_file):
    """Analyze traces from log file."""
    spans = defaultdict(list)

    with open(log_file, 'r') as f:
        for line in f:
            span_name, duration_ms = parse_log_line(line)
            if span_name and duration_ms is not None:
                spans[span_name].append(duration_ms)

    if not spans:
        print("No timing spans found in logs")
        print("Make sure RUST_LOG is set to at least 'info' level")
        return

    # Calculate statistics for each span
    print("\n" + "="*80)
    print("TRACING SPAN ANALYSIS")
    print("="*80)
    print(f"\n{'Span Name':<35} {'Count':>8} {'Mean':>10} {'Median':>10} {'StdDev':>10} {'Total':>12}")
    print("-"*80)

    # Sort by total time descending
    sorted_spans = sorted(spans.items(), key=lambda x: sum(x[1]), reverse=True)

    for span_name, durations in sorted_spans:
        count = len(durations)
        mean_ms = mean(durations)
        median_ms = median(durations)
        total_ms = sum(durations)
        stddev_ms = stdev(durations) if count > 1 else 0

        print(f"{span_name:<35} {count:>8} {mean_ms:>9.2f}ms {median_ms:>9.2f}ms {stddev_ms:>9.2f}ms {total_ms:>10.1f}ms")

    # Show top bottlenecks
    print("\n" + "="*80)
    print("TOP BOTTLENECKS (by total time)")
    print("="*80)
    for i, (span_name, durations) in enumerate(sorted_spans[:10], 1):
        total_ms = sum(durations)
        mean_ms = mean(durations)
        print(f"{i:2}. {span_name:<35} Total: {total_ms:>10.1f}ms  Avg: {mean_ms:>8.2f}ms  Calls: {len(durations):>6}")

if __name__ == '__main__':
    if len(sys.argv) < 2:
        print("Usage: analyze_traces.py <log_file>")
        sys.exit(1)

    analyze_traces(sys.argv[1])
