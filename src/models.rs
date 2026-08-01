use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PaperlessConnection {
    pub id: String,
    pub name: String,
    pub base_url: String,
    #[serde(skip_serializing)]
    pub token_secret_key: String,
    pub custom_ca_pem: Option<String>,
    pub api_version: Option<i64>,
    pub server_version: Option<String>,
    pub last_tested_at: Option<String>,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Openai,
    Claude,
    Lmstudio,
    Custom,
}

impl ProviderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Claude => "claude",
            Self::Lmstudio => "lmstudio",
            Self::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiFamily {
    Responses,
    ChatCompletions,
    Messages,
}

impl ApiFamily {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Responses => "responses",
            Self::ChatCompletions => "chat_completions",
            Self::Messages => "messages",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ProviderProfileRecord {
    pub id: String,
    pub name: String,
    pub provider_kind: String,
    pub base_url: String,
    pub model: String,
    pub api_family: String,
    pub chat_output_token_field: String,
    #[serde(skip_serializing)]
    pub api_key_secret_key: Option<String>,
    #[serde(skip_serializing)]
    pub headers_secret_key: Option<String>,
    pub capabilities_json: String,
    pub reasoning_effort: Option<String>,
    pub temperature: Option<f64>,
    pub max_output_tokens: i64,
    pub request_timeout_seconds: i64,
    pub daily_budget_tokens: Option<i64>,
    pub monthly_budget_tokens: Option<i64>,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl ProviderProfileRecord {
    pub fn capabilities(&self) -> Value {
        serde_json::from_str(&self.capabilities_json)
            .unwrap_or_else(|_| Value::Object(Default::default()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ProcessingProfileRecord {
    pub id: String,
    pub name: String,
    pub paperless_connection_id: String,
    pub provider_profile_id: String,
    pub revision: i64,
    pub enabled: bool,
    pub automatic_mode: String,
    pub poll_interval_seconds: i64,
    pub max_documents_per_scan: i64,
    pub concurrency: i64,
    pub use_existing_data: bool,
    pub process_pre_tagged_only: bool,
    pub trigger_tag_ids_json: String,
    pub trigger_match_mode: String,
    pub restrict_tags: bool,
    pub restrict_correspondents: bool,
    pub restrict_document_types: bool,
    pub add_processed_tag: bool,
    pub processed_tag_name: String,
    pub tags_enabled: bool,
    pub correspondent_enabled: bool,
    pub document_type_enabled: bool,
    pub title_enabled: bool,
    pub custom_fields_enabled: bool,
    pub overwrite_tags: bool,
    pub overwrite_correspondent: bool,
    pub overwrite_document_type: bool,
    pub overwrite_title: bool,
    pub prompt_mode: String,
    pub custom_prompt: String,
    pub max_input_tokens: i64,
    pub taxonomy_context_tokens: i64,
    pub include_document_file: bool,
    pub file_rendition: String,
    pub max_file_bytes: i64,
    pub max_file_pages: i64,
    pub ocr_excerpt_tokens: i64,
    pub daily_budget_tokens: Option<i64>,
    pub monthly_budget_tokens: Option<i64>,
    pub budget_timezone: String,
    pub warning_threshold_percent: i64,
    pub created_at: String,
    pub updated_at: String,
}

impl ProcessingProfileRecord {
    pub fn trigger_tag_ids(&self) -> Vec<i64> {
        serde_json::from_str(&self.trigger_tag_ids_json).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CustomFieldRuleRecord {
    pub id: String,
    pub processing_profile_id: String,
    pub paperless_field_id: i64,
    pub name: String,
    pub data_type: String,
    pub extra_data_json: String,
    pub instructions: String,
    pub overwrite_existing: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct JobRecord {
    pub id: String,
    pub processing_profile_id: String,
    pub profile_revision: i64,
    pub paperless_document_id: i64,
    pub document_checksum: String,
    pub document_title: Option<String>,
    pub status: String,
    pub attempt_count: i64,
    pub manual_reprocess: bool,
    pub truncated: bool,
    pub suggestion_json: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub lease_owner: Option<String>,
    pub lease_expires_at: Option<String>,
    pub queued_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    pub input_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub total_tokens: i64,
    pub source: String,
    #[serde(default)]
    pub raw: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub instance_id: String,
    pub service: &'static str,
    pub status: &'static str,
    pub version: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResponse {
    pub configured: bool,
    pub lan_mode: bool,
    pub authenticated: bool,
    pub worker_running: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsSummary {
    pub today_tokens: i64,
    pub month_tokens: i64,
    pub lifetime_tokens: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub completed_jobs: i64,
    pub failed_jobs: i64,
    pub pending_reviews: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsPoint {
    pub bucket: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub total_tokens: i64,
    pub requests: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id_hash: String,
    pub csrf_token: String,
    pub expires_at: DateTime<Utc>,
}
