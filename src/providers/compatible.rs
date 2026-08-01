use std::collections::BTreeMap;

use async_trait::async_trait;
use base64::Engine;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::openai::{
    build_responses_body, parse_openai_models, parse_openai_usage, parse_responses_result,
};
use super::types::{
    AnalysisRequest, AnalysisResult, ApiFamily, InputTokenCount, ModelInfo, ProviderCapabilities,
    ProviderError, ProviderKind, UsageSource, backfill_estimated_usage, estimated_input_tokens,
    parse_and_validate_json, prompt_with_json_fallback,
};
use super::{ProviderAdapter, receive_json};

const LM_STUDIO_BASE_URL: &str = "http://localhost:1234/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibleTransport {
    Responses,
    ChatCompletions,
}

impl CompatibleTransport {
    fn api_family(self) -> ApiFamily {
        match self {
            Self::Responses => ApiFamily::OpenAiResponses,
            Self::ChatCompletions => ApiFamily::OpenAiChatCompletions,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatOutputTokenField {
    MaxTokens,
    MaxCompletionTokens,
}

pub struct OpenAiCompatibleAdapter {
    client: Client,
    kind: ProviderKind,
    base_url: String,
    api_key: Option<String>,
    headers: BTreeMap<String, String>,
    model: String,
    transport: CompatibleTransport,
    output_token_field: ChatOutputTokenField,
    capabilities: ProviderCapabilities,
}

impl OpenAiCompatibleAdapter {
    pub fn lm_studio(
        client: Client,
        model: impl Into<String>,
        transport: CompatibleTransport,
    ) -> Self {
        Self::lm_studio_with_base_url(client, LM_STUDIO_BASE_URL, model, None, transport)
    }

    /// LM Studio preset with a configurable server URL and optional API token.
    /// This supports remote/headless LM Studio while retaining LM Studio
    /// capability defaults and statistics attribution.
    pub fn lm_studio_with_base_url(
        client: Client,
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: Option<String>,
        transport: CompatibleTransport,
    ) -> Self {
        let api_family = transport.api_family();
        Self {
            client,
            kind: ProviderKind::LmStudio,
            base_url: base_url.into().trim().trim_end_matches('/').to_owned(),
            api_key,
            headers: BTreeMap::new(),
            model: model.into(),
            transport,
            output_token_field: ChatOutputTokenField::MaxTokens,
            capabilities: ProviderCapabilities::lm_studio(api_family),
        }
    }

    pub fn custom(
        client: Client,
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: Option<String>,
        transport: CompatibleTransport,
    ) -> Self {
        Self {
            client,
            kind: ProviderKind::CustomOpenAiCompatible,
            base_url: base_url.into().trim().trim_end_matches('/').to_owned(),
            api_key,
            headers: BTreeMap::new(),
            model: model.into(),
            transport,
            output_token_field: ChatOutputTokenField::MaxTokens,
            capabilities: ProviderCapabilities::custom_openai_compatible(),
        }
    }

    pub fn with_capabilities(mut self, capabilities: ProviderCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }

    pub fn with_output_token_field(mut self, field: ChatOutputTokenField) -> Self {
        self.output_token_field = field;
        self
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let mut request = self.client.request(
            method,
            format!("{}/{}", self.base_url, path.trim_start_matches('/')),
        );
        if let Some(api_key) = self.api_key.as_deref().filter(|key| !key.is_empty()) {
            request = request.bearer_auth(api_key);
        }
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }
        request
    }
}

#[async_trait]
impl ProviderAdapter for OpenAiCompatibleAdapter {
    fn kind(&self) -> ProviderKind {
        self.kind
    }

    fn api_family(&self) -> ApiFamily {
        self.transport.api_family()
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
        if !self.capabilities.native_token_count || self.transport != CompatibleTransport::Responses
        {
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
        let (path, body) = match self.transport {
            CompatibleTransport::Responses => (
                "responses",
                build_responses_body(&self.model, request, &self.capabilities),
            ),
            CompatibleTransport::ChatCompletions => (
                "chat/completions",
                build_chat_body(
                    &self.model,
                    request,
                    &self.capabilities,
                    self.output_token_field,
                ),
            ),
        };
        let response = self
            .request(reqwest::Method::POST, path)
            .json(&body)
            .send()
            .await
            .map_err(|error| ProviderError::transport("analysis", error))?;
        let response = receive_json(response, "analysis").await?;
        let mut result = match self.transport {
            CompatibleTransport::Responses => {
                parse_responses_result(response.value, response.request_id, &request.schema)
            }
            CompatibleTransport::ChatCompletions => {
                parse_chat_result(response.value, response.request_id, &request.schema)
            }
        }?;
        backfill_estimated_usage(&mut result.usage, request, &result.raw_text);
        Ok(result)
    }
}

fn build_chat_body(
    model: &str,
    request: &AnalysisRequest,
    capabilities: &ProviderCapabilities,
    output_token_field: ChatOutputTokenField,
) -> Value {
    let system = if capabilities.strict_json_schema {
        request.system_prompt.clone()
    } else {
        prompt_with_json_fallback(request)
    };
    let mut user_content = vec![json!({ "type": "text", "text": request.prompt })];
    for attachment in &request.attachments {
        let encoded = base64::engine::general_purpose::STANDARD.encode(&attachment.bytes);
        if attachment.is_image() && capabilities.image_input {
            user_content.push(json!({
                "type": "image_url",
                "image_url": {
                    "url": format!("data:{};base64,{encoded}", attachment.media_type),
                }
            }));
        } else if attachment.is_pdf() && capabilities.pdf_input {
            user_content.push(json!({
                "type": "file",
                "file": {
                    "filename": attachment.file_name,
                    "file_data": format!("data:{};base64,{encoded}", attachment.media_type),
                }
            }));
        }
    }

    let mut body = Map::from_iter([
        ("model".to_owned(), Value::String(model.to_owned())),
        (
            "messages".to_owned(),
            json!([
                { "role": "system", "content": system },
                { "role": "user", "content": user_content }
            ]),
        ),
    ]);
    let output_field = match output_token_field {
        ChatOutputTokenField::MaxTokens => "max_tokens",
        ChatOutputTokenField::MaxCompletionTokens => "max_completion_tokens",
    };
    body.insert(
        output_field.to_owned(),
        Value::from(request.max_output_tokens),
    );
    if capabilities.strict_json_schema {
        body.insert(
            "response_format".to_owned(),
            json!({
                "type": "json_schema",
                "json_schema": {
                    "name": request.schema_name,
                    "strict": true,
                    "schema": request.schema,
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
        body.insert(
            "reasoning_effort".to_owned(),
            Value::String(effort.as_str().to_owned()),
        );
    }
    Value::Object(body)
}

fn parse_chat_result(
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
    let content = response
        .pointer("/choices/0/message/content")
        .ok_or_else(|| ProviderError::InvalidStructuredOutput {
            detail: "Chat Completions payload contained no message content".to_owned(),
            raw_text: String::new(),
            usage: Box::new(usage.clone()),
        })?;
    let raw_text = match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|block| {
                matches!(
                    block.get("type").and_then(Value::as_str),
                    Some("text") | Some("output_text")
                )
            })
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        Value::Object(_) => {
            serde_json::to_string(content).map_err(|_| ProviderError::InvalidStructuredOutput {
                detail: "Chat Completions message content could not be decoded".to_owned(),
                raw_text: String::new(),
                usage: Box::new(usage.clone()),
            })?
        }
        _ => {
            return Err(ProviderError::InvalidStructuredOutput {
                detail: "Chat Completions message content was not text".to_owned(),
                raw_text: String::new(),
                usage: Box::new(usage),
            });
        }
    };
    if raw_text.is_empty() {
        return Err(ProviderError::InvalidStructuredOutput {
            detail: "Chat Completions message content was empty".to_owned(),
            raw_text,
            usage: Box::new(usage),
        });
    }
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::providers::{Attachment, ReasoningEffort, ReasoningOptions};

    fn request() -> AnalysisRequest {
        AnalysisRequest {
            system_prompt: "Analyze".to_owned(),
            prompt: "OCR".to_owned(),
            schema_name: "analysis".to_owned(),
            schema: json!({
                "type": "object",
                "properties": { "title": { "type": "string" } },
                "required": ["title"],
                "additionalProperties": false
            }),
            attachments: vec![],
            max_output_tokens: 1000,
            temperature: None,
            reasoning: None,
        }
    }

    #[test]
    fn chat_fallback_contains_schema_but_no_response_format() {
        let body = build_chat_body(
            "local",
            &request(),
            &ProviderCapabilities::lm_studio(ApiFamily::OpenAiChatCompletions),
            ChatOutputTokenField::MaxTokens,
        );
        assert!(body.get("response_format").is_none());
        assert!(
            body["messages"][0]["content"]
                .as_str()
                .expect("system message")
                .contains("Return exactly one JSON value")
        );
        assert_eq!(body["max_tokens"], json!(1000));
    }

    #[test]
    fn chat_strict_schema_and_reasoning_are_capability_gated() {
        let mut request = request();
        request.reasoning = Some(ReasoningOptions {
            effort: Some(ReasoningEffort::Medium),
            budget_tokens: None,
        });
        let capabilities = ProviderCapabilities {
            strict_json_schema: true,
            reasoning_effort: true,
            ..ProviderCapabilities::custom_openai_compatible()
        };
        let body = build_chat_body(
            "custom",
            &request,
            &capabilities,
            ChatOutputTokenField::MaxCompletionTokens,
        );
        assert_eq!(
            body.pointer("/response_format/json_schema/strict"),
            Some(&json!(true))
        );
        assert_eq!(body["reasoning_effort"], json!("medium"));
        assert_eq!(body["max_completion_tokens"], json!(1000));
        assert!(body.get("max_tokens").is_none());
    }

    #[test]
    fn lm_studio_drops_pdf_but_sends_image() {
        let mut request = request();
        request.attachments = vec![
            Attachment {
                file_name: "page.png".to_owned(),
                media_type: "image/png".to_owned(),
                bytes: vec![1, 2, 3],
            },
            Attachment {
                file_name: "document.pdf".to_owned(),
                media_type: "application/pdf".to_owned(),
                bytes: vec![4, 5, 6],
            },
        ];
        let body = build_chat_body(
            "local",
            &request,
            &ProviderCapabilities::lm_studio(ApiFamily::OpenAiChatCompletions),
            ChatOutputTokenField::MaxTokens,
        );
        let blocks = body
            .pointer("/messages/1/content")
            .and_then(Value::as_array)
            .expect("content blocks");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[1]["type"], json!("image_url"));
    }

    #[test]
    fn parses_chat_output_and_normalizes_usage() {
        let response = json!({
            "id": "chat-1",
            "choices": [{ "message": { "content": "{\"title\":\"Invoice\"}" } }],
            "usage": {
                "prompt_tokens": 50,
                "completion_tokens": 12,
                "completion_tokens_details": { "reasoning_tokens": 8 }
            }
        });
        let result = parse_chat_result(response, None, &request().schema).expect("valid result");
        assert_eq!(result.value, json!({ "title": "Invoice" }));
        assert_eq!(result.usage.total_tokens, Some(62));
        assert_eq!(result.usage.reasoning_tokens, Some(8));
    }

    #[test]
    fn remote_lm_studio_constructor_preserves_preset_identity() {
        let adapter = OpenAiCompatibleAdapter::lm_studio_with_base_url(
            Client::new(),
            "https://lmstudio.example.test/v1/",
            "local-model",
            Some("token".to_owned()),
            CompatibleTransport::Responses,
        );
        assert_eq!(adapter.kind(), ProviderKind::LmStudio);
        assert_eq!(adapter.api_family(), ApiFamily::OpenAiResponses);
        assert_eq!(adapter.base_url, "https://lmstudio.example.test/v1");
    }
}
