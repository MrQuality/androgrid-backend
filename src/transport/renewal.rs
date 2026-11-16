//! In-band mTLS certificate renewal manager with single-flight coalescing,
//! bounded-reissuance caching, and rolling per-node rate limiting.
//!
//! Traces to: AG-SPEC-2026-V2 Section 5.1, [INV-002], [NFR-SEC-001], [REQ-OP-005]

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use tokio::sync::{watch, Mutex};
use tonic::metadata::MetadataValue;
use tonic::Status;
use tracing::{info, warn};

use crate::pairing::signer::{CaSigner, SignerError};
use crate::transport::tls::AuthenticatedPeer;
use crate::RenewCertificateResponse;

/// Metadata header carrying the next eligible renewal timestamp (Unix epoch seconds).
pub const HEADER_NEXT_RENEWAL_EPOCH: &str = "x-androgrid-next-renewal-epoch";

/// Metadata header carrying the rate-limit retry-after timestamp (Unix epoch seconds).
pub const HEADER_RETRY_AFTER_EPOCH: &str = "x-androgrid-retry-after-epoch";

/// Policy configuration governing in-band mTLS certificate renewal.
///
/// Traces to: [INV-002], [NFR-SEC-001], [REQ-OP-005]
#[derive(Debug, Clone)]
pub struct CertRenewalPolicy {
    /// Whether in-band certificate renewal is enabled.
    pub allow_cert_renewal: bool,
    /// Server-side renewal window in days before certificate `notAfter`.
    pub renewal_window_days: u32,
    /// Minimum remaining CA lifetime required beyond `renewal_window_days`.
    pub min_ca_extension_days: u32,
    /// Validity duration in days for newly issued worker certificates.
    pub issued_validity_days: u32,
    /// TTL in seconds for the in-memory idempotent response cache.
    pub cache_ttl_secs: u64,
    /// Rolling window in seconds for per-node burst rate limiting.
    pub rate_limit_window_secs: u64,
    /// Maximum issuances permitted per node within `rate_limit_window_secs`.
    pub rate_limit_max_burst: usize,
    /// Maximum entries retained in the in-memory idempotency cache.
    pub max_cache_entries: usize,
}

impl Default for CertRenewalPolicy {
    fn default() -> Self {
        Self {
            allow_cert_renewal: true,
            renewal_window_days: 30,
            min_ca_extension_days: 7,
            issued_validity_days: 365,
            cache_ttl_secs: 600,
            rate_limit_window_secs: 3600,
            rate_limit_max_burst: 3,
            max_cache_entries: 10_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RenewalCacheKey {
    node_id: String,
    leaf_fingerprint_sha256: [u8; 32],
    csr_sha256: [u8; 32],
}

#[derive(Debug, Clone)]
struct CachedRenewalEntry {
    response: RenewCertificateResponse,
    expires_at_epoch_secs: u64,
}

type InFlightSender = watch::Sender<Option<Result<RenewCertificateResponse, Status>>>;
type InFlightMap = Arc<std::sync::Mutex<HashMap<RenewalCacheKey, InFlightSender>>>;

/// RAII guard ensuring an in-flight single-flight key is always removed on drop
/// (including task cancellation or panic).
struct InFlightGuard {
    map: InFlightMap,
    key: RenewalCacheKey,
    tx: InFlightSender,
    completed: bool,
}

impl InFlightGuard {
    fn finish(mut self, result: &Result<RenewCertificateResponse, Status>) {
        let _ = self.tx.send(Some(result.clone()));
        self.completed = true;
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.map.lock() {
            guard.remove(&self.key);
        }
        if !self.completed {
            let _ = self.tx.send(Some(Err(Status::cancelled(
                "Certificate renewal leader task was cancelled",
            ))));
        }
    }
}

/// Manages in-band worker certificate renewals over authenticated mTLS 1.3 channels.
///
/// Traces to: [INV-002], [NFR-SEC-001], [REQ-OP-005]
pub struct CertRenewalManager {
    signer: Option<Arc<CaSigner>>,
    policy: CertRenewalPolicy,
    denied_nodes: RwLock<HashSet<String>>,
    denied_fingerprints: RwLock<HashSet<[u8; 32]>>,
    cache: Mutex<HashMap<RenewalCacheKey, CachedRenewalEntry>>,
    in_flight: InFlightMap,
    rate_limits: Mutex<HashMap<String, VecDeque<u64>>>,
}

impl std::fmt::Debug for CertRenewalManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CertRenewalManager")
            .field("has_signer", &self.signer.is_some())
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

impl CertRenewalManager {
    /// Creates a new `CertRenewalManager` with an optional [`CaSigner`] and [`CertRenewalPolicy`].
    pub fn new(signer: Option<Arc<CaSigner>>, policy: CertRenewalPolicy) -> Self {
        Self {
            signer,
            policy,
            denied_nodes: RwLock::new(HashSet::new()),
            denied_fingerprints: RwLock::new(HashSet::new()),
            cache: Mutex::new(HashMap::new()),
            in_flight: Arc::new(std::sync::Mutex::new(HashMap::new())),
            rate_limits: Mutex::new(HashMap::new()),
        }
    }

    /// Adds a `node_id` to the renewal denylist (blocks future renewals for this node).
    pub fn deny_node(&self, node_id: impl Into<String>) {
        if let Ok(mut guard) = self.denied_nodes.write() {
            guard.insert(node_id.into());
        }
    }

    /// Adds a leaf certificate SHA-256 fingerprint to the renewal denylist.
    pub fn deny_fingerprint(&self, fingerprint: [u8; 32]) {
        if let Ok(mut guard) = self.denied_fingerprints.write() {
            guard.insert(fingerprint);
        }
    }

    fn now_epoch_secs() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
    }

    fn status_with_epoch_header(
        mut status: Status,
        header: &'static str,
        epoch_secs: i64,
    ) -> Status {
        if let Ok(val) = MetadataValue::try_from(epoch_secs.max(0).to_string()) {
            status.metadata_mut().insert(header, val);
        }
        status
    }

    /// Authorizes and processes a `RenewCertificate` request.
    ///
    /// Traces to: [INV-002], [NFR-SEC-001], [REQ-OP-005]
    pub async fn renew_certificate(
        &self,
        peer: Option<&AuthenticatedPeer>,
        csr_pem: &str,
    ) -> Result<RenewCertificateResponse, Status> {
        let now_secs = Self::now_epoch_secs();
        let peer = self.authorize_peer(peer, now_secs)?;
        let node_id = peer
            .common_name
            .as_deref()
            .expect("authorize_peer ensures non-empty CN");

        if !self.policy.allow_cert_renewal {
            return Err(Status::failed_precondition(
                "RENEWAL_UNAVAILABLE: in-band certificate renewal is disabled",
            ));
        }

        let signer = self.signer.as_ref().ok_or_else(|| {
            Status::failed_precondition(
                "RENEWAL_UNAVAILABLE: orchestrator CA signing key is not configured",
            )
        })?;

        let window_secs = i64::from(self.policy.renewal_window_days) * 86_400;
        let remaining_secs = peer.not_after_epoch_secs - now_secs;
        if remaining_secs > window_secs {
            let next_eligible = peer.not_after_epoch_secs - window_secs;
            return Err(Self::status_with_epoch_header(
                Status::failed_precondition(format!(
                    "OUTSIDE_RENEWAL_WINDOW: certificate has {remaining_secs}s remaining (window: {window_secs}s)"
                )),
                HEADER_NEXT_RENEWAL_EPOCH,
                next_eligible,
            ));
        }

        let trimmed_csr = csr_pem.trim();
        if trimmed_csr.is_empty() {
            return Err(Status::invalid_argument("CSR PEM must not be empty"));
        }

        let mut csr_hasher = Sha256::new();
        csr_hasher.update(trimmed_csr.as_bytes());
        let csr_sha256: [u8; 32] = csr_hasher.finalize().into();

        let cache_key = RenewalCacheKey {
            node_id: node_id.to_string(),
            leaf_fingerprint_sha256: peer.leaf_fingerprint_sha256,
            csr_sha256,
        };

        let now_u64 = now_secs.max(0) as u64;
        if let Some(cached) = self.lookup_cache(&cache_key, now_u64).await {
            return Ok(cached);
        }

        // Single-flight coalescing with RAII drop guard (cancellation/panic-safe)
        let (rx_opt, guard_opt) = {
            let mut in_flight = self
                .in_flight
                .lock()
                .map_err(|_| Status::internal("Renewal in-flight lock poisoned"))?;
            if let Some(tx) = in_flight.get(&cache_key) {
                (Some(tx.subscribe()), None)
            } else {
                let (tx, _) = watch::channel(None);
                in_flight.insert(cache_key.clone(), tx.clone());
                (
                    None,
                    Some(InFlightGuard {
                        map: Arc::clone(&self.in_flight),
                        key: cache_key.clone(),
                        tx,
                        completed: false,
                    }),
                )
            }
        };

        if let Some(mut rx) = rx_opt {
            loop {
                if let Some(res) = rx.borrow().clone() {
                    let check_now = Self::now_epoch_secs();
                    self.authorize_peer(Some(peer), check_now)?;
                    return res;
                }
                if rx.changed().await.is_err() {
                    return Err(Status::cancelled(
                        "Certificate renewal leader task terminated",
                    ));
                }
            }
        }

        let guard = guard_opt.expect("leader branch must hold InFlightGuard");
        let result = self
            .execute_signed_renewal(signer, peer, node_id, trimmed_csr, &cache_key)
            .await;
        guard.finish(&result);
        result
    }

    fn authorize_peer<'a>(
        &self,
        peer: Option<&'a AuthenticatedPeer>,
        now_secs: i64,
    ) -> Result<&'a AuthenticatedPeer, Status> {
        let peer = peer.ok_or_else(|| {
            Status::unauthenticated(
                "RenewCertificate strictly requires an authenticated mTLS client certificate [INV-002]",
            )
        })?;

        if peer.organizational_unit.as_deref() != Some("Edge Compute") {
            return Err(Status::permission_denied(
                "RenewCertificate requires a worker certificate with OU=Edge Compute",
            ));
        }

        let node_id = peer
            .common_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                Status::permission_denied(
                    "Worker mTLS certificate is missing Subject Common Name (CN)",
                )
            })?;

        if self
            .denied_nodes
            .read()
            .map(|g| g.contains(node_id))
            .unwrap_or(false)
            || self
                .denied_fingerprints
                .read()
                .map(|g| g.contains(&peer.leaf_fingerprint_sha256))
                .unwrap_or(false)
        {
            warn!(node_id = %node_id, "Rejected certificate renewal for denied worker [REQ-OP-005]");
            return Err(Status::permission_denied(
                "Worker certificate renewal has been denied",
            ));
        }

        if now_secs < peer.not_before_epoch_secs || now_secs >= peer.not_after_epoch_secs {
            return Err(Status::unauthenticated(
                "Client certificate is outside its validity period at issuance time",
            ));
        }

        Ok(peer)
    }

    async fn lookup_cache(
        &self,
        key: &RenewalCacheKey,
        now_u64: u64,
    ) -> Option<RenewCertificateResponse> {
        let mut cache = self.cache.lock().await;
        cache.retain(|_, v| v.expires_at_epoch_secs > now_u64);
        cache.get(key).map(|entry| entry.response.clone())
    }

    async fn execute_signed_renewal(
        &self,
        signer: &CaSigner,
        peer: &AuthenticatedPeer,
        node_id: &str,
        csr_pem: &str,
        cache_key: &RenewalCacheKey,
    ) -> Result<RenewCertificateResponse, Status> {
        let now_u64 = Self::now_epoch_secs().max(0) as u64;
        if let Some(cached) = self.lookup_cache(cache_key, now_u64).await {
            return Ok(cached);
        }

        // Enforce rolling per-node burst rate limit and prune expired entries
        {
            let mut limits = self.rate_limits.lock().await;
            let cutoff = now_u64.saturating_sub(self.policy.rate_limit_window_secs);
            limits.retain(|_, q| {
                while q.front().is_some_and(|&ts| ts <= cutoff) {
                    q.pop_front();
                }
                !q.is_empty()
            });
            let history = limits.get(node_id);
            if history.is_some_and(|q| q.len() >= self.policy.rate_limit_max_burst) {
                let history = history.unwrap();
                let oldest = history.front().copied().unwrap_or(now_u64);
                let retry_after = oldest.saturating_add(self.policy.rate_limit_window_secs);
                return Err(Self::status_with_epoch_header(
                    Status::resource_exhausted(
                        "RENEWAL_RATE_LIMITED: per-node certificate renewal burst limit exceeded",
                    ),
                    HEADER_RETRY_AFTER_EPOCH,
                    retry_after as i64,
                ));
            }
        }

        let min_ca_days = self
            .policy
            .renewal_window_days
            .saturating_add(self.policy.min_ca_extension_days);

        let (signed, not_after_epoch_seconds) = signer
            .sign_worker_renewal_csr(
                csr_pem,
                node_id,
                &peer.spki_der,
                self.policy.issued_validity_days,
                min_ca_days,
            )
            .map_err(map_signer_error)?;

        let response = RenewCertificateResponse {
            client_cert_pem: signed.client_cert_pem,
            ca_cert_pem: signed.ca_cert_pem,
            not_after_epoch_seconds,
        };

        {
            let mut limits = self.rate_limits.lock().await;
            limits
                .entry(node_id.to_string())
                .or_default()
                .push_back(now_u64);
        }

        {
            let mut cache = self.cache.lock().await;
            cache.retain(|_, v| v.expires_at_epoch_secs > now_u64);
            if self.policy.max_cache_entries > 0 && cache.len() >= self.policy.max_cache_entries {
                if let Some(oldest_key) = cache
                    .iter()
                    .min_by_key(|(_, v)| v.expires_at_epoch_secs)
                    .map(|(k, _)| k.clone())
                {
                    cache.remove(&oldest_key);
                }
            }
            if self.policy.max_cache_entries > 0 {
                cache.insert(
                    cache_key.clone(),
                    CachedRenewalEntry {
                        response: response.clone(),
                        expires_at_epoch_secs: now_u64.saturating_add(self.policy.cache_ttl_secs),
                    },
                );
            }
        }

        info!(
            node_id = %node_id,
            not_after = not_after_epoch_seconds,
            "Renewed worker mTLS client certificate in-band [INV-002], [REQ-OP-005]"
        );

        Ok(response)
    }
}

fn map_signer_error(err: SignerError) -> Status {
    match err {
        SignerError::CaLifetimeInsufficient { .. } => {
            Status::failed_precondition(format!("CA_LIFETIME_INSUFFICIENT: {err}"))
        }
        SignerError::InvalidCsr(_)
        | SignerError::CommonNameMismatch { .. }
        | SignerError::MissingCommonName
        | SignerError::UnsupportedKeyAlgorithm(_)
        | SignerError::KeyRotationRequired => Status::invalid_argument(err.to_string()),
        _ => Status::internal(err.to_string()),
    }
}
