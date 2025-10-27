# StreamFlow v0.2: Compiled Workflow Representation

**Date**: October 25, 2025  
**Type**: Performance Optimization  
**Section**: 1.8 - Compiled Workflow Representation  
**Impact**: 100x orchestration speedup (<1ms → <10µs)

---

## Overview

The **Compiled Workflow Representation** is a performance optimization that transforms workflow definitions into an ultra-efficient binary format designed for the Rust orchestrator. This enables **sub-10 microsecond** orchestration latency instead of the baseline ~1 millisecond, crucial for achieving the >1,000 workflows/sec throughput target.

---

## The Problem

### Baseline Orchestration Performance (~1ms)

When using standard DAG evaluation:

```
Activity completes → Event published
   ↓
Orchestrator receives event (< 1µs)
   ↓
Load workflow definition from PostgreSQL (~500µs)
   ↓
Parse JSON/YAML into memory (~100µs)
   ↓
Iterate through all activities (~200µs)
   ↓
Check dependencies (HashMap lookups) (~200µs)
   ↓
Schedule ready activities to queue (~100µs per activity)
   ↓
Total: ~1,000µs (1ms)
```

### Why This Matters at Scale

**At 1,000 workflows/sec**:
- Each workflow has ~5 activities on average
- Each activity completion triggers evaluation
- Total: ~5,000 evaluations/second
- At 1ms each: **5 full CPU cores just for orchestration**
- PostgreSQL: 5,000 definition loads/second (cache helps but not enough)

**The Goal**: Reduce orchestration overhead to <1% of CPU time

---

## The Solution: Compiled Workflow Format

### Core Concept

Transform the workflow definition **once** at registration time into an optimized binary format that:

1. **Pre-computes dependency graph** (adjacency lists)
2. **Uses bitmasks for state tracking** (64 activities per u64)
3. **Eliminates database lookups** (in-memory caching)
4. **Enables CPU cache-friendly access patterns**

### Data Structure

```rust
pub struct CompiledWorkflow {
    // Workflow metadata
    workflow_id: Uuid,
    workflow_name: String,
    activity_count: usize,
    
    // Activity metadata (indexed)
    activities: Vec<CompiledActivity>,
    
    // Pre-computed dependency graph
    dependencies: Vec<Vec<usize>>,     // dependencies[i] = activities that must complete first
    dependents: Vec<Vec<usize>>,       // dependents[i] = activities that depend on this
    initial_activities: Vec<usize>,    // Activities with no dependencies
    
    // Bitmask configuration
    completion_mask_size: usize,       // Number of u64s needed for bitmask
    
    // Optional conditionals (pre-compiled)
    conditionals: Vec<CompiledConditional>,
}
```

### Example: Payment Workflow

**Original YAML**:
```yaml
workflow: payment_processing
activities:
  - key: validate_payment       # index 0
  - key: authorize_card         # index 1
    edges:
      - preceding_key: validate_payment
  - key: capture_payment        # index 2
    edges:
      - preceding_key: authorize_card
```

**Compiled Representation**:
```rust
CompiledWorkflow {
    workflow_name: "payment_processing",
    activity_count: 3,
    activities: [
        CompiledActivity { index: 0, key: "validate_payment", ... },
        CompiledActivity { index: 1, key: "authorize_card", ... },
        CompiledActivity { index: 2, key: "capture_payment", ... },
    ],
    dependencies: [
        [],      // Activity 0 has no dependencies
        [0],     // Activity 1 depends on activity 0
        [1],     // Activity 2 depends on activity 1
    ],
    dependents: [
        [1],     // Activity 0 is depended on by activity 1
        [2],     // Activity 1 is depended on by activity 2
        [],      // Activity 2 has no dependents
    ],
    initial_activities: [0],  // Activity 0 can start immediately
    completion_mask_size: 1,  // Need 1 u64 (64 bits) for 3 activities
    conditionals: [],
}
```

---

## Ultra-Fast Evaluation with Bitmasks

### Execution State (Minimal Memory Footprint)

```rust
pub struct WorkflowExecutionState {
    workflow_id: Uuid,
    
    // Completion status as bitmask
    // Bit 0 = activity 0, Bit 1 = activity 1, etc.
    completion_mask: Vec<u64>,  // [0b00000111] = activities 0,1,2 complete
    
    // Activity results (sparse - only store what's needed)
    results: HashMap<usize, serde_json::Value>,
    
    // Currently scheduled (prevent duplicates)
    scheduled_mask: Vec<u64>,
}
```

### Bitmask Operations (Blazing Fast)

**Check if activity complete**:
```rust
#[inline]
pub fn is_complete(&self, activity_index: usize) -> bool {
    let word_index = activity_index / 64;
    let bit_index = activity_index % 64;
    (self.completion_mask[word_index] & (1u64 << bit_index)) != 0
}
// Performance: ~1 nanosecond (single CPU instruction)
```

**Mark activity complete**:
```rust
#[inline]
pub fn mark_complete(&mut self, activity_index: usize) {
    let word_index = activity_index / 64;
    let bit_index = activity_index % 64;
    self.completion_mask[word_index] |= 1u64 << bit_index;
}
// Performance: ~1 nanosecond (single CPU instruction)
```

**Check all dependencies satisfied**:
```rust
#[inline]
pub fn dependencies_satisfied(
    &self,
    compiled: &CompiledWorkflow,
    activity_index: usize
) -> bool {
    compiled.dependencies[activity_index]
        .iter()
        .all(|&dep_idx| self.is_complete(dep_idx))
}
// Performance: ~5-10 nanoseconds (few CPU cycles)
```

### Complete Evaluation Algorithm

```rust
pub fn find_ready_activities_compiled(
    compiled: &CompiledWorkflow,
    state: &WorkflowExecutionState
) -> Vec<usize> {
    let mut ready = Vec::with_capacity(8);
    
    // Iterate through all activities (~2ns per activity)
    for activity_index in 0..compiled.activity_count {
        // Quick bitmask checks (~2ns each)
        if state.is_complete(activity_index) || 
           state.is_scheduled(activity_index) {
            continue;
        }
        
        // Check dependencies with bitmask (~5-10ns)
        if state.dependencies_satisfied(compiled, activity_index) {
            ready.push(activity_index);
        }
    }
    
    ready
}
// Total performance: ~10 microseconds for typical workflow
```

---

## Performance Comparison

### Operation-Level Breakdown

| Operation | Uncompiled | Compiled | Speedup |
|-----------|------------|----------|---------|
| **Load workflow definition** | ~500µs (PostgreSQL query) | ~100ns (memory access) | 5,000x |
| **Parse definition** | ~100µs (JSON parsing) | 0µs (already parsed) | ∞ |
| **Check activity complete** | ~50ns (HashMap lookup) | ~1ns (bitmask check) | 50x |
| **Check dependencies** | ~200ns (iterate + HashMap) | ~5-10ns (bitmask AND) | 20-40x |
| **Find all ready activities** | ~500µs (iterate + checks) | ~10µs (bitmask scan) | 50x |
| **TOTAL EVALUATION** | **~1,000µs** | **~10µs** | **100x** |

### System-Level Impact

**Without Compilation (Baseline)**:
- 1,000 workflows/sec × 5 activities = 5,000 evaluations/sec
- 5,000 evaluations × 1ms = 5 CPU cores (100% utilization)
- 5,000 PostgreSQL queries/sec for workflow definitions
- Memory: ~50MB/sec allocation rate

**With Compilation**:
- 5,000 evaluations/sec × 10µs = 0.05 CPU cores (5% utilization)
- 0 PostgreSQL queries (in-memory cache)
- Memory: ~5MB total (compiled workflows cached)

**Result**: 20x more efficient CPU usage, zero database load for hot paths

---

## Implementation Strategies

### Strategy 1: Event-Carried Compiled State

**Concept**: Include compiled workflow in every event

```rust
pub enum WorkflowEvent {
    ActivityCompleted {
        workflow_id: Uuid,
        activity_index: usize,  // Use index, not string key
        result: serde_json::Value,
        compiled: Arc<CompiledWorkflow>,  // Shared reference
        state: WorkflowExecutionState,    // Current state
    }
}
```

**Orchestrator evaluation**:
```rust
async fn on_activity_completed(event: WorkflowEvent) {
    // No database lookup needed - everything in event!
    let ready = find_ready_activities_compiled(&event.compiled, &event.state);
    
    for idx in ready {
        schedule_activity(&event.compiled, idx).await;
    }
}
// Performance: <10µs total (zero I/O)
```

**Pros**:
- Zero database lookups during execution
- Self-contained events (easier debugging)
- Stateless orchestrator (horizontal scaling)

**Cons**:
- Larger events (~1-10KB vs ~100 bytes)
- More memory if events queued
- Compiled form duplicated across events

### Strategy 2: In-Memory LRU Cache

**Concept**: Cache compiled workflows in memory

```rust
pub struct Orchestrator {
    compiled_cache: Arc<RwLock<LruCache<Uuid, Arc<CompiledWorkflow>>>>,
    db_pool: PgPool,
}
```

**Orchestrator evaluation**:
```rust
async fn on_activity_completed(&self, workflow_id: Uuid, activity_key: &str) {
    // Try cache first (~100ns)
    let compiled = match self.compiled_cache.read().get(&workflow_id) {
        Some(c) => c.clone(),  // Cache hit!
        None => {
            // Cache miss: load and compile (~500µs, rare)
            let def = self.load_from_db(workflow_id).await?;
            let compiled = Arc::new(compile_workflow(&def)?);
            self.compiled_cache.write().put(workflow_id, compiled.clone());
            compiled
        }
    };
    
    // Load state (~100-200µs from PostgreSQL)
    let state = self.load_state(workflow_id).await?;
    
    // Find ready (~10µs)
    let ready = find_ready_activities_compiled(&compiled, &state);
    
    // Total: ~120µs (cache hit) or ~620µs (cache miss)
}
```

**Pros**:
- Small event size
- Shared memory (one copy per workflow)
- Automatic eviction of cold workflows

**Cons**:
- Still need to load state from PostgreSQL (~100-200µs)
- Cache misses on cold workflows

### Strategy 3: Hybrid (RECOMMENDED)

**Combine both approaches**:

1. **Store compiled form in PostgreSQL**
   ```sql
   CREATE TABLE compiled_workflows (
       workflow_name TEXT PRIMARY KEY,
       version TEXT NOT NULL,
       compiled_data BYTEA NOT NULL
   );
   ```

2. **LRU cache for hot workflows**
   - Keep last 1,000 workflows in memory
   - 10MB memory footprint typical

3. **Include in events for very hot workflows**
   - Flag workflows with >100 executions/minute
   - Include compiled form in their events
   - Reduces state load overhead

**Result**: Best of both worlds
- <10µs for very hot workflows (event-carried)
- ~120µs for hot workflows (cache hit)
- ~520µs for cold workflows (database load + compile)

---

## Compilation Process

### At Workflow Registration

```rust
pub fn compile_workflow(definition: &WorkflowDefinition) -> Result<CompiledWorkflow> {
    // 1. Build index mapping
    let activity_map: HashMap<String, usize> = definition.activities
        .iter()
        .enumerate()
        .map(|(idx, a)| (a.key.clone(), idx))
        .collect();
    
    // 2. Build dependency graph
    let mut dependencies = vec![Vec::new(); definition.activities.len()];
    let mut dependents = vec![Vec::new(); definition.activities.len()];
    
    for (idx, activity) in definition.activities.iter().enumerate() {
        for edge in &activity.edges {
            match edge.edge_type {
                EdgeType::Preceding => {
                    let dep_idx = activity_map[&edge.target_key];
                    dependencies[idx].push(dep_idx);
                    dependents[dep_idx].push(idx);
                }
                EdgeType::Following => {
                    let target_idx = activity_map[&edge.target_key];
                    dependencies[target_idx].push(idx);
                    dependents[idx].push(target_idx);
                }
            }
        }
    }
    
    // 3. Find initial activities (no dependencies)
    let initial_activities: Vec<usize> = (0..definition.activities.len())
        .filter(|&idx| dependencies[idx].is_empty())
        .collect();
    
    // 4. Calculate bitmask size (64 bits per u64)
    let completion_mask_size = (definition.activities.len() + 63) / 64;
    
    Ok(CompiledWorkflow {
        workflow_name: definition.name.clone(),
        activity_count: definition.activities.len(),
        activities: /* ... */,
        dependencies,
        dependents,
        initial_activities,
        completion_mask_size,
        conditionals: vec![],
    })
}
// Performance: ~50-100µs (one-time cost)
```

### Example Compilation

**Input (YAML)**:
```yaml
workflow: data_pipeline
activities:
  - key: fetch_data
  - key: validate_data
    edges: [{preceding_key: fetch_data}]
  - key: transform_data
    edges: [{preceding_key: validate_data}]
  - key: load_data
    edges: [{preceding_key: transform_data}]
```

**Output (Compiled)**:
```rust
CompiledWorkflow {
    activity_count: 4,
    dependencies: [
        [],      // fetch_data: no dependencies
        [0],     // validate_data: depends on fetch_data
        [1],     // transform_data: depends on validate_data
        [2],     // load_data: depends on transform_data
    ],
    initial_activities: [0],  // Only fetch_data can start
}
```

**Execution Trace**:
```
t=0: State = 0b0000, Ready = [0] (fetch_data)
t=1: State = 0b0001, Ready = [1] (validate_data)
t=2: State = 0b0011, Ready = [2] (transform_data)
t=3: State = 0b0111, Ready = [3] (load_data)
t=4: State = 0b1111, Workflow complete
```

---

## Scaling to Large Workflows

### Memory Efficiency

For a workflow with **N activities**:

**Compiled workflow size**:
- Activities: N × 64 bytes = 64N bytes
- Dependencies: ~2N entries × 8 bytes = 16N bytes
- Metadata: ~100 bytes
- **Total: ~80N + 100 bytes**

**Examples**:
- 10 activities: ~1 KB
- 100 activities: ~8 KB
- 1,000 activities: ~80 KB

**Execution state size**:
- Bitmask: (N/64) × 8 bytes ≈ N/8 bytes
- Results: ~M × 100 bytes (where M = activities with stored results)
- **Total: ~N/8 + 100M bytes**

**Cache capacity**:
- 10MB cache = ~1,250 workflows @ 80 activities average
- 100MB cache = ~12,500 workflows @ 80 activities average

### Bitmask Limitations

**Maximum activities per workflow**: 
- Single u64: 64 activities
- Vec<u64>: Unlimited (but practical limit ~10,000)

**For very large workflows** (>1,000 activities):
- Consider chunking into sub-workflows
- Or use sparse bitmaps (only track active portions)

---

## Implementation Phases

### Phase 1: MVP (Launch)
**Timeline**: Weeks 1-16  
**Approach**: Standard DAG evaluation

- Simple, proven algorithm
- ~1ms orchestration latency
- Sufficient for 100-500 workflows/sec
- Easy to debug and reason about

**Skip compilation for MVP**:
- Adds complexity
- Optimization can wait
- Focus on core functionality

### Phase 2: Optimization (Post-Launch)
**Timeline**: Weeks 17-24  
**Approach**: Add compiled representation

1. Implement compilation logic
2. Add LRU cache (Strategy 2)
3. Store compiled form in PostgreSQL
4. Measure performance improvement

**Success criteria**:
- <100µs orchestration latency (P99)
- 90%+ cache hit rate
- 1,000+ workflows/sec sustained

### Phase 3: Scale (Production Hardening)
**Timeline**: Weeks 25-32  
**Approach**: Event-carried state (Strategy 1 + 3)

1. Include compiled form in hot workflow events
2. Dynamic switching based on workflow frequency
3. Advanced caching strategies
4. Performance monitoring dashboard

**Success criteria**:
- <20µs orchestration latency (P99) for hot workflows
- 5,000+ workflows/sec sustained
- <1% CPU overhead for orchestration

---

## Monitoring & Observability

### Key Metrics

```rust
pub struct OrchestrationMetrics {
    // Performance
    eval_time_compiled_p50: Duration,
    eval_time_compiled_p99: Duration,
    eval_time_uncompiled_p50: Duration,
    eval_time_uncompiled_p99: Duration,
    
    // Cache effectiveness
    cache_hit_rate: f64,           // 0.0 to 1.0
    cache_size_bytes: usize,
    cache_entries: usize,
    cache_evictions_per_sec: f64,
    
    // Compilation
    compilations_per_sec: f64,
    compilation_time_p99: Duration,
    
    // Workflow distribution
    compiled_workflow_count: usize,
    uncompiled_workflow_count: usize,
}
```

### Dashboard View

```
Orchestration Performance:
┌─────────────────────────────────────────┐
│ Evaluation Latency (P99)                │
│                                         │
│ Compiled:     █ 8.3µs                  │
│ Uncompiled:   ████████████ 987µs       │
└─────────────────────────────────────────┘

Cache Performance:
┌─────────────────────────────────────────┐
│ Hit Rate:  95.2%  ████████████████░░░░  │
│ Size:      8.3MB / 10MB                 │
│ Entries:   1,847 / 2,000                │
│ Evictions: 3.2/sec                      │
└─────────────────────────────────────────┘

Throughput:
┌─────────────────────────────────────────┐
│ Workflows/sec:     2,341                │
│ Evaluations/sec:  11,847                │
│ CPU overhead:      2.3%                 │
└─────────────────────────────────────────┘
```

---

## Advantages Over Competitors

### vs Temporal

**Temporal**:
- Decision logic in external workers (Go code)
- Each decision requires RPC to worker
- Worker must maintain state
- Latency: 10-50ms per decision

**StreamFlow (compiled)**:
- Decision logic in orchestrator (Rust)
- No RPC needed
- State in bitmask (compact)
- Latency: <10µs per decision

**Result**: 1,000-5,000x faster orchestration

### vs Airflow

**Airflow**:
- Python DAG evaluation
- Full DAG re-parse on each task
- No compilation or optimization
- Latency: 50-500ms per task schedule

**StreamFlow (compiled)**:
- Pre-compiled dependency graph
- Bitmask-based evaluation
- Zero parsing overhead
- Latency: <10µs per task schedule

**Result**: 5,000-50,000x faster orchestration

### vs Conductor

**Conductor**:
- Java-based evaluation
- JSON workflow definitions
- Database query per evaluation
- Latency: 5-20ms per task schedule

**StreamFlow (compiled)**:
- Rust-based evaluation
- Binary compiled format
- In-memory cached
- Latency: <10µs per task schedule

**Result**: 500-2,000x faster orchestration

---

## Summary

The **Compiled Workflow Representation** is a game-changing optimization that:

✅ **100x faster orchestration** (1ms → 10µs)  
✅ **Enables >1,000 workflows/sec** target  
✅ **Reduces CPU overhead** from 100% to <5%  
✅ **Eliminates database load** for hot paths  
✅ **Maintains code simplicity** (compile once, use forever)  
✅ **Scales to large workflows** (bitmask efficiency)  
✅ **Phased implementation** (can ship without it, optimize later)  

This optimization, combined with StreamFlow's event-driven architecture and Rust implementation, positions StreamFlow as the **highest-performance workflow orchestration platform** on the market—10-1000x faster than any competitor for the orchestration hot path.
