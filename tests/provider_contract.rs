use std::{
    net::{IpAddr, Ipv4Addr},
    time::Duration,
};

use paperless_llm::{
    RuntimeOptions,
    api::providers,
    app,
    providers::{AnthropicMessagesAdapter, ProviderAdapter, ReasoningEffort},
};
use serde_json::json;
use tempfile::TempDir;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_partial_json, method, path, query_param, query_param_is_missing},
};

#[tokio::test]
async fn anthropic_model_discovery_follows_cursor_and_retains_capabilities() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(query_param("limit", "1000"))
        .and(query_param_is_missing("after_id"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"id":"first","capabilities":{}}],
            "has_more": true,
            "last_id": "first"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(query_param("limit", "1000"))
        .and(query_param("after_id", "first"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{
                "id":"second",
                "capabilities": {
                    "structured_outputs":{"supported":true},
                    "image_input":{"supported":true},
                    "pdf_input":{"supported":true},
                    "thinking":{"types":{"adaptive":{"supported":true}}},
                    "effort":{"supported":true,"high":{"supported":true}}
                }
            }],
            "has_more": false,
            "last_id": "second"
        })))
        .mount(&server)
        .await;

    let adapter = AnthropicMessagesAdapter::with_base_url(
        reqwest::Client::new(),
        format!("{}/v1", server.uri()),
        "second",
        "test-key",
    );
    let models = adapter.list_models().await.unwrap();
    assert_eq!(
        models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        vec!["first", "second"]
    );
    let capabilities = models[1].capabilities.as_ref().unwrap();
    assert!(capabilities.strict_json_schema);
    assert!(capabilities.supports_reasoning_effort(ReasoningEffort::High));
}

#[tokio::test]
async fn compatible_provider_test_accounts_strict_fallback_and_saved_settings() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"id":"local-model","owned_by":"test"}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_partial_json(json!({
            "response_format": {"type":"json_schema"},
            "max_completion_tokens": 64
        })))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {"message":"response_format is unsupported"}
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_partial_json(json!({
            "model":"local-model",
            "temperature":0.7,
            "reasoning_effort":"low",
            "max_completion_tokens":321
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id":"chat-test",
            "choices":[{"message":{"content":"{\"ok\":true}"}}],
            "usage":{"prompt_tokens":20,"completion_tokens":5,"total_tokens":25}
        })))
        .mount(&server)
        .await;

    let directory = TempDir::new().unwrap();
    let state = app::initialize(RuntimeOptions {
        data_dir: directory.path().to_path_buf(),
        bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
        port: 0,
        open_browser: false,
    })
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO provider_profiles(id,name,provider_kind,base_url,model,api_family,chat_output_token_field,capabilities_json,reasoning_effort,temperature,max_output_tokens,request_timeout_seconds) \
         VALUES('provider','Compatible','custom',?,'local-model','chat_completions','max_completion_tokens',?,'low',0.7,321,5)",
    )
    .bind(format!("{}/v1", server.uri()))
    .bind(json!({
        "temperature": true,
        "reasoning_effort": true,
        "thinking_budget": false,
        "strict_json_schema": false,
        "image_input": false,
        "pdf_input": false,
        "native_token_count": false
    }).to_string())
    .execute(state.database.pool())
    .await
    .unwrap();

    let tested = tokio::time::timeout(Duration::from_secs(5), providers::test(&state, "provider"))
        .await
        .unwrap();
    let result = match tested {
        Ok(result) => result,
        Err(error) => {
            let requests = server.received_requests().await.unwrap();
            panic!(
                "{error}; requests={:?}",
                requests
                    .iter()
                    .map(|request| String::from_utf8_lossy(&request.body).into_owned())
                    .collect::<Vec<_>>()
            );
        }
    };
    if !result.ok {
        let requests = server.received_requests().await.unwrap();
        panic!(
            "{}; requests={:?}",
            result.message,
            requests
                .iter()
                .map(|request| String::from_utf8_lossy(&request.body).into_owned())
                .collect::<Vec<_>>()
        );
    }
    assert!(!result.capabilities.unwrap().structured_output);
    let attempts: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT operation,status,total_tokens FROM usage_events WHERE provider_profile_id='provider' ORDER BY created_at,id",
    )
    .fetch_all(state.database.pool())
    .await
    .unwrap();
    assert_eq!(attempts.len(), 2);
    assert!(
        attempts
            .iter()
            .any(|attempt| attempt.0 == "provider_probe_strict" && attempt.1 == "failed")
    );
    assert!(attempts.iter().any(|attempt| attempt.0 == "provider_test"
        && attempt.1 == "succeeded"
        && attempt.2 == 25));
    let active_reservations: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM budget_reservations WHERE provider_profile_id='provider' AND state='reserved'",
    )
    .fetch_one(state.database.pool())
    .await
    .unwrap();
    assert_eq!(active_reservations, 0);
}
