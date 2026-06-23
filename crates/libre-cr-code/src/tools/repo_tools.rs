//! Repo-registry tools: `discover_repo`, `scan_for_repos`, `clone_repo`.

use crate::error::{ErrorCode, ToolError};
use crate::git::read::{default_branch, discover_repo_root, remotes};
use crate::repo::remote_url::canonicalize_remote_url;
use crate::tools::context::ToolContext;
use crate::tools::registry::{Tool, ToolFuture};
use crate::util::expand_path;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct DiscoverRepo;

impl Tool for DiscoverRepo {
    fn name(&self) -> &'static str {
        "discover_repo"
    }
    fn description(&self) -> &'static str {
        "Look up a previously registered repo by remote URL."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["remote_url"],
            "properties": {
                "remote_url": { "type": "string" }
            }
        })
    }
    fn call<'a>(&'a self, ctx: Arc<ToolContext>, input: Value) -> ToolFuture<'a> {
        Box::pin(async move {
            let url = input
                .get("remote_url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::invalid("remote_url required"))?;
            match ctx.registry.find_by_remote(url)? {
                Some(rec) => {
                    let branch = default_branch(&rec.local_path);
                    Ok(json!({
                        "ok": true,
                        "repo_id": rec.repo_id,
                        "repo_path": rec.local_path.to_string_lossy(),
                        "default_branch": branch,
                    }))
                }
                None => Ok(json!({
                    "ok": false,
                    "error": "unknown_repo",
                    "message": format!("no repo registered for {url}"),
                })),
            }
        })
    }
}

pub struct ScanForRepos;

impl Tool for ScanForRepos {
    fn name(&self) -> &'static str {
        "scan_for_repos"
    }
    fn description(&self) -> &'static str {
        "Walk the given root directories, find git repos, register them."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "roots": { "type": "array" }
            }
        })
    }
    fn call<'a>(&'a self, ctx: Arc<ToolContext>, input: Value) -> ToolFuture<'a> {
        Box::pin(async move {
            let raw_roots: Vec<String> = match input.get("roots") {
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect(),
                _ => ctx.config.discovery.default_roots.clone(),
            };
            let roots: Vec<PathBuf> = raw_roots.iter().map(|s| expand_path(s)).collect();

            let discovered = tokio::task::spawn_blocking({
                let ctx = ctx.clone();
                move || -> Result<Vec<Value>, ToolError> {
                    let mut out = Vec::new();
                    for root in roots {
                        if !root.exists() {
                            continue;
                        }
                        scan_root(&ctx, &root, &mut out)?;
                    }
                    Ok(out)
                }
            })
            .await
            .map_err(|e| ToolError::internal(format!("join: {e}")))??;

            Ok(json!({
                "ok": true,
                "discovered": discovered,
            }))
        })
    }
}

fn scan_root(ctx: &ToolContext, root: &Path, out: &mut Vec<Value>) -> Result<(), ToolError> {
    use ignore::WalkBuilder;
    let walker = WalkBuilder::new(root)
        .git_ignore(false)
        .hidden(false)
        .max_depth(Some(6))
        .build();
    for entry in walker.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        if p.file_name().and_then(|n| n.to_str()) == Some(".git") {
            if let Some(parent) = p.parent() {
                if let Ok(remotes) = remotes(parent) {
                    let repo_id = remotes
                        .iter()
                        .find_map(|u| canonicalize_remote_url(u))
                        .unwrap_or_else(|| {
                            format!(
                                "local/{}",
                                parent
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("repo")
                            )
                        });
                    ctx.registry.upsert_repo(&repo_id, parent, &remotes)?;
                    out.push(json!({
                        "repo_id": repo_id,
                        "repo_path": parent.to_string_lossy(),
                        "remotes": remotes,
                    }));
                }
            }
        }
    }
    Ok(())
}

pub struct CloneRepo;

impl Tool for CloneRepo {
    fn name(&self) -> &'static str {
        "clone_repo"
    }
    fn description(&self) -> &'static str {
        "Clone a repo into the managed cache, register it, return repo_id and path."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["remote_url"],
            "properties": {
                "remote_url": { "type": "string" },
                "target_dir": { "type": "string" }
            }
        })
    }
    fn call<'a>(&'a self, ctx: Arc<ToolContext>, input: Value) -> ToolFuture<'a> {
        Box::pin(async move {
            let url = input
                .get("remote_url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::invalid("remote_url required"))?
                .to_string();
            let target_dir = input
                .get("target_dir")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let repo_id = canonicalize_remote_url(&url)
                .ok_or_else(|| ToolError::invalid(format!("unparseable url: {url}")))?;
            let dest = match target_dir {
                Some(t) => expand_path(&t),
                None => ctx.data_dir.join("repos").join(&repo_id),
            };
            if dest.exists() {
                // Already cloned. Register and return.
                let _ = discover_repo_root(&dest);
                let remotes_list = remotes(&dest).unwrap_or_default();
                ctx.registry.upsert_repo(&repo_id, &dest, &remotes_list)?;
                return Ok(json!({
                    "ok": true,
                    "repo_id": repo_id,
                    "repo_path": dest.to_string_lossy(),
                    "note": "already present",
                }));
            }
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let result = tokio::task::spawn_blocking({
                let dest = dest.clone();
                let url = url.clone();
                move || {
                    std::process::Command::new("git")
                        .args(["clone", "--", &url])
                        .arg(&dest)
                        .output()
                }
            })
            .await
            .map_err(|e| ToolError::internal(format!("join: {e}")))?
            .map_err(|e| ToolError::internal(format!("git clone: {e}")))?;
            if !result.status.success() {
                let stderr = String::from_utf8_lossy(&result.stderr).into_owned();
                return Err(ToolError::new(
                    ErrorCode::UnknownRef,
                    format!("git clone failed: {stderr}"),
                ));
            }
            let remotes_list = remotes(&dest).unwrap_or_default();
            ctx.registry.upsert_repo(&repo_id, &dest, &remotes_list)?;
            Ok(json!({
                "ok": true,
                "repo_id": repo_id,
                "repo_path": dest.to_string_lossy(),
            }))
        })
    }
}
