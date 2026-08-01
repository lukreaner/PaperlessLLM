use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use thiserror::Error;
use tracing::error;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("authentication required")]
    Unauthorized,
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("invalid request: {0}")]
    Validation(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("budget limit reached: {0}")]
    Budget(String),
    #[error("upstream service error: {0}")]
    Upstream(String),
    #[error("database error")]
    Database(#[from] sqlx::Error),
    #[error("database migration error")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("I/O error")]
    Io(#[from] std::io::Error),
    #[error("serialization error")]
    Json(#[from] serde_json::Error),
    #[error("URL error")]
    Url(#[from] url::ParseError),
    #[error("internal error: {0}")]
    Internal(String),
}

#[derive(Serialize)]
struct ErrorBody {
    error: &'static str,
    message: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::NotFound(message) => (StatusCode::NOT_FOUND, "not_found", message),
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Authentication required".to_owned(),
            ),
            Self::Forbidden(message) => (StatusCode::FORBIDDEN, "forbidden", message),
            Self::Validation(message) => (StatusCode::BAD_REQUEST, "invalid_request", message),
            Self::Conflict(message) => (StatusCode::CONFLICT, "conflict", message),
            Self::Budget(message) => (StatusCode::TOO_MANY_REQUESTS, "budget_exhausted", message),
            Self::Upstream(message) => (
                StatusCode::BAD_GATEWAY,
                "upstream_error",
                safe_upstream_message(&message),
            ),
            error @ (Self::Database(_)
            | Self::Migration(_)
            | Self::Io(_)
            | Self::Json(_)
            | Self::Url(_)
            | Self::Internal(_)) => {
                error!(error = %crate::diagnostics::safe_error_summary(&error), "request failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "The request could not be completed".to_owned(),
                )
            }
        };

        (
            status,
            Json(ErrorBody {
                error: code,
                message,
            }),
        )
            .into_response()
    }
}

impl From<anyhow::Error> for AppError {
    fn from(value: anyhow::Error) -> Self {
        Self::Internal(value.to_string())
    }
}

/// Convert an upstream-facing description into an allow-listed public value.
///
/// Upstream response bodies and model-generated strings must never cross this
/// boundary. Numeric HTTP/API versions are parsed before interpolation; every
/// other returned value is a fixed application string.
pub(crate) fn safe_upstream_message(message: &str) -> String {
    const SAFE_MESSAGES: &[&str] = &[
        "Paperless did not report an API version",
        "Paperless-ngx v3 is required; server did not report a valid version",
        "Paperless-ngx v3 is required; connected server is not v3",
        "Paperless authentication failed",
        "Paperless API permissions are insufficient",
        "Paperless API v10 is not supported",
        "Paperless rate limit reached",
        "Paperless request timed out",
        "Could not connect to Paperless",
        "Paperless request failed",
        "Paperless pagination repeated a page",
        "Paperless returned an invalid pagination URL",
        "Paperless returned an unsafe pagination URL",
        "Paperless returned a pagination URL without a page query",
        "Paperless pagination did not advance",
        "Paperless pagination attempted to leave the configured origin",
        "provider returned invalid analysis fields",
        "structured output repair failed",
    ];
    if SAFE_MESSAGES.contains(&message) {
        return message.to_owned();
    }

    if let Some(version) = message
        .strip_prefix("Paperless API v10 is required; server selected v")
        .and_then(|value| value.parse::<u32>().ok())
    {
        return format!("Paperless API v10 is required; server selected v{version}");
    }
    if message.starts_with("Paperless returned HTTP ")
        && let Some(status) = validated_http_status_after(message, "Paperless returned HTTP ")
    {
        return format!("Paperless returned HTTP {status}");
    }
    // Accept the former internal format only to sanitize errors already held
    // in memory while upgrading. Nothing following the status is retained.
    if message.starts_with("Paperless returned ")
        && let Some(status) = validated_http_status_after(message, "Paperless returned ")
    {
        return format!("Paperless returned HTTP {status}");
    }
    if message.starts_with("provider ") {
        if let Some(status) = validated_http_status_after(message, "HTTP ") {
            return format!("Provider request failed with HTTP {status}");
        }
        if message.contains(" timed out") {
            return "Provider request timed out".to_owned();
        }
        if message.contains("invalid structured output") {
            return "Provider returned invalid structured output".to_owned();
        }
        if message.contains("invalid response") {
            return "Provider returned an invalid response".to_owned();
        }
        return "Provider request failed".to_owned();
    }
    "Upstream service request failed".to_owned()
}

fn validated_http_status_after(message: &str, marker: &str) -> Option<u16> {
    let tail = message.split_once(marker)?.1;
    let digits = tail.get(..3)?;
    if !digits.bytes().all(|byte| byte.is_ascii_digit())
        || tail
            .as_bytes()
            .get(3)
            .is_some_and(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    digits
        .parse::<u16>()
        .ok()
        .filter(|status| (100..=599).contains(status))
}

#[cfg(test)]
mod tests {
    use super::safe_upstream_message;

    #[test]
    fn upstream_public_messages_drop_all_untrusted_suffixes() {
        assert_eq!(
            safe_upstream_message(
                "Paperless returned 500 Internal Server Error: OCR_PRIVATE sk-secret"
            ),
            "Paperless returned HTTP 500"
        );
        assert_eq!(
            safe_upstream_message(
                "provider analysis failed with HTTP 400: PROMPT_PRIVATE Bearer secret"
            ),
            "Provider request failed with HTTP 400"
        );
        assert_eq!(
            safe_upstream_message("unrecognized upstream OCR_PRIVATE sk-secret"),
            "Upstream service request failed"
        );
    }
}
