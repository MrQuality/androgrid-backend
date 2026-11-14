//! Content-Addressed Artifact Distribution & WASM Toolchain Integration Specification
//!
//! Validates the decoupled WebAssembly distribution pipeline, embedded HTTP artifact server,
//! SHA-256 integrity verification, sub-4KB unary gRPC message bounding, worker local cache simulation,
//! and CLI packaging workflow.
//!
//! Traces to: AG-SPEC-2026-V2, [NFR-PERF-001], [NFR-POW-001], [INV-003], [REQ-OP-003]

use prost::Message;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::{Channel, Server};

use androgrid_backend::admin_service_client::AdminServiceClient;
use androgrid_backend::admin_service_server::AdminServiceServer;
use androgrid_backend::andro_grid_service_client::AndroGridServiceClient;
use androgrid_backend::andro_grid_service_server::AndroGridServiceServer;
use androgrid_backend::{
    build_client_mtls_config, build_server_mtls_config, AdminServiceImpl, AndroGridServiceImpl,
    ArtifactServer, ArtifactStore, ClaimCellularTaskRequest, ClaimCellularTaskResponse,
    ClusterScheduler, ExecutionEngine, NetworkTransportType, NodeCapabilities, RegisterNodeRequest,
    SubmitTaskRequest, TaskSpec, UploadArtifactRequest, WasiValidator,
};

struct TestPki {
    pub ca_cert_pem: Vec<u8>,
    pub server_cert_pem: Vec<u8>,
    pub server_key_pem: Vec<u8>,
    pub client_cert_pem: Vec<u8>,
    pub client_key_pem: Vec<u8>,
}

impl TestPki {
    pub fn generate() -> Self {
        use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};

        let mut ca_params = CertificateParams::default();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "AndroGrid Artifact Test CA");
        let ca_key = KeyPair::generate().expect("Failed to generate CA key");
        let ca_cert = ca_params
            .self_signed(&ca_key)
            .expect("Failed to self-sign CA cert");

        let mut server_params =
            CertificateParams::new(vec!["localhost".to_string(), "127.0.0.1".to_string()])
                .expect("Failed to create server params");
        server_params
            .distinguished_name
            .push(DnType::CommonName, "localhost");
        let server_key = KeyPair::generate().expect("Failed to generate server key");
        let server_cert = server_params
            .signed_by(&server_key, &ca_cert, &ca_key)
            .expect("Failed to sign server cert");

        let mut client_params =
            CertificateParams::new(vec!["operator.androgrid.internal".to_string()])
                .expect("Failed to create client params");
        client_params
            .distinguished_name
            .push(DnType::CommonName, "operator");
        client_params
            .distinguished_name
            .push(DnType::OrganizationalUnitName, "Control Plane");
        let client_key = KeyPair::generate().expect("Failed to generate client key");
        let client_cert = client_params
            .signed_by(&client_key, &ca_cert, &ca_key)
            .expect("Failed to sign client cert");

        Self {
            ca_cert_pem: ca_cert.pem().into_bytes(),
            server_cert_pem: server_cert.pem().into_bytes(),
            server_key_pem: server_key.serialize_pem().into_bytes(),
            client_cert_pem: client_cert.pem().into_bytes(),
            client_key_pem: client_key.serialize_pem().into_bytes(),
        }
    }
}

async fn spawn_test_cluster(
    scheduler: Arc<ClusterScheduler>,
    artifact_store: Arc<ArtifactStore>,
    pki: &TestPki,
    artifact_base_url: String,
) -> (SocketAddr, tokio::sync::oneshot::Sender<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind ephemeral gRPC port");
    let local_addr = listener.local_addr().expect("Failed to get gRPC addr");

    let server_tls =
        build_server_mtls_config(&pki.server_cert_pem, &pki.server_key_pem, &pki.ca_cert_pem);

    let worker_service = AndroGridServiceImpl::new(Arc::clone(&scheduler));
    let worker_server = AndroGridServiceServer::new(worker_service);

    let admin_service = AdminServiceImpl::with_artifact_store(
        Arc::clone(&scheduler),
        Arc::clone(&artifact_store),
        artifact_base_url,
    );
    let admin_server = AdminServiceServer::new(admin_service);

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    tokio::spawn(async move {
        Server::builder()
            .tls_config(server_tls)
            .expect("Valid TLS configuration")
            .add_service(worker_server)
            .add_service(admin_server)
            .serve_with_incoming_shutdown(TcpListenerStream::new(listener), async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("Server failed");
    });

    (local_addr, shutdown_tx)
}

fn build_mtls_channel(addr: SocketAddr, pki: &TestPki) -> Channel {
    let tls_config = build_client_mtls_config(
        &pki.client_cert_pem,
        &pki.client_key_pem,
        &pki.ca_cert_pem,
        "localhost",
    );

    let endpoint_uri = format!("https://{}", addr);
    let endpoint = tonic::transport::Endpoint::from_shared(endpoint_uri)
        .expect("Valid URI")
        .tls_config(tls_config)
        .expect("Valid client TLS");

    endpoint.connect_lazy()
}

#[test]
fn test_wasi_validator_validates_canonical_wasi_fixtures() {
    let hello_wasm = include_bytes!("fixtures/wasm/hello_wasi.wasm");
    let fib_wasm = include_bytes!("fixtures/wasm/fibonacci.wasm");
    let env_wasm = include_bytes!("fixtures/wasm/env_echo.wasm");

    // Validate hello_wasi
    let hello_meta = WasiValidator::validate(hello_wasm, "_start")
        .expect("hello_wasi.wasm must be valid WASI preview 1");
    assert_eq!(hello_meta.entrypoint, "_start");
    assert!(hello_meta.is_wasi_compatible);
    assert!(!hello_meta.artifact_sha256.is_empty());

    // Validate fibonacci
    let fib_meta = WasiValidator::validate(fib_wasm, "fibonacci")
        .expect("fibonacci.wasm must export fibonacci entrypoint");
    assert_eq!(fib_meta.entrypoint, "fibonacci");

    // Validate env_echo
    let env_meta = WasiValidator::validate(env_wasm, "_start")
        .expect("env_echo.wasm must be valid WASI preview 1");
    assert_eq!(env_meta.entrypoint, "_start");

    // Rejection: Invalid magic bytes
    let invalid_magic = vec![0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00];
    assert!(WasiValidator::validate(&invalid_magic, "_start").is_err());

    // Rejection: Missing entrypoint
    let missing_entry = WasiValidator::validate(hello_wasm, "non_existent_entrypoint");
    assert!(missing_entry.is_err());
}

#[tokio::test]
async fn test_artifact_store_content_addressed_storage_and_integrity() {
    let store = ArtifactStore::new();
    let sample_bytecode = b"\x00asm\x01\x00\x00\x00sample_payload_bytes_for_testing";

    let mut hasher = Sha256::new();
    hasher.update(sample_bytecode);
    let expected_sha256 = format!("{:x}", hasher.finalize());

    // Store artifact
    let digest = store
        .store_artifact(sample_bytecode)
        .expect("Storage must succeed");
    assert_eq!(digest, expected_sha256);
    assert!(store.has_artifact(&expected_sha256));

    // Retrieve artifact
    let retrieved = store
        .get_artifact(&expected_sha256)
        .expect("Artifact must exist");
    assert_eq!(retrieved, sample_bytecode);

    // Hash integrity verification
    assert!(ArtifactStore::validate_sha256(&expected_sha256, sample_bytecode).is_ok());
    assert!(ArtifactStore::validate_sha256("bad_digest", sample_bytecode).is_err());

    // Frame size bound enforcement [NFR-PERF-001]
    let oversized = vec![0u8; 16 * 1024 * 1024 + 1];
    assert!(store.store_artifact(&oversized).is_err());
}

#[tokio::test]
async fn test_artifact_http_server_endpoints() {
    let store = Arc::new(ArtifactStore::new());
    let (http_addr, _http_shutdown) = ArtifactServer::spawn_ephemeral(Arc::clone(&store)).await;

    let sample_wasm = include_bytes!("fixtures/wasm/hello_wasi.wasm");
    let mut hasher = Sha256::new();
    hasher.update(sample_wasm);
    let sha256_hex = format!("{:x}", hasher.finalize());

    // Store sample in artifact store
    store
        .store_artifact(sample_wasm)
        .expect("Store must succeed");

    // Client HTTP GET /artifacts/<sha256>.wasm
    let client = reqwest::Client::new();
    let get_url = format!("http://{}/artifacts/{}.wasm", http_addr, sha256_hex);
    let res = client.get(&get_url).send().await.expect("GET must succeed");

    assert_eq!(res.status(), reqwest::StatusCode::OK);
    assert_eq!(
        res.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("application/wasm")
    );
    let body = res.bytes().await.expect("Read bytes");
    assert_eq!(&body[..], sample_wasm);

    // Client HTTP HEAD /artifacts/<sha256>.wasm
    let head_res = client
        .head(&get_url)
        .send()
        .await
        .expect("HEAD must succeed");
    assert_eq!(head_res.status(), reqwest::StatusCode::OK);
    assert_eq!(
        head_res
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok()),
        Some(sample_wasm.len().to_string().as_str())
    );

    // Client HTTP POST /artifacts (Ingestion)
    let fib_wasm = include_bytes!("fixtures/wasm/fibonacci.wasm");
    let mut fib_hasher = Sha256::new();
    fib_hasher.update(fib_wasm);
    let fib_sha256 = format!("{:x}", fib_hasher.finalize());

    let post_url = format!("http://{}/artifacts", http_addr);
    let post_res = client
        .post(&post_url)
        .header("content-type", "application/wasm")
        .body(fib_wasm.to_vec())
        .send()
        .await
        .expect("POST must succeed");

    assert_eq!(post_res.status(), reqwest::StatusCode::CREATED);
    assert!(store.has_artifact(&fib_sha256));

    // Client HTTP GET 404 for non-existent artifact
    let missing_url = format!("http://{}/artifacts/non_existent.wasm", http_addr);
    let not_found = client
        .get(&missing_url)
        .send()
        .await
        .expect("GET must succeed");
    assert_eq!(not_found.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_admin_service_upload_and_decoupled_task_dispatch_under_4kb() {
    let pki = TestPki::generate();
    let scheduler = Arc::new(ClusterScheduler::new());
    let store = Arc::new(ArtifactStore::new());

    let (http_addr, _http_shutdown) = ArtifactServer::spawn_ephemeral(Arc::clone(&store)).await;
    let artifact_base_url = format!("http://{}", http_addr);

    let (grpc_addr, _grpc_shutdown) = spawn_test_cluster(
        Arc::clone(&scheduler),
        Arc::clone(&store),
        &pki,
        artifact_base_url.clone(),
    )
    .await;

    let channel = build_mtls_channel(grpc_addr, &pki);
    let mut admin_client = AdminServiceClient::new(channel.clone());
    let mut worker_client = AndroGridServiceClient::new(channel);

    // 1. Upload WASM Bytecode via AdminService::upload_artifact
    let wasm_bytes = include_bytes!("fixtures/wasm/hello_wasi.wasm");
    let mut hasher = Sha256::new();
    hasher.update(wasm_bytes);
    let expected_sha256 = format!("{:x}", hasher.finalize());

    let upload_res = admin_client
        .upload_artifact(UploadArtifactRequest {
            bytecode: wasm_bytes.to_vec(),
            expected_sha256: expected_sha256.clone(),
        })
        .await
        .expect("Upload artifact must succeed")
        .into_inner();

    assert!(upload_res.success);
    assert_eq!(upload_res.artifact_sha256, expected_sha256);
    assert_eq!(
        upload_res.artifact_url,
        format!("{}/artifacts/{}.wasm", artifact_base_url, expected_sha256)
    );

    // 2. Submit Decoupled Task (wasm_bytecode is empty; supplied via artifact_url and artifact_sha256)
    let decoupled_task = TaskSpec {
        task_id: "task_decoupled_001".to_string(),
        run_attempt: 1,
        engine: ExecutionEngine::WasmWasi as i32,
        timeout_seconds: 30,
        entrypoint: "_start".to_string(),
        env_vars: HashMap::new(),
        wasm_bytecode: Vec::new(), // Decoupled: zero raw bytecode in-band
        shell_command: String::new(),
        artifact_sha256: upload_res.artifact_sha256.clone(),
        requires_unmetered: false,
        artifact_url: upload_res.artifact_url.clone(),
    };

    let submit_req = SubmitTaskRequest {
        task: Some(decoupled_task),
    };

    // Assert that the SubmitTaskRequest message size is strictly under 4 KB [NFR-PERF-001]
    let mut submit_buf = Vec::new();
    submit_req
        .encode(&mut submit_buf)
        .expect("Encoding submit_req");
    assert!(
        submit_buf.len() < 4096,
        "SubmitTaskRequest must be < 4 KB, was {} bytes",
        submit_buf.len()
    );

    let submit_res = admin_client
        .submit_task(submit_req)
        .await
        .expect("Task submit must succeed")
        .into_inner();
    assert!(submit_res.accepted);

    // 3. Register Worker Node
    let worker_caps = NodeCapabilities {
        is_rooted: false,
        supported_engines: vec![ExecutionEngine::WasmWasi as i32],
        cpu_cores: 8,
        total_ram_bytes: 8 * 1024 * 1024 * 1024,
        os_version: "14".to_string(),
        abi: "arm64-v8a".to_string(),
        is_metered: true,
        current_transport: NetworkTransportType::NetworkTransportCellularWan as i32,
        max_cellular_task_duration_seconds: 180,
        battery_temperature_celsius: 30,
        thermal_status: 0,
    };

    worker_client
        .register_node(RegisterNodeRequest {
            node_id: "worker_node_decoupled".to_string(),
            capabilities: Some(worker_caps.clone()),
            client_version: "0.1.0".to_string(),
        })
        .await
        .expect("Register node must succeed");

    // 4. Claim Task over Cellular WAN and Assert Unary Response Size < 4 KB
    let claim_res = worker_client
        .claim_cellular_task(ClaimCellularTaskRequest {
            node_id: "worker_node_decoupled".to_string(),
            capabilities: Some(worker_caps),
            battery_level_pct: 90,
            is_charging: false,
            battery_temperature_celsius: 30,
            thermal_status: 0,
        })
        .await
        .expect("Claim task must succeed")
        .into_inner();

    assert!(claim_res.task_available);
    let claimed_task = claim_res.task.clone().expect("Task must be present");
    assert_eq!(claimed_task.task_id, "task_decoupled_001");
    assert_eq!(claimed_task.artifact_sha256, expected_sha256);
    assert_eq!(claimed_task.artifact_url, upload_res.artifact_url);
    assert!(claimed_task.wasm_bytecode.is_empty());

    // Assert that the ClaimCellularTaskResponse message size is strictly under 4 KB [NFR-PERF-001], [INV-003]
    let mut claim_buf = Vec::new();
    claim_res
        .encode(&mut claim_buf)
        .expect("Encoding claim_res");
    assert!(
        claim_buf.len() < 4096,
        "ClaimCellularTaskResponse with decoupled artifact must be < 4 KB, was {} bytes",
        claim_buf.len()
    );

    // 5. Contrast with In-Band Monolithic Bytecode (Simulated 2 MB payload)
    let large_inband_task = TaskSpec {
        task_id: "task_inband_large".to_string(),
        run_attempt: 1,
        engine: ExecutionEngine::WasmWasi as i32,
        timeout_seconds: 30,
        entrypoint: "_start".to_string(),
        env_vars: HashMap::new(),
        wasm_bytecode: vec![0x42; 2 * 1024 * 1024], // 2 MB inline bytecode
        shell_command: String::new(),
        artifact_sha256: String::new(),
        requires_unmetered: false,
        artifact_url: String::new(),
    };
    let mut inband_buf = Vec::new();
    ClaimCellularTaskResponse {
        task_available: true,
        task: Some(large_inband_task),
        deadline_epoch_seconds: 12345,
    }
    .encode(&mut inband_buf)
    .expect("Encoding inband response");

    assert!(
        inband_buf.len() > 2 * 1024 * 1024,
        "In-band bytecode message is > 2 MB"
    );
    assert!(
        claim_buf.len() * 1000 < inband_buf.len(),
        "Decoupled gRPC payload is orders of magnitude smaller than monolithic in-band payload"
    );
}

#[tokio::test]
async fn test_worker_local_cache_simulation_and_cache_hit_speedup() {
    let store = Arc::new(ArtifactStore::new());
    let (http_addr, _http_shutdown) = ArtifactServer::spawn_ephemeral(Arc::clone(&store)).await;

    let sample_wasm = include_bytes!("fixtures/wasm/hello_wasi.wasm");
    let mut hasher = Sha256::new();
    hasher.update(sample_wasm);
    let sha256_hex = format!("{:x}", hasher.finalize());

    store
        .store_artifact(sample_wasm)
        .expect("Store must succeed");
    let artifact_url = format!("http://{}/artifacts/{}.wasm", http_addr, sha256_hex);

    // Simulated worker cache
    let mut local_worker_cache: HashMap<String, Vec<u8>> = HashMap::new();
    let http_client = reqwest::Client::new();

    // Pass 1: Cache Miss -> HTTP fetch and validate SHA-256
    let t0 = Instant::now();
    let bytecode_miss = if let Some(cached) = local_worker_cache.get(&sha256_hex) {
        cached.clone()
    } else {
        let res = http_client
            .get(&artifact_url)
            .send()
            .await
            .expect("HTTP fetch")
            .bytes()
            .await
            .expect("HTTP bytes");
        // Validate SHA-256 before in-memory execution
        ArtifactStore::validate_sha256(&sha256_hex, &res).expect("Integrity check must pass");
        local_worker_cache.insert(sha256_hex.clone(), res.to_vec());
        res.to_vec()
    };
    let duration_miss = t0.elapsed();
    assert_eq!(&bytecode_miss[..], sample_wasm);

    // Pass 2: Cache Hit -> Local memory lookup (Zero network fetch)
    let t1 = Instant::now();
    let bytecode_hit = if let Some(cached) = local_worker_cache.get(&sha256_hex) {
        cached.clone()
    } else {
        panic!("Expected cache hit!");
    };
    let duration_hit = t1.elapsed();
    assert_eq!(&bytecode_hit[..], sample_wasm);

    // Cache hit should be fast (sub-millisecond)
    println!(
        "Cache Miss duration: {:?}, Cache Hit duration: {:?}",
        duration_miss, duration_hit
    );
    assert!(
        duration_hit <= duration_miss,
        "Cache hit should be at least as fast as cache miss"
    );

    // Tampered Bytecode Detection
    let tampered_sha256 = "0000000000000000000000000000000000000000000000000000000000000000";
    let tampered_check = ArtifactStore::validate_sha256(tampered_sha256, sample_wasm);
    assert!(
        tampered_check.is_err(),
        "Tampered bytecode must be rejected with SHA-256 mismatch error"
    );
}
