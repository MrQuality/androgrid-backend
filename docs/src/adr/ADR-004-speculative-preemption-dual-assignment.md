# ADR-004: Speculative Preemption and Dual-Assignment for Cellular Spot Leases

## Status
Accepted

## Architectural Scope & Requirements
- **Goal Reference:** G-15
- **Traceability:** `[INV-005]`, `[ANTI-003]`, `[REQ-OP-003]`, `[INV-004]`
- **Standard:** ISO/IEC/IEEE 29148

---

## Context
Worker nodes operating on cellular networks are volatile, opportunistically scavenged resources. Mobile devices may drop coverage, be placed into pockets, switch cells, receive voice calls, or experience OS background throttling at any moment.

This volatility presents specific scheduling dilemmas:
1. **Straggler Delays:** If a batch job waits indefinitely for a sluggish cellular device, overall cluster pipeline throughput degrades drastically.
2. **Disconnected Execution Boundary (`[INV-003]`):** Cellular devices tear down sockets during execution to allow radio sleep. The control plane cannot push an interrupt or cancellation command to a cellular worker node.
3. **Bandwidth and Duration Limits:** Long compute jobs on cellular drain data plans and battery charge.

---

## Decision
We architect **Speculative Preemption and Dual-Assignment** for cellular spot workloads:
1. **Spot Lease Constraints (`[ANTI-003]`):**
   - Cellular tasks are designated as short-lived speculative spot leases.
   - Tasks requiring unmetered connections (`requires_unmetered == true`) or tasks with timeouts exceeding 180 seconds are strictly refused scheduling on cellular nodes.
   - Cellular spot lease deadlines are capped at a maximum of 180 seconds.
2. **Opportunistic Speculative Preemption (`[INV-005]`):**
   - If an unmetered Wi-Fi worker node becomes available while a cellular spot task is still running, the orchestrator speculatively dual-assigns the identical task to the Wi-Fi node.
   - The orchestrator monotonically increments the task's attempt counter: `run_attempt = run_attempt + 1`.
   - Both worker nodes execute the computation concurrently in a race condition.
3. **First-to-Commit Resolution:**
   - The first worker to submit a valid result claims the task and triggers the atomic CAS transition to `TASK_STATE_COMPLETED`.
   - The later submission from the other worker is atomically discarded via monotonic attempt fencing (`[INV-004]`).

### Architectural Rationale
- **Asynchronous Ingress Compatibility:** Because disconnected mobile nodes cannot receive inbound control commands (`[INV-001]`), speculative preemption decouples task reassignment from worker cancellation.
- **Tail Latency Reduction:** Fast Wi-Fi workers dramatically compress long tail execution times caused by throttled cellular workers.
- **Zero Double-Spend:** Monotonic attempt fencing mathematically prevents both workers from committing conflicting results.

---

## Consequences

### Positive
- Cluster tail latency is bounded and straggler tasks are prevented from blocking pipelines.
- Efficient scavenging of opportunistic cellular compute without sacrificing cluster reliability.
- Zero inbound network signaling required to manage preemptions.

### Trade-offs
- Redundant compute occurs when both workers complete execution before one commits, consuming extra battery on the slower device.

---

## Alternatives Considered
- **Strict Pre-cancellation Signaling:** Rejected because cellular workers operate offline with severed sockets (`[INV-003]`) and cannot be contacted via inbound RPCs (`[INV-001]`).
- **Pessimistic Sequential Failover (Wait for Timeout):** Rejected because waiting for the full 180-second cellular timeout before reassigning stalls batch workflows unnecessarily.
- **Total Exclusion of Cellular Compute:** Rejected because it discards valuable opportunistic compute capacity in field deployments.
