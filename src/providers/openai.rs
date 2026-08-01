use async_trait::async_trait;
use base64::Engine;
use reqwest::Client;
use serde_json::{Map, Value, json};

use super::types::{
    AnalysisRequest, AnalysisResult, ApiFamily, InputTokenCount, ModelInfo, ProviderCapabilities,
    ProviderError, ProviderKind, TokenUsage, UsageSource, backfill_estimated_usage,
    estimated_input_tokens, parse_and_validate_json, prompt_with_json_fallback,
};
use super::{ProviderAdapter, receive_json};

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

pub struct OpenAiResponsesAdapter {
    client: Client,
    base_url: String,
    api_key: Option<String>,
    model: String,
    capabilities: ProviderCapabilities,
}

impl OpenAiResponsesAdapter {
    pub fn new(client: Client, model: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            client,
            base_url: DEFAULT_BASE_URL.to_owned(),
            api_key: Some(api_key.into()),
            model: model.into(),
            capabilities: ProviderCapabilities::openai_responses(),
        }
    }

    /// Builds an OpenAI Responses adapter for a compatible gateway. API keys
    /// are optional so local authenticated proxies can be used.
    pub fn with_base_url(
        client: Client,
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: Option<String>,
    ) -> Self {
        Self {
            client,
            base_url: normalize_base_url(base_url.into()),
            api_key,
            model: model.into(),
            capabilities: ProviderCapabilities::openai_responses(),
        }
    }

    pub fn with_capabilities(mut self, capabilities: ProviderCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let request = self.client.request(
            method,
            format!("{}/{}", self.base_url, path.trim_start_matches('/')),
        );
        match self.api_key.as_deref().filter(|key| !key.is_empty()) {
            Some(api_key) => request.bearer_auth(api_key),
            None => request,
        }
    }
}

#[async_trait]
impl ProviderAdapter for OpenAiResponsesAdapter {
    fn kind(&self) -> ProviderKind {
        ProviderKind::OpenAi
    }

    fn api_family(&self) -> ApiFamily {
        ApiFamily::OpenAiResponses
    }

    fn selected_model(&self) -> &str {
        &self.model
    }

    fn capabilities(&self) -> &ProviderCapabilities {
        &self.capabilities
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let response = self
            .request(reqwest::Method::GET, "models")
            .send()
            .await
            .map_err(|error| ProviderError::transport("model listing", error))?;
        let response = receive_json(response, "model listing").await?;
        parse_openai_models(&response.value)
    }

    async fn count_input_tokens(
        &self,
        request: &AnalysisRequest,
    ) -> Result<InputTokenCount, ProviderError> {
        if !self.capabilities.native_token_count {
            return Ok(estimated_input_tokens(request));
        }
        let mut body = build_responses_body(&self.model, request, &self.capabilities);
        if let Some(object) = body.as_object_mut() {
            object.remove("max_output_tokens");
            object.remove("reasoning");
            object.remove("temperature");
        }
        let response = self
            .request(reqwest::Method::POST, "responses/input_tokens")
            .json(&body)
            .send()
            .await
            .map_err(|error| ProviderError::transport("input token count", error))?;
        let response = receive_json(response, "input token count").await?;
        let input_tokens = response
            .value
            .get("input_tokens")
            .or_else(|| response.value.pointer("/usage/input_tokens"))
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
        let body = build_responses_body(&self.model, request, &self.capabilities);
        let response = self
            .request(reqwest::Method::POST, "responses")
            .json(&body)
            .send()
            .await
            .map_err(|error| ProviderError::transport("analysis", error))?;
        let response = receive_json(response, "analysis").await?;
        let mut result =
            parse_responses_result(response.value, response.request_id, &request.schema)?;
        backfill_estimated_usage(&mut result.usage, request, &result.raw_text);
        Ok(result)
    }
}

pub(crate) fn build_responses_body(
    model: &str,
    request: &AnalysisRequest,
    capabilities: &ProviderCapabilities,
) -> Value {
    let instructions = if capabilities.strict_json_schema {
        request.system_prompt.clone()
    } else {
        prompt_with_json_fallback(request)
    };
    let mut content = vec![json!({
        "type": "input_text",
        "text": request.prompt,
    })];
    for attachment in &request.attachments {
        let encoded = base64::engine::general_purpose::STANDARD.encode(&attachment.bytes);
        if attachment.is_image() && capabilities.image_input {
            content.push(json!({
                "type": "input_image",
                "image_url": format!("data:{};base64,{encoded}", attachment.media_type),
            }));
        } else if attachment.is_pdf() && capabilities.pdf_input {
            content.push(json!({
                "type": "input_file",
                "filename": attachment.file_name,
                "file_data": format!("data:{};base64,{encoded}", attachment.media_type),
            }));
        }
    }

    let mut body = Map::from_iter([
        ("model".to_owned(), Value::String(model.to_owned())),
        ("instructions".to_owned(), Value::String(instructions)),
        (
            "input".to_owned(),
            json!([{ "role": "user", "content": content }]),
        ),
        (
            "max_output_tokens".to_owned(),
            Value::from(request.max_output_tokens),
        ),
    ]);
    if capabilities.strict_json_schema {
        body.insert(
            "text".to_owned(),
            json!({
                "format": {
                    "type": "json_schema",
                    "name": request.schema_name,
                    "schema": request.schema,
                    "strict": true,
                }
            }),
        );
    }
    if capabilities.temperature
        && let Some(temperature) = request.temperature
        && temperature.is_finite()
    {
        body.insert("temperature".to_owned(), json!(temperature));
    }
    if capabilities.reasoning_effort
        && let Some(effort) = request
            .reasoning
            .as_ref()
            .and_then(|reasoning| reasoning.effort)
        && capabilities.supports_reasoning_effort(effort)
    {
        body.insert("reasoning".to_owned(), json!({ "effort": effort.as_str() }));
    }
    Value::Object(body)
}

pub(crate) fn parse_responses_result(
    response: Value,
    request_id: Option<String>,
    schema: &Value,
) -> Result<AnalysisResult, ProviderError> {
    let usage = parse_openai_usage(response.get("usage"));
    let request_id = request_id.or_else(|| {
        response
            .get("id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    });
    let raw_text = extract_responses_text(&response).ok_or_else(|| {
        ProviderError::InvalidStructuredOutput {
            detail: "Responses payload contained no output text".to_owned(),
            raw_text: String::new(),
            usage: Box::new(usage.clone()),
        }
    })?;
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

pub(crate) fn extract_responses_text(response: &Value) -> Option<String> {
    if let Some(output_text) = response.get("output_text").and_then(Value::as_str) {
        return Some(output_text.to_owned());
    }
    let fragments = response
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|item| {
            item.get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(|content| {
            let kind = content.get("type").and_then(Value::as_str);
            if matches!(kind, Some("output_text") | Some("text")) {
                content.get("text").and_then(Value::as_str)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    (!fragments.is_empty()).then(|| fragments.join(""))
}

pub(crate) fn parse_openai_usage(usage: Option<&Value>) -> TokenUsage {
    let Some(usage) = usage else {
        return TokenUsage::default();
    };
    let input_tokens = number_at(usage, &["input_tokens", "prompt_tokens"]);
    let output_tokens = number_at(usage, &["output_tokens", "completion_tokens"]);
    let cache_read_tokens = usage
        .pointer("/input_tokens_details/cached_tokens")
        .or_else(|| usage.pointer("/prompt_tokens_details/cached_tokens"))
        // Kimi/Moonshot report cache hits directly on the usage object.
        .or_else(|| usage.get("cached_tokens"))
        .and_then(Value::as_u64);
    let cache_write_tokens = usage
        .pointer("/input_tokens_details/cache_write_tokens")
        .or_else(|| usage.pointer("/prompt_tokens_details/cache_write_tokens"))
        .or_else(|| usage.get("cache_write_tokens"))
        .and_then(Value::as_u64);
    let reasoning_tokens = usage
        .pointer("/output_tokens_details/reasoning_tokens")
        .or_else(|| usage.pointer("/completion_tokens_details/reasoning_tokens"))
        .and_then(Value::as_u64)
        .map(|reasoning| {
            output_tokens
                .map(|output| reasoning.min(output))
                .unwrap_or(reasoning)
        });
    let total_tokens = usage
        .get("total_tokens")
        .and_then(Value::as_u64)
        .or_else(|| match (input_tokens, output_tokens) {
            (Some(input), Some(output)) => Some(input.saturating_add(output)),
            _ => None,
        });
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

pub(crate) fn parse_openai_models(value: &Value) -> Result<Vec<ModelInfo>, ProviderError> {
    let models = value.get("data").and_then(Value::as_array).ok_or_else(|| {
        ProviderError::InvalidResponse("model list omitted the `data` array".to_owned())
    })?;
    Ok(models
        .iter()
        .filter_map(|model| {
            Some(ModelInfo {
                id: model.get("id")?.as_str()?.to_owned(),
                display_name: model
                    .get("display_name")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                owned_by: model
                    .get("owned_by")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                capabilities: None,
                max_input_tokens: None,
                max_output_tokens: None,
            })
        })
        .collect())
}

fn number_at(value: &Value, names: &[&str]) -> Option<u64> {
    names
        .iter()
        .find_map(|name| value.get(*name).and_then(Value::as_u64))
}

fn normalize_base_url(base_url: String) -> String {
    base_url.trim().trim_end_matches('/').to_owned()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::providers::{Attachment, ReasoningEffort, ReasoningOptions};

    fn request() -> AnalysisRequest {
        AnalysisRequest {
            system_prompt: "Classify the document.".to_owned(),
            prompt: "OCR text".to_owned(),
            schema_name: "document_analysis".to_owned(),
            schema: json!({
                "type": "object",
                "properties": { "title": { "type": "string" } },
                "required": ["title"],
                "additionalProperties": false
            }),
            attachments: vec![],
            max_output_tokens: 8000,
            temperature: None,
            reasoning: Some(ReasoningOptions {
                effort: Some(ReasoningEffort::High),
                budget_tokens: None,
            }),
        }
    }

    #[test]
    fn reasoning_request_omits_temperature_by_default() {
        let body = build_responses_body(
            "gpt-reasoning",
            &request(),
            &ProviderCapabilities::openai_responses(),
        );
        assert!(body.get("temperature").is_none());
        assert_eq!(body.pointer("/reasoning/effort"), Some(&json!("high")));
        assert_eq!(
            body.pointer("/text/format/type"),
            Some(&json!("json_schema"))
        );
        assert_eq!(body.pointer("/text/format/strict"), Some(&json!(true)));
    }

    #[test]
    fn capability_gates_optional_fields_and_uses_prompt_fallback() {
        let mut request = request();
        request.temperature = Some(0.3);
        let capabilities = ProviderCapabilities {
            temperature: false,
            reasoning_effort: false,
            strict_json_schema: false,
            ..ProviderCapabilities::openai_responses()
        };
        let body = build_responses_body("model", &request, &capabilities);
        assert!(body.get("temperature").is_none());
        assert!(body.get("reasoning").is_none());
        assert!(body.get("text").is_none());
        assert!(
            body["instructions"]
                .as_str()
                .expect("instructions")
                .contains("Return exactly one JSON value")
        );
    }

    #[test]
    fn supported_temperature_is_still_omitted_when_unset() {
        let capabilities = ProviderCapabilities {
            temperature: true,
            ..ProviderCapabilities::openai_responses()
        };
        let body = build_responses_body("model", &request(), &capabilities);
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn serializes_image_and_pdf_as_data_urls() {
        let mut request = request();
        request.attachments = vec![
            Attachment {
                file_name: "page.png".to_owned(),
                media_type: "image/png".to_owned(),
                bytes: vec![1, 2, 3],
            },
            Attachment {
                file_name: "invoice.pdf".to_owned(),
                media_type: "application/pdf".to_owned(),
                bytes: vec![4, 5, 6],
            },
        ];
        let body = build_responses_body("gpt", &request, &ProviderCapabilities::openai_responses());
        assert!(
            body.pointer("/input/0/content/1/image_url")
                .and_then(Value::as_str)
                .is_some_and(|url| url.starts_with("data:image/png;base64,"))
        );
        assert_eq!(
            body.pointer("/input/0/content/2/filename"),
            Some(&json!("invoice.pdf"))
        );
    }

    #[test]
    fn normalizes_reported_usage_without_double_counting_reasoning() {
        let usage = json!({
            "input_tokens": 100,
            "input_tokens_details": { "cached_tokens": 40, "cache_write_tokens": 10 },
            "output_tokens": 30,
            "output_tokens_details": { "reasoning_tokens": 20 }
        });
        let normalized = parse_openai_usage(Some(&usage));
        assert_eq!(normalized.input_tokens, Some(100));
        assert_eq!(normalized.cache_read_tokens, Some(40));
        assert_eq!(normalized.cache_write_tokens, Some(10));
        assert_eq!(normalized.output_tokens, Some(30));
        assert_eq!(normalized.reasoning_tokens, Some(20));
        assert_eq!(normalized.total_tokens, Some(130));
    }

    #[test]
    fn normalizes_kimi_top_level_cached_tokens() {
        let usage = json!({
            "prompt_tokens": 100,
            "completion_tokens": 25,
            "total_tokens": 125,
            "cached_tokens": 60
        });
        let normalized = parse_openai_usage(Some(&usage));
        assert_eq!(normalized.input_tokens, Some(100));
        assert_eq!(normalized.cache_read_tokens, Some(60));
        assert_eq!(normalized.output_tokens, Some(25));
        assert_eq!(normalized.total_tokens, Some(125));
    }

    #[test]
    fn extracts_nested_responses_output() {
        let response = json!({
            "output": [{
                "type": "message",
                "content": [{ "type": "output_text", "text": "{\"title\":\"A\"}" }]
            }]
        });
        assert_eq!(
            extract_responses_text(&response).as_deref(),
            Some("{\"title\":\"A\"}")
        );
    }

    #[test]
    fn invalid_structured_output_preserves_reported_usage_for_repair() {
        let response = json!({
            "id": "resp_bad",
            "output_text": "not valid JSON",
            "usage": { "input_tokens": 21, "output_tokens": 4, "total_tokens": 25 }
        });
        let error = parse_responses_result(response, None, &request().schema)
            .err()
            .expect("invalid structured output");
        match error {
            ProviderError::InvalidStructuredOutput {
                raw_text, usage, ..
            } => {
                assert_eq!(raw_text, "not valid JSON");
                assert_eq!(usage.input_tokens, Some(21));
                assert_eq!(usage.output_tokens, Some(4));
                assert_eq!(usage.total_tokens, Some(25));
            }
            other => panic!("unexpected error: {other}"),
        }
    }
}
