use axum::{
    Json,
    extract::State,
    http::{HeaderValue, header},
};
use serde::Serialize;

use crate::{app::AppState, diagnostics::DiagnosticLogResponse};

#[derive(Debug, Serialize)]
pub(super) struct DiagnosticLogsResponse {
    #[serde(flatten)]
    log: DiagnosticLogResponse,
    worker: DiagnosticWorkerSnapshot,
}

/// A current, deliberately low-detail worker snapshot. In particular, this
/// omits the worker's raw last error and active job identifier.
#[derive(Debug, Serialize)]
struct DiagnosticWorkerSnapshot {
    state: &'static str,
    active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    scan_discovered: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scan_queued: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_scan_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_scan_at: Option<String>,
}

/// This handler is mounted inside the authenticated API router. The path is
/// deliberately fixed server-side; callers cannot browse the data directory.
pub(super) async fn tail(
    State(state): State<AppState>,
) -> (
    [(axum::http::HeaderName, HeaderValue); 1],
    Json<DiagnosticLogsResponse>,
) {
    let runtime = state.worker.runtime_state().await;
    let scan_discovered = runtime
        .scan_progress
        .as_ref()
        .map(|progress| progress.discovered);
    let scan_queued = runtime
        .scan_progress
        .as_ref()
        .map(|progress| progress.queued);
    let worker = DiagnosticWorkerSnapshot {
        state: match runtime.state.as_str() {
            "running" => "running",
            "scanning" => "scanning",
            "pausing" => "pausing",
            "paused" => "paused",
            "error" => "error",
            _ => "unknown",
        },
        active: runtime.active_job_id.is_some(),
        scan_discovered,
        scan_queued,
        last_scan_at: runtime.last_scan_at,
        next_scan_at: runtime.next_scan_at,
    };
    (
        [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
        Json(DiagnosticLogsResponse {
            log: crate::diagnostics::read_desktop_log(&state.runtime.data_dir).await,
            worker,
        }),
    )
}
