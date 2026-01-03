# Memory Profiling Guide

## Important: macOS ARM64 (Apple Silicon) Specifics

On macOS ARM64, tikv-jemallocator uses **prefixed malloc** functions, which means:
- The static variable must be named `_rjem_malloc_conf` (not `malloc_conf`)
- The environment variable must be `_RJEM_MALLOC_CONF` (not `MALLOC_CONF`)

This is handled automatically in the code and scripts.

## Quick Reference - Manual Profiling

The automated `scripts/profile_memory.sh` has integration issues. Use this manual process instead:

### Step 1: Build with jemalloc and debug symbols

```bash
# Use the 'profiling' profile which keeps symbols for jeprof
cargo build --profile profiling --features profiling
```

Note: The `profiling` profile inherits from `release` but keeps debug symbols (`strip = false, debug = 2`).

### Step 2: Set up profiling environment

```bash
# Set output directory first
export PROFILE_DIR="var/memory-profile-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$PROFILE_DIR"
export _RJEM_MALLOC_CONF="prof_active:true,prof_prefix:$(pwd)/$PROFILE_DIR/jeprof.out,lg_prof_interval:30"
```

### Step 3: Run benchmark with profiling

```bash
# From the profile directory
scripts/profiling.sh --test test_sustained_throughput --trace-level debug --profile
```

The heap dumps will be created in the current directory as `jeprof.out.*.heap`.

### Step 4: Analyze heap dumps

```bash
# List all heap dumps (sorted by time, newest first)
ls -t ${PROFILE_DIR}/jeprof.out.*.heap

# Get the final/largest dump
FINAL_DUMP=$(ls -t ${PROFILE_DIR}/jeprof.out.*.heap | head -1)

# Generate text report of top allocations
jeprof --show_bytes --text target/profiling/kruxiaflow "$FINAL_DUMP" > ${PROFILE_DIR}/allocation_report.txt

# View top 30 allocations
head -30 ${PROFILE_DIR}/allocation_report.txt

# Generate flamegraph SVG (requires graphviz)
jeprof --show_bytes --svg target/profiling/kruxiaflow "$FINAL_DUMP" > ${PROFILE_DIR}/flamegraph.svg

# Open flamegraph
open flamegraph.svg
```

## Alternative: Using heaptrack (macOS/Linux)

If jemalloc profiling isn't working, use heaptrack:

### Install heaptrack

```bash
# macOS
brew install heaptrack

# Ubuntu/Debian
sudo apt-get install heaptrack heaptrack-gui
```

### Run with heaptrack

```bash
# Build release binary
cargo build --release

# Run with heaptrack
heaptrack ./target/release/kruxiaflow serve --port 8080 --workers 20 &
SERVER_PID=$!

# Wait for server to start
sleep 5

# Run benchmark
./scripts/profiling.sh --test test_sustained_throughput --skip-server-start

# Stop server
kill $SERVER_PID

# Analyze (GUI)
heaptrack_gui heaptrack.kruxiaflow.*.gz

# Or text analysis
heaptrack_print heaptrack.kruxiaflow.*.gz | head -50
```

## What to Look For

### Top Allocations

The allocation report shows functions sorted by total memory allocated:

```
Total: 124.03 MB
 42.5 MB (34.3%)  kruxiaflow_core::orchestrator::workflow_state::save_materialized_state
 28.7 MB (23.1%)  sqlx::postgres::connection::establish
 15.2 MB (12.3%)  kruxiaflow_core::events::postgres_event_source::poll
  8.1 MB  (6.5%)  tokio::runtime::thread_pool::spawn
  ...
```

Look for:
- Functions allocating >10% of total memory
- Workflow/event processing functions (likely culprits)
- Database connection/query functions
- Repeated allocations that should be pooled

### Flamegraph Analysis

The SVG flamegraph shows call stacks with width proportional to memory:

- **Wide bars** = Large allocations
- **Tall stacks** = Deep call chains
- **Repeated patterns** = Likely leak if growing over time

Focus on:
1. Widest bars at the top (biggest allocators)
2. Workflow-related functions
3. Any surprising large allocations

## Expected Findings

Based on investigation, expect to find one or more of:

### 1. Workflow State Accumulation (Most Likely)

```
save_materialized_state: 40 MB
├─ serialize_workflow_state: 35 MB
└─ in-memory state cache: 5 MB
```

**Fix**: Implement cleanup for completed workflows

### 2. Event Buffer Growth

```
poll_events: 25 MB
├─ event_buffer: 20 MB
└─ consumer_positions: 5 MB
```

**Fix**: Implement event truncation/archival

### 3. SQLx Statement Cache

```
prepare_cached: 15 MB
├─ statement_cache: 12 MB
└─ query_metadata: 3 MB
```

**Fix**: Configure max cache size

## Troubleshooting

### Invalid conf pair errors

If you see errors like `<jemalloc>: Invalid conf pair: prof:true`, this means you're trying to set options that are already compiled in. The Kruxia Flow binary has profiling support compiled in via the `malloc_conf` static variable in main.rs.

**Solution**: Use `_RJEM_MALLOC_CONF` (not `MALLOC_CONF`) and only set runtime activation options:
```bash
# ✅ Correct - use _RJEM_ prefix, only activate and configure output
export _RJEM_MALLOC_CONF="prof_active:true,prof_prefix:jeprof.out,lg_prof_interval:30"

# ❌ Wrong - prof:true is already compiled in
export _RJEM_MALLOC_CONF="prof:true,prof_active:true,..."

# ❌ Wrong - missing _RJEM_ prefix
export MALLOC_CONF="prof_active:true,..."
```

### No heap dumps created

1. **Check profiling is activated with correct variable**:
```bash
echo $_RJEM_MALLOC_CONF
```
Should show: `prof_active:true,...` (note the `_RJEM_` prefix!)

2. **Verify binary was built with jemalloc**:
```bash
cargo build --release --features profiling
```

3. **Check the binary has profiling enabled**:
```bash
strings target/release/kruxiaflow | grep "prof:true"
```
Should output: `prof:true,prof_active:false,lg_prof_sample:19,prof_final:true`

4. **Ensure program allocates enough memory**:
The default `lg_prof_interval:30` means heap dumps are created every ~1GB of allocations. For shorter runs, use a smaller interval like `lg_prof_interval:27` (~128MB).

### jeprof not found

Install jemalloc:
```bash
# macOS
brew install jemalloc

# Ubuntu/Debian
sudo apt-get install libjemalloc-dev
```

### "Cannot find symbols"

Build with debug symbols:
```bash
cargo build --release
# (debug=true already set in Cargo.toml)
```

### Heap dumps too small

Increase profiling dump frequency (sampling rate is already set in compiled binary):
```bash
# Dump heap profile more frequently (lg_prof_interval=27 means every ~128MB allocated)
export _RJEM_MALLOC_CONF="prof_active:true,prof_prefix:jeprof.out,lg_prof_interval:27"
```

To change sampling rate, you need to rebuild with different `lg_prof_sample` in main.rs.

## Next Steps After Profiling

1. **Identify top allocator** from report
2. **Review code** at identified function
3. **Implement fix**:
   - Add cleanup for completed workflows
   - Add event truncation
   - Configure cache limits
4. **Re-run test** to verify fix
5. **Ensure memory growth <20%**

## References

- [jemalloc profiling](https://github.com/jemalloc/jemalloc/wiki/Use-Case:-Heap-Profiling)
- [heaptrack documentation](https://github.com/KDE/heaptrack)
- Investigation results: `docs/performance/investigation-results-2025-11-08.md`
