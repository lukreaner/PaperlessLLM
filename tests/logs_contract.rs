use std::path::Path;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use paperless_llm::{
    RuntimeOptions,
    app::{build_router, initialize},
    worker,
};
use tempfile::TempDir;
use tower::ServiceExt;

fn runtime(path: &Path) -> RuntimeOptions {
    RuntimeOptions {
        data_dir: path.to_owned(),
        bind: "127.0.0.1".parse().expect("loopback"),
        port: 8765,
        open_browser: false,
    }
}

#[tokio::test]
async fn diagnostic_log_responses_are_never_cacheable() {
    let directory = TempDir::new().expect("temporary directory");
    let state = initialize(runtime(directory.path())).await.expect("state");
    let response = build_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/logs")
                .header(header::HOST, "localhost:8765")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("logs response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL),
        Some(&"no-store, private".parse().expect("header value"))
    );
}

#[tokio::test]
async fn diagnostic_log_api_returns_only_structured_safe_context() {
    let directory = TempDir::new().expect("temporary directory");
    std::fs::write(
        directory.path().join("paperless-llm.log"),
        concat!(
            "PAPERLESSLLM-DESKTOP-LOG-V1\n",
            "2026-08-01T10:00:00Z WARN manual Paperless scan failed document_id=73 ",
            "error=Paperless pagination attempted to leave the configured origin ",
            "custom_headers={Authorization:Bearer private-token} ",
            "ocr_text=private document body\n",
            "2026-08-01T10:00:01Z WARN provider request failed status=failed ",
            "error=provider analysis failed with HTTP 400: ",
            "PROMPT_PRIVATE_DIAGNOSTIC_ECHO sk-hostile-diagnostic-token\n"
        ),
    )
    .expect("diagnostic log");
    let state = initialize(runtime(directory.path())).await.expect("state");
    let response = build_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/logs")
                .header(header::HOST, "localhost:8765")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("logs response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL),
        Some(&"no-store, private".parse().expect("header value"))
    );
    let body = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("response body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("JSON response");
    assert_eq!(json["entries"][0]["event"], "paperless.scan_failed");
    assert_eq!(json["entries"][0]["target"], "paperless");
    assert_eq!(json["entries"][0]["fields"]["document_id"], "73");
    assert_eq!(
        json["entries"][0]["fields"]["error_code"],
        "paperless_pagination_origin"
    );
    assert_eq!(json["worker"]["state"], "running");
    assert_eq!(json["worker"]["active"], false);
    let serialized = String::from_utf8(body.to_vec()).expect("UTF-8 JSON");
    assert!(!serialized.contains("private-token"));
    assert!(!serialized.contains("private document body"));
    assert!(!serialized.contains("custom_headers"));
    assert!(!serialized.contains("ocr_text"));
    assert!(!serialized.contains("PROMPT_PRIVATE_DIAGNOSTIC_ECHO"));
    assert!(!serialized.contains("sk-hostile-diagnostic-token"));
}

#[tokio::test]
async fn diagnostic_log_worker_snapshot_distinguishes_paused_state() {
    let directory = TempDir::new().expect("temporary directory");
    let state = initialize(runtime(directory.path())).await.expect("state");
    worker::set_paused(&state, true)
        .await
        .expect("pause worker");
    let response = build_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/logs")
                .header(header::HOST, "localhost:8765")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("logs response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("response body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("JSON response");
    assert_eq!(json["worker"]["state"], "paused");
    assert_eq!(json["worker"]["active"], false);
}
