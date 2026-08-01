use chrono_tz::Tz;
use serde_json::Value;
use url::Url;
use uuid::Uuid;

use crate::{
    AppError, AppResult,
    api::types::{AppSettings, Catalogs},
    app::AppState,
    models::{PaperlessConnection, ProviderProfileRecord},
};

pub const CONNECTION_ID: &str = "default";
pub const PROCESSING_PROFILE_ID: &str = "default";
pub const PAPERLESS_TOKEN_KEY: &str = "paperless:default:token";
const SETTINGS_KEY: &str = "dashboard_settings";
pub const ACTIVE_PROVIDER_SETTING_KEY: &str = "active_provider_id";
const CATALOG_CACHE_KEY: &str = "paperless_catalog_cache";

pub async fn load_settings(state: &AppState) -> AppResult<AppSettings> {
    let mut settings = state
        .database
        .get_setting::<AppSettings>(SETTINGS_KEY)
        .await?
        .unwrap_or_default();
    if let Some(connection) = load_connection(state).await? {
        settings.paperless.base_url = connection.base_url;
        settings.paperless.custom_ca_pem = connection.custom_ca_pem;
        settings.paperless.api_token_configured =
            state.secrets.exists(&connection.token_secret_key).await?;
    }
    if settings.active_provider_id.is_none() {
        settings.active_provider_id = sqlx::query_scalar(
            "SELECT provider_profile_id FROM processing_profiles WHERE enabled=1 LIMIT 1",
        )
        .fetch_optional(state.database.pool())
        .await?;
    }
    let providers: Vec<(String, Option<i64>, Option<i64>)> = sqlx::query_as(
        "SELECT id, daily_budget_tokens, monthly_budget_tokens FROM provider_profiles WHERE enabled=1",
    )
    .fetch_all(state.database.pool())
    .await?;
    for (id, daily, monthly) in providers {
        settings
            .limits
            .provider_budgets
            .insert(id, super::types::BudgetValue { daily, monthly });
    }
    settings.paperless.api_token = None;
    settings.security.lan_enabled = settings.security.lan_enabled || state.requires_auth();
    Ok(settings)
}

pub async fn save_settings(state: &AppState, mut settings: AppSettings) -> AppResult<AppSettings> {
    // A non-loopback listener can never opt out of authentication through an
    // API payload. Loopback deployments may still opt into authentication for
    // a same-host TLS reverse proxy.
    settings.security.lan_enabled |= state.runtime.is_lan_mode();
    validate_settings(&settings)?;
    if settings.security.lan_enabled && !state.auth.has_admin_password().await? {
        return Err(AppError::Conflict(
            "set an administrator password before enabling LAN access".to_owned(),
        ));
    }
    let supplied_token = settings
        .paperless
        .api_token
        .take()
        .filter(|token| !token.trim().is_empty());
    if let Some(token) = supplied_token {
        state.secrets.set(PAPERLESS_TOKEN_KEY, token.trim()).await?;
    }
    let has_token = state.secrets.exists(PAPERLESS_TOKEN_KEY).await?;
    settings.paperless.api_token_configured = has_token;

    if !settings.paperless.base_url.trim().is_empty() {
        sqlx::query(
            "INSERT INTO paperless_connections(id, name, base_url, token_secret_key, custom_ca_pem, updated_at) \
             VALUES(?, 'Paperless-ngx', ?, ?, ?, strftime('%Y-%m-%dT%H:%M:%fZ','now')) \
             ON CONFLICT(id) DO UPDATE SET base_url=excluded.base_url, token_secret_key=excluded.token_secret_key, custom_ca_pem=excluded.custom_ca_pem, updated_at=excluded.updated_at",
        )
        .bind(CONNECTION_ID)
        .bind(settings.paperless.base_url.trim().trim_end_matches('/'))
        .bind(PAPERLESS_TOKEN_KEY)
        .bind(settings.paperless.custom_ca_pem.as_deref().filter(|pem| !pem.trim().is_empty()))
        .execute(state.database.pool())
        .await?;
    }

    for (provider_id, budget) in &settings.limits.provider_budgets {
        sqlx::query(
            "UPDATE provider_profiles SET daily_budget_tokens=?, monthly_budget_tokens=?, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?",
        )
        .bind(normalize_limit(budget.daily))
        .bind(normalize_limit(budget.monthly))
        .bind(provider_id)
        .execute(state.database.pool())
        .await?;
    }

    if let Some(provider_id) = settings.active_provider_id.as_deref() {
        let provider: Option<ProviderProfileRecord> =
            sqlx::query_as("SELECT * FROM provider_profiles WHERE id=? AND enabled=1")
                .bind(provider_id)
                .fetch_optional(state.database.pool())
                .await?;
        if provider.is_none() {
            return Err(AppError::Validation(
                "the selected provider profile does not exist".to_owned(),
            ));
        }
        if settings.paperless.base_url.trim().is_empty() || !has_token {
            return Err(AppError::Validation(
                "a Paperless URL and API token are required before setup can finish".to_owned(),
            ));
        }
        let provider = provider.expect("checked above");
        sqlx::query("UPDATE provider_profiles SET max_output_tokens=?, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?")
            .bind(settings.limits.max_output_tokens as i64)
            .bind(provider_id)
            .execute(state.database.pool())
            .await?;
        upsert_processing_profile(state, &settings, &provider).await?;
        sync_custom_field_rules(state, &settings).await?;
    }

    let mut persisted = settings.clone();
    persisted.redact_secrets();
    state
        .database
        .set_setting(ACTIVE_PROVIDER_SETTING_KEY, &persisted.active_provider_id)
        .await?;
    state.database.set_setting(SETTINGS_KEY, &persisted).await?;
    state.set_configured_lan(persisted.security.lan_enabled);
    // Budget increases and other processing changes should wake durable
    // waiting work immediately. The notification is intentionally best-effort.
    state.worker.notify_state_changed();
    load_settings(state).await
}

async fn upsert_processing_profile(
    state: &AppState,
    settings: &AppSettings,
    provider: &ProviderProfileRecord,
) -> AppResult<()> {
    let processing = &settings.processing;
    let file = &settings.file_input;
    let limits = &settings.limits;
    sqlx::query(
        "INSERT INTO processing_profiles(\
            id,name,paperless_connection_id,provider_profile_id,enabled,automatic_mode,poll_interval_seconds,\
            max_documents_per_scan,use_existing_data,process_pre_tagged_only,trigger_tag_ids_json,trigger_match_mode,\
            restrict_tags,restrict_correspondents,restrict_document_types,add_processed_tag,processed_tag_name,\
            tags_enabled,correspondent_enabled,document_type_enabled,title_enabled,custom_fields_enabled,\
            overwrite_tags,overwrite_correspondent,overwrite_document_type,overwrite_title,prompt_mode,custom_prompt,\
            max_input_tokens,include_document_file,file_rendition,max_file_bytes,max_file_pages,ocr_excerpt_tokens,\
            daily_budget_tokens,monthly_budget_tokens,budget_timezone,warning_threshold_percent,updated_at\
         ) VALUES(?, 'Default', ?, ?, 1, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, strftime('%Y-%m-%dT%H:%M:%fZ','now')) \
         ON CONFLICT(id) DO UPDATE SET provider_profile_id=excluded.provider_profile_id,enabled=1,automatic_mode=excluded.automatic_mode,\
            poll_interval_seconds=excluded.poll_interval_seconds,max_documents_per_scan=excluded.max_documents_per_scan,\
            use_existing_data=excluded.use_existing_data,process_pre_tagged_only=excluded.process_pre_tagged_only,\
            trigger_tag_ids_json=excluded.trigger_tag_ids_json,trigger_match_mode=excluded.trigger_match_mode,restrict_tags=excluded.restrict_tags,\
            restrict_correspondents=excluded.restrict_correspondents,restrict_document_types=excluded.restrict_document_types,add_processed_tag=excluded.add_processed_tag,\
            processed_tag_name=excluded.processed_tag_name,tags_enabled=excluded.tags_enabled,correspondent_enabled=excluded.correspondent_enabled,\
            document_type_enabled=excluded.document_type_enabled,title_enabled=excluded.title_enabled,custom_fields_enabled=excluded.custom_fields_enabled,\
            overwrite_tags=excluded.overwrite_tags,overwrite_correspondent=excluded.overwrite_correspondent,overwrite_document_type=excluded.overwrite_document_type,\
            overwrite_title=excluded.overwrite_title,prompt_mode=excluded.prompt_mode,custom_prompt=excluded.custom_prompt,max_input_tokens=excluded.max_input_tokens,\
            include_document_file=excluded.include_document_file,file_rendition=excluded.file_rendition,max_file_bytes=excluded.max_file_bytes,\
            max_file_pages=excluded.max_file_pages,ocr_excerpt_tokens=excluded.ocr_excerpt_tokens,daily_budget_tokens=excluded.daily_budget_tokens,\
            monthly_budget_tokens=excluded.monthly_budget_tokens,budget_timezone=excluded.budget_timezone,warning_threshold_percent=excluded.warning_threshold_percent,\
            updated_at=excluded.updated_at",
    )
    .bind(PROCESSING_PROFILE_ID)
    .bind(CONNECTION_ID)
    .bind(&provider.id)
    .bind(if processing.review_mode { "review" } else { "automatic" })
    .bind((processing.poll_interval_minutes * 60) as i64)
    .bind(processing.max_documents_per_scan.unwrap_or(0) as i64)
    .bind(processing.use_existing_correspondents_and_tags)
    .bind(processing.process_only_tagged)
    .bind(serde_json::to_string(&processing.trigger_tag_ids)?)
    .bind(&processing.trigger_match)
    .bind(processing.restrict_existing_tags)
    .bind(processing.restrict_existing_correspondents)
    .bind(processing.restrict_existing_document_types)
    .bind(processing.add_processed_tag)
    .bind(processing.processed_tag_name.trim())
    .bind(processing.functions.tags)
    .bind(processing.functions.correspondent)
    .bind(processing.functions.document_type)
    .bind(processing.functions.title)
    .bind(processing.functions.custom_fields)
    .bind(processing.overwrite.tags)
    .bind(processing.overwrite.correspondent)
    .bind(processing.overwrite.document_type)
    .bind(processing.overwrite.title)
    .bind(if processing.custom_prompt_mode == "expert" { "expert" } else { "additional" })
    .bind(&processing.custom_prompt)
    .bind(limits.max_input_tokens as i64)
    .bind(file.enabled)
    .bind(&file.rendition)
    .bind((file.max_megabytes * 1024 * 1024) as i64)
    .bind(file.max_pages as i64)
    .bind(file.ocr_excerpt_tokens.min(2_000) as i64)
    .bind(normalize_limit(limits.global_budget.daily))
    .bind(normalize_limit(limits.global_budget.monthly))
    .bind(&limits.reset_timezone)
    .bind(limits.warning_percent as i64)
    .execute(state.database.pool())
    .await?;
    Ok(())
}

async fn sync_custom_field_rules(state: &AppState, settings: &AppSettings) -> AppResult<()> {
    let selected = &settings.processing.custom_field_ids;
    if selected.is_empty() {
        sqlx::query("DELETE FROM custom_field_rules WHERE processing_profile_id=?")
            .bind(PROCESSING_PROFILE_ID)
            .execute(state.database.pool())
            .await?;
        return Ok(());
    }
    let cache = state
        .database
        .get_setting::<Catalogs>(CATALOG_CACHE_KEY)
        .await?;
    let Some(cache) = cache else {
        return Err(AppError::Validation(
            "load the Paperless custom-field catalog before selecting fields".to_owned(),
        ));
    };
    for id in selected {
        let field = cache
            .custom_fields
            .iter()
            .find(|field| field.id == *id)
            .ok_or_else(|| {
                AppError::Validation(format!("custom field {id} is no longer available"))
            })?;
        let data_type = field.data_type.as_deref().unwrap_or_default();
        if data_type == "documentlink" || data_type == "document_link" {
            return Err(AppError::Validation(
                "document-link custom fields are not supported in v1".to_owned(),
            ));
        }
        sqlx::query(
            "INSERT INTO custom_field_rules(id,processing_profile_id,paperless_field_id,name,data_type,extra_data_json,overwrite_existing,enabled) \
             VALUES(?,?,?,?,?,?,?,1) ON CONFLICT(processing_profile_id,paperless_field_id) DO UPDATE SET name=excluded.name,data_type=excluded.data_type,extra_data_json=excluded.extra_data_json,overwrite_existing=excluded.overwrite_existing,enabled=1",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(PROCESSING_PROFILE_ID)
        .bind(field.id)
        .bind(&field.name)
        .bind(data_type)
        .bind(serde_json::to_string(field.extra_data.as_ref().unwrap_or(&Value::Null))?)
        .bind(settings.processing.overwrite.custom_fields)
        .execute(state.database.pool())
        .await?;
    }
    let selected_json = serde_json::to_string(selected)?;
    sqlx::query(
        "DELETE FROM custom_field_rules WHERE processing_profile_id=? AND paperless_field_id NOT IN (SELECT value FROM json_each(?))",
    )
    .bind(PROCESSING_PROFILE_ID)
    .bind(selected_json)
    .execute(state.database.pool())
    .await?;
    Ok(())
}

pub async fn load_connection(state: &AppState) -> AppResult<Option<PaperlessConnection>> {
    Ok(
        sqlx::query_as("SELECT * FROM paperless_connections WHERE id=?")
            .bind(CONNECTION_ID)
            .fetch_optional(state.database.pool())
            .await?,
    )
}

pub async fn cache_catalogs(state: &AppState, catalogs: &Catalogs) -> AppResult<()> {
    state
        .database
        .set_setting(CATALOG_CACHE_KEY, catalogs)
        .await
}

fn validate_settings(settings: &AppSettings) -> AppResult<()> {
    if !settings.paperless.base_url.trim().is_empty() {
        let url = Url::parse(settings.paperless.base_url.trim()).map_err(|_| {
            AppError::Validation("Paperless URL must be an absolute HTTP(S) URL".to_owned())
        })?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(AppError::Validation(
                "Paperless URL must be an absolute HTTP(S) URL".to_owned(),
            ));
        }
    }
    if !(1..=1_440).contains(&settings.processing.poll_interval_minutes) {
        return Err(AppError::Validation(
            "polling interval must be between 1 and 1440 minutes".to_owned(),
        ));
    }
    if settings
        .processing
        .max_documents_per_scan
        .is_some_and(|limit| !(1..=1_000_000).contains(&limit))
    {
        return Err(AppError::Validation(
            "maximum documents per scan must be between 1 and 1000000, or empty for all".to_owned(),
        ));
    }
    if !matches!(settings.processing.trigger_match.as_str(), "any" | "all") {
        return Err(AppError::Validation(
            "trigger matching must be Any or All".to_owned(),
        ));
    }
    if settings.processing.process_only_tagged && settings.processing.trigger_tag_ids.is_empty() {
        return Err(AppError::Validation(
            "select at least one trigger tag when pre-tag filtering is enabled".to_owned(),
        ));
    }
    if settings
        .processing
        .trigger_tag_ids
        .iter()
        .any(|id| *id <= 0)
    {
        return Err(AppError::Validation(
            "trigger tag IDs must be positive".to_owned(),
        ));
    }
    if settings.processing.add_processed_tag
        && settings.processing.processed_tag_name.trim().is_empty()
    {
        return Err(AppError::Validation(
            "AI-processed tag name is required".to_owned(),
        ));
    }
    if ![
        settings.processing.functions.tags,
        settings.processing.functions.correspondent,
        settings.processing.functions.document_type,
        settings.processing.functions.title,
        settings.processing.functions.custom_fields,
    ]
    .into_iter()
    .any(|enabled| enabled)
    {
        return Err(AppError::Validation(
            "enable at least one AI function".to_owned(),
        ));
    }
    if !(1_000..=2_000_000).contains(&settings.limits.max_input_tokens) {
        return Err(AppError::Validation(
            "maximum input tokens is outside the supported range".to_owned(),
        ));
    }
    if !(256..=200_000).contains(&settings.limits.max_output_tokens) {
        return Err(AppError::Validation(
            "maximum generated tokens is outside the supported range".to_owned(),
        ));
    }
    if !(1..=100).contains(&settings.limits.warning_percent) {
        return Err(AppError::Validation(
            "budget warning threshold must be 1 to 100 percent".to_owned(),
        ));
    }
    settings.limits.reset_timezone.parse::<Tz>().map_err(|_| {
        AppError::Validation("reset timezone must be a valid IANA timezone".to_owned())
    })?;
    validate_limit(settings.limits.global_budget.daily)?;
    validate_limit(settings.limits.global_budget.monthly)?;
    for budget in settings.limits.provider_budgets.values() {
        validate_limit(budget.daily)?;
        validate_limit(budget.monthly)?;
    }
    if settings.file_input.max_megabytes == 0 || settings.file_input.max_megabytes > 100 {
        return Err(AppError::Validation(
            "file limit must be between 1 and 100 MiB".to_owned(),
        ));
    }
    if settings.file_input.max_pages == 0 || settings.file_input.max_pages > 1_000 {
        return Err(AppError::Validation(
            "PDF page limit must be between 1 and 1000".to_owned(),
        ));
    }
    if settings.file_input.ocr_excerpt_tokens > 2_000 {
        return Err(AppError::Validation(
            "file requests may include at most 2000 OCR tokens".to_owned(),
        ));
    }
    if settings.security.lan_enabled && !settings.security.reverse_proxy_tls {
        return Err(AppError::Validation(
            "LAN mode requires confirmation that an HTTPS reverse proxy is configured".to_owned(),
        ));
    }
    if !(1..=720).contains(&settings.security.session_hours) {
        return Err(AppError::Validation(
            "session lifetime must be between 1 and 720 hours".to_owned(),
        ));
    }
    Ok(())
}

fn validate_limit(value: Option<i64>) -> AppResult<()> {
    if value.is_some_and(|value| value < 0) {
        return Err(AppError::Validation(
            "token budgets cannot be negative".to_owned(),
        ));
    }
    Ok(())
}

fn normalize_limit(value: Option<i64>) -> Option<i64> {
    value.filter(|value| *value > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RuntimeOptions, app};

    #[test]
    fn defaults_match_v1_guardrails() {
        let settings = AppSettings::default();
        assert_eq!(settings.processing.poll_interval_minutes, 5);
        assert_eq!(settings.processing.max_documents_per_scan, None);
        assert_eq!(settings.limits.max_input_tokens, 32_000);
        assert_eq!(settings.limits.max_output_tokens, 8_000);
        assert!(settings.processing.restrict_existing_tags);
        assert_eq!(settings.processing.processed_tag_name, "ai-processed");
    }

    #[tokio::test]
    async fn settings_persist_all_documents_or_an_explicit_scan_cap() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let state = app::initialize(RuntimeOptions {
            data_dir: directory.path().to_owned(),
            bind: "127.0.0.1".parse().expect("loopback address"),
            port: 8765,
            open_browser: false,
        })
        .await
        .expect("application state");
        sqlx::query(
            "INSERT INTO paperless_connections(id,name,base_url,token_secret_key) \
             VALUES(?,?,?,?)",
        )
        .bind(CONNECTION_ID)
        .bind("Paperless")
        .bind("http://localhost:8000")
        .bind(PAPERLESS_TOKEN_KEY)
        .execute(state.database.pool())
        .await
        .expect("Paperless connection");
        sqlx::query(
            "INSERT INTO provider_profiles(id,name,provider_kind,base_url,model,api_family) \
             VALUES('provider','Local','lmstudio','http://localhost:1234/v1','model','responses')",
        )
        .execute(state.database.pool())
        .await
        .expect("provider profile");
        let provider: ProviderProfileRecord =
            sqlx::query_as("SELECT * FROM provider_profiles WHERE id='provider'")
                .fetch_one(state.database.pool())
                .await
                .expect("saved provider profile");

        let mut settings = AppSettings::default();
        upsert_processing_profile(&state, &settings, &provider)
            .await
            .expect("save unlimited discovery profile");
        let unlimited: i64 =
            sqlx::query_scalar("SELECT max_documents_per_scan FROM processing_profiles WHERE id=?")
                .bind(PROCESSING_PROFILE_ID)
                .fetch_one(state.database.pool())
                .await
                .expect("unlimited scan setting");
        assert_eq!(unlimited, 0);

        settings.processing.max_documents_per_scan = Some(2_500);
        upsert_processing_profile(&state, &settings, &provider)
            .await
            .expect("save capped discovery profile");
        let capped: i64 =
            sqlx::query_scalar("SELECT max_documents_per_scan FROM processing_profiles WHERE id=?")
                .bind(PROCESSING_PROFILE_ID)
                .fetch_one(state.database.pool())
                .await
                .expect("capped scan setting");
        assert_eq!(capped, 2_500);
    }

    #[tokio::test]
    async fn migration_replaces_only_the_legacy_fifty_document_default() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        sqlx::query(
            "CREATE TABLE processing_profiles(id TEXT PRIMARY KEY, max_documents_per_scan INTEGER NOT NULL)",
        )
        .execute(&pool)
        .await
        .expect("processing profile table");
        sqlx::query(
            "INSERT INTO processing_profiles(id,max_documents_per_scan) VALUES('legacy',50),('custom',250)",
        )
        .execute(&pool)
        .await
        .expect("legacy rows");

        sqlx::raw_sql(include_str!(
            "../../migrations/0003_unlimited_document_scans.sql"
        ))
        .execute(&pool)
        .await
        .expect("scan-limit migration");

        let values: Vec<(String, i64)> =
            sqlx::query_as("SELECT id,max_documents_per_scan FROM processing_profiles ORDER BY id")
                .fetch_all(&pool)
                .await
                .expect("migrated limits");
        assert_eq!(
            values,
            vec![("custom".to_owned(), 250), ("legacy".to_owned(), 0)]
        );
    }
}
