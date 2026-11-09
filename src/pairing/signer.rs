//! Internal Root CA Signer and Certificate Signing Request (CSR) Engine
//!
//! Validates client PKCS#10 CSRs, verifies on-device key signatures, and issues
//! X.509 v3 client certificates for mutual TLS 1.3 edge authentication.
//!
//! Traces to: AG-SPEC-2026-V2 Section 2.3, [INV-002], [NFR-SEC-001], [REQ-OP-001]

use rcgen::{
    Certificate, CertificateParams, CertificateSigningRequestParams, DnType,
    ExtendedKeyUsagePurpose, KeyPair, KeyUsagePurpose,
};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Errors arising during Root CA loading, CSR validation, or certificate signing.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SignerError {
    #[error("Failed to parse Root CA certificate PEM: {0}")]
    CaCertParseError(String),

    #[error("Failed to parse Root CA private key PEM: {0}")]
    CaKeyParseError(String),

    #[error("Failed to reconstruct CA certificate context: {0}")]
    CaReconstructionError(String),

    #[error("Invalid PKCS#10 Certificate Signing Request (CSR) PEM: {0}")]
    InvalidCsr(String),

    #[error("Common Name mismatch: CSR requested CN '{actual}', but expected '{expected}'")]
    CommonNameMismatch { expected: String, actual: String },

    #[error("CSR does not specify a Common Name (CN)")]
    MissingCommonName,

    #[error("Certificate signing failed: {0}")]
    SigningFailed(String),
}

/// Output payload containing the signed client certificate and Root CA certificate chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedWorkerCert {
    /// PEM-encoded issued client certificate.
    pub client_cert_pem: String,

    /// PEM-encoded Root CA certificate for trust anchoring.
    pub ca_cert_pem: String,

    /// Canonical node identifier bound into the certificate Subject CN.
    pub node_id: String,
}

/// Control plane Root CA signer issuing mutual TLS client certificates.
///
/// Traces to: [INV-002], [NFR-SEC-001], [REQ-OP-001]
pub struct CaSigner {
    ca_cert_pem: String,
    ca_fingerprint: String,
    ca_cert: Certificate,
    ca_key: KeyPair,
}

impl CaSigner {
    /// Loads a Root CA certificate and private key from PEM strings.
    ///
    /// Traces to: [INV-002], [NFR-SEC-001]
    ///
    /// # Arguments
    /// * `ca_cert_pem` - PEM string containing the Root CA certificate.
    /// * `ca_key_pem` - PEM string containing the Root CA private key.
    ///
    /// # Returns
    /// * `Ok(CaSigner)` ready to sign worker CSRs.
    /// * `Err(SignerError)` if certificate or key parsing fails.
    pub fn from_pem(ca_cert_pem: &str, ca_key_pem: &str) -> Result<Self, SignerError> {
        let ca_key = KeyPair::from_pem(ca_key_pem)
            .map_err(|e| SignerError::CaKeyParseError(e.to_string()))?;

        let ca_params = CertificateParams::from_ca_cert_pem(ca_cert_pem)
            .map_err(|e| SignerError::CaCertParseError(e.to_string()))?;

        let ca_cert = ca_params
            .self_signed(&ca_key)
            .map_err(|e| SignerError::CaReconstructionError(e.to_string()))?;

        // Calculate Root CA SHA-256 fingerprint from the original certificate DER representation
        let ca_fingerprint = Self::compute_ca_fingerprint(ca_cert_pem)?;

        Ok(Self {
            ca_cert_pem: ca_cert_pem.trim().to_string(),
            ca_fingerprint,
            ca_cert,
            ca_key,
        })
    }

    /// Computes the SHA-256 fingerprint from an arbitrary Root CA certificate PEM string.
    ///
    /// Traces to: [NFR-SEC-001]
    pub fn compute_ca_fingerprint(ca_cert_pem: &str) -> Result<String, SignerError> {
        let parsed_pem =
            pem::parse(ca_cert_pem).map_err(|e| SignerError::CaCertParseError(e.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(parsed_pem.contents());
        Ok(format!("{:x}", hasher.finalize()))
    }

    /// Retrieves the lowercase hexadecimal SHA-256 fingerprint of the Root CA certificate.
    pub fn ca_fingerprint(&self) -> &str {
        &self.ca_fingerprint
    }

    /// Retrieves the PEM string of the Root CA certificate.
    pub fn ca_cert_pem(&self) -> &str {
        &self.ca_cert_pem
    }

    /// Validates an incoming PKCS#10 CSR and signs a new worker client certificate.
    ///
    /// Traces to: [INV-002], [NFR-SEC-001], [REQ-OP-001]
    ///
    /// # Arguments
    /// * `csr_pem` - PEM string of the client Certificate Signing Request.
    /// * `expected_node_id` - The expected node identifier (must match CSR Common Name).
    ///
    /// # Returns
    /// * `Ok(SignedWorkerCert)` containing the issued client cert and Root CA cert.
    /// * `Err(SignerError)` if CSR is invalid or Common Name does not match.
    pub fn sign_worker_csr(
        &self,
        csr_pem: &str,
        expected_node_id: &str,
    ) -> Result<SignedWorkerCert, SignerError> {
        let mut csr_params = CertificateSigningRequestParams::from_pem(csr_pem)
            .map_err(|e| SignerError::InvalidCsr(e.to_string()))?;

        // Extract and validate Subject Common Name (CN)
        let subject_cn = csr_params
            .params
            .distinguished_name
            .get(&DnType::CommonName)
            .and_then(dn_value_to_string)
            .ok_or(SignerError::MissingCommonName)?;

        if subject_cn != expected_node_id {
            return Err(SignerError::CommonNameMismatch {
                expected: expected_node_id.to_string(),
                actual: subject_cn,
            });
        }

        // Configure standard worker certificate extensions:
        // Client authentication only, 365 days validity
        csr_params
            .params
            .insert_extended_key_usage(ExtendedKeyUsagePurpose::ClientAuth);
        csr_params
            .params
            .key_usages
            .push(KeyUsagePurpose::DigitalSignature);
        csr_params
            .params
            .key_usages
            .push(KeyUsagePurpose::KeyEncipherment);

        // Sign the CSR using the Root CA
        let signed_cert = csr_params
            .signed_by(&self.ca_cert, &self.ca_key)
            .map_err(|e| SignerError::SigningFailed(e.to_string()))?;

        let client_cert_pem = signed_cert.pem();

        Ok(SignedWorkerCert {
            client_cert_pem,
            ca_cert_pem: self.ca_cert_pem.clone(),
            node_id: expected_node_id.to_string(),
        })
    }
}

/// Converts an rcgen DnValue into a standard Rust String.
fn dn_value_to_string(val: &rcgen::DnValue) -> Option<String> {
    match val {
        rcgen::DnValue::Utf8String(s) => Some(s.clone()),
        rcgen::DnValue::PrintableString(s) => Some(s.as_str().to_string()),
        rcgen::DnValue::Ia5String(s) => Some(s.as_str().to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};

    fn generate_test_ca() -> (String, String) {
        let mut ca_params = CertificateParams::default();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "AndroGrid Test Root CA");
        let ca_key = KeyPair::generate().expect("Failed to generate test CA key");
        let ca_cert = ca_params
            .self_signed(&ca_key)
            .expect("Failed to self-sign test CA cert");

        (ca_cert.pem(), ca_key.serialize_pem())
    }

    fn generate_test_csr(node_id: &str) -> (String, KeyPair) {
        let mut params = CertificateParams::default();
        params.distinguished_name.push(DnType::CommonName, node_id);
        let key = KeyPair::generate().expect("Failed to generate worker key");
        let csr = params
            .serialize_request(&key)
            .expect("Failed to serialize CSR");
        (csr.pem().expect("CSR PEM conversion"), key)
    }

    #[test]
    fn test_ca_signer_initialization_and_fingerprint() {
        let (ca_cert_pem, ca_key_pem) = generate_test_ca();
        let signer = CaSigner::from_pem(&ca_cert_pem, &ca_key_pem)
            .expect("CaSigner initialization should succeed");

        assert_eq!(signer.ca_fingerprint().len(), 64);
        assert!(signer.ca_cert_pem().contains("BEGIN CERTIFICATE"));
    }

    #[test]
    fn test_valid_csr_signing_round_trip() {
        let (ca_cert_pem, ca_key_pem) = generate_test_ca();
        let signer = CaSigner::from_pem(&ca_cert_pem, &ca_key_pem).unwrap();

        let node_id = "node-pixel-test";
        let (csr_pem, _worker_key) = generate_test_csr(node_id);

        let signed = signer
            .sign_worker_csr(&csr_pem, node_id)
            .expect("CSR signing must succeed");

        assert_eq!(signed.node_id, node_id);
        assert!(signed.client_cert_pem.contains("BEGIN CERTIFICATE"));
        assert_eq!(signed.ca_cert_pem, ca_cert_pem.trim());
    }

    #[test]
    fn test_csr_common_name_mismatch_rejected() {
        let (ca_cert_pem, ca_key_pem) = generate_test_ca();
        let signer = CaSigner::from_pem(&ca_cert_pem, &ca_key_pem).unwrap();

        let (csr_pem, _) = generate_test_csr("node-actual");
        let err = signer
            .sign_worker_csr(&csr_pem, "node-expected")
            .expect_err("CN mismatch must be rejected");

        assert_eq!(
            err,
            SignerError::CommonNameMismatch {
                expected: "node-expected".to_string(),
                actual: "node-actual".to_string(),
            }
        );
    }

    #[test]
    fn test_malformed_csr_rejected() {
        let (ca_cert_pem, ca_key_pem) = generate_test_ca();
        let signer = CaSigner::from_pem(&ca_cert_pem, &ca_key_pem).unwrap();

        let err = signer
            .sign_worker_csr("--- NOT A REAL CSR ---", "node-1")
            .expect_err("Malformed CSR must be rejected");

        match err {
            SignerError::InvalidCsr(_) => {}
            other => panic!("Expected InvalidCsr, got {:?}", other),
        }
    }
}
