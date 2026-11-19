# ADR-008: Unified Dual-Plane mTLS 1.3 (`50051`) HTTP/2 `Range` Blob Distribution

## Status
Accepted

## Architectural Scope & Requirements
- **Traceability:** `[REQ-OP-003]`, `[REQ-VER-003]`, `[INV-002]`, `[NFR-PERF-001]`, `[NFR-POW-001]`
- **Standard:** ISO/IEC/IEEE 29148
- **GitHub Issue:** #48

---

## Context

AndroGrid previously distributed WebAssembly modules via inline `TaskSpec.wasm_bytecode` or content-addressed unary `FetchArtifact` RPC over mTLS port `50051`. Three limitations blocked large-payload scalability and future OTA binary delivery:

1. **16 MB Unary Frame & Memory Ceiling:** `AdminServiceImpl::upload_artifact` (`src/transport/admin_service.rs:287-314`) and unary `FetchArtifact` cap payloads inside `MAX_FRAME_SIZE_BYTES` (16 MB), while `ArtifactStore` (`src/artifact/store.rs:40-43`) cached all blobs in an in-memory `RwLock<HashMap<String, Vec<u8>>>`.
2. **Plaintext `50052` Artifact Exposure:** `ArtifactServer` (`src/artifact/server.rs:42-90`) exposed `GET /artifacts/:filename` and `POST /artifacts` over plaintext HTTP on port `50052`, bypassing TLS 1.3 mTLS (`[INV-002]`) and the single `50051` Cloudflare WAN ingress.
3. **Cellular Power & Proxy Stream Hazards:** An HTTP client with default keep-alive pooling violates `[NFR-POW-001]` (`< 200 ms` cellular RRC socket teardown), and proxies stripping `Content-Length` break naive `ByteArray(contentLength)` pre-allocations.

---

## Empirical Validation (Technical Spikes)

Three disposable technical spikes verified feasibility before production changes:
- **Dual-Plane mTLS `50051` (`dual_plane_mtls_spike`):** `tonic 0.12.3` `Routes::into_axum_router()` + `axum 0.7` + `tokio-rustls 0.26` + `hyper-util 0.1` (ALPN `["h2", "http/1.1"]`, `allow_unauthenticated()` bootstrap) served gRPC and HTTP `/v1/blobs` on one TLS 1.3 port (`1 MiB` `PUT` + `Range` + `GET` in `27.76 ms`), confirming `TlsConnectInfo` has private fields in `tonic 0.12.3` (requiring explicit `AuthenticatedPeer` extension forwarding) and `rustls::crypto::ring::default_provider().install_default()` is required.
- **Streaming `> 2 MiB` Upload (`streaming_put_spike`):** Axum `DefaultBodyLimit::disable()` + `into_data_stream()` streamed `5 MiB` to disk via incremental `Sha256` in `89.72 ms` and rejected `7 MiB` with `413 Payload Too Large` and **0 leftover `.tmp` files** via RAII `Drop`.
- **Android Ephemeral Cellular `OkHttpClient` (`OkHttpCellularSpikeTest`):** `ConnectionPool(0, 1, MILLISECONDS)` + `body.close()` + `evictAll()` + `dispatcher.executorService.shutdown()` tore down in `0.061 ms` (`< 200 ms` `[NFR-POW-001]`), and `Call.cancel()` aborted in-flight downloads in `20.70 ms`.

---

## Decision

### 1. Dual-Plane Listener (`50051`) & `AuthenticatedPeer` Extension Forwarding
- Replace `tonic::transport::Server::builder()` in `src/main.rs` with a `tokio_rustls` + `hyper-util` TLS 1.3 accept loop that parses the leaf client certificate **once per connection** via `parse_leaf_identity` (`src/transport/tls.rs`) into `Option<AuthenticatedPeer>` and attaches it to every Hyper request's `Extensions` (`Routes::into_axum_router().merge(blob_router)`).
- Update `require_client_mtls_interceptor` to inspect `request.extensions().get::<AuthenticatedPeer>()` first (retaining `request.peer_certs()` fallback for unit tests).
- Strip `GET /artifacts/:filename` and `POST /artifacts` from port `50052` (`ArtifactServer::router_full`) whenever `mtls_enabled == true`, keeping only `/health` and the read-only local dashboard on loopback `50052`.

### 2. `/v1/blobs` RBAC, Streaming Disk `ArtifactStore`, and `BlobKind`
- **`PUT /v1/blobs` (`OU=Control Plane` only):** Reject missing `AuthenticatedPeer` (`401`) or non-`Control Plane` `OU` (`403`, blocking `Edge Compute` workers per `ADR-005`). Stream into `<uuid>.tmp` guarded by an RAII `Drop` `TempFileGuard`, validate `X-Artifact-Kind: wasm | apk` (`400` on invalid), enforce size caps mid-stream (`16 MB` max for `Wasm` `[NFR-PERF-001]`; `ClusterConfig.max_apk_bytes` default `128 MB` for `Apk` -> `413`), and verify lowercase-normalized `X-Expected-Sha256` (`400` on bad hex, `422` on mismatch). Under a short commit mutex, commit `<sha256>.meta.json` then `<sha256>.blob` (`201 Created`, or idempotent `200 OK` if matching `kind`, `409 Conflict` if `kind` differs).
- **`GET` / `HEAD /v1/blobs/{sha256}` (`Edge Compute` or `Control Plane`):** Return `401` if unauthenticated, `403` for other `OU`s, `400` on invalid SHA-256, `404` if `<sha256>.blob` is absent. Support single-range `bytes=<start>-[<end>]` only (clamping `end >= total` to `total - 1`; returning `416 Range Not Satisfiable` + `Content-Range: bytes */<total>` on multi-range, suffix ranges, `start > end`, `start >= total`, or `total == 0`). Always emit `Accept-Ranges: bytes`, `Content-Length`, `X-Artifact-Size`, and `X-Artifact-Sha256`.
- **`AdminServiceImpl::validate_wasm_artifact`:** Reject non-`Wasm` blobs so `.apk` artifacts can never be scheduled as WASM compute tasks.
- **`androgrid-cli`:** Update `Submit` and `Group Submit` to upload `--wasm` modules via `reqwest` `PUT /v1/blobs` when TLS flags (`--ca-cert`, `--client-cert`, `--client-key`) are provided, falling back to unary `UploadArtifact` only when TLS flags are omitted.

### 3. Android `OkHttpClient` Streaming Resolver (`OkHttpBlobFetcher`)
- Declare explicit `com.squareup.okhttp3:okhttp:4.12.0` and extract shared `OkHttpBlobFetcher` (`ArtifactResolver.kt`) with `suspendCancellableCoroutine` (`call.cancel()`).
- **Wi-Fi (`WorkerForegroundService`):** Maintain a persistent `OkHttpClient` when `hasMtlsCredentials == true` (evicting idle connections in `stopSyncLoop()` / `handleServiceTimeout()`, shutting down `dispatcher.executorService` in `onDestroy()`); pass `blobFetcher = null` in plaintext dev mode (`hasMtlsCredentials == false`).
- **Cellular (`CellularTransportClient`):** Construct an ephemeral `OkHttpClient` (`ConnectionPool(0, 1, MILLISECONDS)`) per cache miss and execute `connectionPool.evictAll()` + `dispatcher.executorService.shutdown()` in `finally` (`[NFR-POW-001]`).
- **Stream integrity & mapping:** Pre-allocate `ByteArray(expectedSize)` from `X-Artifact-Size` / `Content-Length` (`1..16 MB`) or fall back to a `16 MB`-capped `ByteArrayOutputStream`. Reject 0-byte, truncated (`bytesRead != expectedSize`), oversized, or bad-SHA payloads with `PAYLOAD_CORRUPT` / `PAYLOAD_TOO_LARGE` via typed `BlobHttpException` / `BlobPayloadException`, caching only verified bytes in RAM-only `ArtifactMemoryCache` (`[REQ-VER-003]`).

---

## Verification
- Backend integration tests covering `/v1/blobs` `401`/`403` RBAC, `> 2 MiB` streaming `PUT`, `413` + RAII `.tmp` cleanup, `422`/`409`/`200`, `206`/`416` `Range` handling, and `validate_wasm_artifact` `BlobKind::Apk` rejection.
- Android `MockWebServer` tests covering `ArtifactResolver` pre-allocated RAM download, truncated/corrupted `PAYLOAD_CORRUPT` rejection, `Call.cancel()` abortion, and `< 200 ms` cellular `OkHttpClient` teardown.
