#![allow(clippy::result_large_err)]
//! AndroGrid Control Plane Core Library
//!
//! Exposes generated Protocol Buffer models, Tonic service definitions,
//! and protocol validation primitives.
//!
//! Traces to: AG-SPEC-2026-V2, [REQ-OP-001], [REQ-OP-002], [REQ-OP-003], [REQ-OP-004], [NFR-PERF-001]

pub mod proto {
    pub mod androgrid {
        pub mod v1 {
            tonic::include_proto!("androgrid.v1");
        }
    }
}

pub use proto::androgrid::v1::*;

pub mod scheduler;
pub use scheduler::*;

pub mod artifact;
pub use artifact::*;

pub mod transport;
pub use transport::*;

pub mod config;
pub use config::*;

pub mod pairing;
pub use pairing::{
    export_png, render_ansi_qr, CaSigner, EnrollmentContext, EnrollmentServiceImpl,
    HttpEnrollRequest, HttpEnrollResponse, PairingError, PairingPayload, PairingTokenRecord,
    PairingTokenStore, SignedWorkerCert, SignerError, DEFAULT_MAX_ATTEMPTS,
    DEFAULT_PAIRING_TOKEN_TTL_SECONDS,
};

use thiserror::Error;

/// Maximum allowable gRPC message frame payload bound in bytes (16 MB).
///
/// Traces to: [NFR-PERF-001]
pub const MAX_FRAME_SIZE_BYTES: usize = 16 * 1024 * 1024;

/// Errors associated with message frame sizing and validation boundaries.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum FrameBoundError {
    #[error("Message frame payload of {size} bytes exceeds maximum bound of {max} bytes")]
    ExceedsMaxBound { size: usize, max: usize },
}

/// Verifies that an encoded payload respects the maximum frame size constraint.
///
/// Traces to: [NFR-PERF-001]
///
/// # Arguments
/// * `payload_len` - The serialized length of the gRPC message in bytes.
///
/// # Returns
/// * `Ok(())` if the size is within the 16 MB boundary.
/// * `Err(FrameBoundError::ExceedsMaxBound)` if the size exceeds 16 MB.
pub fn validate_frame_bound(payload_len: usize) -> Result<(), FrameBoundError> {
    if payload_len > MAX_FRAME_SIZE_BYTES {
        Err(FrameBoundError::ExceedsMaxBound {
            size: payload_len,
            max: MAX_FRAME_SIZE_BYTES,
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn test_node_capabilities_default_construction() {
        let caps = NodeCapabilities {
            is_rooted: false,
            supported_engines: vec![ExecutionEngine::WasmWasi as i32],
            cpu_cores: 8,
            total_ram_bytes: 8 * 1024 * 1024 * 1024,
            os_version: "14".to_string(),
            abi: "arm64-v8a".to_string(),
            is_metered: true,
            current_transport: NetworkTransportType::NetworkTransportCellularWan as i32,
            max_cellular_task_duration_seconds: 180,
            battery_temperature_celsius: 28,
            thermal_status: 0,
        };

        assert_eq!(caps.cpu_cores, 8);
        assert!(caps.is_metered);
        assert_eq!(caps.max_cellular_task_duration_seconds, 180);
        assert_eq!(caps.battery_temperature_celsius, 28);
        assert_eq!(caps.thermal_status, 0);
    }

    #[test]
    fn test_claim_cellular_task_request_round_trip() {
        let req = ClaimCellularTaskRequest {
            node_id: "worker-node-alpha".to_string(),
            capabilities: Some(NodeCapabilities {
                is_rooted: false,
                supported_engines: vec![ExecutionEngine::WasmWasi as i32],
                cpu_cores: 4,
                total_ram_bytes: 4 * 1024 * 1024 * 1024,
                os_version: "14".to_string(),
                abi: "arm64-v8a".to_string(),
                is_metered: true,
                current_transport: NetworkTransportType::NetworkTransportCellularWan as i32,
                max_cellular_task_duration_seconds: 180,
                battery_temperature_celsius: 31,
                thermal_status: 1,
            }),
            battery_level_pct: 85,
            is_charging: false,
            battery_temperature_celsius: 31,
            thermal_status: 1,
        };

        let mut buf = Vec::new();
        req.encode(&mut buf).expect("Encoding should succeed");

        assert!(validate_frame_bound(buf.len()).is_ok());

        let decoded = ClaimCellularTaskRequest::decode(&buf[..]).expect("Decoding should succeed");

        assert_eq!(decoded.node_id, "worker-node-alpha");
        assert_eq!(decoded.battery_level_pct, 85);
        assert_eq!(decoded.battery_temperature_celsius, 31);
        assert_eq!(decoded.thermal_status, 1);
        let decoded_caps = decoded.capabilities.expect("Capabilities must be present");
        assert_eq!(decoded_caps.abi, "arm64-v8a");
        assert_eq!(decoded_caps.os_version, "14");
        assert_eq!(decoded_caps.battery_temperature_celsius, 31);
        assert_eq!(decoded_caps.thermal_status, 1);
    }

    #[test]
    fn test_frame_bound_enforcement_boundary() {
        assert!(validate_frame_bound(1024).is_ok());
        assert!(validate_frame_bound(MAX_FRAME_SIZE_BYTES).is_ok());
        assert_eq!(
            validate_frame_bound(MAX_FRAME_SIZE_BYTES + 1),
            Err(FrameBoundError::ExceedsMaxBound {
                size: MAX_FRAME_SIZE_BYTES + 1,
                max: MAX_FRAME_SIZE_BYTES
            })
        );
    }
}
