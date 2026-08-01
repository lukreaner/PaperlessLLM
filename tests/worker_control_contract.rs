use std::path::Path;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use paperless_llm::{
    RuntimeOptions,
    app::{build_router, initialize},
};
use serde_json::Value;
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

async fn json_body(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("response body");
    serde_json::from_slice(&body).expect("JSON response")
}

#[tokio::test]
async fn frequently_polled_status_uses_only_non_secret_configuration_queries() {
    let directory = TempDir::new().expect("temporary directory");
    let state = initialize(runtime(directory.path())).await.expect("state");
    let database = state.database.clone();
    sqlx::query(
        "INSERT INTO paperless_connections(id,name,base_url,token_secret_key) \
         VALUES('default','Paperless','http://127.0.0.1:1','status-invalid-secret'); \
         INSERT INTO provider_profiles(id,name,provider_kind,base_url,model,api_family) \
         VALUES('provider','Provider','custom','http://127.0.0.1:1/v1','model','responses'), \
               ('selected','Selected','custom','http://127.0.0.1:2/v1','model','responses'); \
         INSERT INTO processing_profiles(id,name,paperless_connection_id,provider_profile_id,enabled) \
         VALUES('profile','Default','default','provider',1); \
         INSERT INTO secret_values(key,nonce,ciphertext) \
         VALUES('status-invalid-secret',X'00',X'00'); \
         INSERT INTO app_settings(key,value_json) \
         VALUES('dashboard_settings','{ deliberately malformed settings')",
    )
    .execute(database.pool())
    .await
    .expect("status fixtures");
    database
        .set_setting(
            "paperless_health",
            &serde_json::json!({
                "ok": true,
                "tested_at": "2026-08-01T12:00:00Z",
                "message": "Paperless ready"
            }),
        )
        .await
        .expect("Paperless health");
    database
        .set_setting("active_provider_id", &Some("selected".to_owned()))
        .await
        .expect("active provider fast-path setting");
    database
        .set_setting(
            "provider_health:selected",
            &serde_json::json!({
                "ok": true,
                "tested_at": "2026-08-01T12:00:00Z",
                "message": "Selected provider ready"
            }),
        )
        .await
        .expect("provider health");
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/status")
                .header(header::HOST, "localhost:8765")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("status response");

    assert_eq!(response.status(), StatusCode::OK);
    let status = json_body(response).await;
    assert_eq!(status["setup_required"], false);
    assert_eq!(status["paperless"]["state"], "connected");
    assert_eq!(status["provider"]["state"], "connected");
    assert_eq!(status["provider"]["message"], "Selected provider ready");
}

#[tokio::test]
async fn worker_control_routes_report_and_persist_pause_state() {
    let directory = TempDir::new().expect("temporary directory");
    let state = initialize(runtime(directory.path())).await.expect("state");
    let database = state.database.clone();
    let app = build_router(state);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/worker")
                .header(header::HOST, "localhost:8765")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("worker response");
    assert_eq!(response.status(), StatusCode::OK);
    let worker = json_body(response).await;
    assert_eq!(worker["paused"], false);
    assert!(worker.get("active_job").is_none());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/worker/pause")
                .header(header::HOST, "localhost:8765")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("pause response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/worker/pause")
                .header(header::HOST, "localhost:8765")
                .header("x-paperlessllm-action", "worker-control")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("pause response");
    assert_eq!(response.status(), StatusCode::OK);
    let worker = json_body(response).await;
    assert_eq!(worker["paused"], true);
    assert_eq!(worker["state"], "paused");
    assert_eq!(
        database
            .get_setting::<bool>("worker_paused")
            .await
            .expect("pause setting"),
        Some(true)
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/scan")
                .header(header::HOST, "localhost:8765")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("scan response");
    assert_eq!(response.status(), StatusCode::CONFLICT);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/worker/resume")
                .header(header::HOST, "localhost:8765")
                .header("x-paperlessllm-action", "worker-control")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("resume response");
    assert_eq!(response.status(), StatusCode::OK);
    let worker = json_body(response).await;
    assert_eq!(worker["paused"], false);
    assert_eq!(
        database
            .get_setting::<bool>("worker_paused")
            .await
            .expect("pause setting"),
        Some(false)
    );
}

#[tokio::test]
async fn cancel_route_rejects_only_waiting_jobs_with_an_audit_reason() {
    let directory = TempDir::new().expect("temporary directory");
    let state = initialize(runtime(directory.path())).await.expect("state");
    let database = state.database.clone();
    paperless_llm::worker::set_paused(&state, true)
        .await
        .expect("pause worker");
    sqlx::query(
        "INSERT INTO paperless_connections(id,name,base_url,token_secret_key) \
         VALUES('paperless','Paperless','http://127.0.0.1:1','paperless:test'); \
         INSERT INTO provider_profiles(id,name,provider_kind,base_url,model,api_family) \
         VALUES('provider','Provider','custom','http://127.0.0.1:1/v1','model','responses'); \
         INSERT INTO processing_profiles(id,name,paperless_connection_id,provider_profile_id,enabled) \
         VALUES('profile','Default','paperless','provider',1); \
         INSERT INTO jobs(id,processing_profile_id,profile_revision,paperless_document_id,document_checksum,status,lease_owner,lease_expires_at) \
         VALUES('waiting','profile',1,1,'checksum-waiting','queued',NULL,NULL), \
               ('active','profile',1,2,'checksum-active','awaiting_review',NULL,NULL)",
    )
    .execute(database.pool())
    .await
    .expect("job fixtures");
    let app = build_router(state);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/jobs/waiting/cancel")
                .header(header::HOST, "localhost:8765")
                .header("x-paperlessllm-action", "worker-control")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("cancel response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["state"], "rejected");
    assert_eq!(body["reason"], "cancelled_by_user");
    let audit: (String, Option<String>, Option<String>) =
        sqlx::query_as("SELECT status,error_code,completed_at FROM jobs WHERE id='waiting'")
            .fetch_one(database.pool())
            .await
            .expect("cancelled job");
    assert_eq!(audit.0, "rejected");
    assert_eq!(audit.1.as_deref(), Some("cancelled_by_user"));
    assert!(audit.2.is_some());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/jobs/active/cancel")
                .header(header::HOST, "localhost:8765")
                .header("x-paperlessllm-action", "worker-control")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("cancel response");
    assert_eq!(response.status(), StatusCode::CONFLICT);
}
