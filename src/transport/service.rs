//! Tonic gRPC service implementation for AndroGridService.
//!
//! Exposes both Wi-Fi declarative synchronization and cellular disconnected unary RPCs.
//! Traces to: AG-SPEC-2026-V2 Section 4.1, Section 5.1, [REQ-OP-001], [REQ-OP-002], [REQ-OP-003], [REQ-OP-004]

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tonic::{Request, Response, Status};

use crate::proto::androgrid::v1::andro_grid_service_server::AndroGridService;
use crate::scheduler::ClusterScheduler;
use crate::validate_frame_bound;
use crate::{
    ClaimCellularTaskRequest, ClaimCellularTaskResponse, RegisterNodeRequest, RegisterNodeResponse,
    SubmitCellularResultRequest, SubmitCellularResultResponse, SyncNodeStateRequest,
    SyncNodeStateResponse,
};

/// Production implementation of AndroGridService backed by the cluster scheduler.
#[derive(Debug, Clone)]
pub struct AndroGridServiceImpl {
    scheduler: Arc<ClusterScheduler>,
}

impl AndroGridServiceImpl {
    /// Creates a new service instance bound to the provided scheduler.
    pub fn new(scheduler: Arc<ClusterScheduler>) -> Self {
        Self { scheduler }
    }

    /// Helper to get current epoch timestamp in seconds.
    fn now_epoch_s() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    /// Helper to get current epoch timestamp in milliseconds.
    fn now_epoch_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}

#[tonic::async_trait]
impl AndroGridService for AndroGridServiceImpl {
    /// Initial node registration over local Wi-Fi.
    ///
    /// Traces to: [REQ-OP-001], [INV-006], [REQ-VER-002]
    async fn register_node(
        &self,
        request: Request<RegisterNodeRequest>,
    ) -> Result<Response<RegisterNodeResponse>, Status> {
        let req = request.into_inner();
        let capabilities = req
            .capabilities
            .ok_or_else(|| Status::invalid_argument("NodeCapabilities must be provided"))?;

        let now_ms = Self::now_epoch_ms();
        let response =
            self.scheduler
                .register_node(req.node_id, capabilities, req.client_version, now_ms)?;

        Ok(Response::new(response))
    }

    /// Declarative Wi-Fi state synchronization.
    ///
    /// Traces to: [REQ-OP-002], [INV-005]
    async fn sync_node_state(
        &self,
        request: Request<SyncNodeStateRequest>,
    ) -> Result<Response<SyncNodeStateResponse>, Status> {
        let req = request.into_inner();
        let now_ms = Self::now_epoch_ms();

        let response = self.scheduler.sync_node_state(
            &req.node_id,
            &req.session_token,
            &req.completed_tasks,
            now_ms,
        )?;

        Ok(Response::new(response))
    }

    /// Disconnected unary task claim over cellular WAN.
    ///
    /// Traces to: [REQ-OP-003], [INV-003], [INV-005], [ANTI-003], [NFR-POW-001]
    async fn claim_cellular_task(
        &self,
        request: Request<ClaimCellularTaskRequest>,
    ) -> Result<Response<ClaimCellularTaskResponse>, Status> {
        let req = request.into_inner();
        let now_s = Self::now_epoch_s();

        let response = self.scheduler.claim_cellular_task(
            &req.node_id,
            req.battery_level_pct,
            req.is_charging,
            req.battery_temperature_celsius,
            req.thermal_status,
            now_s,
        )?;

        Ok(Response::new(response))
    }

    /// Atomic result submission with monotonic attempt fencing.
    ///
    /// Traces to: [REQ-OP-004], [INV-004], [NFR-REL-001], [NFR-PERF-001]
    async fn submit_cellular_result(
        &self,
        request: Request<SubmitCellularResultRequest>,
    ) -> Result<Response<SubmitCellularResultResponse>, Status> {
        let req = request.into_inner();

        // Enforce frame payload bounds [NFR-PERF-001] on execution artifacts
        let total_artifact_len = req.stdout_bytes.len() + req.stderr_bytes.len();
        validate_frame_bound(total_artifact_len)?;

        let response = self.scheduler.submit_cellular_result(&req)?;

        Ok(Response::new(response))
    }
}
