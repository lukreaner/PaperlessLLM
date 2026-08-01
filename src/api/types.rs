use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthStatus {
    pub required: bool,
    pub authenticated: bool,
    pub password_configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csrf_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionHealth {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveJobDto {
    pub id: String,
    pub document_id: i64,
    pub document_title: String,
    pub state: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanProgressDto {
    pub trigger: String,
    pub discovered: usize,
    pub queued: usize,
    pub started_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerStatusDto {
    pub state: String,
    #[serde(default)]
    pub paused: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_scan_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_scan_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_job_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_job: Option<ActiveJobDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan_progress: Option<ScanProgressDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStatus {
    pub version: &'static str,
    pub setup_required: bool,
    pub worker: WorkerStatusDto,
    pub paperless: ConnectionHealth,
    pub provider: ConnectionHealth,
    pub queue: BTreeMap<String, i64>,
    pub auth: AuthStatus,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PaperlessSettings {
    pub base_url: String,
    pub api_token_configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_ca_pem: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FunctionSettings {
    pub tags: bool,
    pub correspondent: bool,
    pub document_type: bool,
    pub title: bool,
    pub custom_fields: bool,
}

impl Default for FunctionSettings {
    fn default() -> Self {
        Self {
            tags: true,
            correspondent: true,
            document_type: true,
            title: true,
            custom_fields: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OverwriteSettings {
    pub tags: bool,
    pub correspondent: bool,
    pub document_type: bool,
    pub title: bool,
    pub custom_fields: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProcessingSettings {
    pub poll_interval_minutes: u64,
    /// `None` means every matching Paperless document is discovered. A value
    /// sets an optional per-scan ceiling without changing the bounded API page
    /// size used by the Paperless client.
    pub max_documents_per_scan: Option<u64>,
    pub review_mode: bool,
    pub use_existing_correspondents_and_tags: bool,
    pub process_only_tagged: bool,
    pub trigger_tag_ids: Vec<i64>,
    pub trigger_match: String,
    pub restrict_existing_tags: bool,
    pub restrict_existing_correspondents: bool,
    pub restrict_existing_document_types: bool,
    pub add_processed_tag: bool,
    pub processed_tag_name: String,
    pub functions: FunctionSettings,
    pub overwrite: OverwriteSettings,
    pub custom_field_ids: Vec<i64>,
    pub custom_prompt_mode: String,
    pub custom_prompt: String,
}

impl Default for ProcessingSettings {
    fn default() -> Self {
        Self {
            poll_interval_minutes: 5,
            max_documents_per_scan: None,
            review_mode: false,
            use_existing_correspondents_and_tags: true,
            process_only_tagged: false,
            trigger_tag_ids: Vec::new(),
            trigger_match: "any".to_owned(),
            restrict_existing_tags: true,
            restrict_existing_correspondents: true,
            restrict_existing_document_types: true,
            add_processed_tag: true,
            processed_tag_name: "ai-processed".to_owned(),
            functions: FunctionSettings::default(),
            overwrite: OverwriteSettings::default(),
            custom_field_ids: Vec::new(),
            custom_prompt_mode: "safe".to_owned(),
            custom_prompt: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FileInputSettings {
    pub enabled: bool,
    pub rendition: String,
    pub max_megabytes: u64,
    pub max_pages: u64,
    pub ocr_excerpt_tokens: u64,
}

impl Default for FileInputSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            rendition: "archive".to_owned(),
            max_megabytes: 10,
            max_pages: 20,
            ocr_excerpt_tokens: 2_000,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BudgetValue {
    pub daily: Option<i64>,
    pub monthly: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LimitSettings {
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    pub warning_percent: u8,
    pub reset_timezone: String,
    pub global_budget: BudgetValue,
    pub provider_budgets: BTreeMap<String, BudgetValue>,
}

impl Default for LimitSettings {
    fn default() -> Self {
        Self {
            max_input_tokens: 32_000,
            max_output_tokens: 8_000,
            warning_percent: 80,
            reset_timezone: "Europe/Berlin".to_owned(),
            global_budget: BudgetValue::default(),
            provider_budgets: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SecuritySettings {
    pub lan_enabled: bool,
    pub bind_address: String,
    pub session_hours: u64,
    pub reverse_proxy_tls: bool,
}

impl Default for SecuritySettings {
    fn default() -> Self {
        Self {
            lan_enabled: false,
            bind_address: "127.0.0.1".to_owned(),
            session_hours: 12,
            reverse_proxy_tls: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub paperless: PaperlessSettings,
    pub processing: ProcessingSettings,
    pub file_input: FileInputSettings,
    pub limits: LimitSettings,
    pub security: SecuritySettings,
    pub active_provider_id: Option<String>,
}

impl AppSettings {
    pub fn redact_secrets(&mut self) {
        self.paperless.api_token = None;
        self.paperless.custom_ca_pem = self
            .paperless
            .custom_ca_pem
            .take()
            .filter(|value| !value.trim().is_empty());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCapabilitiesDto {
    pub structured_output: bool,
    pub reasoning: bool,
    pub temperature: bool,
    pub file_pdf: bool,
    pub file_images: bool,
    pub token_counting: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderProfileDto {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub kind: String,
    pub base_url: String,
    #[serde(default)]
    pub model: String,
    pub transport: String,
    #[serde(default = "default_chat_output_token_field")]
    pub chat_output_token_field: String,
    #[serde(default)]
    pub api_key_configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub headers_configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(default)]
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<ProviderCapabilitiesDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_tested_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_test_ok: Option<bool>,
}

fn default_chat_output_token_field() -> String {
    "max_tokens".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogItem {
    pub id: i64,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalogs {
    pub tags: Vec<CatalogItem>,
    pub correspondents: Vec<CatalogItem>,
    pub document_types: Vec<CatalogItem>,
    pub custom_fields: Vec<CatalogItem>,
    pub fetched_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BootstrapResponse {
    pub status: AppStatus,
    pub settings: AppSettings,
    pub providers: Vec<ProviderProfileDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalogs: Option<Catalogs>,
}

#[derive(Debug, Deserialize)]
pub struct PaperlessTestRequest {
    pub base_url: String,
    pub api_token: Option<String>,
    pub custom_ca_pem: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PaperlessTestResult {
    pub ok: bool,
    pub version: Option<String>,
    pub api_version: Option<u32>,
    pub user: Option<String>,
    pub document_count: Option<i64>,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct ProviderTestResult {
    pub ok: bool,
    pub message: String,
    pub latency_ms: Option<u64>,
    pub capabilities: Option<ProviderCapabilitiesDto>,
}

#[derive(Debug, Serialize)]
pub struct ModelsResponse {
    pub models: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct PasswordRequest {
    pub password: String,
    pub current_password: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OkResponse {
    pub ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReviewSuggestion {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correspondent: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_type: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_fields: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Deserialize)]
pub struct ApproveRequest {
    pub suggestion: ReviewSuggestion,
}
