//! Developer Workload Packaging and Artifact Bundling Engine
//!
//! Validates WebAssembly binaries for WASI compatibility, computes SHA-256 digests,
//! and generates canonical JSON deployment manifests.
//!
//! Traces to: AG-SPEC-2026-V2, [INV-006], [REQ-OP-003], [NFR-PERF-001]

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::artifact::wasi_validator::{WasiValidationError, WasiValidator};

/// Packaging and artifact generation errors.
#[derive(Debug, Error)]
pub enum PackError {
    #[error("Failed to read input WASM file from '{0}': {1}")]
    ReadError(PathBuf, std::io::Error),

    #[error("Failed to write output bundle or manifest to '{0}': {1}")]
    WriteError(PathBuf, std::io::Error),

    #[error("WASI validation failed: {0}")]
    Validation(#[from] WasiValidationError),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Canonical deployment manifest for packaged WebAssembly workloads.
///
/// Traces to: [INV-006], [REQ-OP-003]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactManifest {
    pub schema_version: String,
    pub name: String,
    pub entrypoint: String,
    pub artifact_sha256: String,
    pub size_bytes: usize,
    pub engine: String,
    pub wasi_version: String,
    pub exports: Vec<String>,
    pub wasi_imports: Vec<String>,
}

/// Workload packaging service.
///
/// Traces to: [INV-006], [REQ-OP-003]
pub struct ArtifactPackager;

impl ArtifactPackager {
    /// Validates a WebAssembly module and generates a deployment manifest.
    ///
    /// Traces to: [INV-006], [REQ-OP-003]
    ///
    /// # Arguments
    /// * `wasm_path` - Path to the local .wasm file.
    /// * `entrypoint` - Exported entrypoint symbol (e.g. "_start").
    /// * `output_dest` - Optional output path (file or directory).
    ///
    /// # Returns
    /// * `Ok((ArtifactManifest, Option<PathBuf>))` - Generated manifest and bundle file path if written.
    pub fn pack(
        wasm_path: &Path,
        entrypoint: &str,
        output_dest: Option<&Path>,
    ) -> Result<(ArtifactManifest, Option<PathBuf>), PackError> {
        let bytecode = std::fs::read(wasm_path)
            .map_err(|e| PackError::ReadError(wasm_path.to_path_buf(), e))?;

        let meta = WasiValidator::validate(&bytecode, entrypoint)?;

        let workload_name = wasm_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("workload")
            .to_string();

        let manifest = ArtifactManifest {
            schema_version: "androgrid.artifact.v1".to_string(),
            name: workload_name.clone(),
            entrypoint: meta.entrypoint,
            artifact_sha256: meta.artifact_sha256.clone(),
            size_bytes: meta.size_bytes,
            engine: "EXECUTION_ENGINE_WASM_WASI".to_string(),
            wasi_version: if meta.is_wasi_compatible {
                "preview1".to_string()
            } else {
                "none".to_string()
            },
            exports: meta.exports,
            wasi_imports: meta.wasi_imports,
        };

        let written_path = if let Some(dest) = output_dest {
            let target_file = if dest.is_dir() {
                dest.join(format!("{workload_name}.manifest.json"))
            } else {
                dest.to_path_buf()
            };

            let json_str = serde_json::to_string_pretty(&manifest)?;
            std::fs::write(&target_file, json_str)
                .map_err(|e| PackError::WriteError(target_file.clone(), e))?;
            Some(target_file)
        } else {
            None
        };

        Ok((manifest, written_path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pack_generates_valid_manifest() {
        let minimal_wasm = vec![
            0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x07, 0x0a, 0x01, 0x06, 0x5f, 0x73,
            0x74, 0x61, 0x72, 0x74, 0x00, 0x00,
        ];
        let temp_dir = std::env::temp_dir().join("androgrid_pack_test");
        let _ = std::fs::create_dir_all(&temp_dir);
        let wasm_file = temp_dir.join("test_app.wasm");
        std::fs::write(&wasm_file, &minimal_wasm).expect("Write temp wasm");

        let out_json = temp_dir.join("test_app.manifest.json");
        let (manifest, path) = ArtifactPackager::pack(&wasm_file, "_start", Some(&out_json))
            .expect("Packaging should succeed");

        assert_eq!(manifest.name, "test_app");
        assert_eq!(manifest.entrypoint, "_start");
        assert_eq!(manifest.engine, "EXECUTION_ENGINE_WASM_WASI");
        assert_eq!(manifest.size_bytes, minimal_wasm.len());
        assert_eq!(path, Some(out_json.clone()));
        assert!(out_json.is_file());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
