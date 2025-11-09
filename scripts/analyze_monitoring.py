#!/usr/bin/env python3
"""
Performance Monitoring Analysis Script

Analyzes monitoring data collected during sustained tests to answer key questions:
1. Does performance degrade linearly over 60 seconds or step-wise?
2. Is connection pool exhausted after 30-40 seconds?
3. Are event consumers multiplying instead of reusing?
4. Is there a memory leak in event processing or state management?
"""

import argparse
import csv
import sys
from pathlib import Path
from datetime import datetime
from typing import List, Dict, Tuple
import statistics


def parse_csv(filepath: Path) -> List[Dict]:
    """Parse CSV file and return list of dictionaries."""
    data = []
    with open(filepath, 'r') as f:
        reader = csv.DictReader(f)
        for row in reader:
            data.append(row)
    return data


def analyze_memory_growth(data: List[Dict]) -> Dict:
    """Analyze memory usage patterns."""
    if not data:
        return {}

    rss_values = [float(row['rss_mb']) for row in data]
    timestamps = [row['timestamp'] for row in data]

    # Calculate growth metrics
    initial = rss_values[0]
    final = rss_values[-1]
    peak = max(rss_values)
    growth = final - initial
    growth_pct = (growth / initial * 100) if initial > 0 else 0

    # Detect linear vs step-wise growth
    # Use linear regression to check if growth is linear
    n = len(rss_values)
    x_vals = list(range(n))
    mean_x = sum(x_vals) / n
    mean_y = sum(rss_values) / n

    # Calculate correlation coefficient
    numerator = sum((x_vals[i] - mean_x) * (rss_values[i] - mean_y) for i in range(n))
    denom_x = sum((x - mean_x) ** 2 for x in x_vals)
    denom_y = sum((y - mean_y) ** 2 for y in rss_values)
    r_squared = (numerator ** 2) / (denom_x * denom_y) if denom_x > 0 and denom_y > 0 else 0

    # Detect step changes (jumps >20% in one interval)
    steps = []
    for i in range(1, len(rss_values)):
        delta = rss_values[i] - rss_values[i-1]
        delta_pct = (delta / rss_values[i-1] * 100) if rss_values[i-1] > 0 else 0
        if abs(delta_pct) > 20:
            steps.append({
                'time': timestamps[i],
                'from_mb': rss_values[i-1],
                'to_mb': rss_values[i],
                'delta_mb': delta,
                'delta_pct': delta_pct
            })

    return {
        'initial_mb': initial,
        'final_mb': final,
        'peak_mb': peak,
        'growth_mb': growth,
        'growth_pct': growth_pct,
        'r_squared': r_squared,
        'is_linear': r_squared > 0.9,
        'has_steps': len(steps) > 0,
        'step_changes': steps,
        'values': rss_values,
        'timestamps': timestamps
    }


def analyze_connection_pool(data: List[Dict]) -> Dict:
    """Analyze database connection patterns."""
    if not data:
        return {}

    total_conns = [int(row['total_connections']) for row in data]
    active_conns = [int(row['active']) for row in data]
    idle_conns = [int(row['idle']) for row in data]
    waiting_conns = [int(row['waiting']) for row in data]
    timestamps = [row['timestamp'] for row in data]

    # Find when connections peak
    max_total = max(total_conns)
    max_total_idx = total_conns.index(max_total)
    max_total_time = timestamps[max_total_idx]

    max_active = max(active_conns)
    max_active_idx = active_conns.index(max_active)
    max_active_time = timestamps[max_active_idx]

    # Check for saturation (connections stay at high level)
    avg_second_half = statistics.mean(total_conns[len(total_conns)//2:])
    max_conn_saturated = avg_second_half > max_total * 0.8

    # Check for waiting connections (indicates pool exhaustion)
    max_waiting = max(waiting_conns)
    has_contention = max_waiting > 0

    # Calculate average in time windows
    third = len(total_conns) // 3
    avg_first_third = statistics.mean(total_conns[:third])
    avg_second_third = statistics.mean(total_conns[third:2*third])
    avg_last_third = statistics.mean(total_conns[2*third:])

    return {
        'initial_connections': total_conns[0],
        'final_connections': total_conns[-1],
        'peak_total': max_total,
        'peak_total_time': max_total_time,
        'peak_active': max_active,
        'peak_active_time': max_active_time,
        'max_waiting': max_waiting,
        'has_contention': has_contention,
        'saturated': max_conn_saturated,
        'avg_first_third': avg_first_third,
        'avg_second_third': avg_second_third,
        'avg_last_third': avg_last_third,
        'values': total_conns,
        'active_values': active_conns,
        'timestamps': timestamps
    }


def analyze_consumer_positions(data: List[Dict]) -> Dict:
    """Analyze event consumer behavior."""
    if not data:
        return {}

    # Group by consumer_id
    consumers = {}
    for row in data:
        consumer_id = row['consumer_id'].strip()
        if consumer_id not in consumers:
            consumers[consumer_id] = []
        consumers[consumer_id].append({
            'timestamp': row['timestamp'],
            'last_event_id': int(row['last_event_id'])
        })

    # Analyze each consumer
    consumer_analysis = {}
    for consumer_id, positions in consumers.items():
        event_ids = [p['last_event_id'] for p in positions]
        initial = event_ids[0]
        final = event_ids[-1]
        events_processed = final - initial

        consumer_analysis[consumer_id] = {
            'initial_position': initial,
            'final_position': final,
            'events_processed': events_processed,
            'positions': positions
        }

    # Check if consumers are multiplying
    unique_consumers = len(consumers)
    expected_consumers = 1  # Assuming 1 orchestrator consumer

    return {
        'unique_consumers': unique_consumers,
        'expected_consumers': expected_consumers,
        'consumers_multiplying': unique_consumers > expected_consumers,
        'consumer_details': consumer_analysis
    }


def analyze_thread_lifecycle(data: List[Dict]) -> Dict:
    """Analyze worker thread patterns."""
    if not data:
        return {}

    thread_counts = [int(row['thread_count']) for row in data]
    timestamps = [row['timestamp'] for row in data]

    initial = thread_counts[0]
    final = thread_counts[-1]
    peak = max(thread_counts)
    min_count = min(thread_counts)

    # Check if threads are accumulating
    avg_first_half = statistics.mean(thread_counts[:len(thread_counts)//2])
    avg_second_half = statistics.mean(thread_counts[len(thread_counts)//2:])
    threads_accumulating = avg_second_half > avg_first_half * 1.2

    return {
        'initial_threads': initial,
        'final_threads': final,
        'peak_threads': peak,
        'min_threads': min_count,
        'threads_accumulating': threads_accumulating,
        'avg_first_half': avg_first_half,
        'avg_second_half': avg_second_half,
        'values': thread_counts,
        'timestamps': timestamps
    }


def analyze_performance_degradation(memory_data: List[Dict],
                                   conn_data: List[Dict],
                                   system_data: List[Dict]) -> Dict:
    """Analyze if performance degrades over time."""
    if not system_data:
        return {}

    # Track event count growth over time
    event_counts = [int(row['event_count']) for row in system_data]
    timestamps = [row['timestamp'] for row in system_data]

    # Calculate event processing rate in different time windows
    third = len(event_counts) // 3
    if third > 1:
        # Events processed in each third
        first_third_events = event_counts[third-1] - event_counts[0]
        second_third_events = event_counts[2*third-1] - event_counts[third]
        third_third_events = event_counts[-1] - event_counts[2*third]

        # Time windows (assuming equal intervals)
        time_window = third * 2  # sampling interval * samples

        rate_first = first_third_events / time_window if time_window > 0 else 0
        rate_second = second_third_events / time_window if time_window > 0 else 0
        rate_third = third_third_events / time_window if time_window > 0 else 0

        # Check for degradation (>20% slowdown)
        degradation = None
        if rate_first > 0:
            if rate_third < rate_first * 0.8:
                degradation = "linear"
            elif rate_second < rate_first * 0.8 or rate_third < rate_second * 0.8:
                degradation = "step-wise"

        return {
            'rate_first_third': rate_first,
            'rate_second_third': rate_second,
            'rate_third_third': rate_third,
            'degradation_type': degradation,
            'has_degradation': degradation is not None
        }

    return {}


def print_analysis(output_dir: Path):
    """Print comprehensive analysis of monitoring data."""
    print("=" * 80)
    print("PERFORMANCE MONITORING ANALYSIS")
    print("=" * 80)
    print()

    # Load all data
    memory_file = output_dir / "memory_usage.csv"
    conn_file = output_dir / "db_connections.csv"
    consumer_file = output_dir / "consumer_positions.csv"
    thread_file = output_dir / "thread_count.csv"
    system_file = output_dir / "system_stats.csv"

    memory_data = parse_csv(memory_file) if memory_file.exists() else []
    conn_data = parse_csv(conn_file) if conn_file.exists() else []
    consumer_data = parse_csv(consumer_file) if consumer_file.exists() else []
    thread_data = parse_csv(thread_file) if thread_file.exists() else []
    system_data = parse_csv(system_file) if system_file.exists() else []

    # 1. Memory Analysis
    print("1. MEMORY USAGE ANALYSIS")
    print("-" * 80)
    mem_analysis = analyze_memory_growth(memory_data)
    if mem_analysis:
        print(f"  Initial RSS: {mem_analysis['initial_mb']:.2f} MB")
        print(f"  Final RSS: {mem_analysis['final_mb']:.2f} MB")
        print(f"  Peak RSS: {mem_analysis['peak_mb']:.2f} MB")
        print(f"  Growth: {mem_analysis['growth_mb']:.2f} MB ({mem_analysis['growth_pct']:.1f}%)")
        print(f"  Growth pattern: {'LINEAR' if mem_analysis['is_linear'] else 'NON-LINEAR'} (R²={mem_analysis['r_squared']:.3f})")

        if mem_analysis['has_steps']:
            print(f"\n  ⚠️  Detected {len(mem_analysis['step_changes'])} sudden memory jumps:")
            for step in mem_analysis['step_changes'][:5]:  # Show first 5
                print(f"    - {step['time']}: {step['from_mb']:.2f} → {step['to_mb']:.2f} MB ({step['delta_pct']:+.1f}%)")

        # Memory leak detection
        if mem_analysis['growth_pct'] > 50:
            print(f"\n  🔴 MEMORY LEAK LIKELY: Growth >{mem_analysis['growth_pct']:.1f}% over test period")
        elif mem_analysis['growth_pct'] > 20:
            print(f"\n  🟡 POSSIBLE MEMORY LEAK: Growth {mem_analysis['growth_pct']:.1f}% may indicate accumulation")
        else:
            print(f"\n  ✅ Memory growth is normal ({mem_analysis['growth_pct']:.1f}%)")
    else:
        print("  No memory data available")
    print()

    # 2. Connection Pool Analysis
    print("2. DATABASE CONNECTION POOL ANALYSIS")
    print("-" * 80)
    conn_analysis = analyze_connection_pool(conn_data)
    if conn_analysis:
        print(f"  Initial connections: {conn_analysis['initial_connections']}")
        print(f"  Final connections: {conn_analysis['final_connections']}")
        print(f"  Peak total: {conn_analysis['peak_total']} (at {conn_analysis['peak_total_time']})")
        print(f"  Peak active: {conn_analysis['peak_active']} (at {conn_analysis['peak_active_time']})")
        print(f"  Max waiting: {conn_analysis['max_waiting']}")

        print(f"\n  Connection usage by time window:")
        print(f"    First third:  {conn_analysis['avg_first_third']:.1f} avg")
        print(f"    Second third: {conn_analysis['avg_second_third']:.1f} avg")
        print(f"    Last third:   {conn_analysis['avg_last_third']:.1f} avg")

        # Pool exhaustion detection
        if conn_analysis['max_waiting'] > 5:
            print(f"\n  🔴 CONNECTION POOL EXHAUSTED: {conn_analysis['max_waiting']} connections waiting")
            print(f"     Recommendation: Increase max_connections from current limit")
        elif conn_analysis['saturated']:
            print(f"\n  🟡 POOL APPROACHING SATURATION: Sustained high usage in second half")
        else:
            print(f"\n  ✅ Connection pool is adequate")

        if conn_analysis['has_contention']:
            print(f"  ⚠️  Lock contention detected (waiting connections)")
    else:
        print("  No connection data available")
    print()

    # 3. Event Consumer Analysis
    print("3. EVENT CONSUMER ANALYSIS")
    print("-" * 80)
    consumer_analysis = analyze_consumer_positions(consumer_data)
    if consumer_analysis:
        print(f"  Unique consumers: {consumer_analysis['unique_consumers']}")
        print(f"  Expected consumers: {consumer_analysis['expected_consumers']}")

        if consumer_analysis['consumers_multiplying']:
            print(f"\n  🔴 CONSUMERS MULTIPLYING: Found {consumer_analysis['unique_consumers']} consumers, expected {consumer_analysis['expected_consumers']}")
        else:
            print(f"\n  ✅ Consumer count is correct")

        print(f"\n  Consumer details:")
        for consumer_id, details in consumer_analysis['consumer_details'].items():
            print(f"    {consumer_id}:")
            print(f"      Events processed: {details['events_processed']}")
            print(f"      Position: {details['initial_position']} → {details['final_position']}")
    else:
        print("  No consumer data available")
    print()

    # 4. Thread Lifecycle Analysis
    print("4. WORKER THREAD LIFECYCLE ANALYSIS")
    print("-" * 80)
    thread_analysis = analyze_thread_lifecycle(thread_data)
    if thread_analysis:
        print(f"  Initial threads: {thread_analysis['initial_threads']}")
        print(f"  Final threads: {thread_analysis['final_threads']}")
        print(f"  Peak threads: {thread_analysis['peak_threads']}")
        print(f"  Min threads: {thread_analysis['min_threads']}")

        print(f"\n  Thread count by time window:")
        print(f"    First half:  {thread_analysis['avg_first_half']:.1f} avg")
        print(f"    Second half: {thread_analysis['avg_second_half']:.1f} avg")

        if thread_analysis['threads_accumulating']:
            print(f"\n  🔴 THREADS ACCUMULATING: {thread_analysis['avg_second_half']:.0f} threads in second half vs {thread_analysis['avg_first_half']:.0f} in first")
            print(f"     Old worker threads may not be terminating properly")
        else:
            print(f"\n  ✅ Thread lifecycle is normal")
    else:
        print("  No thread data available")
    print()

    # 5. Performance Degradation Analysis
    print("5. PERFORMANCE DEGRADATION ANALYSIS")
    print("-" * 80)
    perf_analysis = analyze_performance_degradation(memory_data, conn_data, system_data)
    if perf_analysis and 'has_degradation' in perf_analysis:
        print(f"  Event processing rate by time window:")
        print(f"    First third:  {perf_analysis['rate_first_third']:.2f} events/sec")
        print(f"    Second third: {perf_analysis['rate_second_third']:.2f} events/sec")
        print(f"    Last third:   {perf_analysis['rate_third_third']:.2f} events/sec")

        if perf_analysis['has_degradation']:
            print(f"\n  🔴 PERFORMANCE DEGRADATION DETECTED: {perf_analysis['degradation_type'].upper()}")
            if perf_analysis['degradation_type'] == "linear":
                print(f"     Performance degrades steadily over time")
            else:
                print(f"     Performance drops in discrete steps")
        else:
            print(f"\n  ✅ No significant performance degradation")
    else:
        print("  Insufficient data for degradation analysis")
    print()

    # 6. Summary - Answer Key Questions
    print("=" * 80)
    print("ANSWERS TO INVESTIGATION QUESTIONS")
    print("=" * 80)
    print()

    # Question 1: Linear or step-wise degradation?
    print("Q1: Does performance degrade linearly over 60 seconds or step-wise?")
    if perf_analysis and perf_analysis.get('has_degradation'):
        print(f"A1: {perf_analysis['degradation_type'].upper()} degradation detected")
        if mem_analysis and mem_analysis.get('has_steps'):
            print(f"    Memory also shows step changes, suggesting discrete resource allocation events")
    else:
        print("A1: No significant degradation pattern detected")
    print()

    # Question 2: Connection pool exhaustion?
    print("Q2: Is connection pool exhausted after 30-40 seconds?")
    if conn_analysis:
        if conn_analysis.get('max_waiting', 0) > 0:
            print(f"A2: YES - Pool exhaustion detected ({conn_analysis['max_waiting']} waiting connections)")
            print(f"    Peak at: {conn_analysis['peak_total_time']}")
        elif conn_analysis.get('saturated'):
            print(f"A2: PARTIALLY - Pool is saturated but not completely exhausted")
            print(f"    Connections remain high ({conn_analysis['avg_last_third']:.1f} avg) in last third")
        else:
            print(f"A2: NO - Connection pool has adequate capacity")
            print(f"    Peak usage: {conn_analysis['peak_total']} connections")
    print()

    # Question 3: Consumers multiplying?
    print("Q3: Are event consumers multiplying instead of reusing?")
    if consumer_analysis:
        if consumer_analysis.get('consumers_multiplying'):
            print(f"A3: YES - Found {consumer_analysis['unique_consumers']} consumers (expected {consumer_analysis['expected_consumers']})")
        else:
            print(f"A3: NO - Consumer count is normal ({consumer_analysis['unique_consumers']} active)")
    print()

    # Question 4: Memory leak?
    print("Q4: Is there a memory leak in event processing or state management?")
    if mem_analysis:
        growth_pct = mem_analysis.get('growth_pct', 0)
        if growth_pct > 50:
            print(f"A4: LIKELY - Memory grew {growth_pct:.1f}% over test period")
            print(f"    Growth: {mem_analysis['growth_mb']:.2f} MB ({mem_analysis['initial_mb']:.2f} → {mem_analysis['final_mb']:.2f})")
            if mem_analysis.get('is_linear'):
                print(f"    Pattern: Linear growth (R²={mem_analysis['r_squared']:.3f}) suggests continuous leak")
            else:
                print(f"    Pattern: Non-linear growth suggests allocation without cleanup")
        elif growth_pct > 20:
            print(f"A4: POSSIBLE - Memory grew {growth_pct:.1f}%, warrants investigation")
        else:
            print(f"A4: NO - Memory growth is normal ({growth_pct:.1f}%)")
            print(f"    This is expected for caching and runtime state")
    print()

    print("=" * 80)


def main():
    parser = argparse.ArgumentParser(description="Analyze performance monitoring data")
    parser.add_argument("output_dir", type=Path, help="Directory containing monitoring CSV files")
    args = parser.parse_args()

    if not args.output_dir.exists():
        print(f"Error: Directory not found: {args.output_dir}", file=sys.stderr)
        return 1

    print_analysis(args.output_dir)
    return 0


if __name__ == "__main__":
    sys.exit(main())
