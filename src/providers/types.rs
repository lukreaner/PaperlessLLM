use std::error::Error;
use std::fmt::{Debug, Display, Formatter};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The wire protocol used for a provider request. This value is suitable for
/// persisting alongside usage statistics.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiFamily {
    #[serde(rename = "openai_responses", alias = "open_ai_responses")]
    OpenAiResponses,
    #[serde(rename = "openai_chat_completions", alias = "open_ai_chat_completions")]
    OpenAiChatCompletions,
    AnthropicMessages,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    #[serde(rename = "openai", alias = "open_ai")]
    OpenAi,
    Anthropic,
    LmStudio,
    Mistral,
    Kimi,
    Moonshot,
    Google,
    #[serde(
        rename = "custom_openai_compatible",
        alias = "custom_open_ai_compatible"
    )]
    CustomOpenAiCompatible,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    #[serde(rename = "xhigh")]
    XHigh,
    Max,
}

impl ReasoningEffort {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "minimal" => Some(Self::Minimal),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" => Some(Self::XHigh),
            "max" => Some(Self::Max),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReasoningOptions {
    /// OpenAI reasoning effort, and Anthropic adaptive-thinking effort on
    /// models that support it.
    pub effort: Option<ReasoningEffort>,
    /// Anthropic's explicit thinking budget. When this is set it takes
    /// precedence over adaptive thinking.
    pub budget_tokens: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderCapabilities {
    pub temperature: bool,
    pub reasoning_effort: bool,
    /// Whether the model accepts `thinking: {"type":"adaptive"}`. Claude
    /// effort support is separate and may exist without adaptive thinking.
    #[serde(default)]
    pub adaptive_thinking: bool,
    pub thinking_budget: bool,
    pub strict_json_schema: bool,
    pub image_input: bool,
    pub pdf_input: bool,
    pub native_token_count: bool,
    /// Exact effort levels advertised by a model-aware provider endpoint.
    /// `None` means the provider did not expose per-level metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_efforts: Option<Vec<ReasoningEffort>>,
}

impl ProviderCapabilities {
    pub fn openai_responses() -> Self {
        Self {
            // Reasoning models reject temperature. It is deliberately off in
            // the preset and can be enabled after a successful capability
            // probe for a particular model.
            temperature: false,
            reasoning_effort: true,
            adaptive_thinking: false,
            thinking_budget: false,
            strict_json_schema: true,
            image_input: true,
            pdf_input: true,
            native_token_count: true,
            reasoning_efforts: None,
        }
    }

    pub fn anthropic_messages() -> Self {
        Self {
            temperature: true,
            reasoning_effort: true,
            adaptive_thinking: true,
            thinking_budget: true,
            strict_json_schema: true,
            image_input: true,
            pdf_input: true,
            native_token_count: true,
            reasoning_efforts: Some(vec![
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
                ReasoningEffort::XHigh,
                ReasoningEffort::Max,
            ]),
        }
    }

    pub fn lm_studio(api_family: ApiFamily) -> Self {
        Self {
            temperature: true,
            reasoning_effort: matches!(api_family, ApiFamily::OpenAiResponses),
            adaptive_thinking: false,
            thinking_budget: false,
            // LM Studio/model combinations differ. Setup may replace these
            // conservative defaults after probing the selected model.
            strict_json_schema: false,
            image_input: true,
            pdf_input: false,
            native_token_count: false,
            reasoning_efforts: None,
        }
    }

    pub fn custom_openai_compatible() -> Self {
        Self {
            temperature: true,
            reasoning_effort: false,
            adaptive_thinking: false,
            thinking_budget: false,
            strict_json_schema: false,
            image_input: false,
            pdf_input: false,
            native_token_count: false,
            reasoning_efforts: None,
        }
    }

    pub fn hosted_openai_compatible() -> Self {
        Self {
            // Hosted presets may reject sampling parameters for reasoning
            // models. Start with every optional capability disabled and let
            // the setup-time probe enable only what the endpoint proves.
            temperature: false,
            reasoning_effort: false,
            adaptive_thinking: false,
            thinking_budget: false,
            strict_json_schema: false,
            image_input: false,
            pdf_input: false,
            native_token_count: false,
            reasoning_efforts: None,
        }
    }

    pub fn google_openai_compatible() -> Self {
        Self {
            reasoning_effort: true,
            reasoning_efforts: Some(vec![
                ReasoningEffort::None,
                ReasoningEffort::Minimal,
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
            ]),
            ..Self::hosted_openai_compatible()
        }
    }

    pub fn mistral_openai_compatible() -> Self {
        Self {
            reasoning_effort: true,
            reasoning_efforts: Some(vec![
                ReasoningEffort::None,
                ReasoningEffort::Minimal,
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
                ReasoningEffort::XHigh,
            ]),
            ..Self::hosted_openai_compatible()
        }
    }

    pub fn moonshot_openai_compatible() -> Self {
        Self {
            reasoning_effort: true,
            reasoning_efforts: Some(vec![
                ReasoningEffort::Low,
                ReasoningEffort::High,
                ReasoningEffort::Max,
            ]),
            ..Self::hosted_openai_compatible()
        }
    }

    pub fn supports_reasoning_effort(&self, effort: ReasoningEffort) -> bool {
        self.reasoning_effort
            && self
                .reasoning_efforts
                .as_ref()
                .is_none_or(|efforts| efforts.contains(&effort))
    }
}

#[derive(Clone)]
pub struct Attachment {
    pub file_name: String,
    pub media_type: String,
    pub bytes: Vec<u8>,
}

impl Attachment {
    pub fn is_pdf(&self) -> bool {
        self.media_type.eq_ignore_ascii_case("application/pdf")
    }

    pub fn is_image(&self) -> bool {
        self.media_type
            .get(..6)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("image/"))
    }
}

/// Provider-neutral analysis input. It intentionally does not implement
/// `Debug`, because prompts and attachments can contain document data.
#[derive(Clone)]
pub struct AnalysisRequest {
    pub system_prompt: String,
    pub prompt: String,
    pub schema_name: String,
    pub schema: Value,
    pub attachments: Vec<Attachment>,
    pub max_output_tokens: u32,
    pub temperature: Option<f64>,
    pub reasoning: Option<ReasoningOptions>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum UsageSource {
    Reported,
    PreflightCounted,
    Estimated,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// Reasoning/thinking tokens are a subset of `output_tokens` and must not
    /// be added to `total_tokens` a second time.
    pub reasoning_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub source: UsageSource,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InputTokenCount {
    pub input_tokens: u64,
    pub source: UsageSource,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: Option<String>,
    pub owned_by: Option<String>,
    /// Provider-reported capabilities for this exact model, when available.
    pub capabilities: Option<ProviderCapabilities>,
    pub max_input_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProbeResult {
    pub provider: ProviderKind,
    pub api_family: ApiFamily,
    pub selected_model: String,
    pub selected_model_found: bool,
    pub models: Vec<ModelInfo>,
    pub capabilities: ProviderCapabilities,
}

/// Analysis output is not `Debug` to reduce the risk of accidentally logging
/// document-derived content.
pub struct AnalysisResult {
    pub value: Value,
    pub raw_text: String,
    pub usage: TokenUsage,
    pub request_id: Option<String>,
}

pub enum ProviderError {
    Configuration(String),
    Transport {
        operation: &'static str,
        timed_out: bool,
    },
    Remote {
        operation: &'static str,
        status: u16,
    },
    InvalidResponse(String),
    /// The provider completed an inference request, but its output did not
    /// satisfy the requested schema. Keep usage and the raw output in memory
    /// so the worker can account accurately and make one informed repair call.
    /// The raw output is intentionally never included in Display.
    InvalidStructuredOutput {
        detail: String,
        raw_text: String,
        usage: Box<TokenUsage>,
    },
}

impl Debug for ProviderError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Configuration(_) => formatter.write_str("ProviderError::Configuration"),
            Self::Transport {
                operation,
                timed_out,
            } => formatter
                .debug_struct("ProviderError::Transport")
                .field("operation", operation)
                .field("timed_out", timed_out)
                .finish(),
            Self::Remote { operation, status } => formatter
                .debug_struct("ProviderError::Remote")
                .field("operation", operation)
                .field("status", status)
                .finish(),
            Self::InvalidResponse(_) => formatter.write_str("ProviderError::InvalidResponse"),
            Self::InvalidStructuredOutput { .. } => {
                formatter.write_str("ProviderError::InvalidStructuredOutput")
            }
        }
    }
}

impl ProviderError {
    pub(crate) fn transport(operation: &'static str, error: reqwest::Error) -> Self {
        Self::Transport {
            operation,
            timed_out: error.is_timeout(),
        }
    }

    pub(crate) fn remote(operation: &'static str, status: reqwest::StatusCode) -> Self {
        Self::Remote {
            operation,
            status: status.as_u16(),
        }
    }
}

impl Display for ProviderError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Configuration(detail) => write!(formatter, "provider configuration: {detail}"),
            Self::Transport {
                operation,
                timed_out,
            } => {
                if *timed_out {
                    write!(formatter, "provider {operation} timed out")
                } else {
                    write!(formatter, "provider {operation} request failed")
                }
            }
            Self::Remote { operation, status } => {
                write!(formatter, "provider {operation} failed with HTTP {status}")
            }
            Self::InvalidResponse(_) => {
                write!(formatter, "provider returned an invalid response")
            }
            Self::InvalidStructuredOutput { .. } => {
                write!(formatter, "provider returned invalid structured output")
            }
        }
    }
}

impl Error for ProviderError {}

pub(crate) fn prompt_with_json_fallback(request: &AnalysisRequest) -> String {
    let schema = serde_json::to_string(&request.schema).unwrap_or_else(|_| "{}".to_owned());
    format!(
        "{}\n\nReturn exactly one JSON value and no markdown. It must satisfy this JSON Schema:\n{}",
        request.system_prompt, schema
    )
}

pub(crate) fn estimated_input_tokens(request: &AnalysisRequest) -> InputTokenCount {
    let schema_len = serde_json::to_string(&request.schema)
        .map(|schema| schema.chars().count())
        .unwrap_or_default();
    let text_chars =
        request.system_prompt.chars().count() + request.prompt.chars().count() + schema_len;
    let text_tokens = text_chars.div_ceil(4) as u64;
    // Binary-input tokenization varies greatly by model. One token per three
    // bytes is intentionally conservative for budget preflight.
    let attachment_tokens = request
        .attachments
        .iter()
        .map(|attachment| attachment.bytes.len().div_ceil(3) as u64)
        .sum::<u64>();
    InputTokenCount {
        input_tokens: text_tokens.saturating_add(attachment_tokens),
        source: UsageSource::Estimated,
    }
}

pub(crate) fn backfill_estimated_usage(
    usage: &mut TokenUsage,
    request: &AnalysisRequest,
    raw_text: &str,
) {
    if usage.source != UsageSource::Unknown {
        return;
    }
    let input_tokens = estimated_input_tokens(request).input_tokens;
    let output_tokens = raw_text.chars().count().div_ceil(4) as u64;
    usage.input_tokens = Some(input_tokens);
    usage.output_tokens = Some(output_tokens);
    usage.total_tokens = Some(input_tokens.saturating_add(output_tokens));
    usage.source = UsageSource::Estimated;
}

pub(crate) fn parse_and_validate_json(
    raw_text: &str,
    schema: &Value,
) -> Result<Value, ProviderError> {
    let candidate = extract_json(raw_text).ok_or_else(|| {
        ProviderError::InvalidResponse("response did not contain a JSON value".to_owned())
    })?;
    validate_schema(&candidate, schema, "$")?;
    Ok(candidate)
}

fn extract_json(raw_text: &str) -> Option<Value> {
    let trimmed = raw_text.trim();
    if let Ok(value) = serde_json::from_str(trimmed) {
        return Some(value);
    }

    if let Some(fence_start) = trimmed.find("```") {
        let after_fence = &trimmed[fence_start + 3..];
        let after_language = after_fence
            .strip_prefix("json")
            .or_else(|| after_fence.strip_prefix("JSON"))
            .unwrap_or(after_fence)
            .trim_start_matches([' ', '\t', '\r', '\n']);
        if let Some(fence_end) = after_language.find("```")
            && let Ok(value) = serde_json::from_str(after_language[..fence_end].trim())
        {
            return Some(value);
        }
    }

    for (index, character) in trimmed.char_indices() {
        if character != '{' && character != '[' {
            continue;
        }
        let mut stream = serde_json::Deserializer::from_str(&trimmed[index..]).into_iter::<Value>();
        if let Some(Ok(value)) = stream.next() {
            return Some(value);
        }
    }
    None
}

fn validate_schema(value: &Value, schema: &Value, path: &str) -> Result<(), ProviderError> {
    if let Some(constant) = schema.get("const")
        && value != constant
    {
        return schema_error(path, "does not equal the schema constant");
    }
    if let Some(variants) = schema.get("enum").and_then(Value::as_array)
        && !variants.contains(value)
    {
        return schema_error(path, "is not one of the allowed values");
    }
    if let Some(alternatives) = schema.get("anyOf").and_then(Value::as_array)
        && !alternatives
            .iter()
            .any(|alternative| validate_schema(value, alternative, path).is_ok())
    {
        return schema_error(path, "does not match any allowed schema");
    }
    if let Some(alternatives) = schema.get("oneOf").and_then(Value::as_array) {
        let matches = alternatives
            .iter()
            .filter(|alternative| validate_schema(value, alternative, path).is_ok())
            .count();
        if matches != 1 {
            return schema_error(path, "does not match exactly one allowed schema");
        }
    }

    let type_matches = match schema.get("type") {
        None => true,
        Some(Value::String(expected)) => matches_type(value, expected),
        Some(Value::Array(expected)) => expected
            .iter()
            .filter_map(Value::as_str)
            .any(|expected| matches_type(value, expected)),
        _ => false,
    };
    if !type_matches {
        return schema_error(path, "has the wrong JSON type");
    }

    if let Some(object) = value.as_object() {
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for property in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(property) {
                    return schema_error(
                        path,
                        &format!("is missing required property `{property}`"),
                    );
                }
            }
        }
        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            for (name, property_schema) in properties {
                if let Some(property) = object.get(name) {
                    validate_schema(property, property_schema, &format!("{path}.{name}"))?;
                }
            }
            if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
                for name in object.keys() {
                    if !properties.contains_key(name) {
                        return schema_error(
                            path,
                            &format!("contains unexpected property `{name}`"),
                        );
                    }
                }
            }
        }
    }

    if let (Some(items), Some(array)) = (schema.get("items"), value.as_array()) {
        for (index, item) in array.iter().enumerate() {
            validate_schema(item, items, &format!("{path}[{index}]"))?;
        }
        if schema.get("uniqueItems") == Some(&Value::Bool(true)) {
            for index in 0..array.len() {
                if array[index + 1..].contains(&array[index]) {
                    return schema_error(path, "contains duplicate array values");
                }
            }
        }
    }
    if let Some(string) = value.as_str() {
        if let Some(max_length) = schema.get("maxLength").and_then(Value::as_u64)
            && string.chars().count() as u64 > max_length
        {
            return schema_error(path, "is longer than the allowed maximum");
        }
        match schema.get("format").and_then(Value::as_str) {
            Some("date") if chrono::NaiveDate::parse_from_str(string, "%Y-%m-%d").is_err() => {
                return schema_error(path, "is not a valid calendar date");
            }
            Some("uri") if url::Url::parse(string).is_err() => {
                return schema_error(path, "is not a valid absolute URI");
            }
            _ => {}
        }
    }
    Ok(())
}

fn matches_type(value: &Value, expected: &str) -> bool {
    match expected {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "string" => value.is_string(),
        _ => false,
    }
}

fn schema_error<T>(path: &str, detail: &str) -> Result<T, ProviderError> {
    Err(ProviderError::InvalidResponse(format!(
        "JSON at {path} {detail}"
    )))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parses_fenced_json_and_validates_required_fields() {
        let schema = json!({
            "type": "object",
            "properties": { "title": { "type": "string" } },
            "required": ["title"],
            "additionalProperties": false
        });
        let value = parse_and_validate_json("```json\n{\"title\":\"Invoice\"}\n```", &schema)
            .expect("valid JSON");
        assert_eq!(value, json!({ "title": "Invoice" }));
        assert!(parse_and_validate_json("{\"other\":true}", &schema).is_err());
    }

    #[test]
    fn fallback_validator_enforces_formats_lengths_and_unique_arrays() {
        let schema = json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "maxLength": 4 },
                "date": { "type": "string", "format": "date" },
                "url": { "type": "string", "format": "uri" },
                "tags": { "type": "array", "items": { "type": "string" }, "uniqueItems": true }
            },
            "required": ["title", "date", "url", "tags"],
            "additionalProperties": false
        });
        assert!(
            parse_and_validate_json(
                r#"{"title":"Paid","date":"2026-08-01","url":"https://example.com","tags":["a","b"]}"#,
                &schema
            )
            .is_ok()
        );
        assert!(
            parse_and_validate_json(
                r#"{"title":"Too long","date":"2026-02-31","url":"relative","tags":["a","a"]}"#,
                &schema
            )
            .is_err()
        );
    }

    #[test]
    fn remote_errors_never_retain_or_render_response_messages() {
        let error = ProviderError::remote("analysis", reqwest::StatusCode::BAD_REQUEST);
        let rendered = error.to_string();
        assert_eq!(rendered, "provider analysis failed with HTTP 400");
        assert!(!rendered.contains("error"));
    }

    #[test]
    fn estimated_count_includes_attachments() {
        let request = AnalysisRequest {
            system_prompt: "system".to_owned(),
            prompt: "document".to_owned(),
            schema_name: "result".to_owned(),
            schema: json!({ "type": "object" }),
            attachments: vec![Attachment {
                file_name: "page.png".to_owned(),
                media_type: "image/png".to_owned(),
                bytes: vec![0; 300],
            }],
            max_output_tokens: 100,
            temperature: None,
            reasoning: None,
        };
        let count = estimated_input_tokens(&request);
        assert!(count.input_tokens >= 100);
        assert_eq!(count.source, UsageSource::Estimated);
    }

    #[test]
    fn missing_provider_usage_is_backfilled_without_reasoning_double_count() {
        let request = AnalysisRequest {
            system_prompt: "system".to_owned(),
            prompt: "document".to_owned(),
            schema_name: "result".to_owned(),
            schema: json!({ "type": "object" }),
            attachments: vec![],
            max_output_tokens: 100,
            temperature: None,
            reasoning: None,
        };
        let mut usage = TokenUsage::default();
        backfill_estimated_usage(&mut usage, &request, "{\"ok\":true}");
        assert_eq!(usage.source, UsageSource::Estimated);
        assert!(usage.input_tokens.is_some());
        assert!(usage.output_tokens.is_some());
        assert_eq!(
            usage.total_tokens,
            Some(usage.input_tokens.unwrap() + usage.output_tokens.unwrap())
        );
        assert_eq!(usage.reasoning_tokens, None);
    }

    #[test]
    fn persisted_openai_names_do_not_gain_an_extra_separator() {
        assert_eq!(
            serde_json::to_value(ProviderKind::OpenAi).expect("serialize kind"),
            json!("openai")
        );
        assert_eq!(
            serde_json::to_value(ApiFamily::OpenAiResponses).expect("serialize family"),
            json!("openai_responses")
        );
    }
}
