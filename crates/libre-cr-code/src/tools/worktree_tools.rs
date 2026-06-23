//! Worktree tools.

use crate::error::ToolError;
use crate::tools::context::ToolContext;
use crate::tools::registry::{Tool, ToolFuture};
use crate::util::expand_path;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct PrepareWorktree;

impl Tool for PrepareWorktree {
    fn name(&self) -> &'static str {
        "prepare_worktree"
    }
    fn description(&self) -> &'static str {
        "Fetch a ref and materialize it as a worktree under the managed cache. \
         An existing worktree is re-fetched and reset if the ref moved; pass \
         expected_sha to skip the fetch when the worktree is already on it."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["repo_id", "ref"],
            "properties": {
                "repo_id": { "type": "string" },
                "ref": { "type": "string" },
                "name": { "type": "string" },
                "expected_sha": { "type": "string" }
            }
        })
    }
    fn call<'a>(&'a self, ctx: Arc<ToolContext>, input: Value) -> ToolFuture<'a> {
        Box::pin(async move {
            let repo_id = input
                .get("repo_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::invalid("repo_id required"))?
                .to_string();
            let r = input
                .get("ref")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::invalid("ref required"))?
                .to_string();
            let name = input
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let expected_sha = input
                .get("expected_sha")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let wt = ctx.worktrees.clone();
            let path = wt
                .prepare(&repo_id, &r, name.as_deref(), expected_sha.as_deref())
                .await?;
            Ok(json!({
                "ok": true,
                "worktree_path": path.to_string_lossy(),
            }))
        })
    }
}

pub struct ListWorktrees;

impl Tool for ListWorktrees {
    fn name(&self) -> &'static str {
        "list_worktrees"
    }
    fn description(&self) -> &'static str {
        "List worktrees (optionally filtered to a single repo)."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "repo_id": { "type": "string" } }
        })
    }
    fn call<'a>(&'a self, ctx: Arc<ToolContext>, input: Value) -> ToolFuture<'a> {
        Box::pin(async move {
            let repo_id = input
                .get("repo_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let list = ctx.registry.list_worktrees(repo_id.as_deref())?;
            let payload: Vec<Value> = list
                .into_iter()
                .map(|w| {
                    json!({
                        "repo_id": w.repo_id,
                        "ref": w.ref_name,
                        "path": w.worktree_path.to_string_lossy(),
                        "last_used_at": w.last_used_at,
                        "created_at": w.created_at,
                    })
                })
                .collect();
            Ok(json!({ "ok": true, "worktrees": payload }))
        })
    }
}

pub struct RemoveWorktree;

impl Tool for RemoveWorktree {
    fn name(&self) -> &'static str {
        "remove_worktree"
    }
    fn description(&self) -> &'static str {
        "Remove a worktree (force)."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["worktree_path"],
            "properties": { "worktree_path": { "type": "string" } }
        })
    }
    fn call<'a>(&'a self, ctx: Arc<ToolContext>, input: Value) -> ToolFuture<'a> {
        Box::pin(async move {
            let p = input
                .get("worktree_path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::invalid("worktree_path required"))?;
            let path = expand_path(p);
            ctx.worktrees.remove(&path).await?;
            Ok(json!({ "ok": true }))
        })
    }
}
