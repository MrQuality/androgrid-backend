//! Content-Addressed Artifact Storage Engine
//!
//! Provides thread-safe, cryptographically verified storage for WebAssembly bytecode modules.
//! Artifacts are keyed by their SHA-256 hexadecimal digests to guarantee content integrity
//! and prevent cache tampering.
//!
//! Traces to: AG-SPEC-2026-V2, [NFR-PERF-001], [NFR-POW-001], [INV-003], [REQ-OP-003]

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use thiserror::Error;

use crate::{validate_frame_bound, FrameBoundError};

/// Artifact storage and integrity validation errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ArtifactError {
    #[error("Message frame payload bounds exceeded: {0}")]
    FrameBound(#[from] FrameBoundError),

    #[error("SHA-256 digest mismatch: expected '{expected}', computed '{computed}'")]
    HashMismatch { expected: String, computed: String },

    #[error("Artifact not found: '{0}'")]
    NotFound(String),

    #[error("IO error interacting with artifact storage: {0}")]
    IoError(String),
}

/// Content-addressed storage for WebAssembly bytecode artifacts.
///
/// Thread-safe in-memory cache with optional filesystem persistence backing.
/// Every inserted artifact is verified against SHA-256 and maximum frame bounds.
///
/// Traces to: [NFR-PERF-001], [REQ-OP-003]
#[derive(Debug)]
pub struct ArtifactStore {
    cache: RwLock<HashMap<String, Vec<u8>>>,
    storage_dir: Option<PathBuf>,
}

impl Default for ArtifactStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ArtifactStore {
    /// Creates a new in-memory content-addressed artifact store.
    pub fn new() -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            storage_dir: None,
        }
    }

    /// Creates an artifact store backed by persistent directory on disk.
    ///
    /// # Arguments
    /// * `storage_dir` - Directory path where artifact binaries are persisted.
    pub fn with_directory<P: AsRef<Path>>(storage_dir: P) -> Self {
        let dir = storage_dir.as_ref().to_path_buf();
        let _ = std::fs::create_dir_all(&dir);
        Self {
            cache: RwLock::new(HashMap::new()),
            storage_dir: Some(dir),
        }
    }

    /// Computes the SHA-256 hexadecimal digest for raw bytecode.
    ///
    /// # Arguments
    /// * `bytecode` - Binary WebAssembly payload bytes.
    pub fn compute_sha256(bytecode: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytecode);
        format!("{:x}", hasher.finalize())
    }

    /// Verifies that bytecode matches the expected SHA-256 digest.
    ///
    /// Traces to: [NFR-PERF-001], [REQ-OP-003]
    ///
    /// # Arguments
    /// * `expected_sha256` - Expected 64-character hexadecimal SHA-256 digest.
    /// * `bytecode` - Binary payload to verify.
    pub fn validate_sha256(expected_sha256: &str, bytecode: &[u8]) -> Result<(), ArtifactError> {
        let computed = Self::compute_sha256(bytecode);
        if expected_sha256.trim().eq_ignore_ascii_case(&computed) {
            Ok(())
        } else {
            Err(ArtifactError::HashMismatch {
                expected: expected_sha256.to_string(),
                computed,
            })
        }
    }

    /// Stores a WebAssembly artifact, computing its SHA-256 digest.
    ///
    /// Traces to: [NFR-PERF-001], [REQ-OP-003]
    ///
    /// # Arguments
    /// * `bytecode` - Raw WebAssembly module bytes.
    ///
    /// # Returns
    /// * `Ok(String)` - The 64-character SHA-256 hexadecimal digest.
    /// * `Err(ArtifactError)` - If frame size exceeds 16 MB or IO failure occurs.
    pub fn store_artifact(&self, bytecode: &[u8]) -> Result<String, ArtifactError> {
        validate_frame_bound(bytecode.len())?;
        let sha256_hex = Self::compute_sha256(bytecode);

        if let Some(ref dir) = self.storage_dir {
            let file_path = dir.join(format!("{sha256_hex}.wasm"));
            if !file_path.exists() {
                std::fs::write(&file_path, bytecode)
                    .map_err(|e| ArtifactError::IoError(e.to_string()))?;
            }
        }

        let mut cache = self
            .cache
            .write()
            .map_err(|e| ArtifactError::IoError(e.to_string()))?;
        cache.insert(sha256_hex.clone(), bytecode.to_vec());

        Ok(sha256_hex)
    }

    /// Stores a WebAssembly artifact after validating against an expected SHA-256 digest.
    ///
    /// Traces to: [NFR-PERF-001], [REQ-OP-003]
    ///
    /// # Arguments
    /// * `bytecode` - Raw WebAssembly module bytes.
    /// * `expected_sha256` - Expected SHA-256 hash.
    pub fn store_artifact_verified(
        &self,
        bytecode: &[u8],
        expected_sha256: &str,
    ) -> Result<String, ArtifactError> {
        if !expected_sha256.is_empty() {
            Self::validate_sha256(expected_sha256, bytecode)?;
        }
        self.store_artifact(bytecode)
    }

    /// Retrieves an artifact by its SHA-256 hexadecimal digest.
    ///
    /// # Arguments
    /// * `sha256_hex` - The 64-character hexadecimal digest.
    pub fn get_artifact(&self, sha256_hex: &str) -> Option<Vec<u8>> {
        let clean_sha = sha256_hex
            .trim()
            .trim_end_matches(".wasm")
            .to_ascii_lowercase();

        if let Ok(cache) = self.cache.read() {
            if let Some(bytes) = cache.get(&clean_sha) {
                return Some(bytes.clone());
            }
        }

        if let Some(ref dir) = self.storage_dir {
            let file_path = dir.join(format!("{clean_sha}.wasm"));
            if file_path.is_file() {
                if let Ok(bytes) = std::fs::read(&file_path) {
                    if let Ok(mut cache) = self.cache.write() {
                        cache.insert(clean_sha, bytes.clone());
                    }
                    return Some(bytes);
                }
            }
        }

        None
    }

    /// Checks whether an artifact is present in the store.
    pub fn has_artifact(&self, sha256_hex: &str) -> bool {
        let clean_sha = sha256_hex
            .trim()
            .trim_end_matches(".wasm")
            .to_ascii_lowercase();

        if let Ok(cache) = self.cache.read() {
            if cache.contains_key(&clean_sha) {
                return true;
            }
        }

        if let Some(ref dir) = self.storage_dir {
            let file_path = dir.join(format!("{clean_sha}.wasm"));
            if file_path.is_file() {
                return true;
            }
        }

        false
    }

    /// Returns the byte length of a cached artifact, if present.
    pub fn get_artifact_size(&self, sha256_hex: &str) -> Option<usize> {
        let clean_sha = sha256_hex
            .trim()
            .trim_end_matches(".wasm")
            .to_ascii_lowercase();

        if let Ok(cache) = self.cache.read() {
            if let Some(bytes) = cache.get(&clean_sha) {
                return Some(bytes.len());
            }
        }

        if let Some(ref dir) = self.storage_dir {
            let file_path = dir.join(format!("{clean_sha}.wasm"));
            if let Ok(meta) = std::fs::metadata(file_path) {
                return Some(meta.len() as usize);
            }
        }

        None
    }

    /// Lists all SHA-256 keys of currently indexed artifacts.
    pub fn list_artifacts(&self) -> Vec<String> {
        if let Ok(cache) = self.cache.read() {
            cache.keys().cloned().collect()
        } else {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MAX_FRAME_SIZE_BYTES;

    #[test]
    fn test_store_and_retrieve_round_trip() {
        let store = ArtifactStore::new();
        let payload = b"\x00asm\x01\x00\x00\x00simple_test_payload";

        let hash = store
            .store_artifact(payload)
            .expect("Storage should succeed");
        assert_eq!(hash.len(), 64);
        assert!(store.has_artifact(&hash));
        assert_eq!(store.get_artifact_size(&hash), Some(payload.len()));

        let fetched = store.get_artifact(&hash).expect("Must find artifact");
        assert_eq!(fetched, payload);
    }

    #[test]
    fn test_validate_sha256_rejects_corrupted_payload() {
        let payload = b"unmodified_payload";
        let hash = ArtifactStore::compute_sha256(payload);

        assert!(ArtifactStore::validate_sha256(&hash, payload).is_ok());
        let corrupted = b"tampered_payload";
        assert!(ArtifactStore::validate_sha256(&hash, corrupted).is_err());
    }

    #[test]
    fn test_oversized_payload_rejected_at_boundary() {
        let store = ArtifactStore::new();
        let oversized = vec![0u8; MAX_FRAME_SIZE_BYTES + 1];
        let res = store.store_artifact(&oversized);
        assert!(matches!(res, Err(ArtifactError::FrameBound(_))));
    }
}
