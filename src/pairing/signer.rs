//! Internal Root CA Signer and Certificate Signing Request (CSR) Engine
//!
//! Validates client PKCS#10 CSRs, verifies on-device key signatures, and issues
//! X.509 v3 client certificates for mutual TLS 1.3 edge authentication.
//!
//! Traces to: AG-SPEC-2026-V2 Section 2.3, [INV-002], [NFR-SEC-001], [REQ-OP-001]

use rcgen::{
    Certificate, CertificateParams, CertificateSigningRequestParams, DnType,
    ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
};
use sha2::{Digest, Sha256};
use thiserror::Error;
use time::{Duration, OffsetDateTime};

/// Errors arising during Root CA loading, CSR validation, or certificate signing.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SignerError {
    #[error("Failed to parse Root CA certificate PEM: {0}")]
    CaCertParseError(String),

    #[error("Failed to parse Root CA private key PEM: {0}")]
    CaKeyParseError(String),

    #[error("Failed to reconstruct CA certificate context: {0}")]
    CaReconstructionError(String),

    #[error("Certificate provided as Root CA is not a CA (BasicConstraints CA:TRUE required)")]
    NotACaCertificate,

    #[error("Expected a single Root CA certificate in PEM input, found {0}")]
    MultipleCertificatesInCaPem(usize),

    #[error("Root CA private key does not match the public key in the Root CA certificate")]
    CaKeyMismatch,

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
        let ca_cert_der = parse_single_cert_der(ca_cert_pem)?;

        let (_, parsed_ca) = x509_parser::parse_x509_certificate(&ca_cert_der)
            .map_err(|e| SignerError::CaCertParseError(e.to_string()))?;
        if !parsed_ca.is_ca() {
            return Err(SignerError::NotACaCertificate);
        }

        let ca_key = KeyPair::from_pem(ca_key_pem)
            .map_err(|e| SignerError::CaKeyParseError(e.to_string()))?;

        if parsed_ca.public_key().raw != ca_key.public_key_der() {
            return Err(SignerError::CaKeyMismatch);
        }

        let ca_params = CertificateParams::from_ca_cert_pem(ca_cert_pem)
            .map_err(|e| SignerError::CaCertParseError(e.to_string()))?;

        let ca_cert = ca_params
            .self_signed(&ca_key)
            .map_err(|e| SignerError::CaReconstructionError(e.to_string()))?;

        // Calculate Root CA SHA-256 fingerprint from the original certificate DER representation
        let mut hasher = Sha256::new();
        hasher.update(&ca_cert_der);
        let ca_fingerprint = format!("{:x}", hasher.finalize());

        Ok(Self {
            ca_cert_pem: ca_cert_pem.trim().to_string(),
            ca_fingerprint,
            ca_cert,
            ca_key,
        })
    }

    /// Signs arbitrary leaf [`CertificateParams`] using the loaded Root CA.
    pub(crate) fn sign_leaf_params(
        &self,
        params: CertificateParams,
        leaf_key: &KeyPair,
    ) -> Result<Certificate, SignerError> {
        params
            .signed_by(leaf_key, &self.ca_cert, &self.ca_key)
            .map_err(|e| SignerError::SigningFailed(e.to_string()))
    }

    /// Computes the SHA-256 fingerprint from an arbitrary Root CA certificate PEM string.
    ///
    /// Traces to: [NFR-SEC-001]
    pub fn compute_ca_fingerprint(ca_cert_pem: &str) -> Result<String, SignerError> {
        let ca_cert_der = parse_single_cert_der(ca_cert_pem)?;
        let mut hasher = Sha256::new();
        hasher.update(&ca_cert_der);
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
        self.sign_worker_csr_with_days(csr_pem, expected_node_id, 365)
    }

    /// Validates an incoming PKCS#10 CSR and signs a new worker client certificate
    /// with an explicit validity duration in days.
    ///
    /// Traces to: [INV-002], [NFR-SEC-001], [REQ-OP-001]
    pub fn sign_worker_csr_with_days(
        &self,
        csr_pem: &str,
        expected_node_id: &str,
        days_valid: u32,
    ) -> Result<SignedWorkerCert, SignerError> {
        if days_valid == 0 {
            return Err(SignerError::SigningFailed(
                "Certificate validity days must be greater than 0".to_string(),
            ));
        }

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

        // Sanitize worker certificate Subject DN so CSRs cannot inject privileged OU values.
        let mut sanitized_dn = rcgen::DistinguishedName::new();
        sanitized_dn.push(DnType::CommonName, expected_node_id);
        sanitized_dn.push(DnType::OrganizationName, "AndroGrid Worker");
        sanitized_dn.push(DnType::OrganizationalUnitName, "Edge Compute");
        csr_params.params.distinguished_name = sanitized_dn;

        // Explicitly reset untrusted CSR extensions to prevent privilege/SAN injection:
        // Client authentication only, ECDSA digitalSignature only, CA:FALSE, zero SANs.
        csr_params.params.is_ca = IsCa::NoCa;
        csr_params.params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        csr_params.params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        csr_params.params.subject_alt_names.clear();

        let now = OffsetDateTime::now_utc();
        csr_params.params.not_before = now - Duration::minutes(5);
        csr_params.params.not_after = now + Duration::days(i64::from(days_valid));

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

/// Parses a PEM string and returns the DER bytes of a single `CERTIFICATE` block.
fn parse_single_cert_der(ca_cert_pem: &str) -> Result<Vec<u8>, SignerError> {
    let cert_blocks: Vec<pem::Pem> = pem::parse_many(ca_cert_pem)
        .map_err(|e| SignerError::CaCertParseError(e.to_string()))?
        .into_iter()
        .filter(|p| p.tag() == "CERTIFICATE")
        .collect();

    if cert_blocks.is_empty() {
        return Err(SignerError::CaCertParseError(
            "No CERTIFICATE PEM block found".to_string(),
        ));
    }
    if cert_blocks.len() > 1 {
        return Err(SignerError::MultipleCertificatesInCaPem(cert_blocks.len()));
    }

    Ok(cert_blocks.into_iter().next().unwrap().into_contents())
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

    #[test]
    fn test_sign_worker_csr_strips_malicious_extensions_and_sans() {
        let (ca_cert_pem, ca_key_pem) = generate_test_ca();
        let signer = CaSigner::from_pem(&ca_cert_pem, &ca_key_pem).unwrap();

        let mut malicious_params =
            CertificateParams::new(vec!["evil.internal".to_string()]).unwrap();
        malicious_params
            .distinguished_name
            .push(DnType::CommonName, "node-malicious");
        malicious_params
            .distinguished_name
            .push(DnType::OrganizationalUnitName, "Control Plane");
        malicious_params.extended_key_usages = vec![
            ExtendedKeyUsagePurpose::ServerAuth,
            ExtendedKeyUsagePurpose::CodeSigning,
        ];
        let worker_key = KeyPair::generate().unwrap();
        let csr_pem = malicious_params
            .serialize_request(&worker_key)
            .unwrap()
            .pem()
            .unwrap();

        let signed = signer
            .sign_worker_csr(&csr_pem, "node-malicious")
            .expect("Sanitized signing should succeed");

        let parsed_pem = pem::parse(&signed.client_cert_pem).unwrap();
        let (_, cert) = x509_parser::parse_x509_certificate(parsed_pem.contents()).unwrap();

        assert!(!cert.is_ca(), "Signed worker cert must never be a CA");
        assert!(
            cert.subject_alternative_name().unwrap().is_none(),
            "SANs from untrusted CSR must be stripped"
        );
        let ou = cert
            .subject()
            .iter_organizational_unit()
            .next()
            .unwrap()
            .as_str()
            .unwrap();
        assert_eq!(ou, "Edge Compute");

        let eku = cert.extended_key_usage().unwrap().unwrap().value;
        assert!(eku.client_auth);
        assert!(!eku.server_auth);
        assert!(!eku.code_signing);

        let ku = cert.key_usage().unwrap().unwrap().value;
        assert!(ku.digital_signature());
        assert!(!ku.key_cert_sign());
    }

    #[test]
    fn test_ca_signer_rejects_non_ca_certificate() {
        let (ca_cert_pem, ca_key_pem) = generate_test_ca();
        let signer = CaSigner::from_pem(&ca_cert_pem, &ca_key_pem).unwrap();
        let (csr_pem, worker_key) = generate_test_csr("node-leaf");
        let signed = signer.sign_worker_csr(&csr_pem, "node-leaf").unwrap();

        let err = CaSigner::from_pem(&signed.client_cert_pem, &worker_key.serialize_pem())
            .err()
            .expect("Leaf cert must not be accepted as CA");
        assert_eq!(err, SignerError::NotACaCertificate);
    }

    #[test]
    fn test_ca_signer_rejects_multi_cert_fullchain() {
        let (ca_cert_pem, ca_key_pem) = generate_test_ca();
        let fullchain = format!("{}\n{}", ca_cert_pem, ca_cert_pem);
        let err = CaSigner::from_pem(&fullchain, &ca_key_pem)
            .err()
            .expect("Multi-cert PEM must be rejected as Root CA");
        assert_eq!(err, SignerError::MultipleCertificatesInCaPem(2));
    }

    #[test]
    fn test_ca_signer_rejects_mismatched_ca_key() {
        let (ca_cert_a, _) = generate_test_ca();
        let (_, ca_key_b) = generate_test_ca();
        let err = CaSigner::from_pem(&ca_cert_a, &ca_key_b)
            .err()
            .expect("Mismatched CA key must be rejected");
        assert_eq!(err, SignerError::CaKeyMismatch);
    }

    #[test]
    fn test_compute_ca_fingerprint_rejects_multi_cert_and_non_cert() {
        let (ca_cert_pem, ca_key_pem) = generate_test_ca();
        let fullchain = format!("{}\n{}", ca_cert_pem, ca_cert_pem);
        assert_eq!(
            CaSigner::compute_ca_fingerprint(&fullchain).unwrap_err(),
            SignerError::MultipleCertificatesInCaPem(2)
        );
        assert!(matches!(
            CaSigner::compute_ca_fingerprint(&ca_key_pem).unwrap_err(),
            SignerError::CaCertParseError(_)
        ));
    }
}
