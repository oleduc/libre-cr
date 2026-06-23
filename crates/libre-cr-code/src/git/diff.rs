//! `git_diff` between two refs.

use crate::error::{ErrorCode, ToolError};
use crate::util::validate_ref;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct DiffFile {
    pub path: String,
    pub status: String,
    pub hunks: Vec<Hunk>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Hunk {
    pub header: String,
    pub lines: Vec<String>,
}

pub async fn git_diff(
    repo_path: &Path,
    from_ref: &str,
    to_ref: &str,
    paths: Option<&[String]>,
) -> Result<Vec<DiffFile>, ToolError> {
    validate_ref(from_ref)?;
    validate_ref(to_ref)?;
    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("-C")
        .arg(repo_path)
        .args(["diff", "--unified=3"])
        .arg(format!("{from_ref}..{to_ref}"));
    // `--` separator is always emitted, even with no paths, so any subsequent
    // accidentally-added arg won't be interpreted as a ref.
    cmd.arg("--");
    if let Some(p) = paths {
        for path in p {
            cmd.arg(path);
        }
    }
    let out = cmd
        .output()
        .await
        .map_err(|e| ToolError::internal(format!("git diff: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        return Err(ToolError::new(
            ErrorCode::UnknownRef,
            format!("git diff failed: {stderr}"),
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    Ok(parse_unified(&text))
}

fn parse_unified(text: &str) -> Vec<DiffFile> {
    let mut files: Vec<DiffFile> = Vec::new();
    let mut current_file: Option<DiffFile> = None;
    let mut current_hunk: Option<Hunk> = None;

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            if let Some(mut f) = current_file.take() {
                if let Some(h) = current_hunk.take() {
                    f.hunks.push(h);
                }
                files.push(f);
            }
            let path = rest
                .split_whitespace()
                .last()
                .map(|s| s.trim_start_matches("b/").to_string())
                .unwrap_or_default();
            current_file = Some(DiffFile {
                path,
                status: "modified".to_string(),
                hunks: Vec::new(),
            });
        } else if line.starts_with("new file") {
            if let Some(f) = current_file.as_mut() {
                f.status = "added".to_string();
            }
        } else if line.starts_with("deleted file") {
            if let Some(f) = current_file.as_mut() {
                f.status = "deleted".to_string();
            }
        } else if line.starts_with("@@") {
            if let Some(f) = current_file.as_mut() {
                if let Some(h) = current_hunk.take() {
                    f.hunks.push(h);
                }
            }
            current_hunk = Some(Hunk {
                header: line.to_string(),
                lines: Vec::new(),
            });
        } else if let Some(h) = current_hunk.as_mut() {
            h.lines.push(line.to_string());
        }
    }

    if let Some(mut f) = current_file.take() {
        if let Some(h) = current_hunk.take() {
            f.hunks.push(h);
        }
        files.push(f);
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_simple_diff() {
        let text = r#"diff --git a/foo b/foo
index 1..2 100644
--- a/foo
+++ b/foo
@@ -1,2 +1,2 @@
-old
+new
 same
"#;
        let parsed = parse_unified(text);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].path, "foo");
        assert_eq!(parsed[0].hunks.len(), 1);
        assert!(parsed[0].hunks[0].header.starts_with("@@"));
    }

    #[test]
    fn parses_added_file() {
        let text = r#"diff --git a/new b/new
new file mode 100644
index 0..1 100644
--- /dev/null
+++ b/new
@@ -0,0 +1,1 @@
+content
"#;
        let parsed = parse_unified(text);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].status, "added");
    }

    #[test]
    fn _suppress_unused() {
        let _ = ErrorCode::Internal;
    }
}
