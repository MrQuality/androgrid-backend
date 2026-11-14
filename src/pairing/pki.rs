//! Native Pure-Rust Cluster PKI Lifecycle Engine (`androgrid-cli cert`)
//!
//! Eliminates external `openssl` and `bash` dependencies by generating ECDSA P-256
//! X.509 v3 Root CA, orchestrator server fullchain, operator admin mTLS certificates,
//! and offline CSR-signed worker certificates using `rcgen` and `CaSigner`.
//!
//! Traces to: AG-SPEC-2026-V2 Section 2.3, ADR-001, [INV-002], [NFR-SEC-001], [REQ-OP-001]

use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TMP_FILE_SEQ: AtomicU64 = AtomicU64::new(0);

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose,
    Ia5String, IsCa, KeyPair, KeyUsagePurpose, SanType,
};
use serde::Serialize;
use thiserror::Error;
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};

use crate::pairing::signer::{CaSigner, SignerError};

/// Default Subject Alternative Names embedded in orchestrator server certificates
/// for local, Docker, and Android Emulator loopback topologies.
pub const DEFAULT_SERVER_SANS: &[&str] = &[
    "localhost",
    "127.0.0.1",
    "::1",
    "10.0.2.2",
    "host.docker.internal",
];

/// Errors arising during native PKI lifecycle initialization or certificate issuance.
#[derive(Debug, Error)]
pub enum PkiError {
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),

    #[error("Refusing to overwrite existing PKI artifact at {path}; pass --force to overwrite")]
    AlreadyExists { path: String },

    #[error("Could not locate CA certificate and key in {0}")]
    CaNotFound(String),

    #[error("I/O error accessing {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("Certificate generation error: {0}")]
    Rcgen(#[from] rcgen::Error),

    #[error("CA signer error: {0}")]
    Signer(#[from] SignerError),
}

/// Resolved input location for loading an existing Root CA certificate and key.
#[derive(Debug, Clone)]
pub enum CaSource {
    /// Directory containing either `runtime/ca.crt` + `operator/ca.key` or flat `ca.crt` + `ca.key`.
    Dir(PathBuf),
    /// Explicit PEM file paths for CA certificate and CA private key.
    Files { ca_cert: PathBuf, ca_key: PathBuf },
}

/// Parameters for Day-0 cluster PKI initialization (`cert init`).
#[derive(Debug, Clone)]
pub struct PkiInitConfig {
    pub out_dir: PathBuf,
    pub extra_sans: Vec<String>,
    pub no_default_sans: bool,
    pub ca_days: u32,
    pub days: u32,
    pub admin_cn: String,
    pub force: bool,
}

/// Parameters for issuing or rotating an orchestrator server certificate (`cert issue server`).
#[derive(Debug, Clone)]
pub struct PkiIssueServerConfig {
    pub ca_source: CaSource,
    pub out_dir: PathBuf,
    pub extra_sans: Vec<String>,
    pub no_default_sans: bool,
    pub days: u32,
    pub force: bool,
}

/// Parameters for issuing or rotating an operator admin certificate (`cert issue admin`).
#[derive(Debug, Clone)]
pub struct PkiIssueAdminConfig {
    pub ca_source: CaSource,
    pub out_dir: PathBuf,
    pub cn: String,
    pub days: u32,
    pub force: bool,
}

/// Parameters for offline worker CSR signing (`cert issue csr`).
#[derive(Debug, Clone)]
pub struct PkiIssueCsrConfig {
    pub ca_source: CaSource,
    pub csr_path: PathBuf,
    pub node_id: String,
    pub out_path: PathBuf,
    pub ca_out_path: Option<PathBuf>,
    pub days: u32,
    pub force: bool,
}

/// Structured summary returned by `cert init` and `cert issue` subcommands.
///
/// Never includes private key material.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PkiOperationSummary {
    pub action: String,
    pub ca_fingerprint: String,
    pub files: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sans: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    pub days_valid: u32,
    pub not_before: String,
    pub not_after: String,
}

/// Initializes a fresh cluster PKI hierarchy in `<out_dir>/runtime` and `<out_dir>/operator`.
///
/// Traces to: ADR-001, [INV-002], [NFR-SEC-001]
pub fn cert_init(config: &PkiInitConfig) -> Result<PkiOperationSummary, PkiError> {
    validate_days(config.ca_days, "ca-days")?;
    validate_days(config.days, "days")?;
    validate_non_empty_cn(&config.admin_cn, "admin-cn")?;
    build_server_sans(&config.extra_sans, config.no_default_sans)?;

    let runtime_dir = config.out_dir.join("runtime");
    let operator_dir = config.out_dir.join("operator");

    let ca_cert_path = runtime_dir.join("ca.crt");
    let ca_key_path = operator_dir.join("ca.key");
    let server_cert_path = runtime_dir.join("server.crt");
    let server_key_path = runtime_dir.join("server.key");
    let admin_cert_path = operator_dir.join("admin.crt");
    let admin_key_path = operator_dir.join("admin.key");

    if !config.force {
        for protected in [
            &ca_cert_path,
            &ca_key_path,
            &server_cert_path,
            &server_key_path,
            &admin_cert_path,
            &admin_key_path,
        ] {
            if protected.exists() {
                return Err(PkiError::AlreadyExists {
                    path: protected.display().to_string(),
                });
            }
        }
    }

    fs::create_dir_all(&runtime_dir).map_err(|e| PkiError::Io {
        path: runtime_dir.display().to_string(),
        source: e,
    })?;
    fs::create_dir_all(&operator_dir).map_err(|e| PkiError::Io {
        path: operator_dir.display().to_string(),
        source: e,
    })?;

    // 1. Generate Internal Root CA (ECDSA P-256, PKCS#8 PEM)
    let (ca_not_before, ca_not_after, _, _) = build_validity_window(config.ca_days)?;
    let mut ca_params = CertificateParams::default();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
    ];
    ca_params.not_before = ca_not_before;
    ca_params.not_after = ca_not_after;
    let mut ca_dn = DistinguishedName::new();
    ca_dn.push(DnType::CommonName, "AndroGrid Internal Root CA");
    ca_dn.push(DnType::OrganizationName, "AndroGrid Cluster");
    ca_dn.push(DnType::OrganizationalUnitName, "Infrastructure");
    ca_params.distinguished_name = ca_dn;

    let ca_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)?;
    let ca_cert = ca_params.self_signed(&ca_key)?;
    let ca_cert_pem = ca_cert.pem();
    let ca_key_pem = ca_key.serialize_pem();

    write_file_atomic(&ca_cert_path, ca_cert_pem.as_bytes(), false, true)?;
    write_file_atomic(&ca_key_path, ca_key_pem.as_bytes(), true, true)?;

    let signer = CaSigner::from_pem(&ca_cert_pem, &ca_key_pem)?;

    // 2. Generate Orchestrator Server Leaf + Fullchain in runtime/
    let server_res = issue_server_with_signer(
        &signer,
        &runtime_dir,
        &config.extra_sans,
        config.no_default_sans,
        config.days,
        true,
    )?;

    // 3. Generate Operator Admin Client Cert in operator/
    let admin_res =
        issue_admin_with_signer(&signer, &operator_dir, &config.admin_cn, config.days, true)?;

    let mut files = vec![
        ca_cert_path.display().to_string(),
        ca_key_path.display().to_string(),
    ];
    files.extend(server_res.files);
    files.extend(admin_res.files);

    Ok(PkiOperationSummary {
        action: "cert init".to_string(),
        ca_fingerprint: signer.ca_fingerprint().to_string(),
        files,
        sans: server_res.sans,
        cn: Some(config.admin_cn.clone()),
        node_id: None,
        days_valid: config.days,
        not_before: server_res.not_before,
        not_after: server_res.not_after,
    })
}

/// Issues or rotates an orchestrator server certificate using an existing Root CA.
///
/// Traces to: ADR-001, [INV-002], [NFR-SEC-001]
pub fn cert_issue_server(config: &PkiIssueServerConfig) -> Result<PkiOperationSummary, PkiError> {
    validate_days(config.days, "days")?;
    let signer = load_ca_signer(&config.ca_source)?;
    issue_server_with_signer(
        &signer,
        &config.out_dir,
        &config.extra_sans,
        config.no_default_sans,
        config.days,
        config.force,
    )
}

/// Issues or rotates an operator admin mTLS certificate (`OU=Control Plane`).
///
/// Traces to: ADR-001, [INV-002], [NFR-SEC-001]
pub fn cert_issue_admin(config: &PkiIssueAdminConfig) -> Result<PkiOperationSummary, PkiError> {
    validate_days(config.days, "days")?;
    validate_non_empty_cn(&config.cn, "cn")?;
    let signer = load_ca_signer(&config.ca_source)?;
    issue_admin_with_signer(
        &signer,
        &config.out_dir,
        &config.cn,
        config.days,
        config.force,
    )
}

/// Signs a worker PKCS#10 CSR offline using the same hardened policy as online enrollment.
///
/// Traces to: [INV-002], [NFR-SEC-001], [REQ-OP-001]
pub fn cert_issue_csr(config: &PkiIssueCsrConfig) -> Result<PkiOperationSummary, PkiError> {
    validate_days(config.days, "days")?;
    validate_node_id(&config.node_id)?;

    if !config.force && config.out_path.exists() {
        return Err(PkiError::AlreadyExists {
            path: config.out_path.display().to_string(),
        });
    }
    if let Some(ref ca_out) = config.ca_out_path {
        if !config.force && ca_out.exists() {
            return Err(PkiError::AlreadyExists {
                path: ca_out.display().to_string(),
            });
        }
    }

    let csr_pem = fs::read_to_string(&config.csr_path).map_err(|e| PkiError::Io {
        path: config.csr_path.display().to_string(),
        source: e,
    })?;

    let signer = load_ca_signer(&config.ca_source)?;
    let (_, _, not_before_str, not_after_str) = build_validity_window(config.days)?;
    let signed = signer.sign_worker_csr_with_days(&csr_pem, &config.node_id, config.days)?;

    if let Some(parent) = config.out_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| PkiError::Io {
                path: parent.display().to_string(),
                source: e,
            })?;
        }
    }
    write_file_atomic(
        &config.out_path,
        signed.client_cert_pem.as_bytes(),
        false,
        true,
    )?;

    let mut files = vec![config.out_path.display().to_string()];
    if let Some(ref ca_out) = config.ca_out_path {
        if let Some(parent) = ca_out.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|e| PkiError::Io {
                    path: parent.display().to_string(),
                    source: e,
                })?;
            }
        }
        let ca_content = format!("{}\n", signed.ca_cert_pem.trim());
        write_file_atomic(ca_out, ca_content.as_bytes(), false, true)?;
        files.push(ca_out.display().to_string());
    }

    Ok(PkiOperationSummary {
        action: "cert issue csr".to_string(),
        ca_fingerprint: signer.ca_fingerprint().to_string(),
        files,
        sans: Vec::new(),
        cn: Some(config.node_id.clone()),
        node_id: Some(config.node_id.clone()),
        days_valid: config.days,
        not_before: not_before_str,
        not_after: not_after_str,
    })
}

fn issue_server_with_signer(
    signer: &CaSigner,
    out_dir: &Path,
    extra_sans: &[String],
    no_default_sans: bool,
    days: u32,
    force: bool,
) -> Result<PkiOperationSummary, PkiError> {
    let cert_path = out_dir.join("server.crt");
    let key_path = out_dir.join("server.key");

    if !force {
        for p in [&cert_path, &key_path] {
            if p.exists() {
                return Err(PkiError::AlreadyExists {
                    path: p.display().to_string(),
                });
            }
        }
    }

    fs::create_dir_all(out_dir).map_err(|e| PkiError::Io {
        path: out_dir.display().to_string(),
        source: e,
    })?;

    let (san_types, san_strings) = build_server_sans(extra_sans, no_default_sans)?;
    let (not_before, not_after, not_before_str, not_after_str) = build_validity_window(days)?;

    let mut params = CertificateParams::default();
    params.is_ca = IsCa::NoCa;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.subject_alt_names = san_types;
    params.not_before = not_before;
    params.not_after = not_after;

    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "localhost");
    dn.push(DnType::OrganizationName, "AndroGrid Cluster");
    dn.push(DnType::OrganizationalUnitName, "Orchestrator");
    params.distinguished_name = dn;

    let server_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)?;
    let server_cert = signer.sign_leaf_params(params, &server_key)?;

    // Fullchain PEM: leaf certificate followed by Root CA certificate for mobile pinning
    let fullchain_pem = format!(
        "{}\n{}\n",
        server_cert.pem().trim(),
        signer.ca_cert_pem().trim()
    );
    let key_pem = server_key.serialize_pem();

    write_file_atomic(&key_path, key_pem.as_bytes(), true, true)?;
    write_file_atomic(&cert_path, fullchain_pem.as_bytes(), false, true)?;

    Ok(PkiOperationSummary {
        action: "cert issue server".to_string(),
        ca_fingerprint: signer.ca_fingerprint().to_string(),
        files: vec![
            cert_path.display().to_string(),
            key_path.display().to_string(),
        ],
        sans: san_strings,
        cn: Some("localhost".to_string()),
        node_id: None,
        days_valid: days,
        not_before: not_before_str,
        not_after: not_after_str,
    })
}

fn issue_admin_with_signer(
    signer: &CaSigner,
    out_dir: &Path,
    cn: &str,
    days: u32,
    force: bool,
) -> Result<PkiOperationSummary, PkiError> {
    let cert_path = out_dir.join("admin.crt");
    let key_path = out_dir.join("admin.key");

    if !force {
        for p in [&cert_path, &key_path] {
            if p.exists() {
                return Err(PkiError::AlreadyExists {
                    path: p.display().to_string(),
                });
            }
        }
    }

    fs::create_dir_all(out_dir).map_err(|e| PkiError::Io {
        path: out_dir.display().to_string(),
        source: e,
    })?;

    let (not_before, not_after, not_before_str, not_after_str) = build_validity_window(days)?;

    let mut params = CertificateParams::default();
    params.is_ca = IsCa::NoCa;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    params.not_before = not_before;
    params.not_after = not_after;

    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, cn);
    dn.push(DnType::OrganizationName, "AndroGrid Cluster");
    dn.push(DnType::OrganizationalUnitName, "Control Plane");
    params.distinguished_name = dn;

    let admin_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)?;
    let admin_cert = signer.sign_leaf_params(params, &admin_key)?;

    write_file_atomic(&key_path, admin_key.serialize_pem().as_bytes(), true, true)?;
    write_file_atomic(&cert_path, admin_cert.pem().as_bytes(), false, true)?;

    Ok(PkiOperationSummary {
        action: "cert issue admin".to_string(),
        ca_fingerprint: signer.ca_fingerprint().to_string(),
        files: vec![
            cert_path.display().to_string(),
            key_path.display().to_string(),
        ],
        sans: Vec::new(),
        cn: Some(cn.to_string()),
        node_id: None,
        days_valid: days,
        not_before: not_before_str,
        not_after: not_after_str,
    })
}

fn load_ca_signer(source: &CaSource) -> Result<CaSigner, PkiError> {
    let (ca_cert_path, ca_key_path) = match source {
        CaSource::Files { ca_cert, ca_key } => (ca_cert.clone(), ca_key.clone()),
        CaSource::Dir(dir) => {
            let split_cert = dir.join("runtime").join("ca.crt");
            let split_key = dir.join("operator").join("ca.key");
            if split_cert.exists() && split_key.exists() {
                (split_cert, split_key)
            } else {
                let flat_cert = dir.join("ca.crt");
                let flat_key = dir.join("ca.key");
                if flat_cert.exists() && flat_key.exists() {
                    (flat_cert, flat_key)
                } else {
                    return Err(PkiError::CaNotFound(dir.display().to_string()));
                }
            }
        }
    };

    let ca_cert_pem = fs::read_to_string(&ca_cert_path).map_err(|e| PkiError::Io {
        path: ca_cert_path.display().to_string(),
        source: e,
    })?;
    let ca_key_pem = fs::read_to_string(&ca_key_path).map_err(|e| PkiError::Io {
        path: ca_key_path.display().to_string(),
        source: e,
    })?;

    Ok(CaSigner::from_pem(&ca_cert_pem, &ca_key_pem)?)
}

fn build_server_sans(
    extra_sans: &[String],
    no_default_sans: bool,
) -> Result<(Vec<SanType>, Vec<String>), PkiError> {
    let mut unique = BTreeSet::new();
    let mut ordered = Vec::new();

    if !no_default_sans {
        for s in DEFAULT_SERVER_SANS {
            if unique.insert((*s).to_string()) {
                ordered.push((*s).to_string());
            }
        }
    }

    for raw in extra_sans {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.chars().any(char::is_whitespace) {
            return Err(PkiError::InvalidArgument(format!(
                "Invalid SAN entry '{raw}': must be a non-empty DNS name or IP address"
            )));
        }
        if unique.insert(trimmed.to_string()) {
            ordered.push(trimmed.to_string());
        }
    }

    if ordered.is_empty() {
        return Err(PkiError::InvalidArgument(
            "Server certificate requires at least one SAN when --no-default-sans is set"
                .to_string(),
        ));
    }

    let mut san_types = Vec::with_capacity(ordered.len());
    for entry in &ordered {
        if let Ok(ip) = entry.parse::<IpAddr>() {
            san_types.push(SanType::IpAddress(ip));
        } else {
            validate_dns_san(entry)?;
            let dns = Ia5String::try_from(entry.as_str()).map_err(|e| {
                PkiError::InvalidArgument(format!("Invalid DNS SAN '{entry}': {e}"))
            })?;
            san_types.push(SanType::DnsName(dns));
        }
    }

    Ok((san_types, ordered))
}

fn validate_dns_san(entry: &str) -> Result<(), PkiError> {
    if entry.contains("://")
        || entry.contains('/')
        || entry.contains(':')
        || entry.chars().any(char::is_whitespace)
    {
        return Err(PkiError::InvalidArgument(format!(
            "Invalid DNS SAN '{entry}': expected a bare hostname or IP address without scheme, port, path, or whitespace"
        )));
    }

    let host = entry.strip_prefix("*.").unwrap_or(entry);
    if host.is_empty() || host.len() > 253 {
        return Err(PkiError::InvalidArgument(format!(
            "Invalid DNS SAN '{entry}': hostname length must be between 1 and 253 characters"
        )));
    }

    let labels: Vec<&str> = host.split('.').collect();
    if labels
        .iter()
        .all(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(PkiError::InvalidArgument(format!(
            "Invalid SAN '{entry}': appears to be a malformed IP address"
        )));
    }

    for label in labels {
        let bytes = label.as_bytes();
        if bytes.is_empty()
            || bytes.len() > 63
            || bytes.first() == Some(&b'-')
            || bytes.last() == Some(&b'-')
            || !bytes
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || *b == b'-')
        {
            return Err(PkiError::InvalidArgument(format!(
                "Invalid DNS SAN '{entry}': labels must contain only ASCII letters, digits, or internal hyphens"
            )));
        }
    }

    Ok(())
}

fn build_validity_window(
    days: u32,
) -> Result<(OffsetDateTime, OffsetDateTime, String, String), PkiError> {
    let now = OffsetDateTime::now_utc();
    // 5-minute backdate absorbs minor NTP clock skew between host and mobile nodes
    let not_before = now - Duration::minutes(5);
    let not_after = now + Duration::days(i64::from(days));

    let not_before_str = not_before
        .format(&Rfc3339)
        .map_err(|e| PkiError::InvalidArgument(e.to_string()))?;
    let not_after_str = not_after
        .format(&Rfc3339)
        .map_err(|e| PkiError::InvalidArgument(e.to_string()))?;

    Ok((not_before, not_after, not_before_str, not_after_str))
}

fn validate_days(days: u32, flag_name: &str) -> Result<(), PkiError> {
    if days == 0 {
        return Err(PkiError::InvalidArgument(format!(
            "--{flag_name} must be greater than 0"
        )));
    }
    Ok(())
}

fn validate_non_empty_cn(value: &str, flag_name: &str) -> Result<(), PkiError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_control) {
        return Err(PkiError::InvalidArgument(format!(
            "--{flag_name} must be non-empty and contain no control characters"
        )));
    }
    Ok(())
}

fn validate_node_id(node_id: &str) -> Result<(), PkiError> {
    if node_id.trim().is_empty() || node_id.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(PkiError::InvalidArgument(format!(
            "Invalid --node-id '{node_id}': must be non-empty without whitespace or control characters"
        )));
    }
    Ok(())
}

fn write_file_atomic(
    target: &Path,
    contents: &[u8],
    is_secret: bool,
    overwrite: bool,
) -> Result<(), PkiError> {
    if !overwrite && target.exists() {
        return Err(PkiError::AlreadyExists {
            path: target.display().to_string(),
        });
    }

    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let file_name = target
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("artifact");
    let seq = TMP_FILE_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp_path = parent.join(format!(".{file_name}.tmp.{}.{seq}", std::process::id()));

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(if is_secret { 0o600 } else { 0o644 });
    }
    #[cfg(not(unix))]
    let _ = is_secret;

    {
        let mut file = options.open(&tmp_path).map_err(|e| PkiError::Io {
            path: tmp_path.display().to_string(),
            source: e,
        })?;
        file.write_all(contents).map_err(|e| PkiError::Io {
            path: tmp_path.display().to_string(),
            source: e,
        })?;
        file.sync_all().map_err(|e| PkiError::Io {
            path: tmp_path.display().to_string(),
            source: e,
        })?;
    }

    fs::rename(&tmp_path, target).map_err(|e| {
        let _ = fs::remove_file(&tmp_path);
        PkiError::Io {
            path: target.display().to_string(),
            source: e,
        }
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::tls::{build_client_mtls_config, build_server_mtls_config};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_test_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "androgrid_pki_{prefix}_{}_{}",
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_cert_init_creates_split_layout_and_valid_fullchain() {
        let dir = temp_test_dir("init");
        let summary = cert_init(&PkiInitConfig {
            out_dir: dir.clone(),
            extra_sans: vec!["192.168.1.50".to_string(), "cluster.local".to_string()],
            no_default_sans: false,
            ca_days: 1825,
            days: 365,
            admin_cn: "operator".to_string(),
            force: false,
        })
        .expect("cert init should succeed");

        assert_eq!(summary.ca_fingerprint.len(), 64);
        assert!(dir.join("runtime/ca.crt").exists());
        assert!(dir.join("runtime/server.crt").exists());
        assert!(dir.join("runtime/server.key").exists());
        assert!(dir.join("operator/ca.key").exists());
        assert!(dir.join("operator/admin.crt").exists());
        assert!(dir.join("operator/admin.key").exists());

        // Fullchain server.crt must contain 2 certificates: leaf + Root CA
        let server_pem = fs::read_to_string(dir.join("runtime/server.crt")).unwrap();
        let server_blocks: Vec<_> = pem::parse_many(&server_pem)
            .unwrap()
            .into_iter()
            .filter(|p| p.tag() == "CERTIFICATE")
            .collect();
        assert_eq!(server_blocks.len(), 2);

        let (_, leaf) = x509_parser::parse_x509_certificate(server_blocks[0].contents()).unwrap();
        assert!(!leaf.is_ca());
        let ou = leaf
            .subject()
            .iter_organizational_unit()
            .next()
            .unwrap()
            .as_str()
            .unwrap();
        assert_eq!(ou, "Orchestrator");

        let san_ext = leaf.subject_alternative_name().unwrap().unwrap().value;
        assert!(san_ext.general_names.len() >= 7);

        // Verify admin cert carries OU=Control Plane
        let admin_pem = fs::read_to_string(dir.join("operator/admin.crt")).unwrap();
        let admin_der = pem::parse(&admin_pem).unwrap();
        let (_, admin_cert) = x509_parser::parse_x509_certificate(admin_der.contents()).unwrap();
        let admin_ou = admin_cert
            .subject()
            .iter_organizational_unit()
            .next()
            .unwrap()
            .as_str()
            .unwrap();
        assert_eq!(admin_ou, "Control Plane");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for key_rel in [
                "runtime/server.key",
                "operator/ca.key",
                "operator/admin.key",
            ] {
                let mode = fs::metadata(dir.join(key_rel))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777;
                assert_eq!(mode, 0o600, "Expected 0600 on {key_rel}");
            }
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_cert_init_refuses_overwrite_without_force() {
        let dir = temp_test_dir("force");
        let cfg = PkiInitConfig {
            out_dir: dir.clone(),
            extra_sans: vec![],
            no_default_sans: false,
            ca_days: 1825,
            days: 365,
            admin_cn: "operator".to_string(),
            force: false,
        };
        cert_init(&cfg).unwrap();

        let err = cert_init(&cfg).expect_err("Second init without --force must fail");
        match err {
            PkiError::AlreadyExists { .. } => {}
            other => panic!("Expected AlreadyExists, got {other:?}"),
        }

        let mut forced = cfg;
        forced.force = true;
        cert_init(&forced).expect("Init with --force should succeed");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_cert_issue_server_admin_and_csr_preserve_ca() {
        let dir = temp_test_dir("issue");
        let init_summary = cert_init(&PkiInitConfig {
            out_dir: dir.clone(),
            extra_sans: vec![],
            no_default_sans: false,
            ca_days: 1825,
            days: 365,
            admin_cn: "operator".to_string(),
            force: false,
        })
        .unwrap();

        let original_ca = fs::read_to_string(dir.join("runtime/ca.crt")).unwrap();
        let original_ca_key = fs::read_to_string(dir.join("operator/ca.key")).unwrap();

        // Issue rotated server cert with --no-default-sans
        let rotated_dir = dir.join("rotated_server");
        let srv_summary = cert_issue_server(&PkiIssueServerConfig {
            ca_source: CaSource::Dir(dir.clone()),
            out_dir: rotated_dir.clone(),
            extra_sans: vec!["prod.androgrid.example".to_string()],
            no_default_sans: true,
            days: 90,
            force: false,
        })
        .unwrap();
        assert_eq!(srv_summary.ca_fingerprint, init_summary.ca_fingerprint);
        assert_eq!(srv_summary.sans, vec!["prod.androgrid.example".to_string()]);

        // Issue rotated admin cert
        let adm_dir = dir.join("rotated_admin");
        let adm_summary = cert_issue_admin(&PkiIssueAdminConfig {
            ca_source: CaSource::Dir(dir.clone()),
            out_dir: adm_dir.clone(),
            cn: "ci-bot".to_string(),
            days: 30,
            force: false,
        })
        .unwrap();
        assert_eq!(adm_summary.cn.as_deref(), Some("ci-bot"));

        // Issue offline CSR worker cert
        let mut csr_params = CertificateParams::default();
        csr_params
            .distinguished_name
            .push(DnType::CommonName, "node_pixel8_offline");
        let worker_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let csr_pem = csr_params
            .serialize_request(&worker_key)
            .unwrap()
            .pem()
            .unwrap();
        let csr_path = dir.join("worker.csr");
        fs::write(&csr_path, csr_pem).unwrap();

        let worker_cert_out = dir.join("worker/client.crt");
        let worker_ca_out = dir.join("worker/ca.crt");
        let csr_summary = cert_issue_csr(&PkiIssueCsrConfig {
            ca_source: CaSource::Dir(dir.clone()),
            csr_path,
            node_id: "node_pixel8_offline".to_string(),
            out_path: worker_cert_out.clone(),
            ca_out_path: Some(worker_ca_out.clone()),
            days: 180,
            force: false,
        })
        .unwrap();
        assert_eq!(csr_summary.node_id.as_deref(), Some("node_pixel8_offline"));
        assert!(worker_cert_out.exists());
        assert!(worker_ca_out.exists());

        // CA files must remain untouched
        assert_eq!(
            fs::read_to_string(dir.join("runtime/ca.crt")).unwrap(),
            original_ca
        );
        assert_eq!(
            fs::read_to_string(dir.join("operator/ca.key")).unwrap(),
            original_ca_key
        );

        // Verify Tonic ServerTlsConfig and ClientTlsConfig accept generated PEMs cleanly
        let _server_tls = build_server_mtls_config(
            &fs::read(rotated_dir.join("server.crt")).unwrap(),
            &fs::read(rotated_dir.join("server.key")).unwrap(),
            original_ca.as_bytes(),
        );
        let _client_tls = build_client_mtls_config(
            &fs::read(adm_dir.join("admin.crt")).unwrap(),
            &fs::read(adm_dir.join("admin.key")).unwrap(),
            original_ca.as_bytes(),
            "prod.androgrid.example",
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validation_rejects_zero_days_empty_sans_and_bad_node_id() {
        let dir = temp_test_dir("invalid");
        assert!(cert_init(&PkiInitConfig {
            out_dir: dir.clone(),
            extra_sans: vec![],
            no_default_sans: false,
            ca_days: 0,
            days: 365,
            admin_cn: "operator".to_string(),
            force: false,
        })
        .is_err());

        assert!(build_server_sans(&[], true).is_err());
        assert!(validate_node_id("bad node id").is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_cert_init_preflight_leaves_no_partial_files_on_san_error() {
        let dir = temp_test_dir("preflight");
        let err = cert_init(&PkiInitConfig {
            out_dir: dir.clone(),
            extra_sans: vec![],
            no_default_sans: true,
            ca_days: 1825,
            days: 365,
            admin_cn: "operator".to_string(),
            force: false,
        })
        .expect_err("Missing SAN with --no-default-sans must fail preflight");
        assert!(matches!(err, PkiError::InvalidArgument(_)));
        assert!(!dir.join("runtime/ca.crt").exists());
        assert!(!dir.join("operator/ca.key").exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_build_server_sans_rejects_malformed_ips_and_urls() {
        for bad in [
            "192.168.1.999",
            "https://192.168.1.50:50051",
            "orchestrator:50051",
            "bad host",
            "*.",
            "-bad.example.com",
            "bad..example.com",
        ] {
            assert!(
                build_server_sans(&[bad.to_string()], true).is_err(),
                "Expected invalid SAN '{bad}' to be rejected"
            );
        }

        let (sans, ordered) = build_server_sans(
            &[
                "*.androgrid.example".to_string(),
                "orchestrator".to_string(),
                "10.0.2.2".to_string(),
            ],
            true,
        )
        .expect("Valid SANs must be accepted");
        assert_eq!(sans.len(), 3);
        assert_eq!(
            ordered,
            vec!["*.androgrid.example", "orchestrator", "10.0.2.2"]
        );
    }

    #[test]
    fn test_cert_issue_refuses_overwrite_without_force() {
        let dir = temp_test_dir("issue_overwrite");
        cert_init(&PkiInitConfig {
            out_dir: dir.clone(),
            extra_sans: vec![],
            no_default_sans: false,
            ca_days: 1825,
            days: 365,
            admin_cn: "operator".to_string(),
            force: false,
        })
        .unwrap();

        let srv_err = cert_issue_server(&PkiIssueServerConfig {
            ca_source: CaSource::Dir(dir.clone()),
            out_dir: dir.join("runtime"),
            extra_sans: vec![],
            no_default_sans: false,
            days: 90,
            force: false,
        })
        .expect_err("server overwrite without --force must fail");
        assert!(matches!(srv_err, PkiError::AlreadyExists { .. }));

        let adm_err = cert_issue_admin(&PkiIssueAdminConfig {
            ca_source: CaSource::Dir(dir.clone()),
            out_dir: dir.join("operator"),
            cn: "operator".to_string(),
            days: 90,
            force: false,
        })
        .expect_err("admin overwrite without --force must fail");
        assert!(matches!(adm_err, PkiError::AlreadyExists { .. }));

        let mut csr_params = CertificateParams::default();
        csr_params
            .distinguished_name
            .push(DnType::CommonName, "node_pixel_overwrite");
        let worker_key = KeyPair::generate().unwrap();
        let csr_pem = csr_params
            .serialize_request(&worker_key)
            .unwrap()
            .pem()
            .unwrap();
        let csr_path = dir.join("worker.csr");
        fs::write(&csr_path, csr_pem).unwrap();

        let out_cert = dir.join("worker.crt");
        let csr_cfg = PkiIssueCsrConfig {
            ca_source: CaSource::Dir(dir.clone()),
            csr_path,
            node_id: "node_pixel_overwrite".to_string(),
            out_path: out_cert,
            ca_out_path: None,
            days: 90,
            force: false,
        };
        cert_issue_csr(&csr_cfg).unwrap();
        let csr_err =
            cert_issue_csr(&csr_cfg).expect_err("csr cert overwrite without --force must fail");
        assert!(matches!(csr_err, PkiError::AlreadyExists { .. }));

        let mut forced_csr = csr_cfg;
        forced_csr.force = true;
        cert_issue_csr(&forced_csr).expect("csr cert overwrite with --force must succeed");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_generated_cert_validity_windows() {
        let dir = temp_test_dir("validity");
        cert_init(&PkiInitConfig {
            out_dir: dir.clone(),
            extra_sans: vec![],
            no_default_sans: false,
            ca_days: 1000,
            days: 120,
            admin_cn: "operator".to_string(),
            force: false,
        })
        .unwrap();

        let assert_span = |pem_str: &str, expected_days: i64| {
            let block = pem::parse_many(pem_str)
                .unwrap()
                .into_iter()
                .find(|p| p.tag() == "CERTIFICATE")
                .unwrap();
            let (_, cert) = x509_parser::parse_x509_certificate(block.contents()).unwrap();
            let validity = cert.validity();
            let span_secs = validity.not_after.timestamp() - validity.not_before.timestamp();
            // 5-minute backdate (300s) + expected_days * 86400
            let expected_secs = expected_days * 86400 + 300;
            assert!(
                (span_secs - expected_secs).abs() <= 5,
                "Expected ~{expected_secs}s validity span, got {span_secs}s"
            );
        };

        assert_span(
            &fs::read_to_string(dir.join("runtime/ca.crt")).unwrap(),
            1000,
        );
        assert_span(
            &fs::read_to_string(dir.join("runtime/server.crt")).unwrap(),
            120,
        );
        assert_span(
            &fs::read_to_string(dir.join("operator/admin.crt")).unwrap(),
            120,
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
