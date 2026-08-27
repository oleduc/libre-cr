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
            description: "The PR's changes (base branch → PR head) as structured per-file hunks, computed on the prepared checkout. Optional `paths` narrows it to specific files — do that for large PRs.".into(),
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
            description: "Return PR conversation comments.".into(),
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
            "get_pr_diff" => Ok(self
                .pr_data
                .get("diff")
                .cloned()
                .unwrap_or(serde_json::json!({"files": []}))),
            "get_pr_comments" => Ok(self
                .pr_data
                .get("comments")
                .cloned()
                .unwrap_or(serde_json::json!({"comments": []}))),
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
}
