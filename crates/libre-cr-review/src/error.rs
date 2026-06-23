//! Error types for the review daemon, mappable to HTTP responses + WS frames.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use libre_cr_common::error::{ErrorCategory, ErrorEnvelope};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unauthorized")]
    Unauthorized,
    #[error("origin rejected")]
    OriginRejected,
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("not found")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("provider unauthorized")]
    ProviderUnauthorized,
    #[error("provider rate limited")]
    ProviderRateLimited,
    #[error("provider timeout")]
    ProviderTimeout,
    #[error("too many tool turns")]
    TooManyToolTurns,
    #[error("code daemon unavailable")]
    CodeDaemonUnavailable,
    #[error("worktree pending")]
    WorktreePending,
    #[error("internal: {0}")]
    Internal(String),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Error {
    pub fn category(&self) -> ErrorCategory {
        match self {
            Error::Unauthorized => ErrorCategory::Unauthorized,
            Error::OriginRejected => ErrorCategory::OriginRejected,
            Error::Validation(_) => ErrorCategory::ValidationFailed,
            Error::NotFound | Error::Conflict(_) => ErrorCategory::ValidationFailed,
            Error::ProviderUnauthorized => ErrorCategory::ProviderUnauthorized,
            Error::ProviderRateLimited => ErrorCategory::ProviderRateLimited,
            Error::ProviderTimeout => ErrorCategory::ProviderTimeout,
            Error::TooManyToolTurns => ErrorCategory::Internal,
            Error::CodeDaemonUnavailable => ErrorCategory::CodeDaemonUnavailable,
            Error::WorktreePending => ErrorCategory::WorktreePending,
            _ => ErrorCategory::Internal,
        }
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Error::Unauthorized => StatusCode::UNAUTHORIZED,
            Error::OriginRejected => StatusCode::FORBIDDEN,
            Error::Validation(_) => StatusCode::BAD_REQUEST,
            Error::NotFound => StatusCode::NOT_FOUND,
            Error::Conflict(_) => StatusCode::CONFLICT,
            Error::ProviderUnauthorized => StatusCode::BAD_GATEWAY,
            Error::ProviderRateLimited => StatusCode::TOO_MANY_REQUESTS,
            Error::ProviderTimeout => StatusCode::GATEWAY_TIMEOUT,
            Error::CodeDaemonUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            Error::WorktreePending => StatusCode::ACCEPTED,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let envelope = ErrorEnvelope::new(self.category(), self.to_string());
        let status = self.status();
        (status, Json(json!(envelope))).into_response()
    }
}

pub type Result<T> = std::result::Result<T, Error>;
