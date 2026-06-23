//! Git tools: log/blame/show/diff.

use crate::error::ToolError;
use crate::git::{blame, diff, log, show};
use crate::tools::context::ToolContext;
use crate::tools::registry::{Tool, ToolFuture};
use crate::util::expand_path;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct GitLog;

impl Tool for GitLog {
    fn name(&self) -> &'static str {
        "git_log"
    }
    fn description(&self) -> &'static str {
        "List recent commits, optionally restricted to a single file or ref."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["repo_path"],
            "properties": {
                "repo_path": { "type": "string" },
                "file": { "type": "string" },
                "ref": { "type": "string" },
                "max_count": { "type": "integer" }
            }
        })
    }
    fn call<'a>(&'a self, _ctx: Arc<ToolContext>, input: Value) -> ToolFuture<'a> {
        Box::pin(async move {
            let repo_path = expand_path(
                input
                    .get("repo_path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::invalid("repo_path required"))?,
            );
            let file = input
                .get("file")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let r = input
                .get("ref")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let max_count = input
                .get("max_count")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(50);

            let commits = tokio::task::spawn_blocking(move || {
                log::git_log(&repo_path, r.as_deref(), file.as_deref(), max_count)
            })
            .await
            .map_err(|e| ToolError::internal(format!("join: {e}")))??;

            let payload: Vec<Value> = commits
                .into_iter()
                .map(|c| {
                    json!({
                        "sha": c.sha,
                        "author": c.author,
                        "email": c.email,
                        "date": c.date,
                        "summary": c.summary,
                    })
                })
                .collect();
            Ok(json!({ "ok": true, "commits": payload }))
        })
    }
}

pub struct GitBlame;

impl Tool for GitBlame {
    fn name(&self) -> &'static str {
        "git_blame"
    }
    fn description(&self) -> &'static str {
        "Blame a file (optionally restricted to a line range)."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["repo_path", "file"],
            "properties": {
                "repo_path": { "type": "string" },
                "file": { "type": "string" },
                "ref": { "type": "string" },
                "start_line": { "type": "integer" },
                "end_line": { "type": "integer" }
            }
        })
    }
    fn call<'a>(&'a self, _ctx: Arc<ToolContext>, input: Value) -> ToolFuture<'a> {
        Box::pin(async move {
            let repo_path = expand_path(
                input
                    .get("repo_path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::invalid("repo_path required"))?,
            );
            let file = input
                .get("file")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::invalid("file required"))?
                .to_string();
            let r = input
                .get("ref")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let start = input
                .get("start_line")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32);
            let end = input
                .get("end_line")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32);

            let lines = blame::git_blame(&repo_path, &file, r.as_deref(), start, end).await?;
            let payload: Vec<Value> = lines
                .into_iter()
                .map(|b| {
                    json!({
                        "line": b.line,
                        "sha": b.sha,
                        "author": b.author,
                        "date": b.date,
                        "summary": b.summary,
                    })
                })
                .collect();
            Ok(json!({ "ok": true, "lines": payload }))
        })
    }
}

pub struct GitShow;

impl Tool for GitShow {
    fn name(&self) -> &'static str {
        "git_show"
    }
    fn description(&self) -> &'static str {
        "Show a single commit. Optionally restrict to one file."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["repo_path", "sha"],
            "properties": {
                "repo_path": { "type": "string" },
                "sha": { "type": "string" },
                "file": { "type": "string" }
            }
        })
    }
    fn call<'a>(&'a self, _ctx: Arc<ToolContext>, input: Value) -> ToolFuture<'a> {
        Box::pin(async move {
            let repo_path = expand_path(
                input
                    .get("repo_path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::invalid("repo_path required"))?,
            );
            let sha = input
                .get("sha")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::invalid("sha required"))?
                .to_string();
            let file = input
                .get("file")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let r = show::git_show(&repo_path, &sha, file.as_deref()).await?;
            Ok(json!({
                "ok": true,
                "message": r.message,
                "author": r.author,
                "date": r.date,
                "diff": r.diff,
            }))
        })
    }
}

pub struct GitDiff;

impl Tool for GitDiff {
    fn name(&self) -> &'static str {
        "git_diff"
    }
    fn description(&self) -> &'static str {
        "Structured diff between two refs."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["repo_path", "from_ref", "to_ref"],
            "properties": {
                "repo_path": { "type": "string" },
                "from_ref": { "type": "string" },
                "to_ref": { "type": "string" },
                "paths": { "type": "array" }
            }
        })
    }
    fn call<'a>(&'a self, _ctx: Arc<ToolContext>, input: Value) -> ToolFuture<'a> {
        Box::pin(async move {
            let repo_path = expand_path(
                input
                    .get("repo_path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::invalid("repo_path required"))?,
            );
            let from = input
                .get("from_ref")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::invalid("from_ref required"))?
                .to_string();
            let to = input
                .get("to_ref")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::invalid("to_ref required"))?
                .to_string();
            let paths: Option<Vec<String>> =
                input.get("paths").and_then(|v| v.as_array()).map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                });
            let files = diff::git_diff(&repo_path, &from, &to, paths.as_deref()).await?;
            let payload: Vec<Value> = files
                .into_iter()
                .map(|f| {
                    json!({
                        "path": f.path,
                        "status": f.status,
                        "hunks": f.hunks.iter().map(|h| json!({
                            "header": h.header,
                            "lines": h.lines,
                        })).collect::<Vec<_>>(),
                    })
                })
                .collect();
            Ok(json!({ "ok": true, "files": payload }))
        })
    }
}
