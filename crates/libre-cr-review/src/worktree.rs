//! Session-level worktree orchestration.
//!
//! When a session is created we kick off a background task that:
//!   1. Calls the code daemon's `discover_repo(remote_url)` for the session's
//!      scraped `pr_data.remote_url`.
//!   2. On hit, calls `prepare_worktree(repo_id, pr_ref)` and writes the
//!      resulting path back onto the session row.
//!   3. On miss, leaves the worktree empty and records a `pending_action:
//!      "clone_required"` so the extension can prompt the user (Phase 5).
//!
//! Readiness is exposed via [`SessionStatusBoard`] so `GET /v1/sessions/:id`
//! returns the latest state. Background work is idempotent on the session's
//! stored worktree state — a second POST for the same `pr_url` won't re-run
//! preparation while one is already in flight.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::error::Result;
use crate::storage::Store;
use crate::tools::code_daemon::CodeDaemonClient;

/// What state the session's worktree is in, per spec.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeState {
    /// Async prep hasn't been observed yet.
    Pending,
    /// Successfully prepared. `path` lives on the session row.
    Ready,
    /// Discovery missed; extension should prompt to clone.
    CloneRequired { remote_url: String },
    /// Preparation failed terminally.
    Failed { message: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionStatus {
    pub state: WorktreeState,
    pub worktree_path: Option<String>,
    pub repo_id: Option<String>,
    pub pending_action: Option<&'static str>,
    pub error: Option<String>,
}

impl SessionStatus {
    pub fn pending() -> Self {
        Self {
            state: WorktreeState::Pending,
            worktree_path: None,
            repo_id: None,
            pending_action: None,
            error: None,
        }
    }
    pub fn ready(path: String, repo_id: Option<String>) -> Self {
        Self {
            state: WorktreeState::Ready,
            worktree_path: Some(path),
            repo_id,
            pending_action: None,
            error: None,
        }
    }
    pub fn clone_required(remote_url: String) -> Self {
        Self {
            state: WorktreeState::CloneRequired {
                remote_url: remote_url.clone(),
            },
            worktree_path: None,
            repo_id: None,
            pending_action: Some("clone_required"),
            error: None,
        }
    }
    pub fn failed(message: String) -> Self {
        Self {
            state: WorktreeState::Failed {
                message: message.clone(),
            },
            worktree_path: None,
            repo_id: None,
            pending_action: None,
            error: Some(message),
        }
    }
}

/// In-memory readiness state, indexed by `session_id`.
#[derive(Clone, Default)]
pub struct SessionStatusBoard {
    inner: Arc<Mutex<HashMap<String, SessionStatus>>>,
}

impl SessionStatusBoard {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn set(&self, session_id: &str, status: SessionStatus) {
        self.inner
            .lock()
            .await
            .insert(session_id.to_string(), status);
    }

    pub async fn get(&self, session_id: &str) -> Option<SessionStatus> {
        self.inner.lock().await.get(session_id).cloned()
    }

    pub async fn contains(&self, session_id: &str) -> bool {
        self.inner.lock().await.contains_key(session_id)
    }
}

/// Inputs for one orchestration run.
pub struct PrepareInputs {
    pub session_id: String,
    pub remote_url: Option<String>,
    pub pr_ref: String,
}

/// Synchronous worker for one session. Performs the discover → prepare flow
/// against the code daemon, persisting outcomes on `store` and `board`.
pub async fn prepare_session(
    store: Store,
    code: Arc<dyn CodeDaemonClient>,
    board: SessionStatusBoard,
    input: PrepareInputs,
) -> Result<SessionStatus> {
    let Some(remote_url) = input.remote_url else {
        let status = SessionStatus::failed("session has no remote_url in pr_data".into());
        board.set(&input.session_id, status.clone()).await;
        return Ok(status);
    };

    // discover_repo
    let discover = match code
        .call(
            "discover_repo",
            serde_json::json!({ "remote_url": remote_url }),
        )
        .await
    {
        Ok(v) => v,
        Err(e) => {
            let status = SessionStatus::failed(format!("discover_repo failed: {e}"));
            board.set(&input.session_id, status.clone()).await;
            return Ok(status);
        }
    };
    let found = discover
        .get("ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !found {
        let status = SessionStatus::clone_required(remote_url);
        board.set(&input.session_id, status.clone()).await;
        return Ok(status);
    }
    let repo_id = discover
        .get("repo_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // prepare_worktree
    let prep_input = serde_json::json!({
        "repo_id": repo_id,
        "ref": input.pr_ref,
    });
    let prep = match code.call("prepare_worktree", prep_input).await {
        Ok(v) => v,
        Err(e) => {
            let status = SessionStatus::failed(format!("prepare_worktree failed: {e}"));
            board.set(&input.session_id, status.clone()).await;
            return Ok(status);
        }
    };
    let path = prep
        .get("worktree_path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let Some(path) = path else {
        let msg = prep
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("prepare_worktree did not return a path")
            .to_string();
        let status = SessionStatus::failed(msg);
        board.set(&input.session_id, status.clone()).await;
        return Ok(status);
    };

    let _ = store
        .set_worktree(&input.session_id, repo_id.as_deref(), Some(&path))
        .await;
    let status = SessionStatus::ready(path, repo_id);
    board.set(&input.session_id, status.clone()).await;
    Ok(status)
}

/// Spawn a detached task. Caller doesn't await — they poll via `GET /v1/sessions/:id`.
pub fn spawn_prepare(
    store: Store,
    code: Arc<dyn CodeDaemonClient>,
    board: SessionStatusBoard,
    input: PrepareInputs,
) {
    tokio::spawn(async move {
        let _ = prepare_session(store, code, board, input).await;
    });
}

/// Extract `(remote_url, pr_ref)` from a session's scraped `pr_data`.
///
/// `pr_data.remote_url` is what the GitHub adapter writes per spec; we also
/// look at common alternates. The PR ref defaults to GitHub's `pull/<n>/head`
/// pseudo-ref when we have a PR number.
pub fn pr_inputs_from_pr_data(
    pr_data: &serde_json::Value,
    pr_number: i64,
) -> (Option<String>, String) {
    let remote = pr_data
        .get("remote_url")
        .and_then(|v| v.as_str())
        .or_else(|| {
            pr_data
                .get("repository_remote_url")
                .and_then(|v| v.as_str())
        })
        .or_else(|| {
            pr_data
                .get("metadata")
                .and_then(|m| m.get("remote_url"))
                .and_then(|v| v.as_str())
        })
        .map(|s| s.to_string());
    let pr_ref = pr_data
        .get("head_ref")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("pull/{pr_number}/head"));
    (remote, pr_ref)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Result as CrResult;
    use crate::tools::code_daemon::CodeDaemonClient;
    use async_trait::async_trait;
    use std::sync::Mutex as StdMutex;

    struct ScriptedClient {
        responses: StdMutex<Vec<serde_json::Value>>,
    }
    impl ScriptedClient {
        fn new(rs: Vec<serde_json::Value>) -> Self {
            Self {
                responses: StdMutex::new(rs),
            }
        }
    }
    #[async_trait]
    impl CodeDaemonClient for ScriptedClient {
        async fn list_tools(&self) -> CrResult<Vec<crate::provider::ToolSchema>> {
            Ok(vec![])
        }
        async fn call(
            &self,
            _name: &str,
            _input: serde_json::Value,
        ) -> CrResult<serde_json::Value> {
            let mut g = self.responses.lock().unwrap();
            if g.is_empty() {
                Ok(serde_json::json!({"ok": true}))
            } else {
                Ok(g.remove(0))
            }
        }
    }

    #[tokio::test]
    async fn happy_path_marks_ready_and_persists_path() {
        let store = Store::open_in_memory().unwrap();
        let sess = store
            .upsert_session("https://github.com/a/b/pull/1", serde_json::json!({}))
            .await
            .unwrap();
        let code: Arc<dyn CodeDaemonClient> = Arc::new(ScriptedClient::new(vec![
            serde_json::json!({"ok": true, "repo_id": "github.com/a/b", "repo_path": "/repo"}),
            serde_json::json!({"ok": true, "worktree_path": "/wt"}),
        ]));
        let board = SessionStatusBoard::new();
        let status = prepare_session(
            store.clone(),
            code,
            board.clone(),
            PrepareInputs {
                session_id: sess.session_id.clone(),
                remote_url: Some("https://github.com/a/b".into()),
                pr_ref: "pull/1/head".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(status.state, WorktreeState::Ready);
        assert_eq!(status.worktree_path.as_deref(), Some("/wt"));
        let s2 = store.get_session(&sess.session_id).await.unwrap().unwrap();
        assert_eq!(s2.worktree_path.as_deref(), Some("/wt"));
        assert_eq!(s2.repo_id.as_deref(), Some("github.com/a/b"));
    }

    #[tokio::test]
    async fn discover_miss_returns_clone_required() {
        let store = Store::open_in_memory().unwrap();
        let sess = store
            .upsert_session("https://github.com/a/b/pull/2", serde_json::json!({}))
            .await
            .unwrap();
        let code: Arc<dyn CodeDaemonClient> = Arc::new(ScriptedClient::new(vec![
            serde_json::json!({"ok": false, "error": "unknown_repo"}),
        ]));
        let board = SessionStatusBoard::new();
        let status = prepare_session(
            store,
            code,
            board.clone(),
            PrepareInputs {
                session_id: sess.session_id,
                remote_url: Some("https://github.com/a/b".into()),
                pr_ref: "pull/2/head".into(),
            },
        )
        .await
        .unwrap();
        assert!(matches!(status.state, WorktreeState::CloneRequired { .. }));
        assert_eq!(status.pending_action, Some("clone_required"));
    }

    #[tokio::test]
    async fn missing_remote_url_is_failure() {
        let store = Store::open_in_memory().unwrap();
        let sess = store
            .upsert_session("https://github.com/a/b/pull/3", serde_json::json!({}))
            .await
            .unwrap();
        let code: Arc<dyn CodeDaemonClient> = Arc::new(ScriptedClient::new(vec![]));
        let board = SessionStatusBoard::new();
        let status = prepare_session(
            store,
            code,
            board,
            PrepareInputs {
                session_id: sess.session_id,
                remote_url: None,
                pr_ref: "pull/3/head".into(),
            },
        )
        .await
        .unwrap();
        assert!(matches!(status.state, WorktreeState::Failed { .. }));
    }

    #[test]
    fn pr_inputs_extracts_remote_url_and_default_ref() {
        let pr_data = serde_json::json!({"remote_url": "https://github.com/x/y"});
        let (u, r) = pr_inputs_from_pr_data(&pr_data, 42);
        assert_eq!(u.as_deref(), Some("https://github.com/x/y"));
        assert_eq!(r, "pull/42/head");
    }

    #[test]
    fn pr_inputs_honors_head_ref_override() {
        let pr_data = serde_json::json!({"remote_url": "x", "head_ref": "refs/heads/feat/foo"});
        let (_u, r) = pr_inputs_from_pr_data(&pr_data, 5);
        assert_eq!(r, "refs/heads/feat/foo");
    }

    #[test]
    fn pr_inputs_falls_back_to_metadata_remote_url() {
        let pr_data = serde_json::json!({"metadata": {"remote_url": "git@github.com:x/y.git"}});
        let (u, _r) = pr_inputs_from_pr_data(&pr_data, 1);
        assert_eq!(u.as_deref(), Some("git@github.com:x/y.git"));
    }
}
