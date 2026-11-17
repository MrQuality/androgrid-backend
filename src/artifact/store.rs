//! Content-Addressed Artifact Storage Engine
//!
//! Provides thread-safe, cryptographically verified storage for WebAssembly bytecode modules.
//! Artifacts are keyed by their SHA-256 hexadecimal digests to guarantee content integrity
//! and prevent cache tampering.
//!
//! Traces to: AG-SPEC-2026-V2, [NFR-PERF-001], [NFR-POW-001], [INV-003], [REQ-OP-003]

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use thiserror::Error;

use crate::{validate_frame_bound, FrameBoundError, MAX_FRAME_SIZE_BYTES};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(1);

/// Artifact kind stored under `/v1/blobs` and validated prior to WASM scheduling.
///
/// Traces to: [NFR-PERF-001], [REQ-OP-003], ADR-008
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlobKind {
    Wasm,
    Apk,
}

impl BlobKind {
    /// Parses `X-Artifact-Kind` header values case-insensitively.
    pub fn parse_header(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "wasm" => Some(Self::Wasm),
            "apk" => Some(Self::Apk),
            _ => None,
        }
    }

    /// Lowercase canonical string representation.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wasm => "wasm",
            Self::Apk => "apk",
        }
    }
}

/// Persistent metadata sidecar stored at `<sha256>.meta.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobMetadata {
    pub kind: BlobKind,
    pub size_bytes: u64,
    pub sha256: String,
}

/// Result of committing an uploaded blob to [ArtifactStore].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobCommitOutcome {
    Created(BlobMetadata),
    IdempotentMatch(BlobMetadata),
    KindConflict { existing_kind: BlobKind },
}

/// Location of a stored blob for streaming `GET` / `HEAD` responses.
#[derive(Debug, Clone)]
pub enum BlobSource {
    File(PathBuf),
    Memory(Vec<u8>),
}

/// RAII guard that removes an uncommitted temporary upload file on drop.
///
/// Guarantees orphaned `.tmp` files are deleted on aborted uploads or early returns.
pub struct TempFileGuard {
    path: PathBuf,
    committed: bool,
}

impl TempFileGuard {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            committed: false,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn mark_committed(mut self) {
        self.committed = true;
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

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
    blob_meta: RwLock<HashMap<String, BlobMetadata>>,
    storage_dir: Option<PathBuf>,
    commit_lock: tokio::sync::Mutex<()>,
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
            blob_meta: RwLock::new(HashMap::new()),
            storage_dir: None,
            commit_lock: tokio::sync::Mutex::new(()),
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
            blob_meta: RwLock::new(HashMap::new()),
            storage_dir: Some(dir),
            commit_lock: tokio::sync::Mutex::new(()),
        }
    }

    /// Returns the configured persistent storage directory, if any.
    pub fn storage_dir(&self) -> Option<&Path> {
        self.storage_dir.as_deref()
    }

    /// Validates 64-char hexadecimal SHA-256 string and normalizes to lowercase.
    pub fn normalize_sha256_hex(raw: &str) -> Option<String> {
        let clean = raw.trim().trim_end_matches(".wasm").to_ascii_lowercase();
        if clean.len() == 64 && clean.bytes().all(|b| b.is_ascii_hexdigit()) {
            Some(clean)
        } else {
            None
        }
    }

    /// Maximum allowed byte length for `kind`.
    pub fn max_bytes_for_kind(kind: BlobKind, max_apk_bytes: usize) -> usize {
        match kind {
            BlobKind::Wasm => MAX_FRAME_SIZE_BYTES,
            BlobKind::Apk => max_apk_bytes,
        }
    }

    /// Allocates a unique `.tmp` path under `storage_dir` (or OS temp dir for in-memory stores).
    pub fn next_temp_path(&self) -> PathBuf {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = self
            .storage_dir
            .clone()
            .unwrap_or_else(std::env::temp_dir);
        let _ = std::fs::create_dir_all(&dir);
        dir.join(format!("upload-{}-{seq}.tmp", std::process::id()))
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
        if let Ok(mut meta) = self.blob_meta.write() {
            meta.entry(sha256_hex.clone()).or_insert_with(|| BlobMetadata {
                kind: BlobKind::Wasm,
                size_bytes: bytecode.len() as u64,
                sha256: sha256_hex.clone(),
            });
        }

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
            let blob_path = dir.join(format!("{clean_sha}.blob"));
            if blob_path.is_file() {
                if let Some(meta) = self.read_disk_metadata(dir, &clean_sha) {
                    if meta.kind != BlobKind::Wasm {
                        return None;
                    }
                }
                if let Ok(bytes) = std::fs::read(&blob_path) {
                    return Some(bytes);
                }
            }
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

    fn read_disk_metadata(&self, dir: &Path, clean_sha: &str) -> Option<BlobMetadata> {
        let meta_path = dir.join(format!("{clean_sha}.meta.json"));
        let raw = std::fs::read_to_string(meta_path).ok()?;
        serde_json::from_str(&raw).ok()
    }

    /// Commits a streamed temporary file into `<sha256>.meta.json` + `<sha256>.blob` atomically.
    ///
    /// Serializes existence check and renames under `commit_lock` to eliminate `kind` races
    /// across concurrent `PUT /v1/blobs` uploads.
    pub async fn commit_temp_blob(
        &self,
        guard: TempFileGuard,
        sha256_hex: String,
        size_bytes: u64,
        kind: BlobKind,
    ) -> Result<BlobCommitOutcome, ArtifactError> {
        let _lock = self.commit_lock.lock().await;
        let clean_sha = sha256_hex.to_ascii_lowercase();

        if let Some(ref dir) = self.storage_dir {
            let blob_path = dir.join(format!("{clean_sha}.blob"));
            if blob_path.is_file() {
                let existing_meta = self.read_disk_metadata(dir, &clean_sha).unwrap_or(BlobMetadata {
                    kind: BlobKind::Wasm,
                    size_bytes,
                    sha256: clean_sha.clone(),
                });
                if existing_meta.kind == kind {
                    return Ok(BlobCommitOutcome::IdempotentMatch(existing_meta));
                }
                return Ok(BlobCommitOutcome::KindConflict {
                    existing_kind: existing_meta.kind,
                });
            }

            let meta = BlobMetadata {
                kind,
                size_bytes,
                sha256: clean_sha.clone(),
            };
            let meta_tmp = dir.join(format!("{clean_sha}.meta.json.tmp"));
            let meta_final = dir.join(format!("{clean_sha}.meta.json"));
            let meta_json = serde_json::to_vec(&meta)
                .map_err(|e| ArtifactError::IoError(e.to_string()))?;
            tokio::fs::write(&meta_tmp, &meta_json)
                .await
                .map_err(|e| ArtifactError::IoError(e.to_string()))?;
            tokio::fs::rename(&meta_tmp, &meta_final)
                .await
                .map_err(|e| ArtifactError::IoError(e.to_string()))?;
            tokio::fs::rename(guard.path(), &blob_path)
                .await
                .map_err(|e| ArtifactError::IoError(e.to_string()))?;
            guard.mark_committed();
            return Ok(BlobCommitOutcome::Created(meta));
        }

        // In-memory store fallback when storage_dir is None
        let bytes = tokio::fs::read(guard.path())
            .await
            .map_err(|e| ArtifactError::IoError(e.to_string()))?;
        if let Ok(meta_map) = self.blob_meta.read() {
            if let Some(existing) = meta_map.get(&clean_sha) {
                if existing.kind == kind {
                    return Ok(BlobCommitOutcome::IdempotentMatch(existing.clone()));
                }
                return Ok(BlobCommitOutcome::KindConflict {
                    existing_kind: existing.kind,
                });
            }
        }
        let meta = BlobMetadata {
            kind,
            size_bytes,
            sha256: clean_sha.clone(),
        };
        if let Ok(mut meta_map) = self.blob_meta.write() {
            meta_map.insert(clean_sha.clone(), meta.clone());
        }
        if let Ok(mut cache) = self.cache.write() {
            cache.insert(clean_sha, bytes);
        }
        Ok(BlobCommitOutcome::Created(meta))
    }

    /// Returns blob metadata and streaming source without caching large `.blob` files in RAM.
    pub fn get_blob_info(&self, sha256_hex: &str) -> Option<(BlobMetadata, BlobSource)> {
        let clean_sha = Self::normalize_sha256_hex(sha256_hex)?;
        if let Some(ref dir) = self.storage_dir {
            let blob_path = dir.join(format!("{clean_sha}.blob"));
            if blob_path.is_file() {
                if let Some(meta) = self.read_disk_metadata(dir, &clean_sha) {
                    return Some((meta, BlobSource::File(blob_path)));
                }
            }
            let legacy_path = dir.join(format!("{clean_sha}.wasm"));
            if legacy_path.is_file() {
                if let Ok(fs_meta) = std::fs::metadata(&legacy_path) {
                    let meta = BlobMetadata {
                        kind: BlobKind::Wasm,
                        size_bytes: fs_meta.len(),
                        sha256: clean_sha,
                    };
                    return Some((meta, BlobSource::File(legacy_path)));
                }
            }
        }

        if let Ok(cache) = self.cache.read() {
            if let Some(bytes) = cache.get(&clean_sha) {
                let meta = self
                    .blob_meta
                    .read()
                    .ok()
                    .and_then(|m| m.get(&clean_sha).cloned())
                    .unwrap_or_else(|| BlobMetadata {
                        kind: BlobKind::Wasm,
                        size_bytes: bytes.len() as u64,
                        sha256: clean_sha.clone(),
                    });
                return Some((meta, BlobSource::Memory(bytes.clone())));
            }
        }
        None
    }

    /// Returns the stored [BlobKind] for a SHA-256 digest if present.
    pub fn get_blob_kind(&self, sha256_hex: &str) -> Option<BlobKind> {
        self.get_blob_info(sha256_hex).map(|(meta, _)| meta.kind)
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

    #[tokio::test]
    async fn test_temp_file_guard_and_commit_idempotency_and_kind_conflict() {
        let temp_dir = std::env::temp_dir().join("androgrid_store_blob_unit_test");
        let _ = std::fs::remove_dir_all(&temp_dir);
        let store = ArtifactStore::with_directory(&temp_dir);

        // 1. Uncommitted TempFileGuard deletes .tmp file on drop
        let aborted_path = store.next_temp_path();
        std::fs::write(&aborted_path, b"partial").unwrap();
        assert!(aborted_path.exists());
        {
            let _guard = TempFileGuard::new(aborted_path.clone());
        }
        assert!(!aborted_path.exists());

        // 2. Commit Wasm blob -> Created -> IdempotentMatch -> KindConflict
        let payload = b"\x00asm\x01\x00\x00\x00blob_test";
        let sha = ArtifactStore::compute_sha256(payload);

        let p1 = store.next_temp_path();
        tokio::fs::write(&p1, payload).await.unwrap();
        let out1 = store
            .commit_temp_blob(
                TempFileGuard::new(p1),
                sha.clone(),
                payload.len() as u64,
                BlobKind::Wasm,
            )
            .await
            .unwrap();
        assert!(matches!(out1, BlobCommitOutcome::Created(_)));

        let p2 = store.next_temp_path();
        tokio::fs::write(&p2, payload).await.unwrap();
        let out2 = store
            .commit_temp_blob(
                TempFileGuard::new(p2),
                sha.clone(),
                payload.len() as u64,
                BlobKind::Wasm,
            )
            .await
            .unwrap();
        assert!(matches!(out2, BlobCommitOutcome::IdempotentMatch(_)));

        let p3 = store.next_temp_path();
        tokio::fs::write(&p3, payload).await.unwrap();
        let out3 = store
            .commit_temp_blob(
                TempFileGuard::new(p3),
                sha.clone(),
                payload.len() as u64,
                BlobKind::Apk,
            )
            .await
            .unwrap();
        assert_eq!(
            out3,
            BlobCommitOutcome::KindConflict {
                existing_kind: BlobKind::Wasm
            }
        );

        let (meta, src) = store.get_blob_info(&sha).expect("blob info");
        assert_eq!(meta.kind, BlobKind::Wasm);
        assert!(matches!(src, BlobSource::File(_)));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
