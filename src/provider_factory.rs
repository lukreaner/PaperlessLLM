use std::{collections::BTreeMap, sync::Arc, time::Duration};

use reqwest::Client;

use crate::{
    AppError, AppResult,
    models::ProviderProfileRecord,
    providers::{
        AnthropicMessagesAdapter, ChatOutputTokenField, CompatibleTransport,
        OpenAiCompatibleAdapter, OpenAiResponsesAdapter, ProviderAdapter, ProviderCapabilities,
    },
    secrets::SecretStore,
};

pub async fn build_adapter(
    record: &ProviderProfileRecord,
    secrets: &SecretStore,
) -> AppResult<Arc<dyn ProviderAdapter>> {
    let client = Client::builder()
        .timeout(Duration::from_secs(
            record.request_timeout_seconds.clamp(5, 1_800) as u64,
        ))
        .connect_timeout(Duration::from_secs(20))
        .user_agent(concat!("PaperlessLLM/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let api_key = match record.api_key_secret_key.as_deref() {
        Some(key) => secrets.get(key).await?.map(|value| value.to_string()),
        None => None,
    };

    let transport = match record.api_family.as_str() {
        "responses" => CompatibleTransport::Responses,
        "chat_completions" => CompatibleTransport::ChatCompletions,
        "messages" if record.provider_kind == "claude" => CompatibleTransport::Responses,
        other => {
            return Err(AppError::Validation(format!(
                "unsupported provider transport: {other}"
            )));
        }
    };
    let chat_output_token_field = match record.chat_output_token_field.as_str() {
        "max_completion_tokens" => ChatOutputTokenField::MaxCompletionTokens,
        _ => ChatOutputTokenField::MaxTokens,
    };

    let adapter: Arc<dyn ProviderAdapter> = match record.provider_kind.as_str() {
        "openai" => {
            let key = api_key.ok_or_else(|| {
                AppError::Validation("the OpenAI provider needs an API key".to_owned())
            })?;
            let adapter = OpenAiResponsesAdapter::with_base_url(
                client,
                &record.base_url,
                &record.model,
                Some(key),
            );
            Arc::new(apply_openai_capabilities(adapter, record))
        }
        "claude" => {
            let key = api_key.ok_or_else(|| {
                AppError::Validation("the Claude provider needs an API key".to_owned())
            })?;
            let base_url = anthropic_api_base(&record.base_url);
            let adapter =
                AnthropicMessagesAdapter::with_base_url(client, base_url, &record.model, key);
            Arc::new(apply_anthropic_capabilities(adapter, record))
        }
        "lmstudio" => {
            let adapter = OpenAiCompatibleAdapter::lm_studio_with_base_url(
                client,
                &record.base_url,
                &record.model,
                api_key,
                transport,
            )
            .with_output_token_field(chat_output_token_field);
            Arc::new(apply_compatible_capabilities(adapter, record))
        }
        "custom" => {
            let mut adapter = OpenAiCompatibleAdapter::custom(
                client,
                &record.base_url,
                &record.model,
                api_key,
                transport,
            )
            .with_output_token_field(chat_output_token_field);
            if let Some(secret_key) = record.headers_secret_key.as_deref()
                && let Some(raw) = secrets.get(secret_key).await?
            {
                let headers: BTreeMap<String, String> =
                    serde_json::from_str(&raw).map_err(|_| {
                        AppError::Internal("stored provider headers are invalid".to_owned())
                    })?;
                for (name, value) in headers {
                    adapter = adapter.with_header(name, value);
                }
            }
            Arc::new(apply_compatible_capabilities(adapter, record))
        }
        other => {
            return Err(AppError::Validation(format!(
                "unsupported provider kind: {other}"
            )));
        }
    };
    Ok(adapter)
}

/// Builds an adapter for a one-off capability probe without mutating the
/// persisted profile. The resulting request path, credentials, model, and
/// transport are otherwise identical to production processing.
pub async fn build_adapter_with_capabilities(
    record: &ProviderProfileRecord,
    secrets: &SecretStore,
    capabilities: &ProviderCapabilities,
) -> AppResult<Arc<dyn ProviderAdapter>> {
    let mut overridden = record.clone();
    overridden.capabilities_json = serde_json::to_string(capabilities)?;
    build_adapter(&overridden, secrets).await
}

fn stored_capabilities(record: &ProviderProfileRecord) -> Option<ProviderCapabilities> {
    if record.capabilities_json.trim() == "{}" {
        return None;
    }
    serde_json::from_str(&record.capabilities_json).ok()
}

fn apply_openai_capabilities(
    adapter: OpenAiResponsesAdapter,
    record: &ProviderProfileRecord,
) -> OpenAiResponsesAdapter {
    match stored_capabilities(record) {
        Some(capabilities) => adapter.with_capabilities(capabilities),
        None => adapter,
    }
}

fn apply_anthropic_capabilities(
    adapter: AnthropicMessagesAdapter,
    record: &ProviderProfileRecord,
) -> AnthropicMessagesAdapter {
    match stored_capabilities(record) {
        Some(capabilities) => adapter.with_capabilities(capabilities),
        None => adapter,
    }
}

fn apply_compatible_capabilities(
    adapter: OpenAiCompatibleAdapter,
    record: &ProviderProfileRecord,
) -> OpenAiCompatibleAdapter {
    match stored_capabilities(record) {
        Some(capabilities) => adapter.with_capabilities(capabilities),
        None => adapter,
    }
}

fn anthropic_api_base(raw: &str) -> String {
    let value = raw.trim().trim_end_matches('/');
    if value.ends_with("/v1") {
        value.to_owned()
    } else {
        format!("{value}/v1")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_preset_gets_native_api_path() {
        assert_eq!(
            anthropic_api_base("https://api.anthropic.com"),
            "https://api.anthropic.com/v1"
        );
        assert_eq!(
            anthropic_api_base("https://gateway.example/v1/"),
            "https://gateway.example/v1"
        );
    }
}
