//! `git_blame` via the git CLI for simplicity and correctness. gix's blame API
//! is unstable; the CLI is fast enough and lets us stay close to the spec.

use crate::error::{ErrorCode, ToolError};
use crate::util::validate_ref;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct BlameLine {
    pub line: u32,
    pub sha: String,
    pub author: String,
    pub date: i64,
    pub summary: String,
}

pub async fn git_blame(
    repo_path: &Path,
    file: &str,
    ref_name: Option<&str>,
    start_line: Option<u32>,
    end_line: Option<u32>,
) -> Result<Vec<BlameLine>, ToolError> {
    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("-C").arg(repo_path).args(["blame", "--porcelain"]);
    if let (Some(s), Some(e)) = (start_line, end_line) {
        cmd.arg(format!("-L{},{}", s, e));
    } else if let Some(s) = start_line {
        cmd.arg(format!("-L{},+1", s));
    }
    if let Some(r) = ref_name {
        validate_ref(r)?;
        cmd.arg(r);
    }
    cmd.arg("--").arg(file);

    let out = cmd
        .output()
        .await
        .map_err(|e| ToolError::internal(format!("git blame: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        return Err(ToolError::new(
            ErrorCode::NotInWorkspace,
            format!("git blame failed: {stderr}"),
        ));
    }
    parse_porcelain(&String::from_utf8_lossy(&out.stdout))
}

fn parse_porcelain(text: &str) -> Result<Vec<BlameLine>, ToolError> {
    let mut out = Vec::new();
    let mut current_sha = String::new();
    let mut current_author = String::new();
    let mut current_author_time: i64 = 0;
    let mut current_summary = String::new();
    let mut commits: std::collections::HashMap<String, (String, i64, String)> =
        std::collections::HashMap::new();
    let mut expect_content = false;
    let mut current_line_num: u32 = 0;

    for line in text.lines() {
        if expect_content && line.starts_with('\t') {
            out.push(BlameLine {
                line: current_line_num,
                sha: current_sha.clone(),
                author: current_author.clone(),
                date: current_author_time,
                summary: current_summary.clone(),
            });
            expect_content = false;
            continue;
        }
        // Header lines.
        if line.len() >= 40
            && line
                .chars()
                .next()
                .map(|c| c.is_ascii_hexdigit())
                .unwrap_or(false)
        {
            // sha + original-line + final-line [+ num-lines]
            let mut parts = line.split_whitespace();
            current_sha = parts.next().unwrap_or("").to_string();
            let _orig = parts.next();
            current_line_num = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            // Lookup cached metadata if seen.
            if let Some((a, t, s)) = commits.get(&current_sha) {
                current_author = a.clone();
                current_author_time = *t;
                current_summary = s.clone();
            }
            expect_content = true;
        } else if let Some(rest) = line.strip_prefix("author ") {
            current_author = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("author-time ") {
            current_author_time = rest.parse().unwrap_or(0);
        } else if let Some(rest) = line.strip_prefix("summary ") {
            current_summary = rest.to_string();
            commits.insert(
                current_sha.clone(),
                (
                    current_author.clone(),
                    current_author_time,
                    current_summary.clone(),
                ),
            );
        }
    }

    Ok(out)
}
