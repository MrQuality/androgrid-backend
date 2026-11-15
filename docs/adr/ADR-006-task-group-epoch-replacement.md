# ADR-006: TaskGroup Workload Abstraction and Atomic Epoch Replacement

## Status
Accepted

## Architectural Scope & Requirements
- **Issue Reference:** `#43` (`[RFC] TaskGroup Workload Abstraction & Atomic Version Replacement`)
- **Traceability:** `[REQ-OP-002]`, `[REQ-OP-003]`, `[REQ-OP-004]`, `[INV-002]`, `[INV-003]`, `[INV-004]`, `[INV-005]`, `[INV-006]`, `[ANTI-003]`, `[NFR-PERF-001]`, `[NFR-REL-001]`
- **Standard:** ISO/IEC/IEEE 29148

---

## Context
Single-task submission (`SubmitTask`) forces operators and automation pipelines to submit, poll, and cancel partitioned WebAssembly workloads one task at a time. Replacing a running workload with a new WASM artifact (`artifact_sha256`) requires manual per-task cancellation and risks orphaned workers executing stale bytecode and consuming battery.

At the same time, the control plane (`ClusterScheduler`) is currently an in-memory `Arc<RwLock<SchedulerInner>>` state machine (`ADR-003`). Introducing a separate background reconciliation loop or partial persistence for `TaskGroup` without persisting `TaskRecord` would create split-brain state across restarts and lock-ordering hazards.

---

## Decision
We introduce a synchronous `TaskGroup` Epoch & Manifest Coordinator directly inside `SchedulerInner`, composing with `ADR-003` (`run_attempt` fencing), `FetchArtifact` content-addressed distribution, and `ADR-005` (`AdminService` mTLS allowlist):

1. **In-Memory Single-Lock Coordination:**
   - `TaskGroupRecord` lives inside `SchedulerInner` alongside `tasks` and `workers` so `SubmitTaskGroup`, `ReplaceTaskGroup`, `CancelTaskGroup`, and result commits execute as atomic single-lock Compare-and-Swap (CAS) operations (`[NFR-REL-001]`).
   - Whole-scheduler persistence (WAL/SQLite) is deferred to a dedicated future ADR covering all of `SchedulerInner` uniformly.

2. **Two-Level Monotonic Fencing (`[INV-004]`):**
   - `group_epoch` (`u64`, starting at `1`) fences workload version replacements across a group (`GROUP_SUPERSEDED`).
   - `run_attempt` (`u32`) continues to fence per-task retries and `[INV-005]` speculative preemptions (`TASK_ATTEMPT_STALE`).
   - `GROUP_SUPERSEDED` is gated on **task cancellation/supersession state** (`task.state == TaskState::Cancelled`) rather than `task.group_epoch < group.group_epoch` alone, allowing in-flight `LEASED`/`RUNNING` tasks under `REPLACE_STRATEGY_DRAIN` to commit within their existing `lease_deadline_epoch_s`.
   - `SubmitCellularResult` returns gRPC `FAILED_PRECONDITION` (`GROUP_SUPERSEDED`) on superseded submissions. `SyncNodeState` batches multiple reports and therefore drops superseded `completed_tasks` per-item, returning their IDs in `acknowledged_task_ids` without failing healthy tasks in the same batch.

3. **Content-Addressed Artifacts Only (`[NFR-PERF-001]`):**
   - `TaskGroup` forbids inline `wasm_bytecode` and requires a shared `TaskTemplate` with `artifact_sha256` pre-registered in `ArtifactStore` via `UploadArtifact`.
   - Trade-off: Operators use a two-step flow (`UploadArtifact` + `SubmitTaskGroup`, automated by `androgrid-cli group submit --wasm ...`), avoiding N-way bytecode duplication over gRPC.

4. **Deterministic Partition IDs and `TaskState::Cancelled`:**
   - Task IDs are formatted deterministically as `{group_id}:e{epoch}:p{index}` (`0..N-1`), validated with `group_id` matching `^[a-zA-Z0-9_-]{1,64}$` and `1 <= partitions.len() <= 1000`.
   - `TaskState` adds `TASK_STATE_CANCELLED = 8` (distinct from `PREEMPTED` used by `[INV-005]`). Standalone `SubmitTask` rejects caller-supplied `group_id`, `group_epoch`, or reserved `<group>:e<epoch>:p<index>` task IDs to prevent group injection and ID collisions.
   - `ReplaceTaskGroupRequest` uses `optional uint64 expected_epoch` (`None` = unconditional, `Some(v)` = strict CAS check returning `ABORTED` on mismatch) and requires explicit `partitions` (no implicit empty-partition reuse). Both `ReplaceTaskGroup` and `CancelTaskGroup` reject terminal groups (`Completed`, `Failed`, `Cancelled`) with `GROUP_TERMINAL`, and standalone `CancelTask` rejects group-owned partitions so group atomicity is preserved.

5. **Rollout Strategies:**
   - `REPLACE_STRATEGY_IMMEDIATE` (default): Cancels all non-terminal tasks in the prior epoch (`TASK_STATE_CANCELLED`), queues Wi-Fi `cancel_task_ids` (cleared once delivered in `SyncNodeStateResponse`), and enqueues new epoch tasks as `PENDING`.
   - `REPLACE_STRATEGY_DRAIN`: Cancels only `PENDING` tasks in the prior epoch, transitions group state to `DRAINING` while prior `LEASED`/`RUNNING` tasks finish within their existing lease deadlines, and enqueues new epoch tasks immediately as `PENDING` (concurrent drain).
   - `REPLACE_STRATEGY_CANARY` is deferred until `#44` (`NodeGroup`) provides node-pool targeting.
   - `cancel_task_ids` on Android is treated as a best-effort battery optimization (requires follow-up `androgrid-android` wiring); correctness relies strictly on control-plane epoch fencing.

---

## Consequences

### Positive
- Atomic workload version replacement with zero split-brain commits (`[NFR-REL-001]`, `[INV-004]`).
- Bounded gRPC request sizes (`[NFR-PERF-001]`) via shared `TaskTemplate` and `ArtifactStore`.
- Safe `expected_epoch` CAS semantics without proto3 zero-sentinel footguns.

### Drawbacks & Blast Radius
- Adding `TASK_STATE_CANCELLED = 8` requires updating exhaustive `TaskState` terminal checks in `src/scheduler/engine.rs`, `src/transport/error.rs`, and `src/bin/androgrid_cli.rs` (as well as `AndroGrid_SDD.md` Sections 4.1 and 6.3).
- In-memory state means groups do not survive an orchestrator process restart until whole-scheduler persistence is added.
- Concurrent `REPLACE_STRATEGY_DRAIN` allows temporary mixed-epoch execution while prior `LEASED` tasks drain.
- `Loom` permutation testing from `#43` DoD is replaced by `proptest` state-machine tests and `tokio::sync::Barrier` concurrency tests, since `SchedulerInner` uses a single `std::sync::RwLock` rather than lock-free atomics.

---

## Alternatives Considered
- **Independent Background Reconciliation Loop:** Rejected due to lock-ordering complexity and split-brain risk against `SchedulerInner`.
- **Per-Group Persistence Without Task Persistence:** Rejected because restoring a `TaskGroupRecord` without its `TaskRecord`s leaves orphaned or duplicated partitions on restart.
- **Canary Rollout Without `NodeGroup` (`#44`):** Rejected because percentage routing across an untargeted mix of Wi-Fi racks and metered cellular phones is non-deterministic.
