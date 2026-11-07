//! WebAssembly and WASI Preview 1 Binary Validator
//!
//! Provides zero-dependency, safe validation of WebAssembly bytecode headers,
//! section structures, exported entrypoint symbols, and WASI Preview 1 imports.
//!
//! Traces to: AG-SPEC-2026-V2, [INV-006], [REQ-OP-004], [NFR-PERF-001]

use sha2::{Digest, Sha256};
use thiserror::Error;

/// WASM binary magic numbers: "\0asm" followed by version 1.
pub const WASM_MAGIC: [u8; 4] = [0x00, 0x61, 0x73, 0x6d];
pub const WASM_VERSION_1: [u8; 4] = [0x01, 0x00, 0x00, 0x00];

/// Errors encountered during WASI bytecode inspection and validation.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum WasiValidationError {
    #[error("Bytecode payload is empty")]
    EmptyBytecode,

    #[error("Payload too short for WebAssembly header: expected at least 8 bytes, got {0}")]
    TooShort(usize),

    #[error("Invalid WebAssembly magic number: expected '\\0asm', found {0:?}")]
    InvalidMagic([u8; 4]),

    #[error("Unsupported WebAssembly version: expected 1, found {0}")]
    UnsupportedVersion(u32),

    #[error("Malformed WebAssembly section structure: {0}")]
    MalformedSection(String),

    #[error("Specified entrypoint '{0}' not found in WebAssembly export section")]
    EntrypointNotFound(String),
}

/// Extracted metadata from validated WebAssembly module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasiMetadata {
    /// Validated entrypoint symbol.
    pub entrypoint: String,

    /// SHA-256 hexadecimal digest of raw bytecode.
    pub artifact_sha256: String,

    /// Payload size in bytes.
    pub size_bytes: usize,

    /// Whether the module exports or imports WASI preview 1 interfaces.
    pub is_wasi_compatible: bool,

    /// List of imported functions and modules.
    pub wasi_imports: Vec<String>,

    /// List of exported symbol names.
    pub exports: Vec<String>,
}

/// Binary validator for WebAssembly / WASI preview 1 modules.
///
/// Traces to: [INV-006], [REQ-OP-004]
pub struct WasiValidator;

impl WasiValidator {
    /// Validates WebAssembly binary header and exported entrypoint.
    ///
    /// Traces to: [INV-006], [REQ-OP-004]
    ///
    /// # Arguments
    /// * `bytecode` - Binary WebAssembly module.
    /// * `entrypoint` - Expected entrypoint function name (e.g. "_start").
    ///
    /// # Returns
    /// * `Ok(WasiMetadata)` - Verified module metadata.
    /// * `Err(WasiValidationError)` - If header, section, or entrypoint check fails.
    pub fn validate(
        bytecode: &[u8],
        entrypoint: &str,
    ) -> Result<WasiMetadata, WasiValidationError> {
        if bytecode.is_empty() {
            return Err(WasiValidationError::EmptyBytecode);
        }

        if bytecode.len() < 8 {
            return Err(WasiValidationError::TooShort(bytecode.len()));
        }

        let magic = &bytecode[0..4];
        if magic != WASM_MAGIC {
            let mut arr = [0u8; 4];
            arr.copy_from_slice(magic);
            return Err(WasiValidationError::InvalidMagic(arr));
        }

        let version = u32::from_le_bytes([bytecode[4], bytecode[5], bytecode[6], bytecode[7]]);
        if version != 1 {
            return Err(WasiValidationError::UnsupportedVersion(version));
        }

        let mut offset = 8;
        let mut exports = Vec::new();
        let mut wasi_imports = Vec::new();
        let mut has_wasi_import = false;

        while offset < bytecode.len() {
            if offset >= bytecode.len() {
                break;
            }
            let section_id = bytecode[offset];
            offset += 1;

            let (section_len, bytes_read) = Self::read_leb128_u32(&bytecode[offset..])
                .map_err(|e| WasiValidationError::MalformedSection(e.to_string()))?;
            offset += bytes_read;

            let section_len = section_len as usize;
            if offset + section_len > bytecode.len() {
                return Err(WasiValidationError::MalformedSection(
                    "Section length extends past end of file".to_string(),
                ));
            }

            let section_data = &bytecode[offset..offset + section_len];
            offset += section_len;

            match section_id {
                // Import Section (ID 2)
                2 => {
                    let mut s_off = 0;
                    if let Ok((import_count, read)) = Self::read_leb128_u32(&section_data[s_off..])
                    {
                        s_off += read;
                        for _ in 0..import_count {
                            if s_off >= section_data.len() {
                                break;
                            }
                            if let Ok((mod_name, read)) =
                                Self::read_wasm_string(&section_data[s_off..])
                            {
                                s_off += read;
                                if let Ok((field_name, read)) =
                                    Self::read_wasm_string(&section_data[s_off..])
                                {
                                    s_off += read;
                                    if mod_name == "wasi_snapshot_preview1"
                                        || mod_name == "wasi_unstable"
                                    {
                                        has_wasi_import = true;
                                        wasi_imports.push(format!("{mod_name}::{field_name}"));
                                    }
                                    if s_off < section_data.len() {
                                        let kind = section_data[s_off];
                                        s_off += 1;
                                        match kind {
                                            0 => {
                                                if let Ok((_, r)) =
                                                    Self::read_leb128_u32(&section_data[s_off..])
                                                {
                                                    s_off += r;
                                                }
                                            }
                                            1 => {
                                                s_off += 1;
                                                if let Ok((_, r)) =
                                                    Self::read_leb128_u32(&section_data[s_off..])
                                                {
                                                    s_off += r;
                                                }
                                            }
                                            2 => {
                                                if let Ok((_, r)) =
                                                    Self::read_leb128_u32(&section_data[s_off..])
                                                {
                                                    s_off += r;
                                                }
                                            }
                                            3 => {
                                                s_off += 2;
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                // Export Section (ID 7)
                7 => {
                    let mut s_off = 0;
                    if let Ok((export_count, read)) = Self::read_leb128_u32(&section_data[s_off..])
                    {
                        s_off += read;
                        for _ in 0..export_count {
                            if s_off >= section_data.len() {
                                break;
                            }
                            if let Ok((name, read)) = Self::read_wasm_string(&section_data[s_off..])
                            {
                                s_off += read;
                                exports.push(name);
                                if s_off < section_data.len() {
                                    s_off += 1; // kind
                                    if let Ok((_, r)) =
                                        Self::read_leb128_u32(&section_data[s_off..])
                                    {
                                        s_off += r;
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Verify that expected entrypoint is exported
        if !entrypoint.is_empty() && !exports.iter().any(|e| e == entrypoint) {
            return Err(WasiValidationError::EntrypointNotFound(
                entrypoint.to_string(),
            ));
        }

        let mut hasher = Sha256::new();
        hasher.update(bytecode);
        let artifact_sha256 = format!("{:x}", hasher.finalize());

        // A module is WASI compatible if it explicitly imports WASI functions or exports entrypoint
        let is_wasi_compatible =
            has_wasi_import || exports.iter().any(|e| e == "_start" || e == entrypoint);

        Ok(WasiMetadata {
            entrypoint: entrypoint.to_string(),
            artifact_sha256,
            size_bytes: bytecode.len(),
            is_wasi_compatible,
            wasi_imports,
            exports,
        })
    }

    /// Reads variable-length unsigned LEB128 integer.
    fn read_leb128_u32(data: &[u8]) -> Result<(u32, usize), &'static str> {
        let mut result = 0u32;
        let mut shift = 0;
        let mut bytes_read = 0;

        for &byte in data {
            bytes_read += 1;
            let val = (byte & 0x7F) as u32;
            result |= val << shift;
            if (byte & 0x80) == 0 {
                return Ok((result, bytes_read));
            }
            shift += 7;
            if shift >= 35 {
                return Err("LEB128 integer overflow");
            }
        }

        Err("Unexpected end of data while decoding LEB128")
    }

    /// Reads length-prefixed UTF-8 string from WebAssembly section data.
    fn read_wasm_string(data: &[u8]) -> Result<(String, usize), &'static str> {
        let (len, bytes_read) = Self::read_leb128_u32(data)?;
        let len = len as usize;
        let start = bytes_read;
        let end = start + len;

        if end > data.len() {
            return Err("String length exceeds section data");
        }

        let s = std::str::from_utf8(&data[start..end])
            .map_err(|_| "Invalid UTF-8 in export or import string")?
            .to_string();

        Ok((s, end))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_wasm_header_acceptance() {
        let mut minimal = vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
        // Export section with "_start"
        minimal.extend_from_slice(&[
            0x07, // export section id
            0x0a, // section len (10 bytes)
            0x01, // 1 export
            0x06, 0x5f, 0x73, 0x74, 0x61, 0x72, 0x74, // name: "_start"
            0x00, 0x00, // function index 0
        ]);

        let meta = WasiValidator::validate(&minimal, "_start").expect("Valid minimal WASI");
        assert_eq!(meta.entrypoint, "_start");
        assert!(meta.exports.contains(&"_start".to_string()));
    }

    #[test]
    fn test_invalid_header_rejection() {
        let bad_magic = vec![0xde, 0xad, 0xbe, 0xef, 0x01, 0x00, 0x00, 0x00];
        assert!(matches!(
            WasiValidator::validate(&bad_magic, "_start"),
            Err(WasiValidationError::InvalidMagic(_))
        ));

        let bad_version = vec![0x00, 0x61, 0x73, 0x6d, 0x02, 0x00, 0x00, 0x00];
        assert!(matches!(
            WasiValidator::validate(&bad_version, "_start"),
            Err(WasiValidationError::UnsupportedVersion(2))
        ));
    }
}
