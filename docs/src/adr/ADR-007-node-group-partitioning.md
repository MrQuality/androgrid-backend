# ADR-007: NodeGroup Partitioning and Targeted Fleet Operations

## Status
Accepted

## Architectural Scope & Requirements
- **Issue Reference:** `#44` (`[RFC] NodeGroup Partitioning & Targeted Fleet Operations`)
- **Traceability:** `[REQ-OP-001]`, `[REQ-OP-002]`, `[REQ-OP-003]`, `[REQ-OP-004]`, `[INV-004]`, `[INV-005]`, `[INV-006]`, `[ANTI-003]`, `[NFR-PERF-001]`, `[NFR-REL-001]`, `[REQ-VER-002]`
- **Standard:** ISO/IEC/IEEE 29148

---

## Context
Worker nodes register individually with hardware capabilities (`is_rooted`, `cpu_cores`, `total_ram_bytes`, `os_version`, `abi`, `current_transport`), but the control plane lacks any logical fleet partitioning abstraction. Operators can only target the entire cluster or individual task IDs, and tasks cannot constrain scheduling to specific hardware tiers or deployment pools (such as rooted rack rigs vs. cellular test devices). Furthermore, `ADR-006` deferred `TaskGroup` canary pool targeting until `NodeGroup` (`#44`) established node-pool primitives.

At the same time, `ClusterScheduler` manages `tasks`, `workers`, and `groups` inside a single `Arc<RwLock<SchedulerInner>>` (`ADR-003`, `ADR-006`). Introducing a separate lock or external registry for node groups would create lock-ordering hazards and split-brain state between node assignment, node draining, and task scheduling.

---

## Decision
We implement a synchronous `NodeGroup` registry, node lifecycle controls (`DrainNodes`, `UpgradeNodes`), and workload affinity matching directly inside `SchedulerInner`, composing with `ADR-005` (`AdminService` mTLS `OU=Control Plane` allowlist) and `ADR-006` (`TaskTemplate` / `TaskGroup`):

1. **In-Memory Single-Lock Coordination (`[NFR-REL-001]`):**
   - `NodeGroupRecord` lives inside `SchedulerInner` alongside `workers`, `tasks`, and `groups`, so `CreateNodeGroup`, `UpdateNodeGroup`, `AssignNodesToGroup`, `DrainNodes`, `UpgradeNodes`, `RegisterNode`, `SyncNodeState`, and `ClaimCellularTask` execute atomically under one `RwLock`.

2. **Hybrid Explicit Partitioning + Dynamic Label Selector:**
   - Workers may report user-defined `labels` in `RegisterNodeRequest` (up to `MAX_GROUP_LABELS = 32` entries; keys matching `^[a-zA-Z0-9_./-]{1,128}$`, values `1..=128` printable non-whitespace ASCII).
   - Worker-supplied keys under the reserved `androgrid.io/` prefix are rejected with `INVALID_ARGUMENT`. The control plane synthesizes authoritative `androgrid.io/rooted`, `androgrid.io/transport`, `androgrid.io/abi`, and `androgrid.io/os` labels from verified capabilities (`[REQ-VER-002]`, `[INV-006]`).
   - Each worker has at most one `explicit_node_group` (1:N partitioning). Assigning a node via `AssignNodesToGroup` atomically removes it from any prior group's `member_node_ids` and pins it to the target group. Pre-registration assignment is supported for not-yet-connected `node_id`s.
   - Unpinned workers (`explicit_node_group == None`) dynamically match any `NodeGroup` whose non-empty `label_selector` is a subset of the worker's effective labels.

3. **Control-Plane State Preservation Across Reconnects:**
   - Re-calling `RegisterNode` for an existing `node_id` updates capabilities, session token, reported labels, and `client_version`, while preserving `explicit_node_group`, `draining`, `active_task_ids`, and `desired_client_version` (clearing `desired_client_version` once `client_version` matches the target).
   - `ClaimCellularTask` updates `current_transport` to `NETWORK_TRANSPORT_CELLULAR_WAN` and `is_metered = true` before evaluating `NodeGroup` membership so `androgrid.io/transport=cellular` is accurate for dual-route devices.

4. **Drain, Upgrade Signaling, and Affinity Enforcement (`[INV-005]`, `[INV-006]`, `[ANTI-003]`):**
   - `DrainNodes` sets `draining = true` on targeted nodes (by `node_group` or `node_ids`, mutually exclusive). Draining nodes receive zero new `PENDING` leases and are excluded from `[INV-005]` speculative preemption, while in-flight tasks may still commit results.
   - `UpgradeNodes` sets `desired_client_version` (and optional pre-drain) on targeted registered nodes not already on `target_version`, surfacing `desired_client_version` in Wi-Fi `SyncNodeStateResponse` only (excluding metered cellular `[INV-003]`). Nodes drained specifically for an upgrade automatically clear `draining` when they reconnect with `client_version == target_version` or when a rollback `UpgradeNodes` targets their current `client_version`, while manual `DrainNodes` remains preserved across reconnects.
   - `TaskSpec` and `TaskTemplate` add `node_group_affinity`. Non-empty affinity is validated against `inner.node_groups` at submission/replacement time (`NOT_FOUND` on unknown group) and enforced in both `claim_cellular_task` and `sync_node_state` (including `[INV-005]` speculative preemption).
   - Security and transport invariants (`[INV-005]`, `[ANTI-003]` 180s/unmetered cap, `[INV-006]` unrooted `RootShell` prohibition) remain strictly enforced regardless of `NodeGroup` membership or affinity.

---

## Consequences

### Positive
- Operators can partition fleets by explicit rack/pool assignment or hardware/topology selectors and execute batch `drain`, `upgrade-nodes`, and affine task scheduling.
- Unknown `node_group_affinity` targets fail fast at submission instead of black-holing tasks in `PENDING`.
- Single-lock coordination preserves linearizable CAS semantics across `SchedulerInner`.

### Drawbacks & Blast Radius
- In-memory `NodeGroupRecord` state does not survive orchestrator restarts until whole-scheduler persistence is introduced (`ADR-006`).
- Adding `labels`, `node_group`, `draining`, and `desired_client_version` requires updating `orchestrator.proto`, `SchedulerInner`, `AdminServiceImpl`, `AndroGridServiceImpl`, and `androgrid-cli`.
- `REPLACE_STRATEGY_CANARY` and `DeleteNodeGroup` remain deferred to follow-up work.

---

## Alternatives Considered
- **Separate `NodeGroupRegistry` Lock:** Rejected due to lock-ordering complexity and race windows between group reassignment and task scheduling.
- **Many-to-Many Explicit Group Membership:** Rejected because fleet partitioning (`rack-01`, `rack-02`, `beta-devices`) benefits from 1:N ownership, while overlapping views remain supported via `label_selector`.
- **Silent Overwrite of Reserved `androgrid.io/` Worker Labels:** Rejected in favor of explicit `INVALID_ARGUMENT` rejection so misconfigured workers receive immediate feedback.
