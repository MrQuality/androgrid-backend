//! Embedded Web Dashboard and Server-Sent Events (SSE) Observability Service
//!
//! Exposes Axum HTTP routes serving the embedded, self-contained single-page dashboard,
//! JSON cluster state snapshots, and real-time SSE event streams.
//!
//! Traces to: AG-SPEC-2026-V2, [INV-004], [INV-005], [NFR-REL-001], [REQ-OP-001], [REQ-OP-004]

use axum::extract::State;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use std::convert::Infallible;
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::observability::snapshot::ClusterSnapshot;
use crate::scheduler::engine::ClusterScheduler;

/// Embedded single-page dashboard HTML asset.
const DASHBOARD_HTML: &str = include_str!("dashboard.html");

/// Builds the Axum router configured with cluster dashboard and observability routes.
///
/// Traces to: [INV-004], [INV-005], [NFR-REL-001]
pub fn dashboard_router(scheduler: Arc<ClusterScheduler>) -> Router {
    Router::new()
        .route("/", get(handle_dashboard))
        .route("/dashboard", get(handle_dashboard))
        .route("/api/snapshot", get(handle_snapshot))
        .route("/api/events", get(handle_events))
        .route("/events", get(handle_events))
        .with_state(scheduler)
}

/// Serves the self-contained HTML5 dashboard interface.
async fn handle_dashboard() -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        "text/html; charset=utf-8".parse().expect("Valid header"),
    );
    headers.insert(
        CACHE_CONTROL,
        "no-cache, no-store, must-revalidate"
            .parse()
            .expect("Valid header"),
    );
    (StatusCode::OK, headers, DASHBOARD_HTML).into_response()
}

/// Serves point-in-time JSON snapshot of cluster state.
///
/// Traces to: [REQ-OP-001], [REQ-OP-004]
async fn handle_snapshot(State(scheduler): State<Arc<ClusterScheduler>>) -> Json<ClusterSnapshot> {
    Json(scheduler.get_snapshot())
}

/// Serves asynchronous Server-Sent Events stream of real-time cluster state transitions.
///
/// Traces to: [INV-004], [INV-005], [NFR-REL-001]
async fn handle_events(
    State(scheduler): State<Arc<ClusterScheduler>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>> + Send + 'static> {
    let rx = scheduler.subscribe_events();
    let stream = BroadcastStream::new(rx).filter_map(|res| match res {
        Ok(ev) => match serde_json::to_string(&ev) {
            Ok(json) => Some(Ok(Event::default().data(json))),
            Err(_) => None,
        },
        Err(_) => None, // Gracefully drop lagged broadcast events without closing stream
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}
