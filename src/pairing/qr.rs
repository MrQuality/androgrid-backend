//! Interactive Terminal QR Code and Pairing Payload Generator
//!
//! Generates high-contrast ANSI UTF-8 block QR codes for terminal rendering,
//! exports standalone PNG images, and serializes compact enrollment payloads.
//!
//! Traces to: AG-SPEC-2026-V2 Section 2.3, [INV-002], [NFR-SEC-001], [REQ-OP-001]

use qrcode::render::unicode::Dense1x2;
use qrcode::QrCode;
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

/// Errors arising during QR code generation, rendering, or file export.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum QrError {
    #[error("Failed to construct QR code matrix: {0}")]
    MatrixConstruction(String),

    #[error("Failed to serialize pairing payload to JSON: {0}")]
    Serialization(String),

    #[error("Failed to save QR code image file to {path}: {detail}")]
    ImageSave { path: String, detail: String },
}

/// Strips `https://` or `http://` URI scheme prefixes and trailing slashes from an endpoint string
/// so mobile gRPC channel builders receive a canonical `host:port` target.
///
/// Traces to: [REQ-OP-001], [INV-002]
pub fn strip_uri_scheme(endpoint: &str) -> String {
    let trimmed = endpoint.trim();
    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    without_scheme.trim_end_matches('/').to_string()
}

/// Compact device pairing payload scanned by the Android worker.
///
/// Traces to: [INV-002], [NFR-SEC-001], [REQ-OP-001]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingPayload {
    /// Control plane gRPC enrollment endpoint in canonical `host:port` format (no URI scheme).
    pub cluster_endpoint: String,

    /// Lowercase hexadecimal SHA-256 fingerprint of the cluster Root CA certificate.
    pub ca_fingerprint: String,

    /// Single-use cryptographic ephemeral pairing token.
    pub pairing_token: String,

    /// Pre-assigned node identifier (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,

    /// Expiration timestamp in epoch seconds.
    pub expires_at_epoch_s: u64,
}

impl PairingPayload {
    /// Serializes the pairing payload to a compact JSON string, normalizing `cluster_endpoint`
    /// to canonical `host:port` format without URI scheme prefixes.
    pub fn to_compact_json(&self) -> Result<String, QrError> {
        let normalized = PairingPayload {
            cluster_endpoint: strip_uri_scheme(&self.cluster_endpoint),
            ca_fingerprint: self.ca_fingerprint.clone(),
            pairing_token: self.pairing_token.clone(),
            node_id: self.node_id.clone(),
            expires_at_epoch_s: self.expires_at_epoch_s,
        };
        serde_json::to_string(&normalized).map_err(|e| QrError::Serialization(e.to_string()))
    }

    /// Deserializes a pairing payload from a JSON string.
    pub fn from_json(json: &str) -> Result<Self, QrError> {
        serde_json::from_str(json).map_err(|e| QrError::Serialization(e.to_string()))
    }
}

/// Renders a high-contrast ANSI UTF-8 QR code string suitable for terminal display.
///
/// Uses half-block Unicode characters (`Dense1x2`) to render two vertically stacked
/// QR modules per character, ensuring a square aspect ratio in standard terminals.
///
/// Traces to: [REQ-OP-001]
///
/// # Arguments
/// * `content` - String to encode into the QR code matrix.
///
/// # Returns
/// * `Ok(String)` containing multi-line UTF-8 string with quiet zone.
/// * `Err(QrError)` if content exceeds QR matrix capacity.
pub fn render_ansi_qr(content: &str) -> Result<String, QrError> {
    let code =
        QrCode::new(content.as_bytes()).map_err(|e| QrError::MatrixConstruction(e.to_string()))?;

    let rendered = code
        .render::<Dense1x2>()
        .dark_color(Dense1x2::Light)
        .light_color(Dense1x2::Dark)
        .build();

    Ok(rendered)
}

/// Exports the QR code as a PNG image file.
///
/// Traces to: [REQ-OP-001]
///
/// # Arguments
/// * `content` - String to encode into the QR code matrix.
/// * `output_path` - Path where the PNG image should be saved.
///
/// # Returns
/// * `Ok(())` on successful file write.
/// * `Err(QrError)` on failure.
pub fn export_png(content: &str, output_path: &Path) -> Result<(), QrError> {
    let code =
        QrCode::new(content.as_bytes()).map_err(|e| QrError::MatrixConstruction(e.to_string()))?;

    let image = code
        .render::<image::Luma<u8>>()
        .min_dimensions(300, 300)
        .build();

    image.save(output_path).map_err(|e| QrError::ImageSave {
        path: output_path.display().to_string(),
        detail: e.to_string(),
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pairing_payload_json_round_trip_strips_uri_scheme() {
        let payload = PairingPayload {
            cluster_endpoint: "https://192.168.1.50:50051".to_string(),
            ca_fingerprint: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                .to_string(),
            pairing_token: "abcd1234efgh5678".to_string(),
            node_id: Some("worker-node-alpha".to_string()),
            expires_at_epoch_s: 1700000300,
        };

        let json = payload
            .to_compact_json()
            .expect("Serialization must succeed");
        assert!(json.contains("cluster_endpoint"));
        assert!(!json.contains("https://"));
        assert!(json.contains("192.168.1.50:50051"));
        assert!(json.contains("ca_fingerprint"));
        assert!(json.contains("pairing_token"));
        assert!(json.contains("worker-node-alpha"));

        let deserialized = PairingPayload::from_json(&json).expect("Deserialization must succeed");
        assert_eq!(deserialized.cluster_endpoint, "192.168.1.50:50051");
        assert_eq!(deserialized.pairing_token, payload.pairing_token);
    }

    #[test]
    fn test_strip_uri_scheme_normalizes_endpoints() {
        assert_eq!(
            strip_uri_scheme("https://10.0.2.2:50051/"),
            "10.0.2.2:50051"
        );
        assert_eq!(
            strip_uri_scheme("http://127.0.0.1:50051"),
            "127.0.0.1:50051"
        );
        assert_eq!(
            strip_uri_scheme("  192.168.1.10:50051  "),
            "192.168.1.10:50051"
        );
    }

    #[test]
    fn test_render_ansi_qr_non_empty() {
        let content = "https://127.0.0.1:50051?token=test";
        let qr = render_ansi_qr(content).expect("QR rendering must succeed");

        assert!(!qr.is_empty());
        // Verify output contains block characters
        assert!(qr.contains('\n'));
    }

    #[test]
    fn test_export_png_file_creation() {
        let content = "androgrid-pairing-test-payload";
        let temp_dir = std::env::temp_dir();
        let png_path = temp_dir.join("androgrid_test_qr.png");

        let res = export_png(content, &png_path);
        assert!(res.is_ok(), "PNG export should succeed: {:?}", res);
        assert!(png_path.exists());

        // Verify file is a valid non-empty PNG
        let bytes = std::fs::read(&png_path).expect("Read exported PNG");
        assert!(bytes.len() > 100);
        // PNG magic number: 0x89 0x50 0x4E 0x47 0x0D 0x0A 0x1A 0x0A
        assert_eq!(&bytes[..4], &[0x89, 0x50, 0x4E, 0x47]);

        let _ = std::fs::remove_file(&png_path);
    }
}
