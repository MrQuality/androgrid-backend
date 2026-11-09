//! Embedded HTTP Static Artifact Distribution Server
//!
//! Exposes a high-performance, asynchronous HTTP static server serving cached WebAssembly
//! bytecode modules by SHA-256 content address.
//!
//! Traces to: AG-SPEC-2026-V2, [NFR-PERF-001], [NFR-POW-001], [INV-003], [REQ-OP-003]

use axum::body::Bytes;
use axum::extract::{Path as AxumPath, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, ETAG};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{error, info};

use crate::artifact::store::ArtifactStore;
use crate::MAX_FRAME_SIZE_BYTES;

/// JSON response payload for successful artifact ingestion.
#[derive(Debug, Serialize)]
pub struct IngestionResponse {
    pub sha256: String,
    pub url: String,
    pub size_bytes: usize,
}

/// JSON response payload for server health probe.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub cached_artifacts: usize,
}

/// HTTP static artifact distribution service.
///
/// Traces to: [NFR-PERF-001], [REQ-OP-003]
pub struct ArtifactServer;

impl ArtifactServer {
    /// Builds the Axum router configured with artifact distribution endpoints.
    pub fn router(store: Arc<ArtifactStore>) -> Router {
        Router::new()
            .route("/artifacts/:filename", get(Self::handle_get_or_head))
            .route("/artifacts", post(Self::handle_post))
            .route("/health", get(Self::handle_health))
            .with_state(store)
    }

    /// Builds the combined Axum router with artifact distribution and bootstrap pairing enrollment.
    ///
    /// Traces to: [INV-002], [NFR-SEC-001], [REQ-OP-001]
    pub fn router_with_enrollment(
        store: Arc<ArtifactStore>,
        enrollment_ctx: crate::pairing::EnrollmentContext,
    ) -> Router {
        Self::router(store).merge(crate::pairing::enrollment_http_router(enrollment_ctx))
    }

    /// Handles GET and HEAD requests for content-addressed WebAssembly artifacts.
    ///
    /// Traces to: [NFR-PERF-001], [REQ-OP-003]
    async fn handle_get_or_head(
        AxumPath(filename): AxumPath<String>,
        State(store): State<Arc<ArtifactStore>>,
    ) -> Response {
        let clean_sha = filename
            .trim()
            .trim_end_matches(".wasm")
            .to_ascii_lowercase();

        if let Some(bytes) = store.get_artifact(&clean_sha) {
            let mut headers = HeaderMap::new();
            headers.insert(CONTENT_TYPE, "application/wasm".parse().unwrap());
            headers.insert(CONTENT_LENGTH, bytes.len().to_string().parse().unwrap());
            headers.insert(ETAG, format!("\"{clean_sha}\"").parse().unwrap());
            headers.insert(
                CACHE_CONTROL,
                "public, max-age=31536000, immutable".parse().unwrap(),
            );

            (StatusCode::OK, headers, bytes).into_response()
        } else {
            (
                StatusCode::NOT_FOUND,
                format!("Artifact with SHA-256 '{clean_sha}' not found"),
            )
                .into_response()
        }
    }

    /// Handles POST requests to upload and register a WebAssembly artifact.
    ///
    /// Traces to: [NFR-PERF-001], [REQ-OP-003]
    async fn handle_post(State(store): State<Arc<ArtifactStore>>, body: Bytes) -> Response {
        if body.len() > MAX_FRAME_SIZE_BYTES {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("Artifact payload exceeds maximum bound of {MAX_FRAME_SIZE_BYTES} bytes"),
            )
                .into_response();
        }

        match store.store_artifact(&body) {
            Ok(sha256) => {
                let resp = IngestionResponse {
                    sha256: sha256.clone(),
                    url: format!("/artifacts/{sha256}.wasm"),
                    size_bytes: body.len(),
                };
                (StatusCode::CREATED, Json(resp)).into_response()
            }
            Err(e) => {
                error!("Artifact ingestion error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
            }
        }
    }

    /// Handles health check requests.
    async fn handle_health(State(store): State<Arc<ArtifactStore>>) -> Response {
        let count = store.list_artifacts().len();
        (
            StatusCode::OK,
            Json(HealthResponse {
                status: "ok",
                cached_artifacts: count,
            }),
        )
            .into_response()
    }

    /// Spawns the artifact server on an ephemeral loopback port for hermetic testing.
    pub async fn spawn_ephemeral(
        store: Arc<ArtifactStore>,
    ) -> (SocketAddr, tokio::sync::oneshot::Sender<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Failed to bind ephemeral HTTP artifact port");
        let local_addr = listener.local_addr().expect("Failed to get local address");

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let app = Self::router(store);

        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("Artifact HTTP server failed");
        });

        (local_addr, shutdown_tx)
    }

    /// Spawns the artifact server on a specified TCP listener.
    pub fn spawn(
        listener: TcpListener,
        store: Arc<ArtifactStore>,
        shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    ) {
        let app = Self::router(store);
        tokio::spawn(async move {
            info!("Starting embedded HTTP static artifact server...");
            if let Err(e) = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
            {
                error!("Embedded HTTP artifact server encountered error: {e}");
            }
        });
    }

    /// Spawns the combined artifact and bootstrap enrollment server on a specified TCP listener.
    ///
    /// Traces to: [INV-002], [NFR-SEC-001], [REQ-OP-001]
    pub fn spawn_with_enrollment(
        listener: TcpListener,
        store: Arc<ArtifactStore>,
        enrollment_ctx: Option<crate::pairing::EnrollmentContext>,
        shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    ) {
        let app = if let Some(ctx) = enrollment_ctx {
            Self::router_with_enrollment(store, ctx)
        } else {
            Self::router(store)
        };

        tokio::spawn(async move {
            info!("Starting embedded HTTP static artifact and bootstrap enrollment server...");
            if let Err(e) = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
            {
                error!("Embedded HTTP server encountered error: {e}");
            }
        });
    }
}
