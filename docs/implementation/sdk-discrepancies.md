# SDK Discrepancies Tracking

This document tracks discrepancies between the Python and Rust SDKs that need to be resolved before v1.0 release.

**Decision**: Python SDK uses seconds for time intervals. Rust SDK should be updated to match.

## Summary Table

| Issue                               | Priority | Python SDK                               | Rust SDK                                  | Resolution                               |
|-------------------------------------|----------|------------------------------------------|-------------------------------------------|------------------------------------------|
| Poll Interval Units                 | HIGH     | `KRUXIAFLOW_WORKER_POLL_INTERVAL` (sec)  | `KRUXIAFLOW_WORKER_POLL_INTERVAL_MS` (ms) | Rust → seconds, drop `_MS` suffix        |
| Activity Timeout Variable Name      | MEDIUM   | `KRUXIAFLOW_WORKER_ACTIVITY_TIMEOUT`     | `KRUXIAFLOW_ACTIVITY_TIMEOUT`             | Rust → add `WORKER_` prefix              |
| Heartbeat Interval Variable Name    | MEDIUM   | `KRUXIAFLOW_WORKER_HEARTBEAT_INTERVAL`   | `KRUXIAFLOW_HEARTBEAT_INTERVAL`           | Rust → add `WORKER_` prefix              |
| Poll Max Activities Default         | LOW      | Default: 10                              | Default: 5                                | Align defaults (TBD which value)         |
| Client Secret File Support          | LOW      | Not implemented                          | `KRUXIAFLOW_CLIENT_SECRET_FILE` supported | Add to Python SDK                        |
| Orchestrator Poll Interval Units    | MEDIUM   | N/A (orchestrator is Rust-only)          | `_MS` suffix (milliseconds)               | Change to seconds, drop `_MS` suffix     |

## Detailed Discrepancies

### 1. Poll Interval Units (HIGH PRIORITY)

The environment variable for poll interval uses different units and naming conventions.

| SDK    | Environment Variable                   | Unit         | Default |
|--------|----------------------------------------|--------------|---------|
| Python | `KRUXIAFLOW_WORKER_POLL_INTERVAL`      | seconds      | 0.1     |
| Rust   | `KRUXIAFLOW_WORKER_POLL_INTERVAL_MS`   | milliseconds | 100     |

**Resolution**: Rust should change to `KRUXIAFLOW_WORKER_POLL_INTERVAL` using seconds (float).

**Files to update**:
- `kruxiaflow/src/commands/worker.rs:80-87`

### 2. Activity Timeout Variable Name (MEDIUM PRIORITY)

The activity timeout environment variable uses inconsistent naming.

| SDK    | Environment Variable                   | Unit    | Default |
|--------|----------------------------------------|---------|---------|
| Python | `KRUXIAFLOW_WORKER_ACTIVITY_TIMEOUT`   | seconds | 300     |
| Rust   | `KRUXIAFLOW_ACTIVITY_TIMEOUT`          | seconds | 300     |

**Resolution**: Rust should use `KRUXIAFLOW_WORKER_ACTIVITY_TIMEOUT` (with `WORKER_` prefix for consistency with other worker config).

**Files to update**:
- `kruxiaflow/src/commands/worker.rs:89-96`

### 3. Heartbeat Interval Variable Name (MEDIUM PRIORITY)

The heartbeat interval environment variable uses inconsistent naming.

| SDK    | Environment Variable                     | Unit    | Default |
|--------|------------------------------------------|---------|---------|
| Python | `KRUXIAFLOW_WORKER_HEARTBEAT_INTERVAL`   | seconds | 30      |
| Rust   | `KRUXIAFLOW_HEARTBEAT_INTERVAL`          | seconds | 30      |

**Resolution**: Rust should use `KRUXIAFLOW_WORKER_HEARTBEAT_INTERVAL` (with `WORKER_` prefix).

**Files to update**:
- `kruxiaflow/src/commands/worker.rs:98-105`

### 4. Poll Max Activities Default (LOW PRIORITY)

The default for maximum activities per poll differs between SDKs.

| SDK    | Environment Variable                     | Default |
|--------|------------------------------------------|---------|
| Python | `KRUXIAFLOW_WORKER_POLL_MAX_ACTIVITIES`  | 10      |
| Rust   | `KRUXIAFLOW_WORKER_POLL_MAX_ACTIVITIES`  | 5       |

**Resolution**: Decide on consistent default:
- **Option A**: Use 10 (Python's current default) - better throughput for single workers
- **Option B**: Use 5 (Rust's current default) - better work distribution across multiple workers

**Recommendation**: Use 10 as default. Users can tune based on deployment topology.

**Files to update**:
- `kruxiaflow/src/commands/worker.rs:65-78` (change default to 10)

### 5. Client Secret File Support (LOW PRIORITY)

Rust supports loading secrets from files (Docker secrets pattern), Python does not.

| SDK    | Feature                          | Status          |
|--------|----------------------------------|-----------------|
| Python | `KRUXIAFLOW_CLIENT_SECRET_FILE`  | Not implemented |
| Rust   | `KRUXIAFLOW_CLIENT_SECRET_FILE`  | Supported       |

**Resolution**: Add `KRUXIAFLOW_CLIENT_SECRET_FILE` support to Python SDK.

**Implementation approach**:
```python
# In WorkerConfig, check for _FILE variant first
def _load_secret(name: str) -> str | None:
    file_var = f"{name}_FILE"
    if file_path := os.environ.get(file_var):
        try:
            return Path(file_path).read_text().strip()
        except OSError:
            pass
    return os.environ.get(name)
```

**Files to update**:
- `py/kruxiaflow/worker/config.py`

### 6. Orchestrator Poll Interval Units (MEDIUM PRIORITY)

Orchestrator config uses `_MS` suffix for milliseconds, inconsistent with the seconds convention.

| Component    | Environment Variable                          | Unit         | Default |
|--------------|-----------------------------------------------|--------------|---------|
| Orchestrator | `KRUXIAFLOW_ORCHESTRATOR_POLL_INTERVAL_MIN_MS` | milliseconds | 50      |
| Orchestrator | `KRUXIAFLOW_ORCHESTRATOR_POLL_INTERVAL_MAX_MS` | milliseconds | 1000    |

**Resolution**: Change to seconds without `_MS` suffix:
- `KRUXIAFLOW_ORCHESTRATOR_POLL_INTERVAL_MIN` (default: 0.05)
- `KRUXIAFLOW_ORCHESTRATOR_POLL_INTERVAL_MAX` (default: 1.0)

**Files to update**:
- `core/src/orchestrator/config.rs:31-50`

## Implementation Plan

### Phase 1: Documentation (Current)
- [x] Document all discrepancies
- [x] Agree on resolutions

### Phase 2: Rust SDK Updates (Pre-v1.0)
- [ ] Update poll interval to seconds
- [ ] Add `WORKER_` prefix to timeout variables
- [ ] Align poll_max_activities default to 10
- [ ] Update orchestrator config to seconds

### Phase 3: Python SDK Updates (Pre-v1.0)
- [ ] Add client secret file support

## Notes

- All time-related environment variables should use **seconds** as the unit
- The `_MS` suffix pattern should be removed from all public APIs
- Worker-specific config should use `KRUXIAFLOW_WORKER_` prefix
- The Python SDK was designed to match Rust interface for future PyO3 migration
