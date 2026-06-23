//! Code-daemon error model. Per `03-code-daemon.md` § Error Model.

use serde::Serialize;

/// Code-daemon error codes. Serialized as `snake_case` strings.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    UnknownRepo,
    UnknownRef,
    WorktreeBusy,
    UnsupportedLanguage,
    NotInWorkspace,
    Internal,
    /// Matches `libre_cr_common::ErrorCategory::ValidationFailed` — the shared
    /// envelope vocabulary across daemons and the extension.
    ValidationFailed,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::UnknownRepo => "unknown_repo",
            ErrorCode::UnknownRef => "unknown_ref",
            ErrorCode::WorktreeBusy => "worktree_busy",
            ErrorCode::UnsupportedLanguage => "unsupported_language",
            ErrorCode::NotInWorkspace => "not_in_workspace",
            ErrorCode::Internal => "internal",
            ErrorCode::ValidationFailed => "validation_failed",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub struct ToolError {
    pub code: ErrorCode,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl ToolError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    #[allow(dead_code)]
    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::ValidationFailed, message)
    }

    /// Render as the on-the-wire envelope used by every tool.
    pub fn to_envelope(&self) -> serde_json::Value {
        let mut obj = serde_json::Map::new();
        obj.insert("ok".to_string(), serde_json::Value::Bool(false));
        obj.insert(
            "error".to_string(),
            serde_json::Value::String(self.code.as_str().to_string()),
        );
        obj.insert(
            "message".to_string(),
            serde_json::Value::String(self.message.clone()),
        );
        if let Some(details) = &self.details {
            obj.insert("details".to_string(), details.clone());
        }
        serde_json::Value::Object(obj)
    }
}

impl From<anyhow::Error> for ToolError {
    fn from(err: anyhow::Error) -> Self {
        ToolError::internal(err.to_string())
    }
}

impl From<std::io::Error> for ToolError {
    fn from(err: std::io::Error) -> Self {
        ToolError::internal(format!("io: {err}"))
    }
}

impl From<serde_json::Error> for ToolError {
    fn from(err: serde_json::Error) -> Self {
        ToolError::invalid(format!("json: {err}"))
    }
}

impl From<rusqlite::Error> for ToolError {
    fn from(err: rusqlite::Error) -> Self {
        ToolError::internal(format!("sqlite: {err}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_shape() {
        let err = ToolError::new(ErrorCode::UnknownRepo, "no such repo");
        let env = err.to_envelope();
        assert_eq!(env["ok"], false);
        assert_eq!(env["error"], "unknown_repo");
        assert_eq!(env["message"], "no such repo");
    }

    #[test]
    fn code_strings() {
        assert_eq!(ErrorCode::UnknownRef.as_str(), "unknown_ref");
        assert_eq!(
            ErrorCode::UnsupportedLanguage.as_str(),
            "unsupported_language"
        );
        assert_eq!(ErrorCode::WorktreeBusy.as_str(), "worktree_busy");
        assert_eq!(ErrorCode::ValidationFailed.as_str(), "validation_failed");
    }
}
