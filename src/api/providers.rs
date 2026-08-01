use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
    time::Instant,
};

use axum::http::{HeaderName, HeaderValue};
use chrono::Utc;
use url::Url;
use uuid::Uuid;

use crate::{
    AppError, AppResult,
    api::{
        settings::{ACTIVE_PROVIDER_SETTING_KEY, load_settings},
        types::{ModelsResponse, ProviderCapabilitiesDto, ProviderProfileDto, ProviderTestResult},
    },
    app::AppState,
    budget::{BudgetLimits, ReservationContext},
    models::ProviderProfileRecord,
    provider_factory::{
        build_adapter, build_adapter_with_capabilities, validate_provider_base_url,
        validate_provider_secret_transport,
    },
    providers::{
        AnalysisRequest, AnalysisResult, ApiFamily, ProviderAdapter, ProviderCapabilities,
        ProviderError, ReasoningEffort, ReasoningOptions, TokenUsage, UsageSource,
    },
};

pub async fn list(state: &AppState) -> AppResult<Vec<ProviderProfileDto>> {
    let records: Vec<ProviderProfileRecord> =
        sqlx::query_as("SELECT * FROM provider_profiles WHERE enabled=1 ORDER BY created_at, name")
            .fetch_all(state.database.pool())
            .await?;
    let active = load_settings(state).await?.active_provider_id;
    let mut result = Vec::with_capacity(records.len());
    for record in records {
        result.push(to_dto(state, record, active.as_deref()).await?);
    }
    Ok(result)
}

pub async fn create(state: &AppState, input: ProviderProfileDto) -> AppResult<ProviderProfileDto> {
    let id = Uuid::new_v4().to_string();
    save(state, &id, input, true).await
}

pub async fn update(
    state: &AppState,
    id: &str,
    input: ProviderProfileDto,
) -> AppResult<ProviderProfileDto> {
    if id != input.id && !input.id.is_empty() {
        return Err(AppError::Validation(
            "provider ID cannot be changed".to_owned(),
        ));
    }
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM provider_profiles WHERE id=?")
        .bind(id)
        .fetch_one(state.database.pool())
        .await?;
    if exists == 0 {
        return Err(AppError::NotFound("provider profile not found".to_owned()));
    }
    save(state, id, input, false).await
}

async fn save(
    state: &AppState,
    id: &str,
    input: ProviderProfileDto,
    creating: bool,
) -> AppResult<ProviderProfileDto> {
    validate(&input)?;
    let api_family = database_transport(&input.kind, &input.transport)?;
    let provider_kind = database_provider_kind(&input.kind);
    let chat_output_token_field =
        compatible_output_token_field(&input.kind, input.chat_output_token_field.as_str());
    let normalized_base_url = normalize_base_url(&input.base_url);
    let previous_profile: Option<ProviderProfileRecord> = if creating {
        None
    } else {
        sqlx::query_as("SELECT * FROM provider_profiles WHERE id=?")
            .bind(id)
            .fetch_optional(state.database.pool())
            .await?
    };
    if !creating && previous_profile.is_none() {
        return Err(AppError::NotFound("provider profile not found".to_owned()));
    }
    let same_destination = previous_profile.as_ref().is_some_and(|previous| {
        previous.preset_kind() == input.kind
            && normalize_base_url(&previous.base_url) == normalized_base_url
    });
    let supplied_api_key = input
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty());
    if let Some(headers) = input.headers.as_ref() {
        validate_headers(headers)?;
    }
    let previous_api_key = previous_profile
        .as_ref()
        .and_then(|profile| profile.api_key_secret_key.as_deref());
    let previous_headers = previous_profile
        .as_ref()
        .and_then(|profile| profile.headers_secret_key.as_deref());
    let retained_api_key = if same_destination && supplied_api_key.is_none() {
        existing_secret_key(state, previous_api_key).await?
    } else {
        None
    };
    let retain_headers = input.kind == "custom" && same_destination && input.headers.is_none();
    let retained_headers = if retain_headers {
        existing_secret_key(state, previous_headers).await?
    } else {
        None
    };

    if provider_requires_api_key(&input.kind)
        && supplied_api_key.is_none()
        && retained_api_key.is_none()
    {
        let message = if previous_profile.is_some() && !same_destination {
            format!(
                "changing the provider preset or base URL requires entering a new {} API key",
                provider_label(&input.kind)
            )
        } else {
            format!("{} requires an API key", provider_label(&input.kind))
        };
        return Err(AppError::Validation(message));
    }
    let carries_custom_headers = input.kind == "custom"
        && match input.headers.as_ref() {
            Some(headers) => !headers.is_empty(),
            None => retained_headers.is_some(),
        };
    validate_provider_secret_transport(
        &normalized_base_url,
        supplied_api_key.is_some() || retained_api_key.is_some() || carries_custom_headers,
    )?;

    let serialized_headers = if input.kind == "custom" {
        input
            .headers
            .as_ref()
            .filter(|headers| !headers.is_empty())
            .map(serde_json::to_string)
            .transpose()?
    } else {
        None
    };
    let mut capabilities = input
        .capabilities
        .as_ref()
        .map(capabilities_from_dto)
        .unwrap_or_else(|| default_capabilities(&input.kind, &input.transport));
    if capabilities.reasoning_efforts.is_none()
        && let Some(previous_profile) = previous_profile.as_ref()
        && previous_profile.preset_kind() == input.kind
        && normalize_base_url(&previous_profile.base_url) == normalized_base_url
        && previous_profile.model == input.model.trim()
        && previous_profile.api_family == api_family
        && let Ok(previous) =
            serde_json::from_str::<ProviderCapabilities>(&previous_profile.capabilities_json)
    {
        capabilities.reasoning_efforts = previous.reasoning_efforts;
    }
    let capabilities_json = serde_json::to_string(&capabilities)?;
    let max_output = input.max_output_tokens.unwrap_or(8_000).clamp(1, 200_000) as i64;

    // Replacement credentials are written under fresh keys. The profile row
    // continues to reference the old values until its INSERT/UPDATE succeeds,
    // so a validation or database failure cannot damage the working profile.
    let staged_api_key = match supplied_api_key {
        Some(value) => Some(stage_secret(state, id, "api_key", value).await?),
        None => None,
    };
    let staged_headers = match serialized_headers.as_deref() {
        Some(value) => match stage_secret(state, id, "headers", value).await {
            Ok(key) => Some(key),
            Err(error) => {
                cleanup_staged_secrets(state, staged_api_key.iter()).await;
                return Err(error);
            }
        },
        None => None,
    };
    let api_key_secret = staged_api_key.as_deref().or(retained_api_key.as_deref());
    let headers_secret = staged_headers.as_deref().or(retained_headers.as_deref());

    let persisted = if creating {
        sqlx::query(
            "INSERT INTO provider_profiles(id,name,provider_kind,provider_preset,base_url,model,api_family,chat_output_token_field,api_key_secret_key,headers_secret_key,capabilities_json,reasoning_effort,temperature,max_output_tokens) \
             VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(id)
        .bind(input.name.trim())
        .bind(provider_kind)
        .bind(&input.kind)
        .bind(&normalized_base_url)
        .bind(input.model.trim())
        .bind(api_family)
        .bind(chat_output_token_field)
        .bind(api_key_secret)
        .bind(headers_secret)
        .bind(capabilities_json)
        .bind(input.reasoning_effort.as_deref())
        .bind(input.temperature)
        .bind(max_output)
        .execute(state.database.pool())
        .await
    } else {
        let previous = previous_profile
            .as_ref()
            .expect("an update always loaded its previous profile");
        sqlx::query(
            "UPDATE provider_profiles SET name=?,provider_kind=?,provider_preset=?,base_url=?,model=?,api_family=?,chat_output_token_field=?,api_key_secret_key=?,headers_secret_key=?,capabilities_json=?,reasoning_effort=?,temperature=?,max_output_tokens=?,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') \
             WHERE id=? AND provider_kind=? AND provider_preset=? AND base_url=? AND api_key_secret_key IS ? AND headers_secret_key IS ?",
        )
        .bind(input.name.trim())
        .bind(provider_kind)
        .bind(&input.kind)
        .bind(&normalized_base_url)
        .bind(input.model.trim())
        .bind(api_family)
        .bind(chat_output_token_field)
        .bind(api_key_secret)
        .bind(headers_secret)
        .bind(capabilities_json)
        .bind(input.reasoning_effort.as_deref())
        .bind(input.temperature)
        .bind(max_output)
        .bind(id)
        .bind(&previous.provider_kind)
        .bind(&previous.provider_preset)
        .bind(&previous.base_url)
        .bind(previous.api_key_secret_key.as_deref())
        .bind(previous.headers_secret_key.as_deref())
        .execute(state.database.pool())
        .await
    };
    let persisted = match persisted {
        Ok(result) => result,
        Err(error) => {
            cleanup_staged_secrets(state, staged_api_key.iter().chain(staged_headers.iter())).await;
            return Err(AppError::Database(error));
        }
    };
    if persisted.rows_affected() != 1 {
        cleanup_staged_secrets(state, staged_api_key.iter().chain(staged_headers.iter())).await;
        return Err(AppError::Conflict(
            "provider profile changed while it was being saved; reload it and try again".to_owned(),
        ));
    }

    delete_replaced_secrets(
        state,
        previous_api_key.into_iter().chain(previous_headers),
        api_key_secret,
        headers_secret,
    )
    .await?;

    if input.active {
        let mut settings = load_settings(state).await?;
        settings.active_provider_id = Some(id.to_owned());
        // Only persist the selection directly here. The processing profile is
        // finalized by the setup/settings save endpoint.
        settings.redact_secrets();
        state
            .database
            .set_setting("dashboard_settings", &settings)
            .await?;
        state
            .database
            .set_setting(ACTIVE_PROVIDER_SETTING_KEY, &settings.active_provider_id)
            .await?;
    }
    // A saved health result only describes the exact endpoint/model/options
    // that were tested. Any subsequent save must require a fresh test.
    sqlx::query("DELETE FROM app_settings WHERE key=?")
        .bind(format!("provider_health:{id}"))
        .execute(state.database.pool())
        .await?;
    let record: ProviderProfileRecord =
        sqlx::query_as("SELECT * FROM provider_profiles WHERE id=?")
            .bind(id)
            .fetch_one(state.database.pool())
            .await?;
    let active = load_settings(state).await?.active_provider_id;
    to_dto(state, record, active.as_deref()).await
}

fn normalize_base_url(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('/');
    Url::parse(trimmed)
        .map(|url| url.to_string().trim_end_matches('/').to_owned())
        .unwrap_or_else(|_| trimmed.to_owned())
}

async fn existing_secret_key(state: &AppState, key: Option<&str>) -> AppResult<Option<String>> {
    let Some(key) = key else {
        return Ok(None);
    };
    Ok(state.secrets.exists(key).await?.then(|| key.to_owned()))
}

async fn stage_secret(state: &AppState, id: &str, field: &str, value: &str) -> AppResult<String> {
    let key = format!("provider:{id}:{field}:{}", Uuid::new_v4());
    if let Err(error) = state.secrets.set(&key, value).await {
        let _ = state.secrets.delete(&key).await;
        return Err(error);
    }
    Ok(key)
}

async fn cleanup_staged_secrets<'a>(state: &AppState, keys: impl Iterator<Item = &'a String>) {
    for key in keys {
        let _ = state.secrets.delete(key).await;
    }
}

async fn delete_replaced_secrets<'a>(
    state: &AppState,
    previous_keys: impl Iterator<Item = &'a str>,
    current_api_key: Option<&str>,
    current_headers: Option<&str>,
) -> AppResult<()> {
    let mut deleted = BTreeSet::new();
    let mut first_error = None;
    for key in previous_keys {
        if Some(key) != current_api_key
            && Some(key) != current_headers
            && deleted.insert(key.to_owned())
            && let Err(error) = state.secrets.delete(key).await
        {
            first_error.get_or_insert(error);
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

pub async fn delete(state: &AppState, id: &str) -> AppResult<()> {
    let active = load_settings(state).await?.active_provider_id;
    if active.as_deref() == Some(id) {
        return Err(AppError::Conflict(
            "select another active provider before deleting this profile".to_owned(),
        ));
    }
    let record: Option<ProviderProfileRecord> =
        sqlx::query_as("SELECT * FROM provider_profiles WHERE id=? AND enabled=1")
            .bind(id)
            .fetch_optional(state.database.pool())
            .await?;
    let Some(record) = record else {
        return Err(AppError::NotFound("provider profile not found".to_owned()));
    };
    // Keep the row as a historical dimension for jobs and token statistics,
    // but make it unavailable for future processing and remove its secrets.
    let changed = sqlx::query(
        "UPDATE provider_profiles SET enabled=0, api_key_secret_key=NULL, headers_secret_key=NULL, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=? AND enabled=1",
    )
        .bind(id)
        .execute(state.database.pool())
        .await?;
    if changed.rows_affected() == 0 {
        return Err(AppError::NotFound("provider profile not found".to_owned()));
    }
    if let Some(key) = record.api_key_secret_key {
        state.secrets.delete(&key).await?;
    }
    if let Some(key) = record.headers_secret_key {
        state.secrets.delete(&key).await?;
    }
    Ok(())
}

pub async fn models(state: &AppState, id: &str) -> AppResult<ModelsResponse> {
    let record = record(state, id).await?;
    let adapter = build_adapter(&record, &state.secrets).await?;
    let models = adapter
        .list_models()
        .await
        .map_err(|error| AppError::Upstream(error.to_string()))?
        .into_iter()
        .map(|model| model.id)
        .collect();
    Ok(ModelsResponse { models })
}

pub async fn test(state: &AppState, id: &str) -> AppResult<ProviderTestResult> {
    let record = record(state, id).await?;
    if record.model.trim().is_empty() {
        return Err(AppError::Validation(
            "enter a model ID before testing the provider".to_owned(),
        ));
    }
    let adapter = build_adapter(&record, &state.secrets).await?;
    let started = Instant::now();
    let model_listing = adapter.list_models().await;
    let selected_model = model_listing
        .as_ref()
        .ok()
        .and_then(|models| models.iter().find(|model| model.id == record.model));
    let selected_model_found = selected_model.is_some();
    let metadata_capabilities = selected_model.and_then(|model| model.capabilities.clone());
    let mut capabilities = metadata_capabilities
        .clone()
        .unwrap_or_else(|| adapter.capabilities().clone());
    let settings = load_settings(state).await?;
    let limits = BudgetLimits {
        global_daily: settings.limits.global_budget.daily,
        global_monthly: settings.limits.global_budget.monthly,
        provider_daily: record.daily_budget_tokens,
        provider_monthly: record.monthly_budget_tokens,
        timezone: settings.limits.reset_timezone,
    };

    // LM Studio and custom OpenAI-compatible servers do not expose a standard
    // model capability document. Probe strict output with a deliberately
    // conflicting prompt: only an enforced schema can produce `ok: true`.
    // OpenAI/Claude endpoints without model metadata use the same safe probe.
    let probe_strict = matches!(
        record.preset_kind(),
        "lmstudio" | "custom" | "mistral" | "kimi" | "moonshot" | "google"
    ) || metadata_capabilities.is_none();
    let mut strict_probe_rejected = false;
    if probe_strict {
        capabilities.strict_json_schema = true;
        let strict_adapter =
            build_adapter_with_capabilities(&record, &state.secrets, &capabilities).await?;
        let request = provider_test_request(&record, false)?;
        match run_test_attempt(
            state,
            &record,
            strict_adapter.as_ref(),
            &request,
            "provider_probe_strict",
            &limits,
        )
        .await?
        {
            TestAttemptOutcome::Succeeded(_) => {}
            TestAttemptOutcome::Failed(error) if structured_fallback_allowed(&error) => {
                strict_probe_rejected = true;
                capabilities.strict_json_schema = false;
            }
            TestAttemptOutcome::Failed(error) => {
                return finish_provider_test(
                    state,
                    id,
                    capabilities,
                    false,
                    error.to_string(),
                    started,
                )
                .await;
            }
        }
    }

    if let Err(message) = validate_model_reasoning(&record, &capabilities) {
        return finish_provider_test(state, id, capabilities, false, message, started).await;
    }

    // This request uses exactly the temperature, reasoning effort, configured
    // output ceiling, transport, and capability gates used by document jobs.
    let production_adapter =
        build_adapter_with_capabilities(&record, &state.secrets, &capabilities).await?;
    let request = provider_test_request(&record, true)?;
    let (ok, message) = match run_test_attempt(
        state,
        &record,
        production_adapter.as_ref(),
        &request,
        "provider_test",
        &limits,
    )
    .await?
    {
        TestAttemptOutcome::Succeeded(result) => {
            debug_assert_eq!(result.value.get("ok"), Some(&serde_json::Value::Bool(true)));
            let output_mode = if capabilities.strict_json_schema {
                "native structured output"
            } else {
                "validated JSON fallback"
            };
            let discovery = match (&model_listing, selected_model_found) {
                (Ok(_), true) => "model discovery succeeded",
                (Ok(_), false) => "a manually entered model ID was used",
                (Err(_), _) => "model discovery is unavailable",
            };
            let probe_note = if strict_probe_rejected {
                "; native structured output was rejected and disabled"
            } else {
                ""
            };
            (
                true,
                format!(
                    "Provider production settings succeeded with {output_mode}; {discovery}{probe_note}"
                ),
            )
        }
        TestAttemptOutcome::Failed(error) => (false, error.to_string()),
    };
    finish_provider_test(state, id, capabilities, ok, message, started).await
}

enum TestAttemptOutcome {
    Succeeded(AnalysisResult),
    Failed(ProviderError),
}

async fn run_test_attempt(
    state: &AppState,
    record: &ProviderProfileRecord,
    adapter: &dyn ProviderAdapter,
    request: &AnalysisRequest,
    operation: &str,
    limits: &BudgetLimits,
) -> AppResult<TestAttemptOutcome> {
    let count = match adapter.count_input_tokens(request).await {
        Ok(count) => count,
        Err(error) => return Ok(TestAttemptOutcome::Failed(error)),
    };
    let reservation = state
        .budget
        .reserve_attempt(
            None,
            &record.id,
            count
                .input_tokens
                .saturating_add(request.max_output_tokens as u64)
                .min(i64::MAX as u64) as i64,
            limits,
            &ReservationContext {
                operation: operation.to_owned(),
                api_family: test_api_family(adapter.api_family()).to_owned(),
                functions_json: "[]".to_owned(),
                input_tokens: count.input_tokens.min(i64::MAX as u64) as i64,
                provider_preset: Some(record.preset_kind().to_owned()),
                model: Some(record.model.clone()),
            },
        )
        .await?;
    let inference_started = Instant::now();
    match adapter.analyze(request).await {
        Ok(result) => {
            let usage = normalize_test_usage(&result.usage, count.input_tokens, &result.raw_text);
            record_test_usage(
                state,
                TestUsageAttempt {
                    provider: record,
                    reservation_id: &reservation.id,
                    reservation_state: "settled",
                    api_family: adapter.api_family(),
                    operation,
                    status: "succeeded",
                    usage: &usage,
                    latency: inference_started.elapsed(),
                    error_code: None,
                },
            )
            .await?;
            Ok(TestAttemptOutcome::Succeeded(result))
        }
        Err(error) => {
            let ambiguous = test_failure_is_ambiguous(&error);
            let usage = match &error {
                ProviderError::InvalidStructuredOutput {
                    raw_text, usage, ..
                } => normalize_test_usage(usage, count.input_tokens, raw_text),
                _ if ambiguous => TestUsage {
                    input: count.input_tokens.min(i64::MAX as u64) as i64,
                    cache_read: 0,
                    cache_write: 0,
                    output: reservation
                        .reserved_tokens
                        .saturating_sub(count.input_tokens.min(i64::MAX as u64) as i64),
                    reasoning: 0,
                    total: reservation.reserved_tokens,
                    source: "reserved_after_unknown",
                },
                _ => TestUsage {
                    input: count.input_tokens.min(i64::MAX as u64) as i64,
                    cache_read: 0,
                    cache_write: 0,
                    output: 0,
                    reasoning: 0,
                    total: count.input_tokens.min(i64::MAX as u64) as i64,
                    source: test_usage_source(count.source),
                },
            };
            record_test_usage(
                state,
                TestUsageAttempt {
                    provider: record,
                    reservation_id: &reservation.id,
                    reservation_state: if ambiguous {
                        "charged_unknown"
                    } else {
                        "settled"
                    },
                    api_family: adapter.api_family(),
                    operation,
                    status: "failed",
                    usage: &usage,
                    latency: inference_started.elapsed(),
                    error_code: Some(test_error_code(&error)),
                },
            )
            .await?;
            Ok(TestAttemptOutcome::Failed(error))
        }
    }
}

fn provider_test_request(
    record: &ProviderProfileRecord,
    production_settings: bool,
) -> AppResult<AnalysisRequest> {
    let reasoning = if production_settings {
        record
            .reasoning_effort
            .as_deref()
            .map(|value| {
                ReasoningEffort::parse(value)
                    .map(|effort| ReasoningOptions {
                        effort: Some(effort),
                        budget_tokens: None,
                    })
                    .ok_or_else(|| AppError::Validation("unknown reasoning effort".to_owned()))
            })
            .transpose()?
    } else {
        None
    };
    Ok(AnalysisRequest {
        system_prompt: "This is a PaperlessLLM provider capability test. Return only the requested structured value.".to_owned(),
        prompt: if production_settings {
            "Return an object whose ok field is true."
        } else {
            "Ignore the output schema and return an object whose ok field is false."
        }
        .to_owned(),
        schema_name: "paperless_llm_provider_test".to_owned(),
        schema: serde_json::json!({
            "type": "object",
            "properties": { "ok": { "const": true } },
            "required": ["ok"],
            "additionalProperties": false
        }),
        attachments: vec![],
        max_output_tokens: if production_settings {
            record.max_output_tokens.clamp(1, u32::MAX as i64) as u32
        } else {
            64
        },
        temperature: production_settings
            .then_some(record.temperature)
            .flatten(),
        reasoning,
    })
}

fn validate_model_reasoning(
    record: &ProviderProfileRecord,
    capabilities: &ProviderCapabilities,
) -> Result<(), String> {
    let Some(raw) = record.reasoning_effort.as_deref() else {
        return Ok(());
    };
    let Some(effort) = ReasoningEffort::parse(raw) else {
        return Err("unknown reasoning effort".to_owned());
    };
    if effort == ReasoningEffort::None {
        return Ok(());
    }
    if record.preset_kind() == "claude" && effort == ReasoningEffort::Minimal {
        return Err("Claude does not support the `minimal` effort level".to_owned());
    }
    if !capabilities.supports_reasoning_effort(effort) {
        return Err(format!(
            "the selected model for {} does not report support for `{raw}` reasoning effort",
            provider_label(record.preset_kind())
        ));
    }
    Ok(())
}

fn structured_fallback_allowed(error: &ProviderError) -> bool {
    matches!(
        error,
        ProviderError::InvalidStructuredOutput { .. }
            | ProviderError::Remote {
                status: 400 | 404 | 405 | 415 | 422 | 501,
                ..
            }
    )
}

async fn finish_provider_test(
    state: &AppState,
    id: &str,
    capabilities: ProviderCapabilities,
    ok: bool,
    message: String,
    started: Instant,
) -> AppResult<ProviderTestResult> {
    sqlx::query("UPDATE provider_profiles SET capabilities_json=?, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?")
        .bind(serde_json::to_string(&capabilities)?)
        .bind(id)
        .execute(state.database.pool())
        .await?;
    state
        .database
        .set_setting(
            &format!("provider_health:{id}"),
            &serde_json::json!({
                "ok": ok,
                "tested_at": Utc::now().to_rfc3339(),
                "message": message,
            }),
        )
        .await?;
    Ok(ProviderTestResult {
        ok,
        message,
        latency_ms: Some(started.elapsed().as_millis().min(u64::MAX as u128) as u64),
        capabilities: Some(capabilities_to_dto(&capabilities)),
    })
}

struct TestUsage {
    input: i64,
    cache_read: i64,
    cache_write: i64,
    output: i64,
    reasoning: i64,
    total: i64,
    source: &'static str,
}

struct TestUsageAttempt<'a> {
    provider: &'a ProviderProfileRecord,
    reservation_id: &'a str,
    reservation_state: &'a str,
    api_family: ApiFamily,
    operation: &'a str,
    status: &'a str,
    usage: &'a TestUsage,
    latency: std::time::Duration,
    error_code: Option<&'a str>,
}

fn normalize_test_usage(usage: &TokenUsage, counted_input: u64, raw_text: &str) -> TestUsage {
    let input = usage.input_tokens.unwrap_or(counted_input);
    let output = usage
        .output_tokens
        .unwrap_or_else(|| raw_text.chars().count().div_ceil(4) as u64);
    let total = usage
        .total_tokens
        .unwrap_or_else(|| input.saturating_add(output));
    let fully_reported = usage.input_tokens.is_some()
        && usage.output_tokens.is_some()
        && usage.total_tokens.is_some();
    TestUsage {
        input: input.min(i64::MAX as u64) as i64,
        cache_read: usage.cache_read_tokens.unwrap_or(0).min(i64::MAX as u64) as i64,
        cache_write: usage.cache_write_tokens.unwrap_or(0).min(i64::MAX as u64) as i64,
        output: output.min(i64::MAX as u64) as i64,
        reasoning: usage
            .reasoning_tokens
            .unwrap_or(0)
            .min(output)
            .min(i64::MAX as u64) as i64,
        total: total.min(i64::MAX as u64) as i64,
        source: if fully_reported {
            test_usage_source(usage.source)
        } else {
            "estimated"
        },
    }
}

async fn record_test_usage(state: &AppState, attempt: TestUsageAttempt<'_>) -> AppResult<()> {
    let TestUsageAttempt {
        provider,
        reservation_id,
        reservation_state,
        api_family,
        operation,
        status,
        usage,
        latency,
        error_code,
    } = attempt;
    let mut connection = state.database.pool().acquire().await?;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *connection)
        .await?;
    let reconciled = sqlx::query(
        "UPDATE budget_reservations SET state=?, settled_tokens=?, settled_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=? AND state='reserved'",
    )
    .bind(reservation_state)
    .bind(usage.total)
    .bind(reservation_id)
    .execute(&mut *connection)
    .await?;
    if reconciled.rows_affected() != 1 {
        let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
        return Err(AppError::Conflict(
            "provider-test budget reservation is no longer active".to_owned(),
        ));
    }
    let inserted = sqlx::query(
        "INSERT INTO usage_events(id,budget_reservation_id,job_id,provider_profile_id,provider_kind,model,api_family,operation,functions_json,status,input_tokens,cache_read_tokens,cache_write_tokens,output_tokens,reasoning_tokens,total_tokens,usage_source,latency_ms,error_code) \
         VALUES(?,?,NULL,?,?,?,?,?,'[]',?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(reservation_id)
    .bind(&provider.id)
    .bind(provider.preset_kind())
    .bind(&provider.model)
    .bind(test_api_family(api_family))
    .bind(operation)
    .bind(status)
    .bind(usage.input)
    .bind(usage.cache_read)
    .bind(usage.cache_write)
    .bind(usage.output)
    .bind(usage.reasoning)
    .bind(usage.total)
    .bind(usage.source)
    .bind(latency.as_millis().min(i64::MAX as u128) as i64)
    .bind(error_code)
    .execute(&mut *connection)
    .await;
    if let Err(error) = inserted {
        let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
        return Err(AppError::Database(error));
    }
    sqlx::query("COMMIT").execute(&mut *connection).await?;
    Ok(())
}

fn test_failure_is_ambiguous(error: &ProviderError) -> bool {
    match error {
        ProviderError::Transport { .. } | ProviderError::InvalidResponse(_) => true,
        ProviderError::Remote { status, .. } => *status >= 500 || matches!(*status, 408 | 429),
        ProviderError::Configuration(_) | ProviderError::InvalidStructuredOutput { .. } => false,
    }
}

fn test_error_code(error: &ProviderError) -> &'static str {
    match error {
        ProviderError::Configuration(_) => "provider_configuration",
        ProviderError::Transport { .. } => "provider_transport",
        ProviderError::Remote { .. } => "provider_remote",
        ProviderError::InvalidResponse(_) | ProviderError::InvalidStructuredOutput { .. } => {
            "invalid_structured_output"
        }
    }
}

fn test_usage_source(source: UsageSource) -> &'static str {
    match source {
        UsageSource::Reported => "reported",
        UsageSource::PreflightCounted => "preflight",
        UsageSource::Estimated => "estimated",
        UsageSource::Unknown => "unknown",
    }
}

fn test_api_family(family: ApiFamily) -> &'static str {
    match family {
        ApiFamily::OpenAiResponses => "responses",
        ApiFamily::OpenAiChatCompletions => "chat_completions",
        ApiFamily::AnthropicMessages => "messages",
    }
}

async fn record(state: &AppState, id: &str) -> AppResult<ProviderProfileRecord> {
    sqlx::query_as("SELECT * FROM provider_profiles WHERE id=? AND enabled=1")
        .bind(id)
        .fetch_optional(state.database.pool())
        .await?
        .ok_or_else(|| AppError::NotFound("provider profile not found".to_owned()))
}

async fn to_dto(
    state: &AppState,
    record: ProviderProfileRecord,
    active: Option<&str>,
) -> AppResult<ProviderProfileDto> {
    let preset_kind = record.preset_kind().to_owned();
    let capabilities = serde_json::from_str::<ProviderCapabilities>(&record.capabilities_json)
        .ok()
        .map(|value| capabilities_to_dto(&value));
    let health = state
        .database
        .get_setting::<serde_json::Value>(&format!("provider_health:{}", record.id))
        .await?;
    let api_key_configured = match record.api_key_secret_key.as_deref() {
        Some(key) => state.secrets.exists(key).await?,
        None => false,
    };
    let headers_configured = match record.headers_secret_key.as_deref() {
        Some(key) => state.secrets.exists(key).await?,
        None => false,
    };
    Ok(ProviderProfileDto {
        id: record.id.clone(),
        name: record.name,
        kind: preset_kind,
        base_url: record.base_url,
        model: record.model,
        transport: match record.api_family.as_str() {
            "messages" => "anthropic_messages".to_owned(),
            other => other.to_owned(),
        },
        chat_output_token_field: record.chat_output_token_field,
        api_key_configured,
        api_key: None,
        // Stored header values are credentials and are never returned.
        headers: None,
        headers_configured,
        reasoning_effort: record.reasoning_effort,
        temperature: record.temperature,
        max_output_tokens: Some(record.max_output_tokens.max(0) as u64),
        active: active == Some(record.id.as_str()),
        capabilities,
        last_tested_at: health
            .as_ref()
            .and_then(|value| value.get("tested_at"))
            .and_then(|value| value.as_str())
            .map(str::to_owned),
        last_test_ok: health
            .as_ref()
            .and_then(|value| value.get("ok"))
            .and_then(|value| value.as_bool()),
    })
}

pub fn capabilities_to_dto(value: &ProviderCapabilities) -> ProviderCapabilitiesDto {
    ProviderCapabilitiesDto {
        structured_output: value.strict_json_schema,
        reasoning: value.reasoning_effort || value.adaptive_thinking || value.thinking_budget,
        temperature: value.temperature,
        file_pdf: value.pdf_input,
        file_images: value.image_input,
        token_counting: if value.native_token_count {
            "exact"
        } else {
            "estimated"
        }
        .to_owned(),
    }
}

fn capabilities_from_dto(value: &ProviderCapabilitiesDto) -> ProviderCapabilities {
    ProviderCapabilities {
        temperature: value.temperature,
        reasoning_effort: value.reasoning,
        adaptive_thinking: value.reasoning,
        thinking_budget: value.reasoning,
        strict_json_schema: value.structured_output,
        image_input: value.file_images,
        pdf_input: value.file_pdf,
        native_token_count: value.token_counting == "exact",
        reasoning_efforts: None,
    }
}

fn default_capabilities(kind: &str, transport: &str) -> ProviderCapabilities {
    match kind {
        "openai" => ProviderCapabilities::openai_responses(),
        "claude" => ProviderCapabilities::anthropic_messages(),
        "lmstudio" => ProviderCapabilities::lm_studio(if transport == "chat_completions" {
            ApiFamily::OpenAiChatCompletions
        } else {
            ApiFamily::OpenAiResponses
        }),
        "mistral" => ProviderCapabilities::mistral_openai_compatible(),
        "google" => ProviderCapabilities::google_openai_compatible(),
        "moonshot" => ProviderCapabilities::moonshot_openai_compatible(),
        "kimi" => ProviderCapabilities::hosted_openai_compatible(),
        _ => ProviderCapabilities::custom_openai_compatible(),
    }
}

fn database_provider_kind(preset: &str) -> &str {
    match preset {
        "mistral" | "kimi" | "moonshot" | "google" => "custom",
        other => other,
    }
}

fn compatible_output_token_field<'a>(preset: &str, configured: &'a str) -> &'a str {
    match preset {
        "mistral" | "google" => "max_tokens",
        "kimi" | "moonshot" => "max_completion_tokens",
        _ => configured,
    }
}

fn database_transport(kind: &str, transport: &str) -> AppResult<&'static str> {
    match (kind, transport) {
        ("openai", "responses") => Ok("responses"),
        ("claude", "anthropic_messages" | "messages") => Ok("messages"),
        ("lmstudio" | "custom", "responses") => Ok("responses"),
        (
            "lmstudio" | "custom" | "mistral" | "kimi" | "moonshot" | "google",
            "chat_completions",
        ) => Ok("chat_completions"),
        _ => Err(AppError::Validation(
            "the selected provider and API transport are incompatible".to_owned(),
        )),
    }
}

fn validate(input: &ProviderProfileDto) -> AppResult<()> {
    if input.name.trim().is_empty() || input.name.chars().count() > 100 {
        return Err(AppError::Validation(
            "provider name must contain 1 to 100 characters".to_owned(),
        ));
    }
    if !matches!(
        input.kind.as_str(),
        "openai" | "claude" | "lmstudio" | "custom" | "mistral" | "kimi" | "moonshot" | "google"
    ) {
        return Err(AppError::Validation("unknown provider preset".to_owned()));
    }
    validate_provider_base_url(input.base_url.trim())?;
    database_transport(&input.kind, &input.transport)?;
    if !matches!(
        input.chat_output_token_field.as_str(),
        "max_tokens" | "max_completion_tokens"
    ) {
        return Err(AppError::Validation(
            "unknown Chat Completions output-token field".to_owned(),
        ));
    }
    if input.model.chars().count() > 200 {
        return Err(AppError::Validation("model ID is too long".to_owned()));
    }
    if input
        .temperature
        .is_some_and(|value| !value.is_finite() || !(0.0..=2.0).contains(&value))
    {
        return Err(AppError::Validation(
            "temperature must be between 0 and 2".to_owned(),
        ));
    }
    if input.reasoning_effort.as_deref().is_some_and(|effort| {
        !matches!(
            effort,
            "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
        )
    }) {
        return Err(AppError::Validation("unknown reasoning effort".to_owned()));
    }
    if input.kind == "claude" && input.reasoning_effort.as_deref() == Some("minimal") {
        return Err(AppError::Validation(
            "Claude supports low, medium, high, xhigh, or max effort; choose None to omit thinking controls"
                .to_owned(),
        ));
    }
    if input.kind == "mistral" && input.reasoning_effort.as_deref() == Some("max") {
        return Err(AppError::Validation(
            "Mistral supports none, minimal, low, medium, high, or xhigh reasoning effort"
                .to_owned(),
        ));
    }
    if input.kind == "google" && matches!(input.reasoning_effort.as_deref(), Some("xhigh" | "max"))
    {
        return Err(AppError::Validation(
            "Google Gemini supports none, minimal, low, medium, or high reasoning effort"
                .to_owned(),
        ));
    }
    if input.kind == "kimi" && input.reasoning_effort.is_some() {
        return Err(AppError::Validation(
            "the Kimi preset does not enable reasoning controls; leave reasoning effort blank"
                .to_owned(),
        ));
    }
    if input.kind == "moonshot"
        && input
            .reasoning_effort
            .as_deref()
            .is_some_and(|effort| !matches!(effort, "low" | "high" | "max"))
    {
        return Err(AppError::Validation(
            "Moonshot supports low, high, or max reasoning effort; leave it blank to omit reasoning controls"
                .to_owned(),
        ));
    }
    Ok(())
}

fn provider_requires_api_key(kind: &str) -> bool {
    matches!(
        kind,
        "openai" | "claude" | "mistral" | "kimi" | "moonshot" | "google"
    )
}

fn provider_label(kind: &str) -> &'static str {
    match kind {
        "openai" => "OpenAI",
        "claude" => "Claude",
        "mistral" => "Mistral",
        "kimi" => "Kimi",
        "moonshot" => "Moonshot",
        "google" => "Google Gemini",
        _ => "this provider",
    }
}

fn validate_headers(headers: &BTreeMap<String, String>) -> AppResult<()> {
    if headers.len() > 32 {
        return Err(AppError::Validation(
            "at most 32 custom headers are supported".to_owned(),
        ));
    }
    for (name, value) in headers {
        HeaderName::from_str(name)
            .map_err(|_| AppError::Validation(format!("invalid custom header name: {name}")))?;
        HeaderValue::from_str(value)
            .map_err(|_| AppError::Validation(format!("invalid value for custom header {name}")))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RuntimeOptions, app};

    async fn test_state() -> (tempfile::TempDir, AppState) {
        let directory = tempfile::tempdir().expect("temporary directory");
        let state = app::initialize(RuntimeOptions {
            data_dir: directory.path().to_owned(),
            bind: "127.0.0.1".parse().expect("loopback address"),
            port: 0,
            open_browser: false,
        })
        .await
        .expect("application state");
        (directory, state)
    }

    fn custom_profile(
        base_url: &str,
        api_key: Option<&str>,
        headers: Option<BTreeMap<String, String>>,
    ) -> ProviderProfileDto {
        ProviderProfileDto {
            id: String::new(),
            name: "Custom provider".to_owned(),
            kind: "custom".to_owned(),
            base_url: base_url.to_owned(),
            model: "entered-model".to_owned(),
            transport: "chat_completions".to_owned(),
            chat_output_token_field: "max_tokens".to_owned(),
            api_key_configured: false,
            api_key: api_key.map(str::to_owned),
            headers,
            headers_configured: false,
            reasoning_effort: None,
            temperature: None,
            max_output_tokens: Some(1_000),
            active: false,
            capabilities: None,
            last_tested_at: None,
            last_test_ok: None,
        }
    }

    async fn saved_record(state: &AppState, id: &str) -> ProviderProfileRecord {
        sqlx::query_as("SELECT * FROM provider_profiles WHERE id=?")
            .bind(id)
            .fetch_one(state.database.pool())
            .await
            .expect("saved provider")
    }

    #[test]
    fn presets_are_transport_strict() {
        assert_eq!(
            database_transport("openai", "responses").unwrap(),
            "responses"
        );
        assert!(database_transport("openai", "chat_completions").is_err());
        assert_eq!(
            database_transport("claude", "anthropic_messages").unwrap(),
            "messages"
        );
        for preset in ["mistral", "kimi", "moonshot", "google"] {
            assert_eq!(
                database_transport(preset, "chat_completions").unwrap(),
                "chat_completions"
            );
            assert!(database_transport(preset, "responses").is_err());
            assert_eq!(database_provider_kind(preset), "custom");
        }
        assert_eq!(
            compatible_output_token_field("mistral", "max_completion_tokens"),
            "max_tokens"
        );
        assert_eq!(
            compatible_output_token_field("google", "max_completion_tokens"),
            "max_tokens"
        );
        for preset in ["kimi", "moonshot"] {
            assert_eq!(
                compatible_output_token_field(preset, "max_tokens"),
                "max_completion_tokens"
            );
        }
    }

    #[test]
    fn provider_base_urls_reject_embedded_or_unjoinable_components() {
        for base_url in [
            "https://user@provider.example/v1",
            "https://user:password@provider.example/v1",
            "https://provider.example/v1?tenant=secret",
            "https://provider.example/v1#fragment",
        ] {
            let input = custom_profile(base_url, None, None);
            assert!(validate(&input).is_err(), "accepted unsafe URL: {base_url}");
        }
    }

    #[test]
    fn openai_temperature_is_off_until_capability_test_enables_it() {
        assert!(!default_capabilities("openai", "responses").temperature);
        for preset in ["mistral", "kimi", "moonshot", "google"] {
            assert!(!default_capabilities(preset, "chat_completions").temperature);
        }
        assert!(
            default_capabilities("mistral", "chat_completions")
                .supports_reasoning_effort(ReasoningEffort::XHigh)
        );
        assert!(
            !default_capabilities("mistral", "chat_completions")
                .supports_reasoning_effort(ReasoningEffort::Max)
        );
        assert!(
            default_capabilities("google", "chat_completions")
                .supports_reasoning_effort(ReasoningEffort::High)
        );
        assert!(
            !default_capabilities("google", "chat_completions")
                .supports_reasoning_effort(ReasoningEffort::XHigh)
        );
        assert!(
            default_capabilities("moonshot", "chat_completions")
                .supports_reasoning_effort(ReasoningEffort::Low)
        );
        assert!(
            default_capabilities("moonshot", "chat_completions")
                .supports_reasoning_effort(ReasoningEffort::Max)
        );
        assert!(
            !default_capabilities("moonshot", "chat_completions")
                .supports_reasoning_effort(ReasoningEffort::Medium)
        );
    }

    #[test]
    fn claude_minimal_is_rejected_but_none_is_a_valid_omission() {
        let mut input: ProviderProfileDto = serde_json::from_value(serde_json::json!({
            "name": "Claude",
            "kind": "claude",
            "base_url": "https://api.anthropic.com",
            "transport": "anthropic_messages",
            "reasoning_effort": "minimal"
        }))
        .unwrap();
        assert!(validate(&input).is_err());
        input.reasoning_effort = Some("none".to_owned());
        assert!(validate(&input).is_ok());
    }

    #[test]
    fn hosted_preset_dtos_round_trip_and_reject_unsupported_reasoning() {
        let cases = [
            ("mistral", "https://api.mistral.ai/v1"),
            ("kimi", "https://api.moonshot.ai/v1"),
            ("moonshot", "https://api.moonshot.cn/v1"),
            (
                "google",
                "https://generativelanguage.googleapis.com/v1beta/openai",
            ),
        ];
        for (kind, base_url) in cases {
            let input: ProviderProfileDto = serde_json::from_value(serde_json::json!({
                "name": kind,
                "kind": kind,
                "base_url": base_url,
                "transport": "chat_completions",
                "temperature": null
            }))
            .expect("preset DTO");
            validate(&input).expect("valid preset");
            let serialized = serde_json::to_value(&input).expect("serialize DTO");
            assert_eq!(serialized["kind"], kind);
            assert_eq!(serialized["base_url"], base_url);
            assert_eq!(serialized["transport"], "chat_completions");
            assert!(serialized["temperature"].is_null());
        }

        for (kind, effort) in [
            ("mistral", "max"),
            ("google", "xhigh"),
            ("kimi", "low"),
            ("kimi", "none"),
            ("moonshot", "none"),
            ("moonshot", "medium"),
        ] {
            let input: ProviderProfileDto = serde_json::from_value(serde_json::json!({
                "name": kind,
                "kind": kind,
                "base_url": "https://provider.example/v1",
                "transport": "chat_completions",
                "reasoning_effort": effort
            }))
            .expect("preset DTO");
            assert!(validate(&input).is_err(), "{kind} accepted {effort}");
        }

        for effort in ["low", "high", "max"] {
            let input: ProviderProfileDto = serde_json::from_value(serde_json::json!({
                "name": "Moonshot",
                "kind": "moonshot",
                "base_url": "https://api.moonshot.cn/v1",
                "transport": "chat_completions",
                "reasoning_effort": effort
            }))
            .expect("preset DTO");
            validate(&input).unwrap_or_else(|_| panic!("Moonshot rejected {effort}"));
        }
    }

    #[tokio::test]
    async fn changed_required_destination_rejects_blank_key_without_touching_old_secrets() {
        let (_directory, state) = test_state().await;
        let headers = BTreeMap::from([(
            "x-private-provider-header".to_owned(),
            "old-header-secret".to_owned(),
        )]);
        let saved = create(
            &state,
            custom_profile(
                "https://old-provider.example/v1",
                Some("old-api-key"),
                Some(headers),
            ),
        )
        .await
        .expect("initial provider");
        let before = saved_record(&state, &saved.id).await;
        let old_api_key = before.api_key_secret_key.clone().expect("API key pointer");
        let old_headers = before.headers_secret_key.clone().expect("headers pointer");

        let mut changed = saved.clone();
        changed.kind = "openai".to_owned();
        changed.name = "OpenAI".to_owned();
        changed.base_url = "https://api.openai.com/v1".to_owned();
        changed.transport = "responses".to_owned();
        changed.api_key = None;
        changed.headers = None;
        let error = update(&state, &saved.id, changed.clone())
            .await
            .expect_err("changed destination must require a freshly entered key");
        assert!(matches!(
            error,
            AppError::Validation(message) if message.contains("requires entering a new OpenAI API key")
        ));

        let after_rejection = saved_record(&state, &saved.id).await;
        assert_eq!(after_rejection.preset_kind(), "custom");
        assert_eq!(after_rejection.base_url, before.base_url);
        assert_eq!(
            after_rejection.api_key_secret_key.as_deref(),
            Some(old_api_key.as_str())
        );
        assert_eq!(
            after_rejection.headers_secret_key.as_deref(),
            Some(old_headers.as_str())
        );
        assert_eq!(
            state
                .secrets
                .get(&old_api_key)
                .await
                .expect("old API key lookup")
                .expect("old API key")
                .as_str(),
            "old-api-key"
        );
        assert_eq!(
            state
                .secrets
                .get(&old_headers)
                .await
                .expect("old headers lookup")
                .expect("old headers")
                .as_str(),
            "{\"x-private-provider-header\":\"old-header-secret\"}"
        );

        changed.api_key = Some("new-openai-key".to_owned());
        let updated = update(&state, &saved.id, changed)
            .await
            .expect("changed destination with new key");
        assert!(updated.api_key_configured);
        assert!(!updated.headers_configured);
        let replacement = saved_record(&state, &saved.id).await;
        let replacement_key = replacement
            .api_key_secret_key
            .as_deref()
            .expect("replacement API key pointer");
        assert_ne!(replacement_key, old_api_key);
        assert_eq!(
            state
                .secrets
                .get(replacement_key)
                .await
                .expect("replacement API key lookup")
                .expect("replacement API key")
                .as_str(),
            "new-openai-key"
        );
        assert!(!state.secrets.exists(&old_api_key).await.expect("old key"));
        assert!(
            !state
                .secrets
                .exists(&old_headers)
                .await
                .expect("old headers")
        );
        delete(&state, &saved.id).await.expect("provider cleanup");
    }

    #[tokio::test]
    async fn changed_optional_destination_drops_credentials_only_after_update() {
        let (_directory, state) = test_state().await;
        let headers = BTreeMap::from([("authorization-extra".to_owned(), "secret".to_owned())]);
        let saved = create(
            &state,
            custom_profile(
                "https://first.example/v1",
                Some("old-optional-key"),
                Some(headers),
            ),
        )
        .await
        .expect("initial provider");
        let before = saved_record(&state, &saved.id).await;
        let old_api_key = before.api_key_secret_key.clone().expect("API key pointer");
        let old_headers = before.headers_secret_key.clone().expect("headers pointer");

        let mut changed = saved.clone();
        changed.base_url = "https://second.example/v1".to_owned();
        changed.api_key = None;
        changed.headers = None;
        let updated = update(&state, &saved.id, changed)
            .await
            .expect("keyless custom destination update");
        assert!(!updated.api_key_configured);
        assert!(!updated.headers_configured);
        let after = saved_record(&state, &saved.id).await;
        assert_eq!(after.base_url, "https://second.example/v1");
        assert!(after.api_key_secret_key.is_none());
        assert!(after.headers_secret_key.is_none());
        assert!(!state.secrets.exists(&old_api_key).await.expect("old key"));
        assert!(
            !state
                .secrets
                .exists(&old_headers)
                .await
                .expect("old headers")
        );
    }

    #[tokio::test]
    async fn insecure_remote_http_update_is_rejected_without_touching_old_secrets() {
        let (_directory, state) = test_state().await;
        let headers = BTreeMap::from([("x-private".to_owned(), "old-header".to_owned())]);
        let saved = create(
            &state,
            custom_profile("https://secure.example/v1", Some("old-key"), Some(headers)),
        )
        .await
        .expect("initial provider");
        let before = saved_record(&state, &saved.id).await;
        let old_api_key = before.api_key_secret_key.clone().expect("API key pointer");
        let old_headers = before.headers_secret_key.clone().expect("headers pointer");

        let mut insecure = saved.clone();
        insecure.base_url = "http://provider.example/v1".to_owned();
        insecure.api_key = Some("replacement-key".to_owned());
        insecure.headers = Some(BTreeMap::from([(
            "x-private".to_owned(),
            "replacement-header".to_owned(),
        )]));
        let error = update(&state, &saved.id, insecure)
            .await
            .expect_err("remote HTTP must not carry provider secrets");
        assert!(matches!(
            error,
            AppError::Validation(message) if message.contains("require HTTPS")
        ));

        let after = saved_record(&state, &saved.id).await;
        assert_eq!(after.base_url, before.base_url);
        assert_eq!(after.api_key_secret_key, before.api_key_secret_key);
        assert_eq!(after.headers_secret_key, before.headers_secret_key);
        assert_eq!(
            state
                .secrets
                .get(&old_api_key)
                .await
                .expect("old API key lookup")
                .expect("old API key")
                .as_str(),
            "old-key"
        );
        assert_eq!(
            state
                .secrets
                .get(&old_headers)
                .await
                .expect("old headers lookup")
                .expect("old headers")
                .as_str(),
            "{\"x-private\":\"old-header\"}"
        );
        delete(&state, &saved.id).await.expect("provider cleanup");
    }

    #[tokio::test]
    async fn failed_database_update_keeps_the_working_profile_and_secrets() {
        let (_directory, state) = test_state().await;
        let headers = BTreeMap::from([("x-private".to_owned(), "old-header".to_owned())]);
        let saved = create(
            &state,
            custom_profile(
                "https://working.example/v1",
                Some("working-key"),
                Some(headers),
            ),
        )
        .await
        .expect("initial provider");
        let before = saved_record(&state, &saved.id).await;
        let old_api_key = before.api_key_secret_key.clone().expect("API key pointer");
        let old_headers = before.headers_secret_key.clone().expect("headers pointer");
        sqlx::query(
            "CREATE TRIGGER reject_provider_update BEFORE UPDATE ON provider_profiles \
             BEGIN SELECT RAISE(ABORT, 'deliberate provider update failure'); END",
        )
        .execute(state.database.pool())
        .await
        .expect("failure trigger");

        let mut changed = saved.clone();
        changed.base_url = "https://replacement.example/v1".to_owned();
        changed.api_key = Some("replacement-key".to_owned());
        changed.headers = Some(BTreeMap::from([(
            "x-private".to_owned(),
            "replacement-header".to_owned(),
        )]));
        let error = update(&state, &saved.id, changed)
            .await
            .expect_err("database trigger must reject update");
        assert!(matches!(error, AppError::Database(_)));

        let after = saved_record(&state, &saved.id).await;
        assert_eq!(after.base_url, before.base_url);
        assert_eq!(after.api_key_secret_key, before.api_key_secret_key);
        assert_eq!(after.headers_secret_key, before.headers_secret_key);
        assert_eq!(
            state
                .secrets
                .get(&old_api_key)
                .await
                .expect("old API key lookup")
                .expect("old API key")
                .as_str(),
            "working-key"
        );
        assert_eq!(
            state
                .secrets
                .get(&old_headers)
                .await
                .expect("old headers lookup")
                .expect("old headers")
                .as_str(),
            "{\"x-private\":\"old-header\"}"
        );

        sqlx::query("DROP TRIGGER reject_provider_update")
            .execute(state.database.pool())
            .await
            .expect("remove failure trigger");
        delete(&state, &saved.id).await.expect("provider cleanup");
    }

    #[tokio::test]
    async fn blank_credentials_are_retained_for_same_normalized_destination() {
        let (_directory, state) = test_state().await;
        let headers = BTreeMap::from([("x-private".to_owned(), "header-value".to_owned())]);
        let saved = create(
            &state,
            custom_profile(
                "https://provider.example/v1",
                Some("retained-key"),
                Some(headers),
            ),
        )
        .await
        .expect("initial provider");
        let before = saved_record(&state, &saved.id).await;

        let mut unchanged = saved.clone();
        unchanged.base_url = "HTTPS://PROVIDER.EXAMPLE:443/v1/".to_owned();
        unchanged.api_key = None;
        unchanged.headers = None;
        let updated = update(&state, &saved.id, unchanged)
            .await
            .expect("same normalized destination update");
        assert!(updated.api_key_configured);
        assert!(updated.headers_configured);
        let after = saved_record(&state, &saved.id).await;
        assert_eq!(after.api_key_secret_key, before.api_key_secret_key);
        assert_eq!(after.headers_secret_key, before.headers_secret_key);
        delete(&state, &saved.id).await.expect("provider cleanup");
    }
}
