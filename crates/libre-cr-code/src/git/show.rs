//! `git_show` via git CLI for unified diff output.

use crate::error::{ErrorCode, ToolError};
use crate::util::validate_ref;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct ShowResult {
    pub message: String,
    pub author: String,
    pub date: i64,
    pub diff: String,
}

pub async fn git_show(
    repo_path: &Path,
    sha: &str,
    file: Option<&str>,
) -> Result<ShowResult, ToolError> {
    validate_ref(sha)?;
    // Header via `git show -s --format=...`
    let header = tokio::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["show", "-s", "--format=%an%n%at%n%B", sha])
        .output()
        .await
        .map_err(|e| ToolError::internal(format!("git show header: {e}")))?;
    if !header.status.success() {
        let stderr = String::from_utf8_lossy(&header.stderr).into_owned();
        return Err(ToolError::new(
            ErrorCode::UnknownRef,
            format!("git show failed: {stderr}"),
        ));
    }
    let header_text = String::from_utf8_lossy(&header.stdout).into_owned();
    let mut parts = header_text.splitn(3, '\n');
    let author = parts.next().unwrap_or("").to_string();
    let date: i64 = parts.next().unwrap_or("0").parse().unwrap_or(0);
    let message = parts
        .next()
        .unwrap_or("")
        .trim_end_matches('\n')
        .to_string();

    // Diff via `git show --format= sha -- [file]`. The `--` separator is
    // always present so a path with a leading `-` is never interpreted as
    // an option.
    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("-C")
        .arg(repo_path)
        .args(["show", "--format="])
        .arg(sha)
        .arg("--");
    if let Some(f) = file {
        cmd.arg(f);
    }
    let diff_out = cmd
        .output()
        .await
        .map_err(|e| ToolError::internal(format!("git show diff: {e}")))?;
    if !diff_out.status.success() {
        let stderr = String::from_utf8_lossy(&diff_out.stderr).into_owned();
        return Err(ToolError::internal(format!(
            "git show diff failed: {stderr}"
        )));
    }
    let diff = String::from_utf8_lossy(&diff_out.stdout).into_owned();
    Ok(ShowResult {
        message,
        author,
        date,
        diff,
    })
}
