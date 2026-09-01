//! Filesystem reads: `read_file`, `list_dir`, `stat_file`. Ref-aware.

use crate::error::{ErrorCode, ToolError};
use crate::git::read::{list_dir_at_ref, list_dir_working_tree, read_blob_at_ref};
use crate::languages::{language_of_file, looks_binary};
use crate::tools::context::ToolContext;
use crate::tools::registry::{Tool, ToolFuture};
use crate::util::{expand_path, safe_join};
use serde_json::{json, Value};
use std::sync::Arc;

pub struct ReadFile;

impl Tool for ReadFile {
    fn name(&self) -> &'static str {
        "read_file"
    }
    fn description(&self) -> &'static str {
        "Read a file from the working tree or from a specific git ref. Each content line is prefixed with its 1-based line number ('   38 | ...') so line references are exact."
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
            let start_line = input.get("start_line").and_then(|v| v.as_u64());
            let end_line = input.get("end_line").and_then(|v| v.as_u64());

            // safe_join also validates working-tree-relative; ref-mode validates by tree lookup.
            let bytes = if let Some(r) = &r {
                read_blob_at_ref(&repo_path, r, &file)?
            } else {
                let p = safe_join(&repo_path, &file)?;
                std::fs::read(&p)
                    .map_err(|e| ToolError::new(ErrorCode::NotInWorkspace, format!("read: {e}")))?
            };
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let total_lines = text.lines().count() as u64;
            let start = start_line.unwrap_or(1).max(1) as usize;
            let end = end_line.unwrap_or(total_lines).max(start_line.unwrap_or(1)) as usize;
            // Numbered so the model never has to count lines in a blob — a
            // reviewer's "line 38" and the model's must be the same line.
            let content = text
                .lines()
                .enumerate()
                .skip(start - 1)
                .take(end.saturating_sub(start - 1))
                .map(|(i, line)| format!("{:>5} | {}", i + 1, line))
                .collect::<Vec<_>>()
                .join("\n");

            Ok(json!({
                "ok": true,
                "content": content,
                "total_lines": total_lines,
            }))
        })
    }
}

pub struct ListDir;

impl Tool for ListDir {
    fn name(&self) -> &'static str {
        "list_dir"
    }
    fn description(&self) -> &'static str {
        "List entries of a directory at the working tree or a specific ref."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["repo_path", "dir"],
            "properties": {
                "repo_path": { "type": "string" },
                "dir": { "type": "string" },
                "ref": { "type": "string" }
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
            let dir = input
                .get("dir")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::invalid("dir required"))?
                .to_string();
            let r = input
                .get("ref")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let entries = if let Some(r) = &r {
                list_dir_at_ref(&repo_path, r, &dir)?
            } else {
                let p = if dir == "." || dir.is_empty() {
                    repo_path.clone()
                } else {
                    safe_join(&repo_path, &dir)?
                };
                list_dir_working_tree(&p)?
            };
            let payload: Vec<Value> = entries
                .into_iter()
                .map(|e| {
                    let mut o = serde_json::Map::new();
                    o.insert("name".to_string(), Value::String(e.name));
                    o.insert("kind".to_string(), Value::String(e.kind.to_string()));
                    if let Some(s) = e.size {
                        o.insert("size".to_string(), Value::Number(s.into()));
                    }
                    Value::Object(o)
                })
                .collect();
            Ok(json!({ "ok": true, "entries": payload }))
        })
    }
}

pub struct StatFile;

impl Tool for StatFile {
    fn name(&self) -> &'static str {
        "stat_file"
    }
    fn description(&self) -> &'static str {
        "Cheap metadata about a file."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["repo_path", "file"],
            "properties": {
                "repo_path": { "type": "string" },
                "file": { "type": "string" },
                "ref": { "type": "string" }
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

            let (bytes, size) = if let Some(r) = &r {
                let b = read_blob_at_ref(&repo_path, r, &file)?;
                let s = b.len() as u64;
                (b, s)
            } else {
                let p = safe_join(&repo_path, &file)?;
                let meta = std::fs::metadata(&p)?;
                let b = std::fs::read(&p).unwrap_or_default();
                (b, meta.len())
            };
            let language = language_of_file(&file);
            let is_binary = looks_binary(&bytes);
            Ok(json!({
                "ok": true,
                "size": size,
                "language": language,
                "is_binary": is_binary,
            }))
        })
    }
}
