//! `detect_languages`.

use crate::error::ToolError;
use crate::languages::language_of_file;
use crate::tools::context::ToolContext;
use crate::tools::registry::{Tool, ToolFuture};
use crate::util::expand_path;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

pub struct DetectLanguages;

impl Tool for DetectLanguages {
    fn name(&self) -> &'static str {
        "detect_languages"
    }
    fn description(&self) -> &'static str {
        "Aggregate language stats across the working tree."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["repo_path"],
            "properties": { "repo_path": { "type": "string" } }
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

            let counts = tokio::task::spawn_blocking(move || {
                let mut counts: HashMap<&'static str, (u64, u64)> = HashMap::new();
                let walker = ignore::WalkBuilder::new(&repo_path)
                    .git_ignore(true)
                    .hidden(true)
                    .build();
                for entry in walker.flatten() {
                    let path = entry.path();
                    if !path.is_file() {
                        continue;
                    }
                    let name = path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or_default();
                    let lang = language_of_file(name);
                    if lang == "Unknown" {
                        continue;
                    }
                    let entry = counts.entry(lang).or_insert((0, 0));
                    entry.0 += 1;
                    if let Ok(text) = std::fs::read_to_string(path) {
                        entry.1 += text.lines().count() as u64;
                    }
                }
                counts
            })
            .await
            .map_err(|e| ToolError::internal(format!("join: {e}")))?;

            let mut payload: Vec<Value> = counts
                .into_iter()
                .map(|(lang, (files, lines))| {
                    json!({
                        "language": lang,
                        "file_count": files,
                        "line_count": lines,
                    })
                })
                .collect();
            payload.sort_by(|a, b| {
                b["file_count"]
                    .as_u64()
                    .unwrap_or(0)
                    .cmp(&a["file_count"].as_u64().unwrap_or(0))
            });
            Ok(json!({ "ok": true, "languages": payload }))
        })
    }
}
