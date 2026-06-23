//! `grep` tool.

use crate::error::ToolError;
use crate::search::grep::{search, GrepOptions};
use crate::tools::context::ToolContext;
use crate::tools::registry::{Tool, ToolFuture};
use crate::util::expand_path;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct Grep;

impl Tool for Grep {
    fn name(&self) -> &'static str {
        "grep"
    }
    fn description(&self) -> &'static str {
        "ripgrep-backed text search. Defaults: regex, max 200 matches."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["repo_path", "pattern"],
            "properties": {
                "repo_path": { "type": "string" },
                "pattern": { "type": "string" },
                "ref": { "type": "string" },
                "paths": { "type": "array" },
                "glob": { "type": "string" },
                "fixed_string": { "type": "boolean" },
                "max_matches": { "type": "integer" }
            }
        })
    }
    fn call<'a>(&'a self, ctx: Arc<ToolContext>, input: Value) -> ToolFuture<'a> {
        Box::pin(async move {
            let repo_path = expand_path(
                input
                    .get("repo_path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::invalid("repo_path required"))?,
            );
            let pattern = input
                .get("pattern")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::invalid("pattern required"))?
                .to_string();
            let paths: Option<Vec<String>> =
                input.get("paths").and_then(|v| v.as_array()).map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                });
            let glob = input
                .get("glob")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let fixed_string = input
                .get("fixed_string")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let max_matches = input
                .get("max_matches")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(ctx.config.grep.default_max_matches);

            let (matches, truncated) = tokio::task::spawn_blocking(move || {
                let opts = GrepOptions {
                    pattern: &pattern,
                    paths: paths.as_deref(),
                    glob: glob.as_deref(),
                    fixed_string,
                    max_matches,
                };
                search(&repo_path, &opts)
            })
            .await
            .map_err(|e| ToolError::internal(format!("join: {e}")))??;

            let payload: Vec<Value> = matches
                .iter()
                .map(|m| {
                    json!({
                        "file": m.file,
                        "line": m.line,
                        "column": m.column,
                        "content": m.content,
                    })
                })
                .collect();

            Ok(json!({
                "ok": true,
                "matches": payload,
                "truncated": truncated,
            }))
        })
    }
}
