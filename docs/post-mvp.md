# Post-MVP

Features and work that have been deliberately postponed. Each entry says what was postponed, why, and what a later design has to settle.

## Dividing orchestration between processes

**Postponed:** 2026-11-09, with parallel event processing inside one orchestrator process (`docs/implementation/2026-11-09-parallel-orchestrator.md`, §2.7).

**Today:** one orchestrator process handles events of different workflows concurrently in its partitions. Two orchestrator processes share one consumer and each handles every event; the handlers are idempotent, so that is safe, but it does not divide the work.

**The idea:** the same partitioning across processes. K consumers, each reading only the workflows whose hash falls in its share and keeping its own durable cursor, each running its own in-process partitions.

**What a design has to settle:**

- A workflow hash that is stable across processes and releases (the in-process one only has to be stable for the life of a process).
- Consumer naming, and how a process learns K and its share.
- Changing K: new consumers must start from the lowest cursor of the old ones, which replays events (safe, the handlers are idempotent) and must be bounded.
- Reading a share efficiently: a filter on the hash in the poll, or an indexed partition column on `workflow_events`.
- What takes over a share when its process dies.

**Why not now:** in-process partitions remove the limit the benchmarks showed (the serial orchestrator). With eight partitions throughput no longer rises with more, so the orchestrator is not what limits a single node; what does has not been measured, and that measurement should come before this work.
