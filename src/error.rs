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
            Self::Upstream(message) => (StatusCode::BAD_GATEWAY, "upstream_error", message),
            error @ (Self::Database(_)
            | Self::Migration(_)
            | Self::Io(_)
            | Self::Json(_)
            | Self::Url(_)
            | Self::Internal(_)) => {
                error!(error = ?error, "request failed");
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
