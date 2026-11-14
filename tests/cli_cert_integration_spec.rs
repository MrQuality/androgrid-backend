//! Offline Integration Tests for `androgrid-cli cert` Subcommands
//!
//! Verifies that cluster PKI initialization and certificate issuance execute
//! completely offline without a running orchestrator, emit clean `--format json`
//! output without private key leaks, and enforce overwrite protection.
//!
//! Traces to: ADR-001, [INV-002], [NFR-SEC-001], [REQ-OP-001]

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use rcgen::{CertificateParams, DnType, KeyPair};

fn temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "androgrid_cli_cert_{name}_{}_{}",
        std::process::id(),
        nanos
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn test_cli_cert_offline_lifecycle_and_json_output() {
    let bin = env!("CARGO_BIN_EXE_androgrid-cli");
    let root = temp_dir("offline");
    let pki_dir = root.join("pki");

    // 1. Offline cert init with --format json (must succeed with no server running)
    let init_out = Command::new(bin)
        .args([
            "--format",
            "json",
            "cert",
            "init",
            "--out",
            pki_dir.to_str().unwrap(),
            "--san",
            "192.168.1.99",
        ])
        .output()
        .expect("Failed to execute androgrid-cli cert init");
    assert!(
        init_out.status.success(),
        "cert init failed: {}",
        String::from_utf8_lossy(&init_out.stderr)
    );
    let init_stdout = String::from_utf8_lossy(&init_out.stdout);
    assert!(
        !init_stdout.contains("PRIVATE KEY"),
        "Private key material must never appear in CLI output"
    );
    let init_json: serde_json::Value =
        serde_json::from_str(&init_stdout).expect("stdout must be valid JSON");
    assert_eq!(init_json["action"], "cert init");
    assert_eq!(init_json["ca_fingerprint"].as_str().unwrap().len(), 64);
    assert!(pki_dir.join("runtime/ca.crt").exists());
    assert!(pki_dir.join("runtime/server.crt").exists());
    assert!(pki_dir.join("runtime/server.key").exists());
    assert!(pki_dir.join("operator/ca.key").exists());
    assert!(pki_dir.join("operator/admin.crt").exists());
    assert!(pki_dir.join("operator/admin.key").exists());

    // 2. Re-running cert init without --force must fail
    let second_init = Command::new(bin)
        .args(["cert", "init", "--out", pki_dir.to_str().unwrap()])
        .output()
        .expect("Failed to run second cert init");
    assert!(
        !second_init.status.success(),
        "cert init without --force must refuse overwriting existing CA"
    );

    // 3. Offline cert issue server with --format json (restart warning must go to stderr, not stdout)
    let rot_srv_dir = root.join("rot_srv");
    let srv_out = Command::new(bin)
        .args([
            "--format",
            "json",
            "cert",
            "issue",
            "server",
            "--ca-dir",
            pki_dir.to_str().unwrap(),
            "--out",
            rot_srv_dir.to_str().unwrap(),
            "--no-default-sans",
            "--san",
            "edge.androgrid.test",
        ])
        .output()
        .expect("Failed to run cert issue server");
    assert!(
        srv_out.status.success(),
        "cert issue server failed: {}",
        String::from_utf8_lossy(&srv_out.stderr)
    );
    let srv_stdout = String::from_utf8_lossy(&srv_out.stdout);
    let srv_stderr = String::from_utf8_lossy(&srv_out.stderr);
    let srv_json: serde_json::Value =
        serde_json::from_str(&srv_stdout).expect("server stdout must be pure JSON");
    assert_eq!(srv_json["sans"], serde_json::json!(["edge.androgrid.test"]));
    assert!(
        srv_stderr.contains("Restart androgrid-backend"),
        "Restart warning should be printed to stderr in json mode"
    );

    // 4. Offline cert issue admin
    let rot_adm_dir = root.join("rot_adm");
    let adm_out = Command::new(bin)
        .args([
            "--format",
            "json",
            "cert",
            "issue",
            "admin",
            "--ca-dir",
            pki_dir.to_str().unwrap(),
            "--out",
            rot_adm_dir.to_str().unwrap(),
            "--cn",
            "ci-operator",
        ])
        .output()
        .expect("Failed to run cert issue admin");
    assert!(adm_out.status.success());
    let adm_json: serde_json::Value =
        serde_json::from_slice(&adm_out.stdout).expect("admin stdout must be pure JSON");
    assert_eq!(adm_json["cn"], "ci-operator");

    // 5. Offline cert issue csr
    let mut csr_params = CertificateParams::default();
    csr_params
        .distinguished_name
        .push(DnType::CommonName, "node_offline_01");
    let worker_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let csr_pem = csr_params
        .serialize_request(&worker_key)
        .unwrap()
        .pem()
        .unwrap();
    let csr_path = root.join("worker.csr");
    fs::write(&csr_path, csr_pem).unwrap();

    let worker_cert_out = root.join("worker/client.crt");
    let worker_ca_out = root.join("worker/ca.crt");
    let csr_out = Command::new(bin)
        .args([
            "--format",
            "json",
            "cert",
            "issue",
            "csr",
            "--csr",
            csr_path.to_str().unwrap(),
            "--node-id",
            "node_offline_01",
            "--ca-dir",
            pki_dir.to_str().unwrap(),
            "--out",
            worker_cert_out.to_str().unwrap(),
            "--ca-out",
            worker_ca_out.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run cert issue csr");
    assert!(
        csr_out.status.success(),
        "cert issue csr failed: {}",
        String::from_utf8_lossy(&csr_out.stderr)
    );
    let csr_json: serde_json::Value =
        serde_json::from_slice(&csr_out.stdout).expect("csr stdout must be pure JSON");
    assert_eq!(csr_json["node_id"], "node_offline_01");
    assert!(worker_cert_out.exists());
    assert!(worker_ca_out.exists());

    let _ = fs::remove_dir_all(&root);
}
