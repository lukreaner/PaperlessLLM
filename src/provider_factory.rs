use std::{collections::BTreeMap, sync::Arc, time::Duration};

use reqwest::{Client, redirect::Policy};
use url::{Host, Url};

use crate::{
    AppError, AppResult,
    models::ProviderProfileRecord,
    providers::{
        AnthropicMessagesAdapter, ChatOutputTokenField, CompatibleTransport,
        OpenAiCompatibleAdapter, OpenAiResponsesAdapter, ProviderAdapter, ProviderCapabilities,
        ProviderKind,
    },
    secrets::SecretStore,
};

pub async fn build_adapter(
    record: &ProviderProfileRecord,
    secrets: &SecretStore,
) -> AppResult<Arc<dyn ProviderAdapter>> {
    let client = provider_http_client(record.request_timeout_seconds)?;
    let api_key = match record.api_key_secret_key.as_deref() {
        Some(key) => secrets.get(key).await?.map(|value| value.to_string()),
        None => None,
    };
    validate_provider_secret_transport(
        &record.base_url,
        api_key.is_some()
            || (record.preset_kind() == "custom" && record.headers_secret_key.is_some()),
    )?;

    let transport = match record.api_family.as_str() {
        "responses" => CompatibleTransport::Responses,
        "chat_completions" => CompatibleTransport::ChatCompletions,
        "messages" if record.preset_kind() == "claude" => CompatibleTransport::Responses,
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

    let adapter: Arc<dyn ProviderAdapter> = match record.preset_kind() {
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
        preset @ ("mistral" | "kimi" | "moonshot" | "google") => {
            let key = api_key.ok_or_else(|| {
                AppError::Validation(format!(
                    "the {} provider needs an API key",
                    hosted_preset_label(preset)
                ))
            })?;
            let adapter = OpenAiCompatibleAdapter::hosted_preset(
                client,
                hosted_preset_kind(preset),
                &record.base_url,
                &record.model,
                key,
            )
            .with_output_token_field(chat_output_token_field);
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

fn provider_http_client(request_timeout_seconds: i64) -> AppResult<Client> {
    Client::builder()
        .timeout(Duration::from_secs(
            request_timeout_seconds.clamp(5, 1_800) as u64
        ))
        .connect_timeout(Duration::from_secs(20))
        // Provider requests carry Bearer credentials and may include arbitrary
        // user-configured secret headers. Require the configured URL to be the
        // final destination instead of forwarding either across a redirect.
        .redirect(Policy::none())
        .user_agent(concat!("PaperlessLLM/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| AppError::Internal(error.to_string()))
}

pub(crate) fn validate_provider_base_url(raw: &str) -> AppResult<Url> {
    let url = Url::parse(raw.trim())
        .map_err(|_| AppError::Validation("provider base URL must be absolute".to_owned()))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(AppError::Validation(
            "provider base URL must use HTTP or HTTPS".to_owned(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::Validation(
            "provider base URL must not contain a username or password".to_owned(),
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(AppError::Validation(
            "provider base URL must not contain a query or fragment".to_owned(),
        ));
    }
    Ok(url)
}

pub(crate) fn validate_provider_secret_transport(
    raw: &str,
    carries_secrets: bool,
) -> AppResult<()> {
    let url = validate_provider_base_url(raw)?;
    if !carries_secrets || url.scheme() == "https" || provider_host_is_loopback(&url) {
        return Ok(());
    }
    Err(AppError::Validation(
        "provider API keys and custom secret headers require HTTPS unless the provider is on this computer (localhost, 127.0.0.1, or ::1)"
            .to_owned(),
    ))
}

fn provider_host_is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        Some(Host::Domain(domain)) => {
            let domain = domain.trim_end_matches('.');
            domain.eq_ignore_ascii_case("localhost")
                || domain.to_ascii_lowercase().ends_with(".localhost")
        }
        None => false,
    }
}

fn hosted_preset_kind(value: &str) -> ProviderKind {
    match value {
        "mistral" => ProviderKind::Mistral,
        "kimi" => ProviderKind::Kimi,
        "moonshot" => ProviderKind::Moonshot,
        "google" => ProviderKind::Google,
        _ => unreachable!("validated hosted provider preset"),
    }
}

fn hosted_preset_label(value: &str) -> &'static str {
    match value {
        "mistral" => "Mistral",
        "kimi" => "Kimi",
        "moonshot" => "Moonshot",
        "google" => "Google Gemini",
        _ => "hosted",
    }
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
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

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

    #[test]
    fn secret_transport_requires_tls_except_for_loopback_hosts() {
        for base_url in [
            "http://localhost:1234/v1",
            "http://worker.localhost:1234/v1",
            "http://127.0.0.1:1234/v1",
            "http://[::1]:1234/v1",
            "https://provider.example/v1",
        ] {
            validate_provider_secret_transport(base_url, true)
                .unwrap_or_else(|_| panic!("safe credential destination rejected: {base_url}"));
        }
        assert!(validate_provider_secret_transport("http://provider.example/v1", true).is_err());
        assert!(validate_provider_secret_transport("http://192.0.2.10:1234/v1", true).is_err());
        assert!(validate_provider_secret_transport("http://provider.example/v1", false).is_ok());
    }

    #[tokio::test]
    async fn provider_client_does_not_follow_redirects_with_secret_headers() {
        let destination = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/credential-capture"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&destination)
            .await;

        let configured = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(302).insert_header(
                "location",
                format!("{}/credential-capture", destination.uri()),
            ))
            .expect(1)
            .mount(&configured)
            .await;

        let response = provider_http_client(30)
            .expect("provider client")
            .get(format!("{}/models", configured.uri()))
            .bearer_auth("must-not-leave-the-configured-server")
            .header("x-private-provider-header", "also-must-not-leave")
            .send()
            .await
            .expect("redirect response");

        assert_eq!(response.status(), reqwest::StatusCode::FOUND);
        destination.verify().await;
        configured.verify().await;
    }
}
