//! Mapping between domain scheduling errors and canonical gRPC Status codes.
//!
//! Traces to: AG-SPEC-2026-V2 Section 6.3, [REQ-OP-003], [REQ-OP-004], [INV-004], [NFR-PERF-001]

use crate::scheduler::errors::SchedulingError;
use crate::FrameBoundError;
use tonic::Status;

/// Converts a domain SchedulingError into a canonical gRPC Status.
///
/// Traces to: Section 6.3 (Canonical gRPC Error Matrix)
impl From<SchedulingError> for Status {
    fn from(err: SchedulingError) -> Self {
        match err {
            // Section 6.3: FAILED_PRECONDITION (9) with code TASK_ATTEMPT_STALE
            SchedulingError::AttemptStale {
                task_id,
                submitted,
                active,
            } => Status::failed_precondition(format!(
                "TASK_ATTEMPT_STALE: submitted attempt {} for task '{}' is lower than active attempt {}",
                submitted, task_id, active
            )),

            // Section 6.3: FAILED_PRECONDITION (9) with code UNMETERED_REQUIRED
            SchedulingError::UnmeteredNetworkRequired(task_id) => Status::failed_precondition(format!(
                "UNMETERED_REQUIRED: task '{}' requires unmetered Wi-Fi connection",
                task_id
            )),

            SchedulingError::AlreadyFinalized(task_id) => Status::failed_precondition(format!(
                "TASK_ATTEMPT_STALE: task '{}' has already been committed and finalized",
                task_id
            )),

            SchedulingError::CellularDurationExceeded {
                task_id,
                duration_seconds,
                max_allowed,
            } => Status::invalid_argument(format!(
                "CELLULAR_DURATION_EXCEEDED: task '{}' timeout {}s exceeds max spot cap of {}s",
                task_id, duration_seconds, max_allowed
            )),

            // Traces to: [INV-006]
            SchedulingError::RootShellForbiddenForUnrootedNode { task_id, node_id } => {
                Status::permission_denied(format!(
                    "ROOT_SHELL_FORBIDDEN: node '{}' is unrooted and cannot execute root shell task '{}'",
                    node_id, task_id
                ))
            }

            SchedulingError::NodeNotFound(node_id) => {
                Status::not_found(format!("NODE_NOT_FOUND: worker node '{}' is not registered", node_id))
            }

            SchedulingError::TaskNotFound(task_id) => {
                Status::not_found(format!("TASK_NOT_FOUND: task '{}' does not exist in cluster pool", task_id))
            }

            // Section 6.3: UNAUTHENTICATED (16) with code CERTIFICATE_INVALID
            SchedulingError::InvalidSessionToken { node_id, .. } => Status::unauthenticated(format!(
                "CERTIFICATE_INVALID: invalid session token for node '{}'",
                node_id
            )),

            SchedulingError::InsufficientBattery {
                node_id,
                battery_pct,
            } => Status::failed_precondition(format!(
                "INSUFFICIENT_BATTERY: node '{}' battery level {}% is below minimum threshold and not charging",
                node_id, battery_pct
            )),

            SchedulingError::InvalidStateTransition { task_id, from, to } => {
                Status::failed_precondition(format!(
                    "INVALID_STATE_TRANSITION: task '{}' cannot transition from {} to {}",
                    task_id, from, to
                ))
            }

            // Traces to: [NFR-POW-001], [REQ-OP-003]
            SchedulingError::ThermalThrottled {
                node_id,
                temperature_celsius,
                thermal_status,
            } => Status::resource_exhausted(format!(
                "THERMAL_THROTTLED: node '{}' is thermally throttled (temp: {}C, status: {})",
                node_id, temperature_celsius, thermal_status
            )),

            // Traces to: [INV-006]
            SchedulingError::RootShellExecutionDisabled => Status::permission_denied(
                "ROOT_SHELL_DISABLED: root shell execution is disabled; start orchestrator with --allow-insecure-root-tasks to enable",
            ),
        }
    }
}

/// Converts a FrameBoundError into a canonical gRPC Status.
///
/// Traces to: Section 6.3 (INVALID_ARGUMENT / PAYLOAD_CORRUPT), [NFR-PERF-001]
impl From<FrameBoundError> for Status {
    fn from(err: FrameBoundError) -> Self {
        match err {
            FrameBoundError::ExceedsMaxBound { size, max } => Status::invalid_argument(format!(
                "PAYLOAD_CORRUPT: message payload of {} bytes exceeds absolute 16 MB bound ({} bytes)",
                size, max
            )),
        }
    }
}
