//! Tonic gRPC service implementation for AndroGridService.
//!
//! Exposes both Wi-Fi declarative synchronization and cellular disconnected unary RPCs.
//! Traces to: AG-SPEC-2026-V2 Section 4.1, Section 5.1, [REQ-OP-001], [REQ-OP-002], [REQ-OP-003], [REQ-OP-004]

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tonic::{Request, Response, Status};

use crate::artifact::store::ArtifactStore;
use crate::proto::androgrid::v1::andro_grid_service_server::AndroGridService;
use crate::scheduler::ClusterScheduler;
use crate::transport::renewal::{CertRenewalManager, CertRenewalPolicy};
use crate::transport::tls::AuthenticatedPeer;
use crate::validate_frame_bound;
use crate::{
    ClaimCellularTaskRequest, ClaimCellularTaskResponse, FetchArtifactRequest,
    FetchArtifactResponse, RegisterNodeRequest, RegisterNodeResponse, RenewCertificateRequest,
    RenewCertificateResponse, SubmitCellularResultRequest, SubmitCellularResultResponse,
    SyncNodeStateRequest, SyncNodeStateResponse,
};

/// Production implementation of AndroGridService backed by the cluster scheduler.
#[derive(Debug, Clone)]
pub struct AndroGridServiceImpl {
    scheduler: Arc<ClusterScheduler>,
    artifact_store: Arc<ArtifactStore>,
    renewal_manager: Arc<CertRenewalManager>,
}

impl AndroGridServiceImpl {
    /// Creates a new service instance bound to the provided scheduler.
    #[deprecated(note = "Use with_artifact_store to share the orchestrator ArtifactStore")]
    pub fn new(scheduler: Arc<ClusterScheduler>) -> Self {
        Self {
            scheduler,
            artifact_store: Arc::new(ArtifactStore::new()),
            renewal_manager: Arc::new(CertRenewalManager::new(None, CertRenewalPolicy::default())),
        }
    }

    /// Creates a new service instance sharing the orchestrator's [`ArtifactStore`].
    pub fn with_artifact_store(
        scheduler: Arc<ClusterScheduler>,
        artifact_store: Arc<ArtifactStore>,
    ) -> Self {
        Self {
            scheduler,
            artifact_store,
            renewal_manager: Arc::new(CertRenewalManager::new(None, CertRenewalPolicy::default())),
        }
    }

    /// Creates a new service instance with shared [`ArtifactStore`] and [`CertRenewalManager`].
    ///
    /// Traces to: [INV-002], [NFR-SEC-001], [REQ-OP-005]
    pub fn with_services(
        scheduler: Arc<ClusterScheduler>,
        artifact_store: Arc<ArtifactStore>,
        renewal_manager: Arc<CertRenewalManager>,
    ) -> Self {
        Self {
            scheduler,
            artifact_store,
            renewal_manager,
        }
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

    /// Enforces that when mTLS client identity metadata is attached, the leaf
    /// certificate's Subject Common Name matches the claimed `node_id`.
    fn verify_peer_identity<T>(request: &Request<T>, claimed_node_id: &str) -> Result<(), Status> {
        if let Some(peer) = request.extensions().get::<AuthenticatedPeer>() {
            match peer.common_name.as_deref() {
                Some(cn) if cn == claimed_node_id => Ok(()),
                Some(cn) => Err(Status::permission_denied(format!(
                    "mTLS identity mismatch: certificate CN '{cn}' cannot act as node '{claimed_node_id}'"
                ))),
                None => Err(Status::permission_denied(
                    "mTLS client certificate is missing Subject Common Name (CN)",
                )),
            }
        } else {
            Ok(())
        }
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
        let req_node_id = request.get_ref().node_id.clone();
        Self::verify_peer_identity(&request, &req_node_id)?;
        let req = request.into_inner();
        let capabilities = req
            .capabilities
            .ok_or_else(|| Status::invalid_argument("NodeCapabilities must be provided"))?;

        let now_ms = Self::now_epoch_ms();
        let response = self.scheduler.register_node_with_labels(
            req.node_id,
            capabilities,
            req.client_version,
            req.labels,
            now_ms,
        )?;

        Ok(Response::new(response))
    }

    /// Declarative Wi-Fi state synchronization.
    ///
    /// Traces to: [REQ-OP-002], [INV-005]
    async fn sync_node_state(
        &self,
        request: Request<SyncNodeStateRequest>,
    ) -> Result<Response<SyncNodeStateResponse>, Status> {
        Self::verify_peer_identity(&request, &request.get_ref().node_id)?;
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
        Self::verify_peer_identity(&request, &request.get_ref().node_id)?;
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
        Self::verify_peer_identity(&request, &request.get_ref().node_id)?;
        let req = request.into_inner();

        // Enforce frame payload bounds [NFR-PERF-001] on execution artifacts
        let total_artifact_len = req.stdout_bytes.len() + req.stderr_bytes.len();
        validate_frame_bound(total_artifact_len)?;

        let response = self.scheduler.submit_cellular_result(&req)?;

        Ok(Response::new(response))
    }

    /// Content-addressed WASM artifact retrieval over the authenticated mTLS 1.3 channel.
    ///
    /// Traces to: [INV-002], [NFR-PERF-001], [NFR-POW-001], [REQ-OP-003]
    async fn fetch_artifact(
        &self,
        request: Request<FetchArtifactRequest>,
    ) -> Result<Response<FetchArtifactResponse>, Status> {
        let req_node_id = request.get_ref().node_id.trim().to_string();
        if req_node_id.is_empty() {
            return Err(Status::invalid_argument(
                "node_id must not be empty in FetchArtifactRequest",
            ));
        }
        Self::verify_peer_identity(&request, &req_node_id)?;

        let req = request.into_inner();
        let clean_sha = req.artifact_sha256.trim().to_ascii_lowercase();
        if clean_sha.len() != 64 || !clean_sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Status::invalid_argument(
                "artifact_sha256 must be a 64-character hexadecimal SHA-256 digest",
            ));
        }

        let wasm_bytecode = self
            .artifact_store
            .get_artifact(&clean_sha)
            .ok_or_else(|| {
                Status::not_found(format!(
                    "Artifact '{clean_sha}' not found in orchestrator store"
                ))
            })?;

        let response = FetchArtifactResponse {
            wasm_bytecode,
            artifact_sha256: clean_sha,
        };
        let encoded_len = prost::Message::encoded_len(&response);
        if encoded_len > crate::MAX_FRAME_SIZE_BYTES {
            return Err(Status::resource_exhausted(format!(
                "FetchArtifactResponse ({encoded_len} bytes) exceeds {} byte frame limit [NFR-PERF-001]",
                crate::MAX_FRAME_SIZE_BYTES
            )));
        }

        Ok(Response::new(response))
    }

    /// Renews an expiring worker mTLS client certificate in-band over TLS 1.3 mTLS.
    ///
    /// Traces to: [INV-002], [NFR-SEC-001], [REQ-OP-005]
    async fn renew_certificate(
        &self,
        request: Request<RenewCertificateRequest>,
    ) -> Result<Response<RenewCertificateResponse>, Status> {
        let peer = request.extensions().get::<AuthenticatedPeer>().cloned();
        let req = request.into_inner();
        let response = self
            .renewal_manager
            .renew_certificate(peer.as_ref(), &req.csr_pem)
            .await?;
        Ok(Response::new(response))
    }
}
