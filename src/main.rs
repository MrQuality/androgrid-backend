//! AndroGrid Control Plane Binary Entry Point
//!
//! Hosts the gRPC control plane service over mutual TLS 1.3 or local loopback,
//! backed by the atomic lock-free ClusterScheduler.
//!
//! Traces to: AG-SPEC-2026-V2 Section 2.2, Section 2.3, [INV-002], [NFR-SEC-001], [NFR-PERF-001]

use clap::Parser;
use std::sync::Arc;
use tokio::signal;
use tonic::transport::Server;
use tracing::{error, info, warn};

use androgrid_backend::admin_service_server::AdminServiceServer;
use androgrid_backend::andro_grid_service_server::AndroGridServiceServer;
use androgrid_backend::enrollment_service_server::EnrollmentServiceServer;
use androgrid_backend::{
    build_server_bootstrap_tls_config, require_client_mtls_interceptor, AdminServiceImpl,
    AndroGridServiceImpl, ArtifactServer, ArtifactStore, CaSigner, ClusterConfig,
    ClusterScheduler, EnrollmentContext, EnrollmentServiceImpl, PairingTokenStore, ThermalPolicy,
};

#[derive(Parser, Debug)]
#[command(
    name = "androgrid-backend",
    about = "AndroGrid Control Plane Orchestrator",
    version = "0.1.0"
)]
struct ServerArgs {
    /// Allow scheduling of insecure root shell tasks (diagnostic / laboratory mode)
    #[arg(long, env = "ANDROGRID_ALLOW_INSECURE_ROOT_TASKS")]
    allow_insecure_root_tasks: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = ServerArgs::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("Initializing AndroGrid Control Plane...");

    let mut config = match ClusterConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            error!("Configuration error: {e}");
            std::process::exit(1);
        }
    };

    if args.allow_insecure_root_tasks {
        config.allow_insecure_root_tasks = true;
    }

    if config.allow_insecure_root_tasks {
        warn!("Insecure root shell execution explicitly ENABLED via --allow-insecure-root-tasks [DIAGNOSTIC MODE]");
    }

    info!(
        listen_addr = %config.listen_addr,
        heartbeat_secs = config.heartbeat_interval_seconds,
        lease_secs = config.lease_timeout_seconds,
        max_frame_mb = config.max_frame_size_bytes / (1024 * 1024),
        allow_insecure_root = config.allow_insecure_root_tasks,
        "Cluster configuration loaded"
    );

    let scheduler = Arc::new(ClusterScheduler::with_options(
        config.allow_insecure_root_tasks,
        ThermalPolicy::default(),
    ));

    let artifact_store = if let Some(ref dir) = config.artifact_storage_dir {
        Arc::new(ArtifactStore::with_directory(dir))
    } else {
        Arc::new(ArtifactStore::new())
    };

    let artifact_base_url = config.resolve_artifact_base_url();

    let token_store = Arc::new(PairingTokenStore::new(config.allow_pairing, 5));

    let (ca_signer, ca_fingerprint) = if let (Some(ca_cert_path), Some(ca_key_path)) =
        (&config.tls_ca_cert_path, &config.tls_ca_key_path)
    {
        match (
            tokio::fs::read_to_string(ca_cert_path).await,
            tokio::fs::read_to_string(ca_key_path).await,
        ) {
            (Ok(ca_pem), Ok(ca_key)) => match CaSigner::from_pem(&ca_pem, &ca_key) {
                Ok(signer) => {
                    let fp = signer.ca_fingerprint().to_string();
                    info!(ca_fingerprint = %fp, "Initialized Root CA CSR signer for pairing enrollment [INV-002]");
                    (Some(Arc::new(signer)), fp)
                }
                Err(e) => {
                    warn!(error = %e, "Failed to initialize Root CA signer; enrollment disabled");
                    (None, String::new())
                }
            },
            (Err(e), _) | (_, Err(e)) => {
                warn!(error = %e, "Could not read Root CA cert or key; enrollment disabled");
                (None, String::new())
            }
        }
    } else {
        (None, String::new())
    };

    let enrollment_ctx = ca_signer.as_ref().map(|signer| EnrollmentContext {
        token_store: Arc::clone(&token_store),
        signer: Arc::clone(signer),
    });

    let mtls_enabled = config.is_mtls_configured();
    let service = AndroGridServiceImpl::new(Arc::clone(&scheduler));
    let service_server = tonic::codegen::InterceptedService::new(
        AndroGridServiceServer::new(service)
            .max_decoding_message_size(config.max_frame_size_bytes)
            .max_encoding_message_size(config.max_frame_size_bytes),
        move |req| {
            if mtls_enabled {
                require_client_mtls_interceptor(req)
            } else {
                Ok(req)
            }
        },
    );

    let cluster_endpoint = config.listen_addr.to_string();
    let admin_service = AdminServiceImpl::with_services(
        Arc::clone(&scheduler),
        Arc::clone(&artifact_store),
        artifact_base_url,
        Arc::clone(&token_store),
        ca_fingerprint,
        cluster_endpoint,
    );
    let admin_server = tonic::codegen::InterceptedService::new(
        AdminServiceServer::new(admin_service)
            .max_decoding_message_size(config.max_frame_size_bytes)
            .max_encoding_message_size(config.max_frame_size_bytes),
        move |req| {
            if mtls_enabled {
                require_client_mtls_interceptor(req)
            } else {
                Ok(req)
            }
        },
    );

    let (artifact_shutdown_tx, artifact_shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let artifact_listener = tokio::net::TcpListener::bind(config.artifact_listen_addr).await?;
    info!(
        artifact_addr = %config.artifact_listen_addr,
        "Starting embedded HTTP static artifact, pairing & real-time web dashboard server [NFR-PERF-001], [REQ-OP-001], [INV-004], [INV-005]"
    );
    ArtifactServer::spawn_full(
        artifact_listener,
        Arc::clone(&artifact_store),
        Arc::clone(&scheduler),
        enrollment_ctx.clone(),
        artifact_shutdown_rx,
    );

    let mut server_builder = Server::builder();

    if mtls_enabled {
        info!("Enforcing TLS 1.3 mTLS with optional bootstrap client auth [REQ-OP-001], [INV-002], [NFR-SEC-001]");
        let ca_path = config.tls_ca_cert_path.as_ref().unwrap();
        let cert_path = config.tls_server_cert_path.as_ref().unwrap();
        let key_path = config.tls_server_key_path.as_ref().unwrap();

        let ca_cert_pem = tokio::fs::read(ca_path).await?;
        let server_cert_pem = tokio::fs::read(cert_path).await?;
        let server_key_pem = tokio::fs::read(key_path).await?;

        let server_tls =
            build_server_bootstrap_tls_config(&server_cert_pem, &server_key_pem, &ca_cert_pem);
        server_builder = server_builder.tls_config(server_tls)?;
    } else {
        warn!("mTLS credentials not specified; binding plaintext service [development mode]");
    }

    info!(
        addr = %config.listen_addr,
        "Starting Tonic gRPC control plane service"
    );

    let shutdown_signal = async move {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C signal handler");
        info!("Received shutdown signal. Draining connections and terminating server...");
        let _ = artifact_shutdown_tx.send(());
    };

    let mut router = server_builder
        .add_service(service_server)
        .add_service(admin_server);

    if let Some(ctx) = enrollment_ctx {
        let enrollment_service =
            EnrollmentServiceImpl::new(Arc::clone(&ctx.token_store), Arc::clone(&ctx.signer));
        let enrollment_server = EnrollmentServiceServer::new(enrollment_service)
            .max_decoding_message_size(config.max_frame_size_bytes)
            .max_encoding_message_size(config.max_frame_size_bytes);
        router = router.add_service(enrollment_server);
    }

    router
        .serve_with_shutdown(config.listen_addr, shutdown_signal)
        .await?;

    info!("AndroGrid Control Plane terminated cleanly");
    Ok(())
}
