use std::{net::IpAddr, path::Path};

use axum::{
    body::Body,
    http::{Request, header},
};
use http_body_util::BodyExt;
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
        bind: IpAddr::from([127, 0, 0, 1]),
        port: 8765,
        open_browser: false,
    }
}

async fn response_json(app: axum::Router, uri: &str) -> Value {
    let response = app
        .oneshot(
            Request::builder()
                .uri(uri)
                .header(header::HOST, "localhost:8765")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("statistics response");
    assert!(response.status().is_success());
    let body = response
        .into_body()
        .collect()
        .await
        .expect("statistics body")
        .to_bytes();
    serde_json::from_slice(&body).expect("statistics JSON")
}

#[tokio::test]
async fn statistics_preserve_every_usage_source_category() {
    let directory = TempDir::new().expect("temporary directory");
    let state = initialize(runtime(directory.path())).await.expect("state");
    sqlx::query(
        "INSERT INTO provider_profiles(id,name,provider_kind,base_url,model,api_family) \
         VALUES('provider','Local','lmstudio','http://localhost:1234/v1','model','responses')",
    )
    .execute(state.database.pool())
    .await
    .expect("provider");
    for (index, source) in [
        "reported",
        "preflight",
        "estimated",
        "unknown",
        "reserved_after_unknown",
    ]
    .into_iter()
    .enumerate()
    {
        sqlx::query(
            "INSERT INTO usage_events(id,provider_profile_id,provider_kind,model,api_family,status,total_tokens,usage_source) \
             VALUES(?, 'provider', 'lmstudio', 'model', 'responses', 'succeeded', 10, ?)",
        )
        .bind(format!("usage-{index}"))
        .bind(source)
        .execute(state.database.pool())
        .await
        .expect("usage event");
    }

    let app = build_router(state);
    let summary = response_json(app.clone(), "/api/stats/summary?range=all").await;
    assert_eq!(summary["totals"]["requests"], 5);
    assert_eq!(summary["totals"]["reported_requests"], 1);
    assert_eq!(summary["totals"]["preflight_counted_requests"], 1);
    assert_eq!(summary["totals"]["estimated_requests"], 1);
    assert_eq!(summary["totals"]["unknown_requests"], 2);

    let series = response_json(app, "/api/stats/series?range=all").await;
    let point = &series["points"][0];
    assert_eq!(point["reported_requests"], 1);
    assert_eq!(point["preflight_counted_requests"], 1);
    assert_eq!(point["estimated_requests"], 1);
    assert_eq!(point["unknown_requests"], 2);
}
