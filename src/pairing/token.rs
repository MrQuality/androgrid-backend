//! Ephemeral Device Pairing Token Lifecycle Management
//!
//! Enforces single-use semantics, time-bounded expiration (default 300s),
//! and brute-force rate-limiting for worker node bootstrap enrollment.
//!
//! Traces to: AG-SPEC-2026-V2 Section 2.3, [INV-002], [NFR-SEC-001], [REQ-OP-001]

use ring::rand::{SecureRandom, SystemRandom};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::{Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

/// Default ephemeral token lifetime in seconds (5 minutes).
/// Traces to: [NFR-SEC-001]
pub const DEFAULT_PAIRING_TOKEN_TTL_SECONDS: u32 = 300;

/// Maximum failed enrollment attempts before rate-limiting triggers.
pub const DEFAULT_MAX_ATTEMPTS: u32 = 5;

/// Sliding window duration in seconds for unknown-token brute-force protection.
pub const GLOBAL_RATE_LIMIT_WINDOW_SECS: u64 = 60;

/// Maximum number of token records retained in memory before forced eviction.
pub const MAX_TRACKED_TOKENS: usize = 10_000;

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

    #[error("Cryptographic random number generator failure")]
    RngFailure,
}

/// Error returned by [`PairingTokenStore::redeem_with`], distinguishing token
/// validation failures from downstream signing/provisioning failures.
#[derive(Debug)]
pub enum RedeemActionError<E> {
    Pairing(PairingError),
    Action(E),
}

struct GlobalAttemptWindow {
    window_start_epoch_s: u64,
    failed_attempts: u32,
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
    global_window: Mutex<GlobalAttemptWindow>,
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
            global_window: Mutex::new(GlobalAttemptWindow {
                window_start_epoch_s: 0,
                failed_attempts: 0,
            }),
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
        let token = generate_csprng_token_hex()?;

        let record = PairingTokenRecord {
            token: token.clone(),
            target_node_id,
            created_at_epoch_s: now,
            expires_at_epoch_s: expires_at,
            is_redeemed: false,
            failed_attempts: 0,
        };

        let mut lock = self.tokens.write().expect("Token store lock poisoned");
        lock.retain(|_, existing| existing.expires_at_epoch_s >= now);
        if lock.len() >= MAX_TRACKED_TOKENS {
            lock.retain(|_, existing| !existing.is_redeemed);
        }
        if lock.len() >= MAX_TRACKED_TOKENS {
            if let Some(oldest_key) = lock
                .iter()
                .min_by_key(|(_, existing)| existing.created_at_epoch_s)
                .map(|(k, _)| k.clone())
            {
                lock.remove(&oldest_key);
            }
        }
        lock.insert(token, record.clone());

        Ok(record)
    }

    /// Atomically validates a pairing token, executes `action`, and marks the token
    /// as redeemed only if `action` succeeds.
    ///
    /// Traces to: [INV-002], [NFR-SEC-001], [REQ-OP-001]
    pub fn redeem_with<F, T, E>(
        &self,
        token: &str,
        requested_node_id: &str,
        action: F,
    ) -> Result<T, RedeemActionError<E>>
    where
        F: FnOnce(&PairingTokenRecord) -> Result<T, E>,
    {
        if !self.enabled {
            return Err(RedeemActionError::Pairing(PairingError::PairingDisabled));
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs();

        self.check_global_window(now)
            .map_err(RedeemActionError::Pairing)?;

        let mut lock = self.tokens.write().expect("Token store lock poisoned");
        let record = match lock.get_mut(token) {
            Some(r) => r,
            None => {
                self.record_global_failure(now);
                return Err(RedeemActionError::Pairing(PairingError::TokenNotFound));
            }
        };

        if record.failed_attempts >= self.max_failed_attempts {
            return Err(RedeemActionError::Pairing(PairingError::RateLimitExceeded));
        }

        if record.is_redeemed {
            record.failed_attempts += 1;
            return Err(RedeemActionError::Pairing(
                PairingError::TokenAlreadyRedeemed,
            ));
        }

        if now > record.expires_at_epoch_s {
            record.failed_attempts += 1;
            return Err(RedeemActionError::Pairing(PairingError::TokenExpired {
                expired_at_epoch_s: record.expires_at_epoch_s,
                current_epoch_s: now,
            }));
        }

        if let Some(ref expected_node) = record.target_node_id {
            if expected_node != requested_node_id {
                record.failed_attempts += 1;
                return Err(RedeemActionError::Pairing(PairingError::NodeIdMismatch {
                    expected: expected_node.clone(),
                    actual: requested_node_id.to_string(),
                }));
            }
        }

        match action(record) {
            Ok(result) => {
                record.is_redeemed = true;
                Ok(result)
            }
            Err(err) => {
                record.failed_attempts += 1;
                Err(RedeemActionError::Action(err))
            }
        }
    }

    fn check_global_window(&self, now: u64) -> Result<(), PairingError> {
        let mut window = self
            .global_window
            .lock()
            .expect("Global attempt window lock poisoned");
        if now.saturating_sub(window.window_start_epoch_s) >= GLOBAL_RATE_LIMIT_WINDOW_SECS {
            window.window_start_epoch_s = now;
            window.failed_attempts = 0;
        }
        if window.failed_attempts >= self.max_failed_attempts.saturating_mul(10) {
            return Err(PairingError::RateLimitExceeded);
        }
        Ok(())
    }

    fn record_global_failure(&self, now: u64) {
        let mut window = self
            .global_window
            .lock()
            .expect("Global attempt window lock poisoned");
        if now.saturating_sub(window.window_start_epoch_s) >= GLOBAL_RATE_LIMIT_WINDOW_SECS {
            window.window_start_epoch_s = now;
            window.failed_attempts = 0;
        }
        window.failed_attempts = window.failed_attempts.saturating_add(1);
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
        self.redeem_with(token, requested_node_id, |record| {
            Ok::<_, std::convert::Infallible>(PairingTokenRecord {
                is_redeemed: true,
                ..record.clone()
            })
        })
        .map_err(|err| match err {
            RedeemActionError::Pairing(p) => p,
            RedeemActionError::Action(never) => match never {},
        })
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

    /// Prunes expired tokens from memory while retaining un-expired redeemed
    /// tokens until their TTL elapses so replay attempts remain distinguishable.
    pub fn prune_expired(&self) -> usize {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs();

        let mut lock = self.tokens.write().expect("Token store lock poisoned");
        let initial_len = lock.len();
        lock.retain(|_, record| record.expires_at_epoch_s >= now);
        initial_len - lock.len()
    }

    /// Checks if pairing functionality is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// Generates a 256-bit cryptographically secure random token encoded as 64 hex chars.
fn generate_csprng_token_hex() -> Result<String, PairingError> {
    let rng = SystemRandom::new();
    let mut bytes = [0u8; 32];
    rng.fill(&mut bytes).map_err(|_| PairingError::RngFailure)?;
    let mut hex = String::with_capacity(64);
    for b in bytes {
        let _ = write!(&mut hex, "{:02x}", b);
    }
    Ok(hex)
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

    #[test]
    fn test_redeem_with_does_not_burn_token_on_action_failure() {
        let store = PairingTokenStore::default();
        let record = store
            .generate_token(Some("worker-node-1".to_string()), Some(300))
            .expect("Token generation should succeed");

        // Simulate downstream CSR validation failure before redemption
        let err = store
            .redeem_with(&record.token, "worker-node-1", |_| {
                Err::<(), _>("malformed CSR")
            })
            .expect_err("Action failure should propagate");
        assert!(matches!(err, RedeemActionError::Action("malformed CSR")));

        // Token must still be unredeemed so a corrected retry succeeds
        let snapshot = store.get_token(&record.token).unwrap();
        assert!(!snapshot.is_redeemed);
        assert_eq!(snapshot.failed_attempts, 1);

        let ok = store
            .redeem_with(&record.token, "worker-node-1", |_| {
                Ok::<_, &str>("issued-cert")
            })
            .expect("Subsequent valid attempt should succeed");
        assert_eq!(ok, "issued-cert");
        assert!(store.get_token(&record.token).unwrap().is_redeemed);
    }

    #[test]
    fn test_prune_expired_keeps_unexpired_redeemed_tokens_and_removes_expired() {
        let store = PairingTokenStore::default();
        let active_redeemed = store.generate_token(None, Some(300)).unwrap();
        store.redeem_token(&active_redeemed.token, "w1").unwrap();

        {
            let mut lock = store.tokens.write().unwrap();
            lock.insert(
                "expired-token".to_string(),
                PairingTokenRecord {
                    token: "expired-token".to_string(),
                    target_node_id: None,
                    created_at_epoch_s: 10,
                    expires_at_epoch_s: 20,
                    is_redeemed: false,
                    failed_attempts: 0,
                },
            );
        }

        let pruned = store.prune_expired();
        assert_eq!(pruned, 1);
        assert!(store.get_token("expired-token").is_none());
        assert!(store.get_token(&active_redeemed.token).is_some());
        assert_eq!(
            store.redeem_token(&active_redeemed.token, "w1"),
            Err(PairingError::TokenAlreadyRedeemed)
        );
    }

    #[test]
    fn test_unknown_token_rate_limit_decays_after_window() {
        let store = PairingTokenStore::new(true, 1);
        for _ in 0..10 {
            assert_eq!(
                store.redeem_token("bogus", "w1"),
                Err(PairingError::TokenNotFound)
            );
        }
        assert_eq!(
            store.redeem_token("bogus", "w1"),
            Err(PairingError::RateLimitExceeded)
        );

        // Simulate window expiry by shifting window_start back past GLOBAL_RATE_LIMIT_WINDOW_SECS
        {
            let mut window = store.global_window.lock().unwrap();
            window.window_start_epoch_s = window
                .window_start_epoch_s
                .saturating_sub(GLOBAL_RATE_LIMIT_WINDOW_SECS + 1);
        }

        assert_eq!(
            store.redeem_token("bogus", "w1"),
            Err(PairingError::TokenNotFound)
        );
    }

    #[test]
    fn test_tokens_are_unique_csprng_hex() {
        let store = PairingTokenStore::default();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..50 {
            let rec = store.generate_token(None, Some(60)).unwrap();
            assert_eq!(rec.token.len(), 64);
            assert!(rec.token.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(seen.insert(rec.token));
        }
    }
}
