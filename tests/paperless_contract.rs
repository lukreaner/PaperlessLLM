use paperless_llm::paperless::{
    DocumentLimit, DocumentQuery, PaperlessClient, PaperlessDocument, TriggerMatchMode,
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
async fn rebases_proxy_pagination_without_contacting_its_advertised_origin() {
    let server = MockServer::start().await;
    let advertised_origin = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags/"))
        .and(query_param("page_size", "1000"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "count":2,
            "next":format!(
                "{}/reverse-proxy-prefix/api/tags/?page=2",
                advertised_origin.uri()
            ),
            "results":[{"id":1,"name":"First"}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/tags/"))
        .and(query_param("page", "2"))
        .and(header("authorization", "Token paperless-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "count":2,"next":null,"results":[{"id":2,"name":"Second"}]
        })))
        .mount(&server)
        .await;

    let tags = client(&server).list_tags().await.unwrap();

    assert_eq!(tags.len(), 2);
    assert!(
        advertised_origin
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn rejects_pagination_links_for_a_different_api_endpoint() {
    let server = MockServer::start().await;
    let advertised_origin = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "count":2,
            "next":format!("{}/api/documents/?page=2", advertised_origin.uri()),
            "results":[{"id":1,"name":"First"}]
        })))
        .mount(&server)
        .await;

    let error = client(&server).list_tags().await.unwrap_err();

    assert!(error.to_string().contains("unsafe pagination URL"));
    assert!(
        advertised_origin
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn sends_all_trigger_filter_and_keeps_version_checksum() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/documents/"))
        .and(query_param("tags__id__all", "3,5"))
        .and(query_param("fields", "id,title,versions"))
        .and(query_param("ordering", "added,id"))
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
    let client = client(&server);
    let mut pages = client
        .document_pages(&DocumentQuery {
            trigger_tag_ids: vec![3, 5],
            trigger_match_mode: TriggerMatchMode::All,
            excluded_tag_id: None,
            limit: DocumentLimit::from_persisted(10),
        })
        .unwrap();
    let documents = pages.next_page().await.unwrap().unwrap();
    assert_eq!(documents[0].effective_checksum(), "latest-sha");
    assert!(pages.next_page().await.unwrap().is_none());
}

#[tokio::test]
async fn document_discovery_stops_at_the_requested_limit() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/documents/"))
        .and(query_param("page_size", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "count":2,
            "next":"https://proxy-advertised.example/api/documents/?page=2",
            "results":[{"id":1,"title":"First","versions":[]}]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = client(&server);
    let mut pages = client
        .document_pages(&DocumentQuery {
            trigger_tag_ids: vec![],
            trigger_match_mode: TriggerMatchMode::Any,
            excluded_tag_id: None,
            limit: DocumentLimit::from_persisted(1),
        })
        .unwrap();

    let documents = pages.next_page().await.unwrap().unwrap();
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].id, 1);
    assert!(pages.next_page().await.unwrap().is_none());
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn document_discovery_cap_is_enforced_across_page_boundaries() {
    let server = MockServer::start().await;
    let first_page = (1..=40)
        .map(|id| json!({"id":id,"title":format!("Document {id}"),"versions":[]}))
        .collect::<Vec<_>>();
    let second_page = (41..=75)
        .map(|id| json!({"id":id,"title":format!("Document {id}"),"versions":[]}))
        .collect::<Vec<_>>();
    Mock::given(method("GET"))
        .and(path("/api/documents/"))
        .and(query_param("page_size", "50"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "count":75,
            "next":format!("{}/api/documents/?page=2", server.uri()),
            "results":first_page
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/documents/"))
        .and(query_param("page", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "count":75,
            "next":"https://proxy-advertised.example/api/documents/?page=3&page_size=50",
            "results":second_page
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = client(&server);
    let mut pages = client
        .document_pages(&DocumentQuery {
            trigger_tag_ids: vec![],
            trigger_match_mode: TriggerMatchMode::Any,
            excluded_tag_id: None,
            limit: DocumentLimit::from_persisted(50),
        })
        .unwrap();

    assert_eq!(pages.next_page().await.unwrap().unwrap().len(), 40);
    let final_page = pages.next_page().await.unwrap().unwrap();
    assert_eq!(final_page.len(), 10);
    assert_eq!(final_page.last().map(|document| document.id), Some(50));
    assert!(pages.next_page().await.unwrap().is_none());
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn unlimited_document_discovery_follows_pages_beyond_fifty_results() {
    let server = MockServer::start().await;
    let first_page = (1..=40)
        .map(|id| json!({"id":id,"title":format!("Document {id}"),"versions":[]}))
        .collect::<Vec<_>>();
    let second_page = (41..=75)
        .map(|id| json!({"id":id,"title":format!("Document {id}"),"versions":[]}))
        .collect::<Vec<_>>();

    Mock::given(method("GET"))
        .and(path("/api/documents/"))
        .and(query_param("page_size", "1000"))
        .and(query_param("tags__id__in", "7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "count":75,
            "next":format!("{}/api/documents/?page=2&tags__id__in=7", server.uri()),
            "results":first_page
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/documents/"))
        .and(query_param("page", "2"))
        .and(query_param("tags__id__in", "7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "count":75,
            "next":null,
            "results":second_page
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = client(&server);
    let mut pages = client
        .document_pages(&DocumentQuery {
            trigger_tag_ids: vec![7],
            trigger_match_mode: TriggerMatchMode::Any,
            excluded_tag_id: None,
            limit: DocumentLimit::All,
        })
        .unwrap();

    let first = pages.next_page().await.unwrap().unwrap();
    assert_eq!(first.len(), 40);
    assert_eq!(first.first().map(|document| document.id), Some(1));
    drop(first);

    let second = pages.next_page().await.unwrap().unwrap();
    assert_eq!(second.len(), 35);
    assert_eq!(second.first().map(|document| document.id), Some(41));
    assert_eq!(second.last().map(|document| document.id), Some(75));
    drop(second);

    assert!(pages.next_page().await.unwrap().is_none());
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
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
