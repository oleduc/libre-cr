//! Row-level types for the storage layer.

use serde::{Deserialize, Serialize};

use libre_cr_common::Selection;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub session_id: String,
    pub pr_url: String,
    pub pr_owner: String,
    pub pr_repo: String,
    pub pr_number: i64,
    pub repo_id: Option<String>,
    pub worktree_path: Option<String>,
    pub pr_data: serde_json::Value,
    pub created_at: i64,
    pub last_active_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_sha: Option<String>,
}

/// Wire conversion for the typed HTTP responses in `libre-cr-common`.
/// Field-for-field — the session row *is* the wire shape.
impl From<Session> for libre_cr_common::http_api::SessionSummary {
    fn from(s: Session) -> Self {
        Self {
            session_id: s.session_id,
            pr_url: s.pr_url,
            pr_owner: s.pr_owner,
            pr_repo: s.pr_repo,
            pr_number: s.pr_number,
            repo_id: s.repo_id,
            worktree_path: s.worktree_path,
            pr_data: s.pr_data,
            created_at: s.created_at,
            last_active_at: s.last_active_at,
            head_sha: s.head_sha,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnKind {
    Question,
    Note,
}

impl TurnKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TurnKind::Question => "question",
            TurnKind::Note => "note",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "question" => Some(Self::Question),
            "note" => Some(Self::Note),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Ok,
    Cancelled,
    Error,
}

impl TurnStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TurnStatus::Ok => "ok",
            TurnStatus::Cancelled => "cancelled",
            TurnStatus::Error => "error",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ok" => Some(Self::Ok),
            "cancelled" => Some(Self::Cancelled),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
}

/// Severity for notes, agent-flagged issues, and exports. Sort order matters
/// — `Critical` is highest.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    // Lowest precedence first so derive(Ord) reflects "severer is greater".
    Info,
    Suggestion,
    Warning,
    Critical,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Suggestion => "suggestion",
            Severity::Warning => "warning",
            Severity::Critical => "critical",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "info" => Some(Self::Info),
            "suggestion" => Some(Self::Suggestion),
            "warning" => Some(Self::Warning),
            "critical" => Some(Self::Critical),
            _ => None,
        }
    }
    /// Heading used in Markdown export. Plural forms read naturally.
    pub fn group_heading(&self) -> &'static str {
        match self {
            Severity::Critical => "Critical",
            Severity::Warning => "Warning",
            Severity::Suggestion => "Suggestions",
            Severity::Info => "Info",
        }
    }
    /// Order in which export sections are emitted: Critical → Warning →
    /// Suggestion → Info.
    pub fn export_order() -> [Severity; 4] {
        [
            Severity::Critical,
            Severity::Warning,
            Severity::Suggestion,
            Severity::Info,
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    pub turn_id: String,
    pub session_id: String,
    pub ordinal: i64,
    pub kind: TurnKind,
    pub status: TurnStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verb: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<Selection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<Severity>,
    #[serde(default)]
    pub usage_in: i64,
    #[serde(default)]
    pub usage_out: i64,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_turn_id: Option<String>,
}

/// Alias for code clarity at use-sites: a note is a `Turn` with `kind = Note`.
pub type Note = Turn;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolTrace {
    pub trace_id: String,
    pub turn_id: String,
    pub ordinal: i64,
    pub tool_name: String,
    pub input_json: serde_json::Value,
    pub output_json: serde_json::Value,
    pub duration_ms: i64,
    pub ok: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_ord() {
        assert!(Severity::Critical > Severity::Warning);
        assert!(Severity::Warning > Severity::Suggestion);
        assert!(Severity::Suggestion > Severity::Info);
    }

    #[test]
    fn severity_export_order() {
        let order = Severity::export_order();
        assert_eq!(order[0], Severity::Critical);
        assert_eq!(order[3], Severity::Info);
    }

    #[test]
    fn severity_parse() {
        assert_eq!(Severity::parse("critical"), Some(Severity::Critical));
        assert_eq!(Severity::parse("WARNING"), Some(Severity::Warning));
        assert_eq!(Severity::parse("nope"), None);
    }
}
