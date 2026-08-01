use std::{collections::HashMap, time::Duration};

use bytes::Bytes;
use reqwest::{
    Client, Method, Response,
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderValue},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use url::Url;
use zeroize::Zeroizing;

use crate::{AppError, AppResult};

const API_ACCEPT: &str = "application/json; version=10";
const MAX_ERROR_CHARS: usize = 500;

#[derive(Clone)]
pub struct PaperlessClient {
    client: Client,
    base_url: Url,
    authorization: HeaderValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionProbe {
    pub api_version: u32,
    pub server_version: String,
    pub document_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperlessTag {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub parent: Option<i64>,
    #[serde(default)]
    pub document_count: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperlessNamedObject {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub document_count: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperlessCustomFieldDefinition {
    pub id: i64,
    pub name: String,
    pub data_type: String,
    #[serde(default)]
    pub extra_data: Value,
    #[serde(default)]
    pub document_count: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PaperlessCustomFieldValue {
    pub field: i64,
    pub value: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperlessDocumentVersion {
    pub id: i64,
    #[serde(default)]
    pub checksum: Option<String>,
    #[serde(default)]
    pub is_root: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperlessDocument {
    pub id: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub tags: Vec<i64>,
    #[serde(default)]
    pub correspondent: Option<i64>,
    #[serde(default)]
    pub document_type: Option<i64>,
    #[serde(default)]
    pub custom_fields: Vec<PaperlessCustomFieldValue>,
    #[serde(default)]
    pub modified: Option<String>,
    #[serde(default)]
    pub added: Option<String>,
    #[serde(default)]
    pub page_count: Option<i64>,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub original_file_name: Option<String>,
    #[serde(default)]
    pub archived_file_name: Option<String>,
    #[serde(default)]
    pub versions: Vec<PaperlessDocumentVersion>,
}

impl PaperlessDocument {
    pub fn effective_checksum(&self) -> String {
        self.versions
            .first()
            .and_then(|version| version.checksum.clone())
            .unwrap_or_else(|| format!("document-{}", self.id))
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DocumentPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correspondent: Option<Option<i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_type: Option<Option<i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_fields: Option<Vec<PaperlessCustomFieldValue>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomFieldDefinitionCreate {
    pub name: String,
    pub data_type: String,
    #[serde(default)]
    pub extra_data: Value,
}

#[derive(Debug, Clone, Copy)]
pub enum TriggerMatchMode {
    Any,
    All,
}

#[derive(Debug, Clone)]
pub struct DocumentQuery {
    pub trigger_tag_ids: Vec<i64>,
    pub trigger_match_mode: TriggerMatchMode,
    pub excluded_tag_id: Option<i64>,
    pub limit: usize,
}

#[derive(Debug, Clone)]
pub struct DownloadedDocument {
    pub bytes: Bytes,
    pub content_type: Option<String>,
    pub filename: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Paginated<T> {
    #[allow(dead_code)]
    count: i64,
    next: Option<String>,
    results: Vec<T>,
}

impl PaperlessClient {
    pub fn new(
        base_url: &str,
        token: Zeroizing<String>,
        custom_ca_pem: Option<&str>,
    ) -> AppResult<Self> {
        let base_url = normalize_base_url(base_url)?;
        let mut builder = Client::builder()
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(15))
            .user_agent(concat!("PaperlessLLM/", env!("CARGO_PKG_VERSION")));
        if let Some(pem) = custom_ca_pem.filter(|pem| !pem.trim().is_empty()) {
            let certificate = reqwest::Certificate::from_pem(pem.as_bytes()).map_err(|_| {
                AppError::Validation(
                    "the configured Paperless CA certificate is invalid".to_owned(),
                )
            })?;
            builder = builder.add_root_certificate(certificate);
        }
        let client = builder
            .build()
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let mut authorization = HeaderValue::from_str(&format!("Token {}", token.as_str()))
            .map_err(|_| {
                AppError::Validation("Paperless token contains invalid characters".to_owned())
            })?;
        authorization.set_sensitive(true);
        Ok(Self {
            client,
            base_url,
            authorization,
        })
    }

    pub async fn probe(&self) -> AppResult<ConnectionProbe> {
        let url = self.api_url("documents/")?;
        let response = self
            .request(Method::GET, url)
            .query(&[("page_size", "1"), ("fields", "id")])
            .send()
            .await
            .map_err(map_transport_error)?;
        let response = ensure_success(response).await?;
        let api_version = response
            .headers()
            .get("x-api-version")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u32>().ok())
            .ok_or_else(|| {
                AppError::Upstream("Paperless did not report an API version".to_owned())
            })?;
        let server_version = response
            .headers()
            .get("x-version")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        if api_version != 10 {
            return Err(AppError::Upstream(format!(
                "Paperless API v10 is required; server selected v{api_version}"
            )));
        }
        if !server_version.starts_with('3') {
            return Err(AppError::Upstream(format!(
                "Paperless-ngx v3 is required; server reported {server_version}"
            )));
        }
        let page: Paginated<Value> = response.json().await.map_err(map_transport_error)?;
        Ok(ConnectionProbe {
            api_version,
            server_version,
            document_count: page.count,
        })
    }

    pub async fn list_tags(&self) -> AppResult<Vec<PaperlessTag>> {
        self.list_all("tags/", &[("ordering", "name")]).await
    }

    pub async fn list_correspondents(&self) -> AppResult<Vec<PaperlessNamedObject>> {
        self.list_all("correspondents/", &[("ordering", "name")])
            .await
    }

    pub async fn list_document_types(&self) -> AppResult<Vec<PaperlessNamedObject>> {
        self.list_all("document_types/", &[("ordering", "name")])
            .await
    }

    pub async fn list_custom_fields(&self) -> AppResult<Vec<PaperlessCustomFieldDefinition>> {
        self.list_all("custom_fields/", &[("ordering", "name")])
            .await
    }

    pub async fn get_document(&self, id: i64) -> AppResult<PaperlessDocument> {
        let response = self
            .request(Method::GET, self.api_url(&format!("documents/{id}/"))?)
            .send()
            .await
            .map_err(map_transport_error)?;
        decode_success(response).await
    }

    pub async fn list_documents(&self, query: &DocumentQuery) -> AppResult<Vec<PaperlessDocument>> {
        for id in query.trigger_tag_ids.iter().copied() {
            if id <= 0 {
                return Err(AppError::Validation(
                    "trigger tag IDs must be positive integers".to_owned(),
                ));
            }
        }
        if query.excluded_tag_id.is_some_and(|id| id <= 0) {
            return Err(AppError::Validation(
                "processed tag ID must be a positive integer".to_owned(),
            ));
        }

        let mut params = vec![
            (
                "fields".to_owned(),
                "id,title,content,tags,correspondent,document_type,custom_fields,modified,added,page_count,mime_type,original_file_name,archived_file_name,versions".to_owned(),
            ),
            ("ordering".to_owned(), "added,id".to_owned()),
            ("page_size".to_owned(), query.limit.clamp(1, 1000).to_string()),
        ];
        if !query.trigger_tag_ids.is_empty() {
            let value = query
                .trigger_tag_ids
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(",");
            params.push((
                match query.trigger_match_mode {
                    TriggerMatchMode::Any => "tags__id__in",
                    TriggerMatchMode::All => "tags__id__all",
                }
                .to_owned(),
                value,
            ));
        }
        if let Some(id) = query.excluded_tag_id {
            params.push(("tags__id__none".to_owned(), id.to_string()));
        }
        let borrowed = params
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect::<Vec<_>>();
        let mut documents: Vec<PaperlessDocument> = self.list_all("documents/", &borrowed).await?;
        documents.truncate(query.limit);
        Ok(documents)
    }

    pub async fn patch_document(
        &self,
        id: i64,
        patch: &DocumentPatch,
    ) -> AppResult<PaperlessDocument> {
        let response = self
            .request(Method::PATCH, self.api_url(&format!("documents/{id}/"))?)
            .json(patch)
            .send()
            .await
            .map_err(map_transport_error)?;
        decode_success(response).await
    }

    pub async fn create_tag(&self, name: &str) -> AppResult<PaperlessTag> {
        self.create_named("tags/", name).await
    }

    pub async fn create_correspondent(&self, name: &str) -> AppResult<PaperlessNamedObject> {
        self.create_named("correspondents/", name).await
    }

    pub async fn create_document_type(&self, name: &str) -> AppResult<PaperlessNamedObject> {
        self.create_named("document_types/", name).await
    }

    pub async fn create_custom_field(
        &self,
        definition: &CustomFieldDefinitionCreate,
    ) -> AppResult<PaperlessCustomFieldDefinition> {
        validate_custom_field_type(&definition.data_type)?;
        let response = self
            .request(Method::POST, self.api_url("custom_fields/")?)
            .json(definition)
            .send()
            .await
            .map_err(map_transport_error)?;
        decode_success(response).await
    }

    pub async fn download_document(
        &self,
        id: i64,
        original: bool,
        max_bytes: usize,
    ) -> AppResult<DownloadedDocument> {
        let response = self
            .request(
                Method::GET,
                self.api_url(&format!("documents/{id}/download/"))?,
            )
            .query(&[("original", if original { "true" } else { "false" })])
            .send()
            .await
            .map_err(map_transport_error)?;
        let mut response = ensure_success(response).await?;
        if response
            .content_length()
            .is_some_and(|length| length > max_bytes as u64)
        {
            return Err(AppError::Validation(format!(
                "document file exceeds the configured {} byte limit",
                max_bytes
            )));
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let filename = response
            .headers()
            .get("content-disposition")
            .and_then(|value| value.to_str().ok())
            .and_then(parse_filename);
        let initial_capacity =
            response.content_length().unwrap_or(0).min(max_bytes as u64) as usize;
        let mut body = Vec::with_capacity(initial_capacity);
        while let Some(chunk) = response.chunk().await.map_err(map_transport_error)? {
            if chunk.len() > max_bytes.saturating_sub(body.len()) {
                return Err(AppError::Validation(format!(
                    "document file exceeds the configured {} byte limit",
                    max_bytes
                )));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(DownloadedDocument {
            bytes: Bytes::from(body),
            content_type,
            filename,
        })
    }

    pub fn document_url(&self, id: i64) -> String {
        self.base_url
            .join(&format!("documents/{id}/details"))
            .unwrap_or_else(|_| self.base_url.clone())
            .to_string()
    }

    async fn create_named<T: DeserializeOwned>(&self, endpoint: &str, name: &str) -> AppResult<T> {
        let name = validate_name(name)?;
        let response = self
            .request(Method::POST, self.api_url(endpoint)?)
            .json(&json!({ "name": name }))
            .send()
            .await
            .map_err(map_transport_error)?;
        decode_success(response).await
    }

    async fn list_all<T: DeserializeOwned>(
        &self,
        endpoint: &str,
        params: &[(&str, &str)],
    ) -> AppResult<Vec<T>> {
        let mut url = self.api_url(endpoint)?;
        url.query_pairs_mut()
            .append_pair("page_size", "1000")
            .extend_pairs(params.iter().copied());
        let mut results = Vec::new();
        loop {
            self.ensure_same_origin(&url)?;
            let response = self
                .request(Method::GET, url.clone())
                .send()
                .await
                .map_err(map_transport_error)?;
            let page: Paginated<T> = decode_success(response).await?;
            results.extend(page.results);
            let Some(next) = page.next else {
                break;
            };
            url = Url::parse(&next)?;
        }
        Ok(results)
    }

    fn request(&self, method: Method, url: Url) -> reqwest::RequestBuilder {
        self.client
            .request(method, url)
            .header(ACCEPT, API_ACCEPT)
            .header(AUTHORIZATION, self.authorization.clone())
    }

    fn api_url(&self, path: &str) -> AppResult<Url> {
        Ok(self.base_url.join(&format!("api/{path}"))?)
    }

    fn ensure_same_origin(&self, url: &Url) -> AppResult<()> {
        if url.scheme() != self.base_url.scheme()
            || url.host_str() != self.base_url.host_str()
            || url.port_or_known_default() != self.base_url.port_or_known_default()
        {
            return Err(AppError::Upstream(
                "Paperless pagination attempted to leave the configured origin".to_owned(),
            ));
        }
        Ok(())
    }
}

pub fn merge_tags(existing: &[i64], additions: &[i64], overwrite: bool) -> Vec<i64> {
    let mut result = if overwrite {
        Vec::new()
    } else {
        existing.to_vec()
    };
    result.extend(additions.iter().copied());
    result.sort_unstable();
    result.dedup();
    result
}

pub fn merge_custom_fields(
    existing: &[PaperlessCustomFieldValue],
    updates: &[PaperlessCustomFieldValue],
    overwrite_by_field: &HashMap<i64, bool>,
) -> Vec<PaperlessCustomFieldValue> {
    let mut values = existing
        .iter()
        .cloned()
        .map(|value| (value.field, value))
        .collect::<HashMap<_, _>>();
    for update in updates {
        let should_overwrite = overwrite_by_field
            .get(&update.field)
            .copied()
            .unwrap_or(false);
        let is_empty = values
            .get(&update.field)
            .is_none_or(|value| is_empty_json_value(&value.value));
        if should_overwrite || is_empty {
            values.insert(update.field, update.clone());
        }
    }
    let mut result = values.into_values().collect::<Vec<_>>();
    result.sort_by_key(|value| value.field);
    result
}

fn is_empty_json_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(value) => value.trim().is_empty(),
        Value::Array(value) => value.is_empty(),
        Value::Object(value) => value.is_empty(),
        _ => false,
    }
}

fn normalize_base_url(raw: &str) -> AppResult<Url> {
    let mut url = Url::parse(raw.trim()).map_err(|_| {
        AppError::Validation("Paperless URL must be an absolute HTTP(S) URL".to_owned())
    })?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(AppError::Validation(
            "Paperless URL must be an absolute HTTP(S) URL".to_owned(),
        ));
    }
    url.set_query(None);
    url.set_fragment(None);
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

fn validate_name(name: &str) -> AppResult<&str> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 128 {
        return Err(AppError::Validation(
            "Paperless metadata names must contain 1 to 128 characters".to_owned(),
        ));
    }
    Ok(name)
}

fn validate_custom_field_type(data_type: &str) -> AppResult<()> {
    if !matches!(
        data_type,
        "string"
            | "longtext"
            | "url"
            | "date"
            | "boolean"
            | "integer"
            | "float"
            | "monetary"
            | "select"
    ) {
        return Err(AppError::Validation(format!(
            "unsupported custom-field type: {data_type}"
        )));
    }
    Ok(())
}

async fn decode_success<T: DeserializeOwned>(response: Response) -> AppResult<T> {
    let response = ensure_success(response).await?;
    response.json().await.map_err(map_transport_error)
}

async fn ensure_success(response: Response) -> AppResult<Response> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let message = body.chars().take(MAX_ERROR_CHARS).collect::<String>();
    match status.as_u16() {
        401 => Err(AppError::Upstream(
            "Paperless authentication failed".to_owned(),
        )),
        403 => Err(AppError::Upstream(
            "Paperless API permissions are insufficient".to_owned(),
        )),
        404 => Err(AppError::NotFound("Paperless object not found".to_owned())),
        406 => Err(AppError::Upstream(
            "Paperless API v10 is not supported".to_owned(),
        )),
        429 => Err(AppError::Upstream(
            "Paperless rate limit reached".to_owned(),
        )),
        _ => Err(AppError::Upstream(format!(
            "Paperless returned {status}: {message}"
        ))),
    }
}

fn map_transport_error(error: reqwest::Error) -> AppError {
    if error.is_timeout() {
        AppError::Upstream("Paperless request timed out".to_owned())
    } else if error.is_connect() {
        AppError::Upstream("Could not connect to Paperless".to_owned())
    } else {
        AppError::Upstream("Paperless request failed".to_owned())
    }
}

fn parse_filename(content_disposition: &str) -> Option<String> {
    content_disposition
        .split(';')
        .map(str::trim)
        .find_map(|part| part.strip_prefix("filename="))
        .map(|value| value.trim_matches('"').to_owned())
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn tag_merge_preserves_existing_by_default() {
        assert_eq!(merge_tags(&[3, 1], &[2, 3], false), vec![1, 2, 3]);
        assert_eq!(merge_tags(&[3, 1], &[2], true), vec![2]);
    }

    #[test]
    fn custom_field_merge_never_drops_unrelated_values() {
        let existing = vec![
            PaperlessCustomFieldValue {
                field: 1,
                value: json!("kept"),
            },
            PaperlessCustomFieldValue {
                field: 2,
                value: Value::Null,
            },
        ];
        let updates = vec![
            PaperlessCustomFieldValue {
                field: 1,
                value: json!("ignored"),
            },
            PaperlessCustomFieldValue {
                field: 2,
                value: json!("filled"),
            },
        ];
        let result = merge_custom_fields(&existing, &updates, &HashMap::new());
        assert_eq!(result[0].value, json!("kept"));
        assert_eq!(result[1].value, json!("filled"));
    }

    #[test]
    fn rejects_cross_origin_pagination() {
        let token = Zeroizing::new("secret".to_owned());
        let client = PaperlessClient::new("https://paperless.example/", token, None).unwrap();
        assert!(
            client
                .ensure_same_origin(&Url::parse("https://evil.example/api/").unwrap())
                .is_err()
        );
    }

    proptest! {
        #[test]
        fn additive_tag_merges_preserve_every_existing_id(
            existing in proptest::collection::vec(1_i64..10_000, 0..100),
            additions in proptest::collection::vec(1_i64..10_000, 0..100),
        ) {
            let merged = merge_tags(&existing, &additions, false);
            prop_assert!(existing.iter().all(|id| merged.contains(id)));
            prop_assert!(additions.iter().all(|id| merged.contains(id)));
            prop_assert!(merged.windows(2).all(|pair| pair[0] < pair[1]));
        }

        #[test]
        fn custom_field_merges_preserve_unselected_values(
            existing in proptest::collection::btree_map(1_i64..1_000, "[a-z]{0,16}", 0..50),
            selected_field in 1_001_i64..2_000,
            selected_value in "[a-z]{1,16}",
        ) {
            let existing_values = existing
                .iter()
                .map(|(field, value)| PaperlessCustomFieldValue {
                    field: *field,
                    value: json!(value),
                })
                .collect::<Vec<_>>();
            let updates = vec![PaperlessCustomFieldValue {
                field: selected_field,
                value: json!(selected_value),
            }];
            let merged = merge_custom_fields(&existing_values, &updates, &HashMap::new());
            for (field, value) in existing {
                prop_assert_eq!(
                    merged.iter().find(|candidate| candidate.field == field).map(|candidate| &candidate.value),
                    Some(&json!(value)),
                );
            }
        }
    }
}
