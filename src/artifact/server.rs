//! Embedded HTTP Static Artifact Distribution Server
//!
//! Exposes a high-performance, asynchronous HTTP static server serving cached WebAssembly
//! bytecode modules by SHA-256 content address.
//!
//! Traces to: AG-SPEC-2026-V2, [NFR-PERF-001], [NFR-POW-001], [INV-003], [REQ-OP-003]

use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Path as AxumPath, Request as AxumRequest, State};
use axum::http::header::{
    ACCEPT_RANGES, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG, RANGE,
};
use axum::http::{HeaderMap, HeaderName, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use tracing::{error, info, warn};

use crate::artifact::store::{
    ArtifactStore, BlobCommitOutcome, BlobKind, BlobSource, TempFileGuard,
};
use crate::transport::tls::AuthenticatedPeer;
use crate::MAX_FRAME_SIZE_BYTES;

#[derive(Clone)]
struct BlobServerState {
    store: Arc<ArtifactStore>,
    max_apk_bytes: usize,
}

#[derive(Debug, Serialize)]
pub struct BlobIngestionResponse {
    pub sha256: String,
    pub url: String,
    pub size_bytes: u64,
    pub kind: BlobKind,
}

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

    /// Builds the comprehensive Axum router with artifact distribution, bootstrap pairing enrollment,
    /// and embedded real-time web dashboard & Server-Sent Events (SSE).
    ///
    /// Traces to: [INV-002], [INV-004], [INV-005], [NFR-SEC-001], [NFR-REL-001], [REQ-OP-001]
    pub fn router_full(
        store: Arc<ArtifactStore>,
        scheduler: Arc<crate::scheduler::ClusterScheduler>,
        enrollment_ctx: Option<crate::pairing::EnrollmentContext>,
    ) -> Router {
        Self::router_full_with_options(store, scheduler, enrollment_ctx, true)
    }

    /// Builds the auxiliary `50052` router, optionally stripping plaintext `/artifacts*` routes
    /// when TLS 1.3 mTLS (`[INV-002]`) is active on port `50051`.
    pub fn router_full_with_options(
        store: Arc<ArtifactStore>,
        scheduler: Arc<crate::scheduler::ClusterScheduler>,
        enrollment_ctx: Option<crate::pairing::EnrollmentContext>,
        expose_plaintext_artifacts: bool,
    ) -> Router {
        let mut router = if expose_plaintext_artifacts {
            Self::router(store)
        } else {
            Router::new()
                .route("/health", get(Self::handle_health))
                .with_state(store)
        }
        .merge(crate::observability::dashboard_router(scheduler));
        if let Some(ctx) = enrollment_ctx {
            router = router.merge(crate::pairing::enrollment_http_router(ctx));
        }
        router
    }

    /// Builds the mTLS-authenticated `/v1/blobs` REST data-plane router for dual-plane port `50051`.
    ///
    /// Traces to: [INV-002], [NFR-PERF-001], [REQ-OP-003], ADR-005, ADR-008
    pub fn blob_router(store: Arc<ArtifactStore>, max_apk_bytes: usize) -> Router {
        let state = BlobServerState {
            store,
            max_apk_bytes,
        };
        Router::new()
            .route("/v1/blobs", put(Self::handle_blob_put))
            .route("/v1/blobs/:sha256", get(Self::handle_blob_get_or_head))
            .layer(DefaultBodyLimit::disable())
            .with_state(state)
    }

    async fn handle_blob_put(State(state): State<BlobServerState>, req: AxumRequest) -> Response {
        let Some(peer) = req.extensions().get::<AuthenticatedPeer>().cloned() else {
            return (StatusCode::UNAUTHORIZED, "Client mTLS certificate required [INV-002]")
                .into_response();
        };
        if peer.organizational_unit.as_deref() != Some("Control Plane") {
            warn!(
                ou = ?peer.organizational_unit,
                "Rejected PUT /v1/blobs from non-Control Plane peer"
            );
            return (
                StatusCode::FORBIDDEN,
                "PUT /v1/blobs requires operator certificate with OU=Control Plane",
            )
                .into_response();
        }

        let headers = req.headers();
        let kind = match headers
            .get("x-artifact-kind")
            .and_then(|v| v.to_str().ok())
        {
            Some(raw) => match BlobKind::parse_header(raw) {
                Some(k) => k,
                None => {
                    return (StatusCode::BAD_REQUEST, "Invalid X-Artifact-Kind header")
                        .into_response()
                }
            },
            None => BlobKind::Wasm,
        };

        let expected_sha = match headers
            .get("x-expected-sha256")
            .and_then(|v| v.to_str().ok())
        {
            Some(raw) => match ArtifactStore::normalize_sha256_hex(raw) {
                Some(norm) => Some(norm),
                None => {
                    return (
                        StatusCode::BAD_REQUEST,
                        "Invalid X-Expected-Sha256 header: expected 64-char hex",
                    )
                        .into_response()
                }
            },
            None => None,
        };

        let max_bytes = ArtifactStore::max_bytes_for_kind(kind, state.max_apk_bytes);
        let tmp_path = state.store.next_temp_path();
        let guard = TempFileGuard::new(tmp_path.clone());

        let mut file = match tokio::fs::File::create(&tmp_path).await {
            Ok(f) => f,
            Err(e) => {
                error!("Failed creating blob temp file: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
            }
        };

        let mut hasher = Sha256::new();
        let mut total_bytes: usize = 0;
        let mut stream = req.into_body().into_data_stream();

        while let Some(chunk_res) = stream.next().await {
            let chunk = match chunk_res {
                Ok(c) => c,
                Err(e) => {
                    return (StatusCode::BAD_REQUEST, format!("Stream read error: {e}"))
                        .into_response()
                }
            };
            total_bytes = match total_bytes.checked_add(chunk.len()) {
                Some(n) => n,
                None => return (StatusCode::PAYLOAD_TOO_LARGE, "Payload size overflow").into_response(),
            };
            if total_bytes > max_bytes {
                drop(file);
                return (
                    StatusCode::PAYLOAD_TOO_LARGE,
                    format!("Blob exceeds maximum allowed {max_bytes} bytes for kind '{}'", kind.as_str()),
                )
                    .into_response();
            }
            hasher.update(&chunk);
            if let Err(e) = file.write_all(&chunk).await {
                return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
            }
        }

        if total_bytes == 0 {
            return (StatusCode::BAD_REQUEST, "Blob payload must not be empty").into_response();
        }
        if let Err(e) = file.flush().await {
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
        drop(file);

        let computed_sha = format!("{:x}", hasher.finalize());
        if let Some(ref exp) = expected_sha {
            if exp != &computed_sha {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    format!("SHA-256 mismatch: expected '{exp}', computed '{computed_sha}'"),
                )
                    .into_response();
            }
        }

        match state
            .store
            .commit_temp_blob(guard, computed_sha, total_bytes as u64, kind)
            .await
        {
            Ok(BlobCommitOutcome::Created(meta)) => {
                info!(sha256 = %meta.sha256, kind = %meta.kind.as_str(), size_bytes = meta.size_bytes, status = 201, "Stored blob");
                (
                    StatusCode::CREATED,
                    Json(BlobIngestionResponse {
                        url: format!("/v1/blobs/{}", meta.sha256),
                        sha256: meta.sha256,
                        size_bytes: meta.size_bytes,
                        kind: meta.kind,
                    }),
                )
                    .into_response()
            }
            Ok(BlobCommitOutcome::IdempotentMatch(meta)) => {
                info!(sha256 = %meta.sha256, kind = %meta.kind.as_str(), size_bytes = meta.size_bytes, status = 200, "Idempotent blob PUT");
                (
                    StatusCode::OK,
                    Json(BlobIngestionResponse {
                        url: format!("/v1/blobs/{}", meta.sha256),
                        sha256: meta.sha256,
                        size_bytes: meta.size_bytes,
                        kind: meta.kind,
                    }),
                )
                    .into_response()
            }
            Ok(BlobCommitOutcome::KindConflict { existing_kind }) => (
                StatusCode::CONFLICT,
                format!(
                    "Blob already exists with kind '{}', cannot overwrite as '{}'",
                    existing_kind.as_str(),
                    kind.as_str()
                ),
            )
                .into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        }
    }

    async fn handle_blob_get_or_head(
        AxumPath(raw_sha): AxumPath<String>,
        State(state): State<BlobServerState>,
        method: Method,
        req: AxumRequest,
    ) -> Response {
        let Some(peer) = req.extensions().get::<AuthenticatedPeer>() else {
            return (StatusCode::UNAUTHORIZED, "Client mTLS certificate required [INV-002]")
                .into_response();
        };
        let allowed_ou = matches!(
            peer.organizational_unit.as_deref(),
            Some("Edge Compute") | Some("Control Plane")
        );
        if !allowed_ou {
            return (
                StatusCode::FORBIDDEN,
                "GET/HEAD /v1/blobs requires OU=Edge Compute or OU=Control Plane",
            )
                .into_response();
        }

        let Some(clean_sha) = ArtifactStore::normalize_sha256_hex(&raw_sha) else {
            return (StatusCode::BAD_REQUEST, "Invalid SHA-256 blob identifier").into_response();
        };

        let Some((meta, source)) = state.store.get_blob_info(&clean_sha) else {
            return (StatusCode::NOT_FOUND, "Blob not found").into_response();
        };

        let total = meta.size_bytes;
        let is_head = method == Method::HEAD;

        let range_selection: Option<(u64, u64)> = if let Some(range_hdr) =
            req.headers().get(RANGE).and_then(|v| v.to_str().ok())
        {
            match parse_single_byte_range(range_hdr, total) {
                Ok(bounds) => Some(bounds),
                Err(()) => {
                    let mut h = HeaderMap::new();
                    h.insert(CONTENT_RANGE, format!("bytes */{total}").parse().unwrap());
                    h.insert(ACCEPT_RANGES, "bytes".parse().unwrap());
                    return if is_head {
                        (StatusCode::RANGE_NOT_SATISFIABLE, h, Body::empty()).into_response()
                    } else {
                        (StatusCode::RANGE_NOT_SATISFIABLE, h, "Range Not Satisfiable")
                            .into_response()
                    };
                }
            }
        } else {
            None
        };

        let (status, start, len) = match range_selection {
            Some((s, e)) => (StatusCode::PARTIAL_CONTENT, s, e - s + 1),
            None => (StatusCode::OK, 0, total),
        };

        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT_RANGES, "bytes".parse().unwrap());
        headers.insert(CONTENT_LENGTH, len.to_string().parse().unwrap());
        headers.insert(
            HeaderName::from_static("x-artifact-size"),
            total.to_string().parse().unwrap(),
        );
        headers.insert(
            HeaderName::from_static("x-artifact-sha256"),
            clean_sha.parse().unwrap(),
        );
        headers.insert(ETAG, format!("\"{clean_sha}\"").parse().unwrap());
        headers.insert(
            CACHE_CONTROL,
            "public, max-age=31536000, immutable".parse().unwrap(),
        );
        headers.insert(
            CONTENT_TYPE,
            match meta.kind {
                BlobKind::Wasm => "application/wasm",
                BlobKind::Apk => "application/vnd.android.package-archive",
            }
            .parse()
            .unwrap(),
        );
        if let Some((s, e)) = range_selection {
            headers.insert(
                CONTENT_RANGE,
                format!("bytes {s}-{e}/{total}").parse().unwrap(),
            );
        }

        info!(sha256 = %clean_sha, range = ?range_selection, status = status.as_u16(), "Served blob");

        if is_head {
            return (status, headers, Body::empty()).into_response();
        }

        match source {
            BlobSource::Memory(bytes) => {
                let s = start as usize;
                let e = s + (len as usize);
                (status, headers, Bytes::from(bytes[s..e].to_vec())).into_response()
            }
            BlobSource::File(path) => {
                let Ok(mut file) = tokio::fs::File::open(&path).await else {
                    return StatusCode::NOT_FOUND.into_response();
                };
                if start > 0 && file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
                let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(8);
                tokio::spawn(async move {
                    let mut remaining = len;
                    let mut buf = vec![0u8; 64 * 1024];
                    while remaining > 0 {
                        let to_read = (remaining as usize).min(buf.len());
                        match file.read(&mut buf[..to_read]).await {
                            Ok(0) => break,
                            Ok(n) => {
                                remaining -= n as u64;
                                if tx.send(Ok(Bytes::copy_from_slice(&buf[..n]))).await.is_err() {
                                    break;
                                }
                            }
                            Err(e) => {
                                let _ = tx.send(Err(e)).await;
                                break;
                            }
                        }
                    }
                });
                let body = Body::from_stream(ReceiverStream::new(rx));
                (status, headers, body).into_response()
            }
        }
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

    /// Spawns the comprehensive HTTP server hosting artifact distribution, bootstrap enrollment,
    /// and the real-time cluster web dashboard & Server-Sent Events (SSE).
    ///
    /// Traces to: [INV-002], [INV-004], [INV-005], [NFR-SEC-001], [NFR-REL-001], [REQ-OP-001]
    pub fn spawn_full(
        listener: TcpListener,
        store: Arc<ArtifactStore>,
        scheduler: Arc<crate::scheduler::ClusterScheduler>,
        enrollment_ctx: Option<crate::pairing::EnrollmentContext>,
        shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    ) {
        Self::spawn_full_with_options(listener, store, scheduler, enrollment_ctx, true, shutdown_rx)
    }

    /// Spawns the auxiliary `50052` HTTP server with configurable plaintext artifact exposure.
    pub fn spawn_full_with_options(
        listener: TcpListener,
        store: Arc<ArtifactStore>,
        scheduler: Arc<crate::scheduler::ClusterScheduler>,
        enrollment_ctx: Option<crate::pairing::EnrollmentContext>,
        expose_plaintext_artifacts: bool,
        shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    ) {
        let app = Self::router_full_with_options(
            store,
            scheduler,
            enrollment_ctx,
            expose_plaintext_artifacts,
        );

        tokio::spawn(async move {
            info!("Starting embedded HTTP static artifact, pairing, and cluster web dashboard server...");
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

/// Parses strictly single-range `bytes=<start>-[<end>]` headers.
/// Rejects multi-range, suffix ranges (`bytes=-N`), non-`bytes` units, `start > end`,
/// `start >= total`, or `total == 0`, and clamps `end >= total` to `total - 1`.
fn parse_single_byte_range(header: &str, total: u64) -> Result<(u64, u64), ()> {
    if total == 0 {
        return Err(());
    }
    let spec = header.trim().strip_prefix("bytes=").ok_or(())?;
    if spec.contains(',') {
        return Err(());
    }
    let (start_str, end_str) = spec.split_once('-').ok_or(())?;
    if start_str.is_empty() {
        // Suffix range `bytes=-N` is explicitly rejected per ADR-008
        return Err(());
    }
    let start: u64 = start_str.trim().parse().map_err(|_| ())?;
    if start >= total {
        return Err(());
    }
    let end: u64 = if end_str.trim().is_empty() {
        total - 1
    } else {
        let parsed: u64 = end_str.trim().parse().map_err(|_| ())?;
        parsed.min(total - 1)
    };
    if start > end {
        return Err(());
    }
    Ok((start, end))
}
