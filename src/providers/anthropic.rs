use std::collections::HashSet;

use async_trait::async_trait;
use base64::Engine;
use reqwest::Client;
use serde_json::{Map, Value, json};

use super::types::{
    AnalysisRequest, AnalysisResult, ApiFamily, InputTokenCount, ModelInfo, ProviderCapabilities,
    ProviderError, ProviderKind, ReasoningEffort, TokenUsage, UsageSource,
    backfill_estimated_usage, estimated_input_tokens, parse_and_validate_json,
    prompt_with_json_fallback,
};
use super::{ProviderAdapter, receive_json};

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com/v1";
const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct AnthropicMessagesAdapter {
    client: Client,
    base_url: String,
    api_key: String,
    model: String,
    capabilities: ProviderCapabilities,
}

impl AnthropicMessagesAdapter {
    pub fn new(client: Client, model: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            client,
            base_url: DEFAULT_BASE_URL.to_owned(),
            api_key: api_key.into(),
            model: model.into(),
            capabilities: ProviderCapabilities::anthropic_messages(),
        }
    }

    pub fn with_base_url(
        client: Client,
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        Self {
            client,
            base_url: base_url.into().trim().trim_end_matches('/').to_owned(),
            api_key: api_key.into(),
            model: model.into(),
            capabilities: ProviderCapabilities::anthropic_messages(),
        }
    }

    pub fn with_capabilities(mut self, capabilities: ProviderCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.client
            .request(
                method,
                format!("{}/{}", self.base_url, path.trim_start_matches('/')),
            )
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
    }
}

#[async_trait]
impl ProviderAdapter for AnthropicMessagesAdapter {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Anthropic
    }

    fn api_family(&self) -> ApiFamily {
        ApiFamily::AnthropicMessages
    }

    fn selected_model(&self) -> &str {
        &self.model
    }

    fn capabilities(&self) -> &ProviderCapabilities {
        &self.capabilities
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let mut models = Vec::new();
        let mut after_id: Option<String> = None;
        let mut seen_cursors = HashSet::new();
        for _ in 0..100 {
            let mut request = self
                .request(reqwest::Method::GET, "models")
                .query(&[("limit", "1000")]);
            if let Some(cursor) = after_id.as_deref() {
                request = request.query(&[("after_id", cursor)]);
            }
            let response = request
                .send()
                .await
                .map_err(|error| ProviderError::transport("model listing", error))?;
            let response = receive_json(response, "model listing", &[&self.api_key]).await?;
            let page = parse_anthropic_model_page(&response.value)?;
            models.extend(page.models);
            let Some(cursor) = page.next_cursor else {
                return Ok(models);
            };
            if !seen_cursors.insert(cursor.clone()) {
                return Err(ProviderError::InvalidResponse(
                    "model listing repeated its pagination cursor".to_owned(),
                ));
            }
            after_id = Some(cursor);
        }
        Err(ProviderError::InvalidResponse(
            "model listing exceeded the pagination safety limit".to_owned(),
        ))
    }

    async fn count_input_tokens(
        &self,
        request: &AnalysisRequest,
    ) -> Result<InputTokenCount, ProviderError> {
        if !self.capabilities.native_token_count {
            return Ok(estimated_input_tokens(request));
        }
        let mut body = build_messages_body(&self.model, request, &self.capabilities);
        if let Some(object) = body.as_object_mut() {
            object.remove("max_tokens");
            object.remove("temperature");
            object.remove("thinking");
        }
        let response = self
            .request(reqwest::Method::POST, "messages/count_tokens")
            .json(&body)
            .send()
            .await
            .map_err(|error| ProviderError::transport("input token count", error))?;
        let response = receive_json(response, "input token count", &[&self.api_key]).await?;
        let input_tokens = response
            .value
            .get("input_tokens")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                ProviderError::InvalidResponse(
                    "input-token response omitted `input_tokens`".to_owned(),
                )
            })?;
        Ok(InputTokenCount {
            input_tokens,
            source: UsageSource::PreflightCounted,
        })
    }

    async fn analyze(&self, request: &AnalysisRequest) -> Result<AnalysisResult, ProviderError> {
        let body = build_messages_body(&self.model, request, &self.capabilities);
        let response = self
            .request(reqwest::Method::POST, "messages")
            .json(&body)
            .send()
            .await
            .map_err(|error| ProviderError::transport("analysis", error))?;
        let response = receive_json(response, "analysis", &[&self.api_key]).await?;
        let mut result =
            parse_messages_result(response.value, response.request_id, &request.schema)?;
        backfill_estimated_usage(&mut result.usage, request, &result.raw_text);
        Ok(result)
    }
}

pub(crate) fn build_messages_body(
    model: &str,
    request: &AnalysisRequest,
    capabilities: &ProviderCapabilities,
) -> Value {
    let system = if capabilities.strict_json_schema {
        request.system_prompt.clone()
    } else {
        prompt_with_json_fallback(request)
    };
    let mut content = vec![json!({ "type": "text", "text": request.prompt })];
    for attachment in &request.attachments {
        let encoded = base64::engine::general_purpose::STANDARD.encode(&attachment.bytes);
        if attachment.is_image() && capabilities.image_input {
            content.push(json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": attachment.media_type,
                    "data": encoded,
                }
            }));
        } else if attachment.is_pdf() && capabilities.pdf_input {
            content.push(json!({
                "type": "document",
                "source": {
                    "type": "base64",
                    "media_type": attachment.media_type,
                    "data": encoded,
                }
            }));
        }
    }

    let mut body = Map::from_iter([
        ("model".to_owned(), Value::String(model.to_owned())),
        ("system".to_owned(), Value::String(system)),
        (
            "messages".to_owned(),
            json!([{ "role": "user", "content": content }]),
        ),
        (
            "max_tokens".to_owned(),
            Value::from(request.max_output_tokens),
        ),
    ]);

    let reasoning = request.reasoning.as_ref();
    let explicit_budget = reasoning
        .and_then(|reasoning| reasoning.budget_tokens)
        .filter(|_| capabilities.thinking_budget);
    let selected_effort = reasoning
        .and_then(|reasoning| reasoning.effort)
        .filter(|effort| {
            !matches!(effort, ReasoningEffort::None | ReasoningEffort::Minimal)
                && capabilities.supports_reasoning_effort(*effort)
                && explicit_budget.is_none()
        });

    let mut output_config = Map::new();
    if capabilities.strict_json_schema {
        output_config.insert(
            "format".to_owned(),
            json!({
                "type": "json_schema",
                "schema": request.schema,
            }),
        );
    }
    if let Some(effort) = selected_effort {
        if capabilities.adaptive_thinking {
            body.insert("thinking".to_owned(), json!({ "type": "adaptive" }));
        }
        output_config.insert(
            "effort".to_owned(),
            Value::String(effort.as_str().to_owned()),
        );
    } else if let Some(budget_tokens) = explicit_budget {
        body.insert(
            "thinking".to_owned(),
            json!({ "type": "enabled", "budget_tokens": budget_tokens }),
        );
    }
    if !output_config.is_empty() {
        body.insert("output_config".to_owned(), Value::Object(output_config));
    }

    // Anthropic models that expose thinking do not consistently accept
    // sampling controls at the same time, so temperature is only serialized
    // for non-thinking requests.
    if capabilities.temperature
        && explicit_budget.is_none()
        && !(selected_effort.is_some() && capabilities.adaptive_thinking)
        && let Some(temperature) = request.temperature
        && temperature.is_finite()
    {
        body.insert("temperature".to_owned(), json!(temperature));
    }
    Value::Object(body)
}

fn parse_messages_result(
    response: Value,
    request_id: Option<String>,
    schema: &Value,
) -> Result<AnalysisResult, ProviderError> {
    let usage = parse_anthropic_usage(response.get("usage"));
    let request_id = request_id.or_else(|| {
        response
            .get("id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    });
    let fragments = response
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>();
    if fragments.is_empty() {
        return Err(ProviderError::InvalidStructuredOutput {
            detail: "Messages payload contained no text content".to_owned(),
            raw_text: String::new(),
            usage: Box::new(usage),
        });
    }
    let raw_text = fragments.join("");
    let value = parse_and_validate_json(&raw_text, schema).map_err(|error| match error {
        ProviderError::InvalidResponse(detail) => ProviderError::InvalidStructuredOutput {
            detail,
            raw_text: raw_text.clone(),
            usage: Box::new(usage.clone()),
        },
        other => other,
    })?;
    Ok(AnalysisResult {
        value,
        raw_text,
        usage,
        request_id,
    })
}

fn parse_anthropic_usage(usage: Option<&Value>) -> TokenUsage {
    let Some(usage) = usage else {
        return TokenUsage::default();
    };
    let input_tokens = usage.get("input_tokens").and_then(Value::as_u64);
    let cache_read_tokens = usage.get("cache_read_input_tokens").and_then(Value::as_u64);
    let cache_write_tokens = usage
        .get("cache_creation_input_tokens")
        .and_then(Value::as_u64);
    let output_tokens = usage.get("output_tokens").and_then(Value::as_u64);
    let reasoning_tokens = usage
        .get("thinking_tokens")
        .or_else(|| usage.pointer("/output_tokens_details/thinking_tokens"))
        .and_then(Value::as_u64)
        .map(|reasoning| {
            output_tokens
                .map(|output| reasoning.min(output))
                .unwrap_or(reasoning)
        });
    let total_tokens = [
        input_tokens,
        cache_read_tokens,
        cache_write_tokens,
        output_tokens,
    ]
    .into_iter()
    .flatten()
    .reduce(u64::saturating_add);
    TokenUsage {
        input_tokens,
        cache_read_tokens,
        cache_write_tokens,
        output_tokens,
        reasoning_tokens,
        total_tokens,
        source: UsageSource::Reported,
    }
}

struct AnthropicModelPage {
    models: Vec<ModelInfo>,
    next_cursor: Option<String>,
}

fn parse_anthropic_model_page(value: &Value) -> Result<AnthropicModelPage, ProviderError> {
    let models = value.get("data").and_then(Value::as_array).ok_or_else(|| {
        ProviderError::InvalidResponse("model list omitted the `data` array".to_owned())
    })?;
    let models = models
        .iter()
        .filter_map(|model| {
            Some(ModelInfo {
                id: model.get("id")?.as_str()?.to_owned(),
                display_name: model
                    .get("display_name")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                owned_by: Some("anthropic".to_owned()),
                capabilities: anthropic_model_capabilities(model),
                max_input_tokens: model.get("max_input_tokens").and_then(Value::as_u64),
                max_output_tokens: model.get("max_tokens").and_then(Value::as_u64),
            })
        })
        .collect();
    let next_cursor = if value
        .get("has_more")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        Some(
            value
                .get("last_id")
                .and_then(Value::as_str)
                .filter(|cursor| !cursor.is_empty())
                .ok_or_else(|| {
                    ProviderError::InvalidResponse(
                        "paginated model list omitted `last_id`".to_owned(),
                    )
                })?
                .to_owned(),
        )
    } else {
        None
    };
    Ok(AnthropicModelPage {
        models,
        next_cursor,
    })
}

fn anthropic_model_capabilities(model: &Value) -> Option<ProviderCapabilities> {
    model.get("capabilities")?.as_object()?;
    let supported = |pointer: &str| {
        model
            .pointer(pointer)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    let effort_supported = supported("/capabilities/effort/supported");
    let adaptive_thinking = supported("/capabilities/thinking/types/adaptive/supported");
    let reasoning_efforts = [
        (ReasoningEffort::Low, "low"),
        (ReasoningEffort::Medium, "medium"),
        (ReasoningEffort::High, "high"),
        (ReasoningEffort::XHigh, "xhigh"),
        (ReasoningEffort::Max, "max"),
    ]
    .into_iter()
    .filter_map(|(effort, name)| {
        supported(&format!("/capabilities/effort/{name}/supported")).then_some(effort)
    })
    .collect::<Vec<_>>();
    Some(ProviderCapabilities {
        temperature: true,
        reasoning_effort: effort_supported,
        adaptive_thinking,
        thinking_budget: supported("/capabilities/thinking/types/enabled/supported"),
        strict_json_schema: supported("/capabilities/structured_outputs/supported"),
        image_input: supported("/capabilities/image_input/supported"),
        pdf_input: supported("/capabilities/pdf_input/supported"),
        native_token_count: true,
        reasoning_efforts: Some(reasoning_efforts),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::providers::{Attachment, ReasoningEffort, ReasoningOptions};

    fn request() -> AnalysisRequest {
        AnalysisRequest {
            system_prompt: "Analyze safely.".to_owned(),
            prompt: "OCR text".to_owned(),
            schema_name: "analysis".to_owned(),
            schema: json!({
                "type": "object",
                "properties": { "tags": { "type": "array", "items": { "type": "string" } } },
                "required": ["tags"],
                "additionalProperties": false
            }),
            attachments: vec![],
            max_output_tokens: 8000,
            temperature: Some(0.3),
            reasoning: Some(ReasoningOptions {
                effort: Some(ReasoningEffort::High),
                budget_tokens: None,
            }),
        }
    }

    #[test]
    fn adaptive_thinking_uses_native_controls_and_omits_temperature() {
        let body = build_messages_body(
            "claude",
            &request(),
            &ProviderCapabilities::anthropic_messages(),
        );
        assert_eq!(body.pointer("/thinking/type"), Some(&json!("adaptive")));
        assert_eq!(body.pointer("/output_config/effort"), Some(&json!("high")));
        assert_eq!(
            body.pointer("/output_config/format/type"),
            Some(&json!("json_schema"))
        );
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn effort_does_not_require_adaptive_thinking() {
        let capabilities = ProviderCapabilities {
            adaptive_thinking: false,
            thinking_budget: false,
            ..ProviderCapabilities::anthropic_messages()
        };
        let body = build_messages_body("claude", &request(), &capabilities);
        assert_eq!(body.pointer("/output_config/effort"), Some(&json!("high")));
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn explicit_thinking_budget_takes_precedence() {
        let mut request = request();
        request.reasoning = Some(ReasoningOptions {
            effort: Some(ReasoningEffort::Low),
            budget_tokens: Some(2048),
        });
        let body = build_messages_body(
            "claude",
            &request,
            &ProviderCapabilities::anthropic_messages(),
        );
        assert_eq!(body.pointer("/thinking/type"), Some(&json!("enabled")));
        assert_eq!(body.pointer("/thinking/budget_tokens"), Some(&json!(2048)));
        assert!(body.pointer("/output_config/effort").is_none());
    }

    #[test]
    fn serializes_native_image_and_pdf_blocks() {
        let mut request = request();
        request.attachments = vec![
            Attachment {
                file_name: "page.jpeg".to_owned(),
                media_type: "image/jpeg".to_owned(),
                bytes: vec![1, 2, 3],
            },
            Attachment {
                file_name: "document.pdf".to_owned(),
                media_type: "application/pdf".to_owned(),
                bytes: vec![4, 5, 6],
            },
        ];
        let body = build_messages_body(
            "claude",
            &request,
            &ProviderCapabilities::anthropic_messages(),
        );
        assert_eq!(
            body.pointer("/messages/0/content/1/type"),
            Some(&json!("image"))
        );
        assert_eq!(
            body.pointer("/messages/0/content/2/type"),
            Some(&json!("document"))
        );
        assert_eq!(
            body.pointer("/messages/0/content/2/source/media_type"),
            Some(&json!("application/pdf"))
        );
    }

    #[test]
    fn normalizes_cache_usage_as_separate_input_categories() {
        let usage = json!({
            "input_tokens": 10,
            "cache_read_input_tokens": 70,
            "cache_creation_input_tokens": 20,
            "output_tokens": 30,
            "thinking_tokens": 25
        });
        let normalized = parse_anthropic_usage(Some(&usage));
        assert_eq!(normalized.input_tokens, Some(10));
        assert_eq!(normalized.cache_read_tokens, Some(70));
        assert_eq!(normalized.cache_write_tokens, Some(20));
        assert_eq!(normalized.output_tokens, Some(30));
        assert_eq!(normalized.reasoning_tokens, Some(25));
        assert_eq!(normalized.total_tokens, Some(130));
    }

    #[test]
    fn none_and_legacy_minimal_omit_claude_thinking_controls() {
        for effort in [ReasoningEffort::None, ReasoningEffort::Minimal] {
            let mut request = request();
            request.reasoning = Some(ReasoningOptions {
                effort: Some(effort),
                budget_tokens: None,
            });
            let body = build_messages_body(
                "claude",
                &request,
                &ProviderCapabilities::anthropic_messages(),
            );
            assert!(body.get("thinking").is_none());
            assert!(body.pointer("/output_config/effort").is_none());
        }
    }

    #[test]
    fn parses_model_capabilities_effort_levels_and_pagination() {
        let page = parse_anthropic_model_page(&json!({
            "data": [{
                "id": "claude-model",
                "display_name": "Claude Model",
                "max_input_tokens": 200000,
                "max_tokens": 64000,
                "capabilities": {
                    "structured_outputs": { "supported": true },
                    "image_input": { "supported": true },
                    "pdf_input": { "supported": false },
                    "thinking": { "types": {
                        "adaptive": { "supported": true },
                        "enabled": { "supported": true }
                    }},
                    "effort": {
                        "supported": true,
                        "low": { "supported": true },
                        "medium": { "supported": true },
                        "high": { "supported": true },
                        "xhigh": { "supported": false },
                        "max": { "supported": true }
                    }
                }
            }],
            "has_more": true,
            "last_id": "claude-model"
        }))
        .expect("valid page");
        assert_eq!(page.next_cursor.as_deref(), Some("claude-model"));
        let model = &page.models[0];
        assert_eq!(model.max_input_tokens, Some(200000));
        assert_eq!(model.max_output_tokens, Some(64000));
        let capabilities = model.capabilities.as_ref().expect("capabilities");
        assert!(capabilities.strict_json_schema);
        assert!(capabilities.reasoning_effort);
        assert!(capabilities.thinking_budget);
        assert!(capabilities.image_input);
        assert!(!capabilities.pdf_input);
        assert!(capabilities.supports_reasoning_effort(ReasoningEffort::Max));
        assert!(!capabilities.supports_reasoning_effort(ReasoningEffort::XHigh));
    }
}
