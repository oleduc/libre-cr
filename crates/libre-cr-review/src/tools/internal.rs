//! Internal review-daemon tools. PR-aware, session-aware.

use crate::error::{Error, Result};
use crate::provider::ToolSchema;
use crate::storage::{Severity, Store};

/// Names registered with the LLM.
pub const INTERNAL_TOOL_NAMES: &[&str] = &[
    "get_pr_diff",
    "get_pr_comments",
    "get_pr_metadata",
    "get_selection",
    "add_note",
    "session_history_search",
];

pub fn internal_tool_schemas() -> Vec<ToolSchema> {
    vec![
        ToolSchema {
            name: "get_pr_diff".into(),
            description: "The PR's changes (base branch → PR head) as structured per-file hunks, computed on the prepared checkout. Optional `paths` narrows it to specific files. On a large PR, calling this without `paths` returns a file *manifest* instead of content (`files_only: true`, each file with its size) — read it, then call again with the paths worth reading.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "paths": {"type": "array", "items": {"type": "string"},
                              "description": "Restrict to these file paths (as in the diff)."}
                }
            }),
        },
        ToolSchema {
            name: "get_pr_comments".into(),
            description: "Existing review comments on this PR: `{ comments: [{ author, body, anchor, file?, line?, start_line?, side?, resolved, thread_id, comment_id }], total, truncated }`. `anchor` says what the thread is attached to: `line` (file, line and side all present — read that location before judging the concern), `file` (a file-level comment: file only), or `none` (GitHub gives no anchor, typically a thread on a line that later commits rewrote; locate it from the body). `resolved: true` means the thread was already dealt with — do not re-raise it as a live concern. Top-level conversation comments are not included. `truncated: true` means some were dropped. An `unavailable` field means the comments could not be read at all — say so instead of reporting that the PR has none.".into(),
            input_schema: serde_json::json!({"type":"object","properties":{}}),
        },
        ToolSchema {
            name: "get_pr_metadata".into(),
            description: "Return PR metadata (title, branches, author, …).".into(),
            input_schema: serde_json::json!({"type":"object","properties":{}}),
        },
        ToolSchema {
            name: "get_selection".into(),
            description: "Return the reviewer's selection at question time, if any.".into(),
            input_schema: serde_json::json!({"type":"object","properties":{}}),
        },
        ToolSchema {
            name: "add_note".into(),
            description: "Save a note for the reviewer's final review draft.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "content": {"type": "string"},
                    "severity": {"type": "string",
                                 "enum": ["info","suggestion","warning","critical"]}
                },
                "required": ["content"]
            }),
        },
        ToolSchema {
            name: "session_history_search".into(),
            description: "Search past Q&A within this session.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {"query": {"type":"string"}},
                "required": ["query"]
            }),
        },
    ]
}

pub struct InternalContext {
    pub session_id: String,
    pub pr_data: serde_json::Value,
    pub selection: Option<libre_cr_common::Selection>,
    pub store: Store,
}

impl InternalContext {
    pub async fn call(&self, name: &str, input: serde_json::Value) -> Result<serde_json::Value> {
        match name {
            "get_pr_diff" => {
                let mut diff = self
                    .pr_data
                    .get("diff")
                    .cloned()
                    .unwrap_or(serde_json::json!({"files": []}));
                // The worktree-backed path filters by `paths`; the stored-diff
                // fallback must honor it too, or a narrowed request returns
                // every file.
                let want: Vec<String> = input
                    .get("paths")
                    .and_then(|p| p.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                if !want.is_empty() {
                    if let Some(files) = diff.get_mut("files").and_then(|f| f.as_array_mut()) {
                        files.retain(|f| {
                            f.get("path")
                                .or_else(|| f.get("file"))
                                .and_then(|v| v.as_str())
                                .map(|p| want.iter().any(|w| w == p))
                                .unwrap_or(false)
                        });
                    }
                }
                Ok(diff)
            }
            // No `comments` key means the extension could not read them from
            // the page — never report that as "this PR has no comments".
            "get_pr_comments" => {
                Ok(self
                    .pr_data
                    .get("comments")
                    .cloned()
                    .unwrap_or(serde_json::json!({
                        "comments": [],
                        "unavailable": true,
                        "note": "Review comments were not captured for this session; \
                                 treat their content as unknown, not absent."
                    })))
            }
            "get_pr_metadata" => Ok(self.pr_data.get("metadata").cloned().unwrap_or_else(|| {
                let mut m = serde_json::Map::new();
                for k in &[
                    "title",
                    "description",
                    "author",
                    "base_branch",
                    "head_branch",
                ] {
                    if let Some(v) = self.pr_data.get(*k) {
                        m.insert((*k).into(), v.clone());
                    }
                }
                serde_json::Value::Object(m)
            })),
            "get_selection" => Ok(match &self.selection {
                None => serde_json::Value::Null,
                Some(s) => serde_json::to_value(s)?,
            }),
            "add_note" => {
                let content = input
                    .get("content")
                    .and_then(|s| s.as_str())
                    .ok_or_else(|| Error::Validation("add_note: content required".into()))?;
                let severity = input
                    .get("severity")
                    .and_then(|s| s.as_str())
                    .and_then(Severity::parse)
                    .unwrap_or(Severity::Info);
                let id = self
                    .store
                    .create_note(&self.session_id, content, severity, None)
                    .await?;
                Ok(serde_json::json!({ "note_id": id }))
            }
            "session_history_search" => {
                let q = input
                    .get("query")
                    .and_then(|s| s.as_str())
                    .ok_or_else(|| Error::Validation("session_history_search: query".into()))?;
                let hits = self.store.search_turns(&self.session_id, q, 10).await?;
                let arr: Vec<_> = hits
                    .into_iter()
                    .map(|(turn_id, snippet)| {
                        serde_json::json!({ "turn_id": turn_id, "snippet": snippet })
                    })
                    .collect();
                Ok(serde_json::json!({ "matches": arr }))
            }
            _ => Err(Error::Validation(format!("unknown internal tool: {name}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn add_note_persists() {
        let store = Store::open_in_memory().unwrap();
        let sess = store
            .upsert_session("https://github.com/a/b/pull/1", serde_json::json!({}))
            .await
            .unwrap();
        let ctx = InternalContext {
            session_id: sess.session_id.clone(),
            pr_data: serde_json::json!({}),
            selection: None,
            store: store.clone(),
        };
        let v = ctx
            .call(
                "add_note",
                serde_json::json!({"content": "yo", "severity": "warning"}),
            )
            .await
            .unwrap();
        assert!(v["note_id"].is_string());
    }

    #[tokio::test]
    async fn get_selection_null_when_absent() {
        let store = Store::open_in_memory().unwrap();
        let sess = store
            .upsert_session("https://github.com/a/b/pull/2", serde_json::json!({}))
            .await
            .unwrap();
        let ctx = InternalContext {
            session_id: sess.session_id,
            pr_data: serde_json::json!({}),
            selection: None,
            store,
        };
        let v = ctx
            .call("get_selection", serde_json::json!({}))
            .await
            .unwrap();
        assert!(v.is_null());
    }

    /// An uncaptured comment list must read as "unknown", not "this PR has
    /// none" — the tool answered a bare empty list for its whole life.
    #[tokio::test]
    async fn get_pr_comments_flags_uncaptured_as_unavailable() {
        let store = Store::open_in_memory().unwrap();
        let sess = store
            .upsert_session("https://github.com/a/b/pull/3", serde_json::json!({}))
            .await
            .unwrap();
        let ctx = InternalContext {
            session_id: sess.session_id.clone(),
            pr_data: serde_json::json!({}),
            selection: None,
            store: store.clone(),
        };
        let v = ctx
            .call("get_pr_comments", serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(v["unavailable"], serde_json::json!(true));

        // A captured list passes through verbatim, with no flag.
        let ctx = InternalContext {
            session_id: sess.session_id,
            pr_data: serde_json::json!({
                "comments": {"comments": [{"author": "r", "body": "b", "file": "f.rs",
                                           "line": 3, "side": "right", "resolved": false}],
                             "total": 1, "truncated": false}
            }),
            selection: None,
            store,
        };
        let v = ctx
            .call("get_pr_comments", serde_json::json!({}))
            .await
            .unwrap();
        assert!(v.get("unavailable").is_none());
        assert_eq!(v["comments"][0]["line"], serde_json::json!(3));
    }
}
