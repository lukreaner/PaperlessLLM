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

#[tokio::test]
async fn activity_reports_safe_failure_context_and_latest_provider_attempt() {
    let directory = TempDir::new().expect("temporary directory");
    let state = initialize(runtime(directory.path())).await.expect("state");
    sqlx::query(
        "INSERT INTO paperless_connections(id,name,base_url,token_secret_key) \
         VALUES('paperless','Paperless','https://paperless.example','unused-secret'); \
         INSERT INTO provider_profiles(id,name,provider_kind,provider_preset,base_url,model,api_family) \
         VALUES('provider','OpenAI profile','openai','openai','https://api.openai.com/v1','safe-model','responses'); \
         INSERT INTO processing_profiles(id,name,paperless_connection_id,provider_profile_id,enabled) \
         VALUES('profile','Default','paperless','provider',1); \
         INSERT INTO jobs(id,processing_profile_id,profile_revision,paperless_document_id,document_checksum,document_title,status,attempt_count,error_code,error_message,updated_at) \
         VALUES \
           ('preflight','profile',1,2388,'checksum-a','Preflight document','failed',1,'analysis_failed','request validation failed','2026-08-01T12:00:00Z'), \
           ('provider-failure','profile',1,2389,'checksum-b','Provider document','failed',2,'analysis_failed','Provider request failed with HTTP 429: api_key=sk-private prompt=private OCR text','2026-08-01T12:01:00Z'), \
           ('repair-preflight','profile',1,2390,'checksum-c','Repair preflight document','failed',1,'validation_failed','request validation failed: OCR_PRIVATE','2026-08-01T12:02:00Z'), \
           ('provider-configuration','profile',1,2391,'checksum-d','Provider configuration document','failed',1,'provider_configuration','request validation failed: api_key=sk-terminal-secret','2026-08-01T12:03:00Z'), \
           ('paperless-configuration','profile',1,2392,'checksum-e','Paperless configuration document','failed',1,'paperless_configuration','request validation failed: Paperless token=paperless-private','2026-08-01T12:04:00Z'), \
           ('paperless-not-found','profile',1,2393,'checksum-f','Paperless missing document','failed',1,'paperless_not_found','not found: private OCR document','2026-08-01T12:05:00Z'), \
           ('paperless-authentication','profile',1,2394,'checksum-g','Paperless authentication document','failed',1,'paperless_authentication','Paperless authentication failed: Token paperless-private','2026-08-01T12:06:00Z'), \
           ('paperless-forbidden','profile',1,2395,'checksum-h','Paperless permission document','failed',1,'paperless_forbidden','Paperless API permissions are insufficient: private path','2026-08-01T12:07:00Z'), \
           ('generic-not-found','profile',1,2396,'checksum-i','Generic missing document','failed',1,'not_found','private missing item','2026-08-01T12:08:00Z'), \
           ('generic-forbidden','profile',1,2397,'checksum-j','Generic forbidden document','failed',1,'forbidden','private forbidden reason','2026-08-01T12:09:00Z'), \
           ('generic-conflict','profile',1,2398,'checksum-k','Generic conflict document','failed',1,'conflict','private conflicting state','2026-08-01T12:10:00Z'); \
         INSERT INTO usage_events(id,job_id,provider_profile_id,provider_kind,model,api_family,operation,status,input_tokens,output_tokens,total_tokens,usage_source,latency_ms,error_code,created_at) \
         VALUES \
           ('usage-failure','provider-failure','provider','openai','safe-model','responses','repair','failed',80,19,99,'reported',321,'provider_remote','2026-08-01T12:00:59Z'), \
           ('usage-before-repair-preflight','repair-preflight','provider','openai','safe-model','responses','analysis','failed',5,2,7,'reported',111,'invalid_structured_output','2026-08-01T12:01:59Z'), \
           ('usage-before-configuration','provider-configuration','provider','openai','safe-model','responses','analysis','failed',6,2,8,'reported',112,'invalid_structured_output','2026-08-01T12:02:59Z')",
    )
    .execute(state.database.pool())
    .await
    .expect("activity fixtures");

    let response = build_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/jobs?state=failed")
                .header(header::HOST, "localhost:8765")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("jobs response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 128 * 1024)
        .await
        .expect("response body");
    let json: Value = serde_json::from_slice(&body).expect("JSON response");

    let items = json["items"].as_array().expect("activity items");
    let job = |document_id: i64| {
        items
            .iter()
            .find(|item| item["document_id"] == document_id)
            .expect("document activity")
    };

    let provider = job(2389);
    assert_eq!(provider["document_id"], 2389);
    assert_eq!(provider["error_code"], "provider_remote");
    assert_eq!(provider["failure_phase"], "provider");
    assert_eq!(provider["error"], "The AI provider returned HTTP 429.");
    assert_eq!(provider["upstream_status"], 429);
    assert_eq!(provider["provider_kind"], "openai");
    assert_eq!(provider["api_family"], "responses");
    assert_eq!(provider["provider_operation"], "repair");
    assert_eq!(provider["provider_attempt"], 2);
    assert_eq!(provider["provider_status"], "failed");
    assert_eq!(provider["provider_latency_ms"], 321);
    assert_eq!(provider["attempt"], 2);
    assert_eq!(provider["token_total"], 99);

    let preflight = job(2388);
    assert_eq!(preflight["document_id"], 2388);
    assert_eq!(preflight["error_code"], "validation_failed");
    assert_eq!(preflight["failure_phase"], "preflight");
    assert!(
        preflight["error"]
            .as_str()
            .is_some_and(|message| message.contains("request preflight"))
    );
    assert!(preflight["provider_operation"].is_null());
    assert!(preflight["provider_status"].is_null());
    assert!(preflight["provider_latency_ms"].is_null());

    let repair_preflight = job(2390);
    assert_eq!(repair_preflight["error_code"], "validation_failed");
    assert_eq!(repair_preflight["failure_phase"], "preflight");
    assert_eq!(repair_preflight["provider_operation"], "analysis");
    assert_eq!(repair_preflight["provider_attempt"], 1);
    assert_eq!(repair_preflight["provider_status"], "failed");
    assert_eq!(repair_preflight["token_total"], 7);
    assert!(repair_preflight["upstream_status"].is_null());

    let provider_configuration = job(2391);
    assert_eq!(
        provider_configuration["error_code"],
        "provider_configuration"
    );
    assert_eq!(provider_configuration["failure_phase"], "provider");
    assert_eq!(provider_configuration["provider_operation"], "analysis");
    assert_eq!(provider_configuration["provider_status"], "failed");
    assert!(
        provider_configuration["error"]
            .as_str()
            .is_some_and(|message| message.contains("URL, credentials, model"))
    );
    assert!(provider_configuration["upstream_status"].is_null());

    let paperless_configuration = job(2392);
    assert_eq!(
        paperless_configuration["error_code"],
        "paperless_configuration"
    );
    assert_eq!(paperless_configuration["failure_phase"], "paperless");
    assert!(
        paperless_configuration["error"]
            .as_str()
            .is_some_and(|message| message.contains("server URL, API token"))
    );
    assert!(paperless_configuration["provider_operation"].is_null());

    let paperless_not_found = job(2393);
    assert_eq!(paperless_not_found["error_code"], "paperless_not_found");
    assert_eq!(paperless_not_found["failure_phase"], "paperless");
    assert_eq!(paperless_not_found["upstream_status"], 404);
    assert!(
        paperless_not_found["error"]
            .as_str()
            .is_some_and(|message| message.contains("no longer exists"))
    );

    let paperless_authentication = job(2394);
    assert_eq!(
        paperless_authentication["error_code"],
        "paperless_authentication"
    );
    assert_eq!(paperless_authentication["failure_phase"], "paperless");
    assert_eq!(paperless_authentication["upstream_status"], 401);

    let paperless_forbidden = job(2395);
    assert_eq!(paperless_forbidden["error_code"], "paperless_forbidden");
    assert_eq!(paperless_forbidden["failure_phase"], "paperless");
    assert_eq!(paperless_forbidden["upstream_status"], 403);

    let generic_not_found = job(2396);
    assert_eq!(generic_not_found["error_code"], "not_found");
    assert_eq!(generic_not_found["failure_phase"], "processing");
    assert_eq!(
        generic_not_found["error"],
        "The requested PaperlessLLM item was not found."
    );

    let generic_forbidden = job(2397);
    assert_eq!(generic_forbidden["error_code"], "forbidden");
    assert_eq!(generic_forbidden["failure_phase"], "processing");
    assert_eq!(
        generic_forbidden["error"],
        "This operation is not permitted."
    );

    let generic_conflict = job(2398);
    assert_eq!(generic_conflict["error_code"], "conflict");
    assert_eq!(generic_conflict["failure_phase"], "processing");
    assert!(
        generic_conflict["error"]
            .as_str()
            .is_some_and(|message| message.contains("conflicts with another action"))
    );

    let serialized = String::from_utf8(body.to_vec()).expect("UTF-8 JSON");
    assert!(!serialized.contains("sk-private"));
    assert!(!serialized.contains("private OCR text"));
    assert!(!serialized.contains("api_key"));
    assert!(!serialized.contains("prompt="));
    assert!(!serialized.contains("OCR_PRIVATE"));
    assert!(!serialized.contains("sk-terminal-secret"));
    assert!(!serialized.contains("paperless-private"));
    assert!(!serialized.contains("private missing item"));
    assert!(!serialized.contains("private forbidden reason"));
    assert!(!serialized.contains("private conflicting state"));
    assert!(!serialized.contains("private OCR document"));
    assert!(!serialized.contains("private path"));
}
