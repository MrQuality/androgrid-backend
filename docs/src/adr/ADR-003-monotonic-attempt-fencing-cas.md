# ADR-003: Monotonic Attempt Fencing and Lock-Free Compare-and-Swap (CAS) State Consensus

## Status
Accepted

## Architectural Scope & Requirements
- **Goal Reference:** G-15
- **Traceability:** `[INV-004]`, `[NFR-REL-001]`, `[REQ-OP-004]`
- **Standard:** ISO/IEC/IEEE 29148

---

## Context
In a distributed edge cluster with mobile worker nodes across cellular WAN and local Wi-Fi, network delays, temporary partitions, cellular packet loss, and worker preemptions frequently occur.

These distributed dynamics give rise to severe concurrency hazards:
1. **Out-of-Order Result Submissions:** A slow cellular worker may submit results after a task has already timed out or been reassigned to a faster Wi-Fi worker.
2. **Duplicate Commits:** Network retransmissions or parallel attempts could overwrite finalized results, corrupting downstream data pipelines.
3. **Lock Contention:** Centralized distributed locks (e.g., distributed mutexes via ZooKeeper or etcd) introduce significant latency overhead, contention bottlenecks, and deadlock vulnerabilities when workers disconnect unexpectedly.

---

## Decision
We implement **Monotonic Attempt Fencing** coupled with **Lock-Free Compare-and-Swap (CAS)** state transitions in the control plane:
1. **Monotonic Run Attempt Tracking:** Every scheduled task maintains an integer `run_attempt` counter. The counter is initialized to `1` on initial lease and monotonically incremented on each retry, reassignment, or speculative preemption.
2. **Fencing Verification (`[INV-004]`):** When a worker submits a result via `SubmitCellularResult` or `SyncNodeState`, the orchestrator verifies:
   - If `req.run_attempt < active_run_attempt`: The submission is immediately and atomically discarded, returning gRPC status `FAILED_PRECONDITION` with error code `TASK_ATTEMPT_STALE`.
   - If `req.run_attempt == active_run_attempt` and the task state is currently `TASK_STATE_LEASED`: The orchestrator performs an atomic CAS transition to `TASK_STATE_COMPLETED` and accepts the result.
   - If `req.run_attempt > active_run_attempt`: Rejected as an invalid protocol state.
3. **Lock-Free In-Memory Consensus:** The control plane orchestrator leverages atomic memory operations and concurrent state maps (`std::sync::RwLock` and atomic state updates) in Rust, eliminating blocking distributed locks.

### Architectural Rationale
- **Formal Invariant Guarantee:** Fencing tokens prevent the classic distributed systems "delayed message hazard", where a delayed message from an earlier attempt arrives after a newer attempt has started.
- **Zero Double-Commit (`[NFR-REL-001]`):** Exactly one attempt can ever transition a task into the finalized `TASK_STATE_COMPLETED` state.
- **High Concurrency Performance:** Lock-free state checks execute in microseconds in memory, enabling the control plane to process hundreds of concurrent submissions without thread starvation.

---

## Consequences

### Positive
- Strict mathematical guarantee against split-brain commits and stale result overwrites.
- Sub-millisecond state validation overhead in the Rust Tokio control plane.
- Clean and unambiguous error signaling (`TASK_ATTEMPT_STALE`) instructing mobile nodes to discard local execution state immediately.

### Trade-offs
- Slower workers whose results are preempted have their computations discarded, representing unavoidable wasted energy on speculative preemption.

---

## Alternatives Considered
- **Distributed Mutex / Pessimistic Locking:** Rejected due to excessive network round trips, single-point-of-failure risks, and deadlock vulnerabilities when mobile devices drop offline while holding a lock.
- **Last-Write-Wins (LWW) Timestamp Consensus:** Rejected because mobile client wall-clock times suffer from clock drift, allowing stale results from slow nodes to corrupt fresh output.
- **Two-Phase Commit (2PC):** Incompatible with offline, disconnected cellular execution models where workers close network channels during task computation.
