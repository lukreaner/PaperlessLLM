mod anthropic;
mod compatible;
mod openai;
mod types;

use async_trait::async_trait;
use serde_json::Value;

pub use anthropic::AnthropicMessagesAdapter;
pub use compatible::{ChatOutputTokenField, CompatibleTransport, OpenAiCompatibleAdapter};
pub use openai::OpenAiResponsesAdapter;
pub use types::{
    AnalysisRequest, AnalysisResult, ApiFamily, Attachment, InputTokenCount, ModelInfo,
    ProbeResult, ProviderCapabilities, ProviderError, ProviderKind, ReasoningEffort,
    ReasoningOptions, TokenUsage, UsageSource,
};

#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn kind(&self) -> ProviderKind;
    fn api_family(&self) -> ApiFamily;
    fn selected_model(&self) -> &str;
    fn capabilities(&self) -> &ProviderCapabilities;

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError>;

    async fn probe(&self) -> Result<ProbeResult, ProviderError> {
        let models = self.list_models().await?;
        Ok(ProbeResult {
            provider: self.kind(),
            api_family: self.api_family(),
            selected_model: self.selected_model().to_owned(),
            selected_model_found: models.iter().any(|model| model.id == self.selected_model()),
            models,
            capabilities: self.capabilities().clone(),
        })
    }

    async fn count_input_tokens(
        &self,
        request: &AnalysisRequest,
    ) -> Result<InputTokenCount, ProviderError>;

    async fn analyze(&self, request: &AnalysisRequest) -> Result<AnalysisResult, ProviderError>;
}

pub(crate) struct HttpJson {
    value: Value,
    request_id: Option<String>,
}

pub(crate) async fn receive_json(
    response: reqwest::Response,
    operation: &'static str,
) -> Result<HttpJson, ProviderError> {
    let status = response.status();
    let request_id = response
        .headers()
        .get("x-request-id")
        .or_else(|| response.headers().get("request-id"))
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    if !status.is_success() {
        // Do not read a rejected response body. Providers may echo document
        // input, prompts, or credentials in diagnostic messages.
        return Err(ProviderError::remote(operation, status));
    }
    let body = response
        .text()
        .await
        .map_err(|error| ProviderError::transport(operation, error))?;
    let value = serde_json::from_str(&body).map_err(|_| {
        ProviderError::InvalidResponse(format!(
            "{operation} response was not valid JSON (HTTP {})",
            status.as_u16()
        ))
    })?;
    Ok(HttpJson { value, request_id })
}
