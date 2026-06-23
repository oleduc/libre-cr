use serde::{Deserialize, Serialize};

/// Machine-readable error categories surfaced over the HTTP / WS API
/// (per `04-review-daemon.md` § Error Handling) and inside MCP tool results
/// (per `03-code-daemon.md` § Error Model).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    // Transport / auth — extension ↔ review daemon
    Unauthorized,
    OriginRejected,
    ValidationFailed,

    // Code-daemon path
    CodeDaemonUnavailable,
    UnknownRepo,
    UnknownRef,
    WorktreeBusy,
    WorktreePending,
    WorktreeFailed,
    NotInWorkspace,
    UnsupportedLanguage,

    // Provider path
    ProviderUnauthorized,
    ProviderRateLimited,
    ProviderTimeout,

    // Catch-all
    Internal,
}

/// Wire envelope for an error returned by either daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorEnvelope {
    pub error: ErrorCategory,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recoverable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl ErrorEnvelope {
    pub fn new(error: ErrorCategory, message: impl Into<String>) -> Self {
        Self {
            error,
            message: message.into(),
            recoverable: None,
            details: None,
        }
    }
}
