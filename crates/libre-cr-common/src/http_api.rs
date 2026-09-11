//! Typed response bodies for the review daemon's HTTP API.
//!
//! These structs are the wire contract mirrored by the extension's
//! `extension/utils/daemon/frames.ts`. Field names are load-bearing: the
//! Rust side is the source of truth and the TS side simply describes what
//! comes over the wire, so renames here are breaking changes.

use serde::{Deserialize, Serialize};

/// `code_daemon` sub-object inside [`HealthResponse`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeDaemonHealth {
    pub connected: bool,
    /// `None` until the code daemon has reported a version.
    pub version: Option<String>,
}

/// `GET /v1/health`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub ok: bool,
    pub version: String,
    /// Wire-protocol version — see [`crate::PROTOCOL_VERSION`].
    pub protocol_version: u32,
    pub code_daemon: CodeDaemonHealth,
}

/// `GET /v1/health/code-daemon`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeDaemonHealthResponse {
    pub connected: bool,
    pub version: Option<String>,
    pub last_error: Option<String>,
    pub restart_count: u32,
}

/// `POST /v1/sessions`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionResponse {
    pub session_id: String,
    pub worktree_ready: bool,
    pub repo_local_path: Option<String>,
    pub pending_action: Option<String>,
    pub pr_diff_changed: bool,
    pub head_sha: Option<String>,
}

/// One session row as serialized on the wire (`GET /v1/sessions`,
/// `GET /v1/sessions/:id`). Mirrors the review daemon's stored session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
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

/// `GET /v1/sessions`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListSessionsResponse {
    pub sessions: Vec<SessionSummary>,
}

/// `GET /v1/sessions/:id`. `turns` and `status` carry review-daemon-internal
/// shapes (turn rows, worktree status) that don't cross any other crate
/// boundary; they stay schemaless here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDetailResponse {
    pub session: SessionSummary,
    pub turns: Vec<serde_json::Value>,
    pub worktree_ready: bool,
    pub status: Option<serde_json::Value>,
    pub head_sha: Option<String>,
    pub last_seen_at: i64,
}

/// `POST /v1/pair/issue`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairIssueResponse {
    pub code: String,
    pub expires_at_epoch_ms: i64,
}

/// `POST /v1/pair`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairRedeemResponse {
    pub token: String,
    pub extension_origin: String,
}

/// One hit in `GET /v1/search`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub session_id: String,
    pub pr_url: String,
    pub turn_id: String,
    pub snippet: String,
    pub score: f64,
}

/// `GET /v1/search`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub results: Vec<SearchHit>,
}

/// Inline comment inside [`GithubReviewStructure`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubInlineComment {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    pub body: String,
}

/// Structured GitHub review payload inside [`ExportResponse`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubReviewStructure {
    pub body: String,
    pub event: String,
    pub comments: Vec<GithubInlineComment>,
}

/// `POST /v1/sessions/:id/export`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportResponse {
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structure: Option<GithubReviewStructure>,
}

/// A single model offered by a provider. Shared between the review daemon's
/// provider layer and the HTTP wire contract (`POST /v1/provider/models`) so
/// the two never drift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// The model's context window, when the provider reports one. Absent is
    /// "not stated", never "small" — the config UI offers to size the caps
    /// from it only when it is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u64>,
}

/// What a provider actually reads from its config. The config UI disables the
/// fields a provider ignores rather than letting a user tune a value that goes
/// nowhere — `temperature` on a backend that rejects sampling parameters, an
/// API key on one that signs in.
///
/// Declared by each provider (`Provider::capabilities`), never inferred from
/// the kind string by the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub api_key: bool,
    pub endpoint: bool,
    pub temperature: bool,
    pub max_tokens: bool,
    /// Whether the model field selects anything.
    pub model: bool,
    /// Whether `list_models` returns something.
    pub model_list: bool,
}

impl Default for ProviderCapabilities {
    /// Everything supported: a new provider opts *out* of what it ignores, so
    /// forgetting to declare leaves fields editable rather than silently
    /// greying them out.
    fn default() -> Self {
        Self {
            api_key: true,
            endpoint: true,
            temperature: true,
            max_tokens: true,
            model: true,
            model_list: true,
        }
    }
}

/// `GET /v1/limits/derive?context_tokens=N` — the character caps that suit a
/// context window of that size. Returned for the config UI to *fill in*; the
/// daemon stores nothing until the form is saved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DerivedLimits {
    pub context_tokens: u64,
    pub chars_per_token: f32,
    pub max_tool_result_chars: usize,
    pub max_turn_tool_chars: usize,
    pub replay_result_chars: usize,
    pub replay_turn_chars: usize,
}

/// `POST /v1/provider/models`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsResponse {
    pub models: Vec<ModelInfo>,
}

/// `GET /v1/provider/detected`. Reports whether ambient API-key environment
/// variables are present in the daemon's environment so the config UI can
/// offer a one-click "use the detected key" option.
///
/// Env vars only (`ANTHROPIC_API_KEY` / `OPENAI_API_KEY`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DetectedCredentials {
    pub anthropic: bool,
    pub openai: bool,
}

/// `POST /v1/provider/chatgpt/login` — the URL the user opens to sign in.
/// The daemon is already listening for the callback when this returns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatGptLoginResponse {
    pub authorize_url: String,
}

/// `GET /v1/provider/chatgpt/status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatGptStatus {
    pub signed_in: bool,
    /// The ChatGPT account the stored tokens belong to, when signed in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    /// True while a sign-in is waiting on the browser callback.
    pub pending: bool,
}

/// One verb in `GET /v1/verbs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerbDescriptor {
    pub id: String,
    pub label: String,
    pub required_selection: String,
    pub description: String,
    pub suggested_tools: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_response_wire_shape() {
        let h = HealthResponse {
            ok: true,
            version: "0.1.0".into(),
            protocol_version: crate::PROTOCOL_VERSION,
            code_daemon: CodeDaemonHealth {
                connected: true,
                version: Some("mock".into()),
            },
        };
        let v = serde_json::to_value(&h).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["protocol_version"], crate::PROTOCOL_VERSION);
        assert_eq!(v["code_daemon"]["connected"], true);
        assert_eq!(v["code_daemon"]["version"], "mock");
    }

    #[test]
    fn create_session_response_emits_nulls_not_missing_keys() {
        let r = CreateSessionResponse {
            session_id: "s_1".into(),
            worktree_ready: false,
            repo_local_path: None,
            pending_action: Some("worktree_pending".into()),
            pr_diff_changed: false,
            head_sha: None,
        };
        let v = serde_json::to_value(&r).unwrap();
        // The pre-typed json! literal emitted explicit nulls; keep that.
        assert!(v.get("repo_local_path").is_some_and(|x| x.is_null()));
        assert!(v.get("head_sha").is_some_and(|x| x.is_null()));
        assert_eq!(v["pending_action"], "worktree_pending");
    }

    #[test]
    fn model_info_omits_absent_display_name() {
        let m = ModelInfo {
            id: "gpt-4o".into(),
            display_name: None,
            context_tokens: None,
        };
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["id"], "gpt-4o");
        assert!(v.get("display_name").is_none());
        // A provider that cannot state a window omits the field; the config UI
        // reads absent as "unknown" and leaves cap sizing unavailable.
        assert!(v.get("context_tokens").is_none());
    }

    #[test]
    fn detected_credentials_wire_shape() {
        let d = DetectedCredentials {
            anthropic: true,
            openai: false,
        };
        let v = serde_json::to_value(d).unwrap();
        assert_eq!(v["anthropic"], true);
        assert_eq!(v["openai"], false);
    }

    #[test]
    fn search_response_round_trip() {
        let r = SearchResponse {
            results: vec![SearchHit {
                session_id: "s".into(),
                pr_url: "u".into(),
                turn_id: "t".into(),
                snippet: "[hit]".into(),
                score: 1.5,
            }],
        };
        let s = serde_json::to_string(&r).unwrap();
        let back: SearchResponse = serde_json::from_str(&s).unwrap();
        assert_eq!(back.results.len(), 1);
        assert_eq!(back.results[0].snippet, "[hit]");
    }
}
