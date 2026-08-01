use paperless_llm::paperless::{
    DocumentQuery, PaperlessClient, PaperlessDocument, TriggerMatchMode,
};
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path, query_param},
};
use zeroize::Zeroizing;

fn client(server: &MockServer) -> PaperlessClient {
    PaperlessClient::new(
        &server.uri(),
        Zeroizing::new("paperless-token".to_owned()),
        None,
    )
    .unwrap()
}

#[tokio::test]
async fn requires_v3_api_v10_headers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/documents/"))
        .and(header("authorization", "Token paperless-token"))
        .and(header("accept", "application/json; version=10"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-api-version", "10")
                .insert_header("x-version", "3.1.0")
                .set_body_json(json!({"count":0,"next":null,"results":[]})),
        )
        .mount(&server)
        .await;
    let probe = client(&server).probe().await.unwrap();
    assert_eq!(probe.api_version, 10);
    assert_eq!(probe.server_version, "3.1.0");
    assert_eq!(probe.document_count, 0);
}

#[tokio::test]
async fn follows_same_origin_pagination_and_parses_nested_tags() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags/"))
        .and(query_param("page_size", "1000"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "count":2,
            "next":format!("{}/api/tags/?page=2", server.uri()),
            "results":[{"id":1,"name":"Parent","parent":null}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/tags/"))
        .and(query_param("page", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "count":2,"next":null,"results":[{"id":2,"name":"Child","parent":1}]
        })))
        .mount(&server)
        .await;
    let tags = client(&server).list_tags().await.unwrap();
    assert_eq!(tags.len(), 2);
    assert_eq!(tags[1].parent, Some(1));
}

#[tokio::test]
async fn sends_all_trigger_filter_and_keeps_version_checksum() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/documents/"))
        .and(query_param("tags__id__all", "3,5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "count":1,"next":null,"results":[{
                "id":42,"title":"Invoice","content":"text","tags":[3,5],
                "versions":[
                    {"id":99,"checksum":"latest-sha","is_root":false},
                    {"id":42,"checksum":"old-sha","is_root":true}
                ]
            }]
        })))
        .mount(&server)
        .await;
    let documents = client(&server)
        .list_documents(&DocumentQuery {
            trigger_tag_ids: vec![3, 5],
            trigger_match_mode: TriggerMatchMode::All,
            excluded_tag_id: None,
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(documents[0].effective_checksum(), "latest-sha");
}

#[tokio::test]
async fn taxonomy_creation_uses_server_defaults_and_permission_errors_are_clear() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/tags/"))
        .and(body_json(json!({"name":"AI label"})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id":7,"name":"AI label","parent":null
        })))
        .mount(&server)
        .await;
    let tag = client(&server).create_tag(" AI label ").await.unwrap();
    assert_eq!(tag.id, 7);

    Mock::given(method("GET"))
        .and(path("/api/documents/12/"))
        .respond_with(ResponseTemplate::new(403).set_body_string("not allowed"))
        .mount(&server)
        .await;
    let error = client(&server).get_document(12).await.unwrap_err();
    assert!(error.to_string().contains("permissions are insufficient"));
}

#[test]
fn metadata_edits_do_not_change_checksum_fallback() {
    let first: PaperlessDocument = serde_json::from_value(json!({
        "id":3,"title":"Before","modified":"2026-01-01T00:00:00Z","versions":[]
    }))
    .unwrap();
    let second: PaperlessDocument = serde_json::from_value(json!({
        "id":3,"title":"After","modified":"2026-02-01T00:00:00Z","versions":[]
    }))
    .unwrap();
    assert_eq!(first.effective_checksum(), second.effective_checksum());
}
