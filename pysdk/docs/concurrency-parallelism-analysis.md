# Python Worker Concurrency and Parallelism Analysis

**Date**: 2026-01-25
**Status**: Post-MVP Consideration
**Priority**: Low (current model handles I/O-bound workloads well)

## Current Implementation

The Python worker uses `asyncio.create_task()` for concurrent activity execution with `asyncio.Semaphore` for limiting concurrent activities.

### Key Characteristics

- **Single-threaded**: All tasks run on one thread
- **Cooperative multitasking**: Tasks yield control at `await` points
- **Concurrency model**: While one task waits on I/O, others can progress

### Code Pattern (poller.py)

```python
for activity in activities:
    await self._semaphore.acquire()
    self._active_count += 1
    task = asyncio.create_task(self._execute_activity_with_permit(activity))
    self._tasks.add(task)
```

## Comparison with Rust Implementation

| Aspect              | Python asyncio              | Rust tokio                  |
|---------------------|-----------------------------|-----------------------------|
| Threading           | Single thread               | Thread pool                 |
| Parallelism         | Cooperative (I/O-bound)     | True parallel               |
| CPU-bound work      | Blocks event loop           | Runs in parallel            |
| Semaphore type      | `asyncio.Semaphore`         | `tokio::sync::Semaphore`    |
| Task spawn          | `create_task()`             | `tokio::spawn()`            |

### Why Current Model Works

Activity execution is primarily **I/O-bound**:
- HTTP calls to the API (poll, heartbeat, complete/fail)
- File uploads/downloads
- Waiting for external services

Tasks spend most time waiting on network I/O, not computing. Multiple activities progress concurrently as they take turns waiting on different I/O operations.

---

## Alternative: ThreadPoolExecutor

### What It Provides

Threads release the GIL during I/O and C extension calls, enabling parallelism for:
- I/O-bound work (already handled well by asyncio)
- C extensions that release GIL (numpy, pandas, etc.)

**Does not help**: Pure Python CPU-bound code still serializes due to GIL.

### Implementation Changes Required

```python
import asyncio
from concurrent.futures import ThreadPoolExecutor

class WorkerPoller:
    def __init__(self, ...):
        ...
        self._executor = ThreadPoolExecutor(max_workers=config.max_concurrent_activities)

    async def _execute_activity(self, activity: PendingActivity) -> None:
        # Run the user's activity handler in a thread
        result = await asyncio.get_event_loop().run_in_executor(
            self._executor,
            self._registry.execute_sync,  # Need sync version
            activity.worker,
            activity.activity_name,
            activity.parameters,
            ctx,
        )
```

### Challenges

- Activity handlers would need to be **sync functions** (not async)
- `ActivityContext.heartbeat()` and file operations are async - would need sync wrappers or separate event loops per thread
- Thread safety for shared state (registry, client token caching)

---

## Alternative: ProcessPoolExecutor

### What It Provides

True parallelism - each process has its own Python interpreter and GIL.

### Implementation Changes Required

```python
from concurrent.futures import ProcessPoolExecutor

class WorkerPoller:
    def __init__(self, ...):
        ...
        self._executor = ProcessPoolExecutor(max_workers=config.max_concurrent_activities)

    async def _execute_activity(self, activity: PendingActivity) -> None:
        result = await asyncio.get_event_loop().run_in_executor(
            self._executor,
            _execute_in_process,  # Top-level function (must be picklable)
            activity.parameters,
            activity.activity_name,
            # Cannot pass: ctx, client, registry (not picklable)
        )
```

### Significant Challenges

| Challenge            | Impact                                                      |
|----------------------|-------------------------------------------------------------|
| **Serialization**    | All params/results must be pickle-able                      |
| **No shared objects**| Can't share `WorkerApiClient`, `ActivityRegistry`, `ActivityContext` |
| **Heartbeats**       | Need IPC mechanism or separate HTTP client per process      |
| **File operations**  | Each process needs its own client connection                |
| **Registry**         | Must re-register activities in each subprocess              |
| **Startup cost**     | Process spawn is expensive (~100ms vs ~1ms for threads)     |
| **Memory**           | Each process duplicates Python interpreter (~30-50MB)       |

### Required Architecture Changes

```python
# Worker process entry point (top-level, picklable)
def _execute_in_process(
    api_url: str,
    client_id: str,
    client_secret: str,
    workflow_id: UUID,
    activity_id: UUID,
    activity_key: str,
    activity_name: str,
    parameters: dict,
    worker_id: str,
) -> dict:
    """Run in subprocess - must recreate everything."""
    # Create fresh client for this process
    client = WorkerApiClient(api_url, client_id, client_secret)

    # Re-import and find the activity handler
    handler = _discover_activity(activity_name)

    # Create context with process-local client
    ctx = ActivityContext(...)

    # Execute synchronously
    result = handler(ctx, **parameters)

    # Return serializable result
    return result.model_dump()
```

---

## Future: Python 3.13+ Free-threaded Mode

Python 3.13 introduced experimental `--disable-gil` for true thread parallelism:

```bash
python3.13t --disable-gil worker.py
```

This would make `ThreadPoolExecutor` achieve true parallelism without multiprocessing complexity.

### Considerations

- Still experimental (as of 2026)
- Some C extensions may not be compatible
- Performance characteristics still being tuned

---

## Recommendations

### Current State (MVP)

**Stay with asyncio** - activity execution is primarily I/O-bound (HTTP, file transfers).

### Future Enhancements (Post-MVP)

1. **Optional thread offload** for CPU-heavy activities:
   ```python
   @activity(offload_to_thread=True)
   def cpu_heavy_task(ctx, data):
       # Runs in ThreadPoolExecutor
       return heavy_computation(data)
   ```

2. **Consider ProcessPoolExecutor only if**:
   - Users report actual CPU-bound bottlenecks
   - The complexity cost is justified by measured performance gains

3. **Monitor Python 3.13+ free-threaded mode** for potential adoption when stable.

### Decision Criteria for Migration

Consider migrating to thread/process pools when:
- Activity handlers perform significant CPU-bound computation
- Users report that async model is bottleneck (not network I/O)
- Measured latency improvements justify added complexity
