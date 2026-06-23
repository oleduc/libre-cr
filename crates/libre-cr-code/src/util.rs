//! Small shared utilities.

use crate::error::{ErrorCode, ToolError};
use std::path::{Path, PathBuf};

/// Resolve a user-relative path like `~/foo` or `~/.config/x` to absolute.
pub fn expand_path(s: &str) -> PathBuf {
    PathBuf::from(shellexpand::tilde(s).into_owned())
}

/// Reject paths that escape `repo_path` via `..`, absolute paths, or symlinks
/// pointing outside. Returns the canonical absolute file path on success.
pub fn safe_join(repo_path: &Path, file: &str) -> Result<PathBuf, ToolError> {
    let candidate = Path::new(file);
    if candidate.is_absolute() {
        return Err(ToolError::new(
            ErrorCode::NotInWorkspace,
            format!("path must be relative to repo: {file}"),
        ));
    }

    let repo_canon = repo_path.canonicalize().map_err(|e| {
        ToolError::new(
            ErrorCode::NotInWorkspace,
            format!("repo path does not exist: {e}"),
        )
    })?;

    let joined = repo_canon.join(candidate);

    // canonicalize() requires the file to exist; for stat use a manual normalizer.
    let normalized = normalize_no_follow(&joined);

    if !normalized.starts_with(&repo_canon) {
        return Err(ToolError::new(
            ErrorCode::NotInWorkspace,
            format!("path escapes repo: {file}"),
        ));
    }

    // If it exists, also follow symlinks and re-check.
    if let Ok(canon) = normalized.canonicalize() {
        if !canon.starts_with(&repo_canon) {
            return Err(ToolError::new(
                ErrorCode::NotInWorkspace,
                format!("symlink escapes repo: {file}"),
            ));
        }
        return Ok(canon);
    }

    Ok(normalized)
}

/// Compute a lexical normalization (resolve `.` and `..`) without touching the
/// filesystem. Used as a first pass before canonicalize().
pub fn normalize_no_follow(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Current unix timestamp (seconds).
pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Validate a git ref or SHA before forwarding it to the `git` CLI as a
/// positional argument. We don't reimplement git's full refname rules
/// (`git check-ref-format` is the canonical reference); we only block the
/// argv-injection class — empty strings, whitespace, embedded NULs, and the
/// leading-`-` form that git would otherwise parse as an option (e.g.
/// `--exec=…`, `-x`). Callers must additionally place this argument *after*
/// any flag the command takes (no `--` separator is needed for a single ref
/// argument, but is required when paths follow).
pub fn validate_ref(r: &str) -> Result<(), ToolError> {
    if r.is_empty() {
        return Err(ToolError::new(
            ErrorCode::UnknownRef,
            "ref must not be empty".to_string(),
        ));
    }
    if r.starts_with('-') {
        return Err(ToolError::new(
            ErrorCode::UnknownRef,
            format!("ref must not start with '-': {r}"),
        ));
    }
    for c in r.chars() {
        if c.is_whitespace() || c == '\0' {
            return Err(ToolError::new(
                ErrorCode::UnknownRef,
                format!("ref contains invalid whitespace or NUL: {r:?}"),
            ));
        }
    }
    Ok(())
}

/// Sanitize a ref string for use as a directory name.
pub fn sanitize_ref(r: &str) -> String {
    r.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '.' | '_' => c,
            _ => '_',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn rejects_absolute() {
        let tmp = TempDir::new().unwrap();
        let err = safe_join(tmp.path(), "/etc/passwd").unwrap_err();
        assert_eq!(err.code, ErrorCode::NotInWorkspace);
    }

    #[test]
    fn rejects_parent_escape() {
        let tmp = TempDir::new().unwrap();
        let err = safe_join(tmp.path(), "../../etc/passwd").unwrap_err();
        assert_eq!(err.code, ErrorCode::NotInWorkspace);
    }

    #[test]
    fn allows_inside() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("hello.txt"), "hi").unwrap();
        let p = safe_join(tmp.path(), "hello.txt").unwrap();
        assert!(p.ends_with("hello.txt"));
    }

    #[test]
    fn normalizes_dotdot() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("a")).unwrap();
        std::fs::write(tmp.path().join("b.txt"), "x").unwrap();
        let p = safe_join(tmp.path(), "a/../b.txt").unwrap();
        assert!(p.ends_with("b.txt"));
    }

    #[test]
    fn sanitize_ref_basic() {
        assert_eq!(sanitize_ref("refs/pull/123/head"), "refs_pull_123_head");
        assert_eq!(sanitize_ref("main"), "main");
        assert_eq!(sanitize_ref("feature/foo"), "feature_foo");
    }

    #[test]
    fn validate_ref_accepts_common_shapes() {
        for ok in [
            "main",
            "v1.2.3",
            "abc1234",
            "pull/123/head",
            "refs/heads/main",
            "feature/x.y",
            "0123456789abcdef0123456789abcdef01234567",
        ] {
            validate_ref(ok).unwrap_or_else(|e| panic!("expected ok for {ok}: {}", e.message));
        }
    }

    #[test]
    fn validate_ref_rejects_argv_injection() {
        for bad in ["--exec=rm", "-x", "", " ", "main\n", "ref\0name"] {
            let err = validate_ref(bad)
                .err()
                .unwrap_or_else(|| panic!("expected validate_ref to reject {bad:?}"));
            assert_eq!(err.code, ErrorCode::UnknownRef);
        }
    }
}
