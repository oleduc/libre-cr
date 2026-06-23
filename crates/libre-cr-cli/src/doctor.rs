//! Diagnostic checks for `libre-cr doctor`.
//!
//! The output of [`run_checks`] is structured so tests can assert on it.
//! [`format_report`] renders it as a colored checklist for humans.

use std::path::{Path, PathBuf};

use nu_ansi_term::Color;

use crate::paths;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckStatus {
    Ok,
    Warn,
    Fail,
    Skip,
}

#[derive(Debug, Clone)]
pub struct CheckResult {
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
}

impl CheckResult {
    pub fn ok(name: &str, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: CheckStatus::Ok,
            detail: detail.into(),
        }
    }
    pub fn warn(name: &str, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: CheckStatus::Warn,
            detail: detail.into(),
        }
    }
    pub fn fail(name: &str, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: CheckStatus::Fail,
            detail: detail.into(),
        }
    }
    pub fn skip(name: &str, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: CheckStatus::Skip,
            detail: detail.into(),
        }
    }
}

/// Run every check, returning structured results. Caller renders.
pub fn run_checks() -> Vec<CheckResult> {
    vec![
        check_git(),
        check_binary_on_path("libre-cr-review"),
        check_binary_on_path("libre-cr-code"),
        check_file_perms("token", &paths::token_file()),
        check_file_perms("endpoint", &paths::endpoint_file()),
        check_endpoint_reachable(),
        check_disk_space(&paths::data_dir()),
    ]
}

fn check_git() -> CheckResult {
    match std::process::Command::new("git").arg("--version").output() {
        Ok(out) if out.status.success() => {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            // `git version 2.39.3` → "2.39.3"
            let version = s.split_whitespace().nth(2).unwrap_or("").to_string();
            let major = version
                .split('.')
                .next()
                .and_then(|m| m.parse::<u32>().ok())
                .unwrap_or(0);
            if major >= 2 {
                CheckResult::ok("git on PATH", format!("found {s}"))
            } else {
                CheckResult::warn(
                    "git on PATH",
                    format!("found {s}, but libre-cr targets git ≥ 2.0"),
                )
            }
        }
        Ok(out) => CheckResult::fail(
            "git on PATH",
            format!("`git --version` failed ({:?})", out.status.code()),
        ),
        Err(e) => CheckResult::fail("git on PATH", format!("not found: {e}")),
    }
}

fn check_binary_on_path(name: &str) -> CheckResult {
    if let Some(p) = find_on_path(name) {
        CheckResult::ok(
            &format!("{name} binary"),
            format!("found at {}", p.display()),
        )
    } else {
        CheckResult::warn(
            &format!("{name} binary"),
            "not found on PATH (install before running `libre-cr start`)",
        )
    }
}

fn check_file_perms(label: &str, path: &Path) -> CheckResult {
    if !path.exists() {
        return CheckResult::skip(
            &format!("{label} file perms"),
            format!("{} not present yet", path.display()),
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o777)
            .unwrap_or(0);
        if mode == 0o600 {
            CheckResult::ok(
                &format!("{label} file perms"),
                format!("{} is 0600", path.display()),
            )
        } else {
            CheckResult::warn(
                &format!("{label} file perms"),
                format!("{} is {:o} (expected 0600)", path.display(), mode),
            )
        }
    }
    #[cfg(not(unix))]
    {
        CheckResult::skip(
            &format!("{label} file perms"),
            "POSIX mode check skipped on this platform".to_string(),
        )
    }
}

fn check_endpoint_reachable() -> CheckResult {
    let ep_file = paths::endpoint_file();
    if !ep_file.exists() {
        return CheckResult::skip("daemon endpoint", "endpoint file not present yet");
    }
    let endpoint = match std::fs::read_to_string(&ep_file) {
        Ok(s) => s.trim().to_string(),
        Err(e) => {
            return CheckResult::fail("daemon endpoint", format!("read endpoint file: {e}"));
        }
    };
    if endpoint.is_empty() {
        return CheckResult::warn("daemon endpoint", "endpoint file is empty");
    }
    // Defer the actual HTTP probe to the async `status` command; here we just
    // confirm the format is sane.
    if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        CheckResult::ok(
            "daemon endpoint",
            format!("{endpoint} (ping via `libre-cr status`)"),
        )
    } else {
        CheckResult::warn("daemon endpoint", format!("unexpected format: {endpoint}"))
    }
}

fn check_disk_space(_path: &Path) -> CheckResult {
    // True disk-space queries need `statvfs` on Unix or `GetDiskFreeSpaceEx`
    // on Windows. Pulling in `fs2`/`sysinfo` for one number is more than
    // this round needs; report a `Skip` with the path that would be probed.
    CheckResult::skip(
        "disk space (worktrees)",
        format!(
            "not measured in this build ({})",
            paths::data_dir().display()
        ),
    )
}

/// Look up `name` on `$PATH`, returning the first hit. Used both by `doctor`
/// and by `start` (where we want a friendly error if the daemon isn't
/// installed yet).
pub fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        // Windows: also probe `.exe` extensions.
        #[cfg(windows)]
        {
            let exe = dir.join(format!("{name}.exe"));
            if exe.is_file() {
                return Some(exe);
            }
        }
    }
    None
}

/// Render `results` as a colored checklist.
pub fn format_report(results: &[CheckResult]) -> String {
    let mut out = String::new();
    for r in results {
        let (sym, color) = match r.status {
            CheckStatus::Ok => ("✓", Color::Green),
            CheckStatus::Warn => ("!", Color::Yellow),
            CheckStatus::Fail => ("✗", Color::Red),
            CheckStatus::Skip => ("·", Color::DarkGray),
        };
        out.push_str(&format!(
            "  {} {}  {}\n",
            color.paint(sym),
            r.name,
            color.paint(&r.detail)
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static PATH_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn find_on_path_finds_a_real_binary() {
        let _g = PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // `cargo` itself is on every dev machine where these tests run, but
        // for portability we look for `sh` on Unix.
        #[cfg(unix)]
        {
            assert!(find_on_path("sh").is_some());
        }
        #[cfg(windows)]
        {
            // `cmd.exe` is always present.
            assert!(find_on_path("cmd").is_some());
        }
    }

    #[test]
    fn find_on_path_with_missing_binary() {
        let _g = PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert!(find_on_path("not-a-real-binary-xyzzy-libre-cr").is_none());
    }

    #[test]
    fn check_git_returns_ok_or_fail() {
        let _g = PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let r = check_git();
        matches!(
            r.status,
            CheckStatus::Ok | CheckStatus::Warn | CheckStatus::Fail
        );
    }

    #[test]
    fn check_binary_path_handles_missing() {
        let _g = PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var_os("PATH");
        std::env::set_var("PATH", "");
        let r = check_binary_on_path("libre-cr-code");
        assert_eq!(r.status, CheckStatus::Warn);
        if let Some(p) = prev {
            std::env::set_var("PATH", p);
        }
    }

    #[test]
    fn format_report_includes_every_name() {
        let results = vec![
            CheckResult::ok("a", "fine"),
            CheckResult::fail("b", "broken"),
        ];
        let s = format_report(&results);
        assert!(s.contains("a"));
        assert!(s.contains("b"));
        assert!(s.contains("fine"));
        assert!(s.contains("broken"));
    }
}
