//! AndroGrid Control Plane Binary Entry Point
//!
//! Hosts the gRPC control plane service over mutual TLS 1.3 or local loopback,
//! backed by the atomic lock-free ClusterScheduler.
//!
//! Traces to: AG-SPEC-2026-V2 Section 2.2, Section 2.3, [INV-002], [NFR-SEC-001], [NFR-PERF-001]

use std::sync::Arc;
use tokio::signal;
use tonic::transport::Server;
use tracing::{error, info, warn};

use androgrid_backend::andro_grid_service_server::AndroGridServiceServer;
use androgrid_backend::{
    build_server_mtls_config, AndroGridServiceImpl, ClusterConfig, ClusterScheduler,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("Initializing AndroGrid Control Plane...");

    let config = match ClusterConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            error!("Configuration error: {e}");
            std::process::exit(1);
        }
    };

    info!(
        listen_addr = %config.listen_addr,
        heartbeat_secs = config.heartbeat_interval_seconds,
        lease_secs = config.lease_timeout_seconds,
        max_frame_mb = config.max_frame_size_bytes / (1024 * 1024),
        "Cluster configuration loaded"
    );

    let scheduler = Arc::new(ClusterScheduler::new());

    let service = AndroGridServiceImpl::new(Arc::clone(&scheduler));
    let service_server = AndroGridServiceServer::new(service)
        .max_decoding_message_size(config.max_frame_size_bytes)
        .max_encoding_message_size(config.max_frame_size_bytes);

    let mut server_builder = Server::builder();

    if config.is_mtls_configured() {
        info!("Enforcing Mutual TLS 1.3 transport security [INV-002], [NFR-SEC-001]");
        let ca_path = config.tls_ca_cert_path.as_ref().unwrap();
        let cert_path = config.tls_server_cert_path.as_ref().unwrap();
        let key_path = config.tls_server_key_path.as_ref().unwrap();

        let ca_cert_pem = tokio::fs::read(ca_path).await?;
        let server_cert_pem = tokio::fs::read(cert_path).await?;
        let server_key_pem = tokio::fs::read(key_path).await?;

        let server_tls =
            build_server_mtls_config(&server_cert_pem, &server_key_pem, &ca_cert_pem);
        server_builder = server_builder.tls_config(server_tls)?;
    } else {
        warn!("mTLS credentials not specified; binding plaintext service [development mode]");
    }

    info!(
        addr = %config.listen_addr,
        "Starting Tonic gRPC control plane service"
    );

    let shutdown_signal = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C signal handler");
        info!("Received shutdown signal. Draining connections and terminating server...");
    };

    server_builder
        .add_service(service_server)
        .serve_with_shutdown(config.listen_addr, shutdown_signal)
        .await?;

    info!("AndroGrid Control Plane terminated cleanly");
    Ok(())
}
