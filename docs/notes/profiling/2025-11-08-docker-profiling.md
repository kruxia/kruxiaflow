# Docker-Based Memory Profiling

This guide explains how to use Docker-based memory profiling for Kruxia Flow, which provides proper symbol resolution on all platforms (especially macOS where native jeprof has limitations).

## Why Docker for Profiling?

**Problem on macOS:**
- jeprof cannot resolve Rust symbols properly due to macOS-specific debug info format
- Allocation reports show raw memory addresses instead of function names
- dSYM generation and ASLR complicate symbol resolution

**Solution:**
- Run profiling inside a Linux Docker container
- jeprof works natively with Linux binaries and DWARF debug info
- Full symbol resolution with function names, file paths, and line numbers

## Quick Start

### 1. Run Memory Profiling

```bash
./scripts/profile_memory_docker.sh
```

This will:
1. Build the profiling Docker image (first time only)
2. Start PostgreSQL if not running
3. Build Kruxia Flow with jemalloc profiling inside the container
4. Run the sustained throughput benchmark
5. Generate allocation reports with full symbol information
6. Save results to `var/memory-profile-TIMESTAMP/`

### 2. View Results

```bash
# View allocation report with function names
cat var/memory-profile-*/allocation_report.txt | head -50

# View call graph visualization
open var/memory-profile-*/callgraph.svg
```

## Options

### Rebuild Docker Image

After modifying dependencies or Dockerfile:

```bash
./scripts/profile_memory_docker.sh --build
```

### Interactive Shell

Drop into the container for manual profiling or debugging:

```bash
./scripts/profile_memory_docker.sh --bash

# Inside container:
./scripts/profile_memory.sh              # Run profiling
jeprof --text target/profiling/kruxiaflow var/memory-profile-*/jeprof.out.*.heap
```

## Architecture

### Docker Setup

**Dockerfile.profiling:**
- Based on `rust:1.83-bookworm`
- Installs jemalloc, jeprof, graphviz, PostgreSQL client
- Pre-installs sqlx-cli for migrations

**docker-compose.yml:**
- `profiling` service with Rust build environment
- Mounts source code and cargo cache
- Connects to PostgreSQL service
- Persists build artifacts across runs

### Profiling Flow

```
Host                          Docker Container (Linux)
────                          ─────────────────────────
profile_memory_docker.sh  →   profile_memory.sh
                              │
                              ├─ cargo build --profile profiling --features profiling
                              ├─ profiling.sh --profiling
                              │  └─ Kruxia Flow server with jemalloc enabled
                              │     └─ Heap dumps → var/jeprof.out.*.heap
                              │
                              ├─ jeprof --text [binary] [heap]  → allocation_report.txt
                              └─ jeprof --svg [binary] [heap]   → callgraph.svg
                                  ↓
                              Results saved to var/ (mounted on host)
```

## Expected Output

With proper symbol resolution, you'll see:

```
Total: 84775072 B
 24528800  28.9%  28.9%  24528800  28.9% kruxiaflow_core::orchestrator::evaluate_workflow
 13109200  15.5%  44.4%  13109200  15.5% tokio::runtime::task::raw::RawTask::new
 11012736  13.0%  57.4%  11012736  13.0% sqlx_postgres::connection::establish
  8671533  10.2%  67.6%   8671533  10.2% hyper::proto::h1::conn::Conn::new
```

Instead of:

```
Total: 84775072 B
84775072 100.0% 100.0% 84775072 100.0% 0x0000000104f5a284
       0   0.0% 100.0% 28253885  33.3% 0x0000000104c367ef
```

## Troubleshooting

### Docker Not Running

```
Error: Docker is not running
```

**Solution:** Start Docker Desktop

### PostgreSQL Connection Issues

```
Error: connection refused
```

**Solution:** Ensure PostgreSQL is running:
```bash
docker-compose up -d postgres
docker exec kruxiaflow-postgres pg_isready -U kruxiaflow
```

### Build Errors

If you encounter build errors, try cleaning and rebuilding:

```bash
./scripts/profile_memory_docker.sh --bash
# Inside container:
cargo clean
cargo build --profile profiling --features profiling
```

### No Heap Dumps Generated

Check the benchmark logs:
```bash
tail -100 var/memory-profile-*/kruxiaflow-profiling.log
```

Verify jemalloc config:
```bash
echo $_RJEM_MALLOC_CONF
# Should show: prof_active:true,prof_prefix:...
```

## Comparison: macOS Native vs Docker

| Aspect | macOS Native | Docker (Linux) |
|--------|--------------|----------------|
| Symbol Resolution | ❌ Raw addresses | ✅ Function names |
| Setup Complexity | Simple | Moderate (Docker required) |
| Performance | Native speed | ~5-10% overhead |
| Consistency | macOS-specific | Matches production |
| Debug Info | Requires dSYM | Built-in DWARF |

## Advanced Usage

### Custom Profiling Parameters

Edit jemalloc config in `profile_memory.sh`:

```bash
# Sample more frequently (every 2^19 bytes instead of 2^30)
export _RJEM_MALLOC_CONF="prof_active:true,prof_prefix:$PROFILE_DIR/jeprof.out,lg_prof_sample:19"

# Dump on interval (every 2^28 bytes = ~268MB)
export _RJEM_MALLOC_CONF="prof_active:true,prof_prefix:$PROFILE_DIR/jeprof.out,lg_prof_interval:28"
```

### Different Benchmarks

```bash
# Inside container
./scripts/profiling.sh --profiling --test test_parallel_workflow_load
./scripts/profiling.sh --profiling --test test_activity_throughput
```

### Manual Profiling

```bash
./scripts/profile_memory_docker.sh --bash

# Inside container:
export _RJEM_MALLOC_CONF="prof_active:true,prof_prefix:/opt/var/custom.heap"
cargo build --profile profiling --features profiling
./target/profiling/kruxiaflow serve --workers 20 &

# ... run your workload ...

# Analyze
jeprof --text target/profiling/kruxiaflow var/custom.heap.* > custom_report.txt
```

## See Also

- [Memory Profiling Guide](memory-profiling-guide.md) - General profiling concepts
- [Performance Investigation](INVESTIGATION-COMPLETE.md) - Memory leak analysis results
- [Benchmark Scripts](../../scripts/profiling.sh) - Load testing configuration
