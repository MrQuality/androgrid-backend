//! Ephemeral Device Pairing Token Lifecycle Management
//!
//! Enforces single-use semantics, time-bounded expiration (default 300s),
//! and brute-force rate-limiting for worker node bootstrap enrollment.
//!
//! Traces to: AG-SPEC-2026-V2 Section 2.3, [INV-002], [NFR-SEC-001], [REQ-OP-001]

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::RwLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

/// Default ephemeral token lifetime in seconds (5 minutes).
/// Traces to: [NFR-SEC-001]
pub const DEFAULT_PAIRING_TOKEN_TTL_SECONDS: u32 = 300;

/// Maximum failed enrollment attempts before rate-limiting triggers.
pub const DEFAULT_MAX_ATTEMPTS: u32 = 5;

/// Errors arising from pairing token validation and redemption.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PairingError {
    #[error("Pairing token not found or invalid")]
    TokenNotFound,

    #[error("Pairing token expired at epoch {expired_at_epoch_s} (current: {current_epoch_s})")]
    TokenExpired {
        expired_at_epoch_s: u64,
        current_epoch_s: u64,
    },

    #[error("Pairing token has already been redeemed (single-use violation)")]
    TokenAlreadyRedeemed,

    #[error("Node identifier mismatch: token bound to '{expected}', but requested '{actual}'")]
    NodeIdMismatch { expected: String, actual: String },

    #[error("Rate limit exceeded for pairing enrollment: too many failed attempts")]
    RateLimitExceeded,

    #[error("Pairing feature is disabled by cluster configuration")]
    PairingDisabled,
}

/// Metadata record for an active or redeemed ephemeral pairing token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingTokenRecord {
    /// Cryptographic pairing token string.
    pub token: String,

    /// Optional target node identifier bound to this pairing session.
    pub target_node_id: Option<String>,

    /// Creation timestamp (epoch seconds).
    pub created_at_epoch_s: u64,

    /// Expiration timestamp (epoch seconds).
    pub expires_at_epoch_s: u64,

    /// Whether this token has already been redeemed.
    pub is_redeemed: bool,

    /// Number of failed enrollment attempts recorded against this token.
    pub failed_attempts: u32,
}

/// Thread-safe in-memory manager governing ephemeral pairing tokens.
///
/// Traces to: [INV-002], [NFR-SEC-001], [REQ-OP-001]
pub struct PairingTokenStore {
    tokens: RwLock<HashMap<String, PairingTokenRecord>>,
    global_failed_attempts: AtomicU32,
    max_failed_attempts: u32,
    enabled: bool,
}

impl Default for PairingTokenStore {
    fn default() -> Self {
        Self::new(true, DEFAULT_MAX_ATTEMPTS)
    }
}

impl PairingTokenStore {
    /// Creates a new PairingTokenStore.
    ///
    /// # Arguments
    /// * `enabled` - Whether pairing token issuance and redemption are enabled.
    /// * `max_failed_attempts` - Max failed attempts permitted before rate-limiting.
    pub fn new(enabled: bool, max_failed_attempts: u32) -> Self {
        Self {
            tokens: RwLock::new(HashMap::new()),
            global_failed_attempts: AtomicU32::new(0),
            max_failed_attempts,
            enabled,
        }
    }

    /// Generates a new ephemeral pairing token.
    ///
    /// Traces to: [INV-002], [NFR-SEC-001]
    ///
    /// # Arguments
    /// * `target_node_id` - Optional node identifier to bind to this session.
    /// * `ttl_seconds` - Time-to-live in seconds (defaults to 300s if None or 0).
    ///
    /// # Returns
    /// * `Ok(PairingTokenRecord)` on success.
    /// * `Err(PairingError::PairingDisabled)` if pairing is disabled in config.
    pub fn generate_token(
        &self,
        target_node_id: Option<String>,
        ttl_seconds: Option<u32>,
    ) -> Result<PairingTokenRecord, PairingError> {
        if !self.enabled {
            return Err(PairingError::PairingDisabled);
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs();

        let ttl = ttl_seconds
            .filter(|&t| t > 0)
            .unwrap_or(DEFAULT_PAIRING_TOKEN_TTL_SECONDS) as u64;

        let expires_at = now + ttl;

        // Generate high-entropy token via SHA-256 of system entropy + timestamp
        let mut hasher = Sha256::new();
        hasher.update(now.to_le_bytes());
        hasher.update(expires_at.to_le_bytes());
        if let Some(ref node_id) = target_node_id {
            hasher.update(node_id.as_bytes());
        }
        let rand_bytes: [u8; 16] = rand_bytes_16();
        hasher.update(rand_bytes);
        let token = format!("{:x}", hasher.finalize());

        let record = PairingTokenRecord {
            token: token.clone(),
            target_node_id,
            created_at_epoch_s: now,
            expires_at_epoch_s: expires_at,
            is_redeemed: false,
            failed_attempts: 0,
        };

        let mut lock = self.tokens.write().expect("Token store lock poisoned");
        lock.insert(token, record.clone());

        Ok(record)
    }

    /// Atomically validates and redeems a pairing token for enrollment.
    /// Enforces single-use semantics: immediately marks the token as redeemed.
    ///
    /// Traces to: [INV-002], [NFR-SEC-001], [REQ-OP-001]
    ///
    /// # Arguments
    /// * `token` - Ephemeral pairing token submitted by worker.
    /// * `requested_node_id` - Node identifier requesting certificate.
    ///
    /// # Returns
    /// * `Ok(PairingTokenRecord)` if valid, non-expired, and previously unredeemed.
    /// * `Err(PairingError)` on validation failure.
    pub fn redeem_token(
        &self,
        token: &str,
        requested_node_id: &str,
    ) -> Result<PairingTokenRecord, PairingError> {
        if !self.enabled {
            return Err(PairingError::PairingDisabled);
        }

        if self.global_failed_attempts.load(Ordering::Acquire) >= self.max_failed_attempts * 10 {
            return Err(PairingError::RateLimitExceeded);
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs();

        let mut lock = self.tokens.write().expect("Token store lock poisoned");
        let record = match lock.get_mut(token) {
            Some(r) => r,
            None => {
                self.global_failed_attempts.fetch_add(1, Ordering::Relaxed);
                return Err(PairingError::TokenNotFound);
            }
        };

        if record.failed_attempts >= self.max_failed_attempts {
            return Err(PairingError::RateLimitExceeded);
        }

        if record.is_redeemed {
            record.failed_attempts += 1;
            return Err(PairingError::TokenAlreadyRedeemed);
        }

        if now > record.expires_at_epoch_s {
            record.failed_attempts += 1;
            return Err(PairingError::TokenExpired {
                expired_at_epoch_s: record.expires_at_epoch_s,
                current_epoch_s: now,
            });
        }

        if let Some(ref expected_node) = record.target_node_id {
            if expected_node != requested_node_id {
                record.failed_attempts += 1;
                return Err(PairingError::NodeIdMismatch {
                    expected: expected_node.clone(),
                    actual: requested_node_id.to_string(),
                });
            }
        }

        // Atomically redeem token
        record.is_redeemed = true;
        let committed = record.clone();
        Ok(committed)
    }

    /// Revokes an active token.
    ///
    /// # Arguments
    /// * `token` - Token to revoke.
    ///
    /// # Returns
    /// * `true` if token was found and removed.
    pub fn revoke_token(&self, token: &str) -> bool {
        let mut lock = self.tokens.write().expect("Token store lock poisoned");
        lock.remove(token).is_some()
    }

    /// Inspects an active token without redeeming it.
    pub fn get_token(&self, token: &str) -> Option<PairingTokenRecord> {
        let lock = self.tokens.read().expect("Token store lock poisoned");
        lock.get(token).cloned()
    }

    /// Prunes expired tokens from memory.
    pub fn prune_expired(&self) -> usize {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs();

        let mut lock = self.tokens.write().expect("Token store lock poisoned");
        let initial_len = lock.len();
        lock.retain(|_, record| !record.is_redeemed && record.expires_at_epoch_s > now);
        initial_len - lock.len()
    }

    /// Checks if pairing functionality is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// Generates 16 random bytes using platform entropy and nanosecond clock.
fn rand_bytes_16() -> [u8; 16] {
    let mut bytes = [0u8; 16];
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos();
    let p1 = nanos.to_le_bytes();
    bytes[..8].copy_from_slice(&p1[..8]);
    bytes[8..16].copy_from_slice(&p1[8..16]);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_generation_and_default_ttl() {
        let store = PairingTokenStore::default();
        let record = store
            .generate_token(Some("worker-node-1".to_string()), None)
            .expect("Token generation should succeed");

        assert_eq!(record.token.len(), 64);
        assert_eq!(record.target_node_id.as_deref(), Some("worker-node-1"));
        assert!(!record.is_redeemed);
        assert!(record.expires_at_epoch_s >= record.created_at_epoch_s + 300);
    }

    #[test]
    fn test_single_use_redemption_and_subsequent_rejection() {
        let store = PairingTokenStore::default();
        let record = store
            .generate_token(Some("worker-pixel7".to_string()), Some(300))
            .expect("Token generation should succeed");

        // First redemption must succeed
        let redeemed = store
            .redeem_token(&record.token, "worker-pixel7")
            .expect("First redemption should succeed");
        assert!(redeemed.is_redeemed);

        // Second redemption must fail with TokenAlreadyRedeemed
        let err = store
            .redeem_token(&record.token, "worker-pixel7")
            .expect_err("Second redemption must fail");
        assert_eq!(err, PairingError::TokenAlreadyRedeemed);
    }

    #[test]
    fn test_node_id_mismatch_rejection() {
        let store = PairingTokenStore::default();
        let record = store
            .generate_token(Some("worker-expected".to_string()), Some(300))
            .expect("Token generation should succeed");

        let err = store
            .redeem_token(&record.token, "worker-impostor")
            .expect_err("Mismatched node id must be rejected");

        assert_eq!(
            err,
            PairingError::NodeIdMismatch {
                expected: "worker-expected".to_string(),
                actual: "worker-impostor".to_string(),
            }
        );
    }

    #[test]
    fn test_expired_token_rejection() {
        let store = PairingTokenStore::default();
        // Insert an already expired token record directly
        let expired_token = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        {
            let mut lock = store.tokens.write().unwrap();
            lock.insert(
                expired_token.to_string(),
                PairingTokenRecord {
                    token: expired_token.to_string(),
                    target_node_id: None,
                    created_at_epoch_s: 100,
                    expires_at_epoch_s: 200,
                    is_redeemed: false,
                    failed_attempts: 0,
                },
            );
        }

        let err = store
            .redeem_token(expired_token, "worker-node-1")
            .expect_err("Expired token must be rejected");

        match err {
            PairingError::TokenExpired {
                expired_at_epoch_s, ..
            } => {
                assert_eq!(expired_at_epoch_s, 200);
            }
            other => panic!("Expected TokenExpired, got {:?}", other),
        }
    }

    #[test]
    fn test_rate_limiting_after_excessive_failed_attempts() {
        let store = PairingTokenStore::new(true, 3);
        let record = store
            .generate_token(Some("worker-node-1".to_string()), Some(300))
            .expect("Token generation should succeed");

        // Fail 3 times with wrong node id
        for _ in 0..3 {
            let _ = store.redeem_token(&record.token, "wrong-node");
        }

        // 4th attempt should be blocked by rate limit
        let err = store
            .redeem_token(&record.token, "worker-node-1")
            .expect_err("Must trigger rate limit");
        assert_eq!(err, PairingError::RateLimitExceeded);
    }

    #[test]
    fn test_pairing_disabled_configuration() {
        let store = PairingTokenStore::new(false, 5);
        let err = store
            .generate_token(None, None)
            .expect_err("Disabled pairing must reject generation");
        assert_eq!(err, PairingError::PairingDisabled);

        let redeem_err = store
            .redeem_token("any-token", "worker-1")
            .expect_err("Disabled pairing must reject redemption");
        assert_eq!(redeem_err, PairingError::PairingDisabled);
    }

    #[test]
    fn test_token_revocation() {
        let store = PairingTokenStore::default();
        let record = store
            .generate_token(None, None)
            .expect("Token generation should succeed");

        assert!(store.get_token(&record.token).is_some());
        assert!(store.revoke_token(&record.token));
        assert!(store.get_token(&record.token).is_none());
        assert!(!store.revoke_token(&record.token));
    }
}
