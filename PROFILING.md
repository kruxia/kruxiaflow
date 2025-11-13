# Memory Profiling with Docker

StreamFlow uses jemalloc for memory profiling. On macOS, jeprof has difficulty resolving Rust symbols, so **we recommend using the Docker-based profiling approach** which runs profiling inside a Linux container where symbol resolution works properly.

## Quick Start

```bash
docker compose up -d streamflow-profiling
DATABASE_URL=postgres://streamflow:streamflow_dev@postgres:5432/streamflow_profiling \
       ./scripts/profiling.sh --test test_sustained_throughput
docker compose down -t0 streamflow-profiling
docker compose run --rm -it streamflow-profiling script/profile_memory.sh
```

This will:
- Run StreamFlow with jemalloc profiling in a Linux container
- Run the sustained throughput benchmark
- Generate allocation reports with full symbol resolution
- Save results to `var/memory/`

## View Results

```bash
# Allocation report with function names
cat var/memory/allocation_report.txt | head -50

# Call graph visualization
open var/memory/callgraph.pdf
```

## Expected Output

With proper symbol resolution, you'll see function names:

```
Total: 84775072 B
 24528800  28.9%  28.9%  24528800  28.9% streamflow_core::orchestrator::evaluate_workflow
 13109200  15.5%  44.4%  13109200  15.5% tokio::runtime::task::raw::RawTask::new
 11012736  13.0%  57.4%  11012736  13.0% sqlx_postgres::connection::establish
```

Instead of raw addresses on macOS:

```
Total: 84775072 B
84775072 100.0% 100.0% 84775072 100.0% 0x0000000104f5a284
       0   0.0% 100.0% 28253885  33.3% 0x0000000104c367ef
```
