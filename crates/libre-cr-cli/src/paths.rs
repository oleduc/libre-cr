//! XDG-aware filesystem path helpers for the wrapper CLI.
//!
//! Layout (see `specs/08-distribution.md` § Configuration Layout):
//!
//! ```text
//! ~/.config/libre-cr/{token, endpoint, review.toml, code.toml}
//! ~/.local/share/libre-cr-{review,code}/
//! ~/.local/state/libre-cr/{log/, run/}
//! ```
//!
//! All helpers resolve `$HOME` lazily so tests can override it via the `HOME`
//! environment variable. We deliberately do not memoize; the cost is a
//! handful of allocations per command invocation, and the test-isolation
//! benefit is large.

use std::path::PathBuf;

use anyhow::{Context, Result};

/// `~/.config/libre-cr/`
pub fn config_dir() -> PathBuf {
    base_config().join("libre-cr")
}

/// `~/.local/state/libre-cr/`
pub fn state_dir() -> PathBuf {
    base_state().join("libre-cr")
}

/// `~/.local/share/libre-cr/`
pub fn data_dir() -> PathBuf {
    base_data().join("libre-cr")
}

/// `~/.local/state/libre-cr/log/`
pub fn log_dir() -> PathBuf {
    state_dir().join("log")
}

/// `~/.local/state/libre-cr/run/`
pub fn run_dir() -> PathBuf {
    state_dir().join("run")
}

/// `~/.local/state/libre-cr/run/review.pid`
pub fn pid_file() -> PathBuf {
    run_dir().join("review.pid")
}

/// `~/.config/libre-cr/endpoint`
pub fn endpoint_file() -> PathBuf {
    config_dir().join("endpoint")
}

/// `~/.config/libre-cr/token`
pub fn token_file() -> PathBuf {
    config_dir().join("token")
}

/// `~/.local/state/libre-cr/log/libre-cr-review.log`
pub fn review_log_file() -> PathBuf {
    log_dir().join("libre-cr-review.log")
}

/// `~/.local/state/libre-cr/log/supervisor.log`
pub fn supervisor_log_file() -> PathBuf {
    log_dir().join("supervisor.log")
}

/// `~/.local/state/libre-cr/log/libre-cr-code.log` — the review daemon
/// pipes the code-daemon child's stderr into this file. See
/// `crates/libre-cr-review/src/code_daemon/transport.rs`.
pub fn code_log_file() -> PathBuf {
    log_dir().join("libre-cr-code.log")
}

/// Create every directory the wrapper expects on first run. Idempotent.
///
/// On Unix the created directories are tightened to mode `0o700` so the token
/// and endpoint files can't be world-read after the fact.
pub fn ensure_dirs() -> Result<()> {
    for d in [config_dir(), state_dir(), data_dir(), log_dir(), run_dir()] {
        std::fs::create_dir_all(&d).with_context(|| format!("create dir {}", d.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&d)?.permissions();
            perms.set_mode(0o700);
            // Best-effort — the user might own the dir already with a wider
            // mode they care about. Don't fail the call.
            let _ = std::fs::set_permissions(&d, perms);
        }
    }
    Ok(())
}

fn home() -> PathBuf {
    // Honor `$HOME` first so integration tests can sandbox the entire tree.
    if let Some(h) = std::env::var_os("HOME") {
        return PathBuf::from(h);
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn base_config() -> PathBuf {
    if let Some(x) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(x);
    }
    home().join(".config")
}

fn base_state() -> PathBuf {
    if let Some(x) = std::env::var_os("XDG_STATE_HOME") {
        return PathBuf::from(x);
    }
    home().join(".local").join("state")
}

fn base_data() -> PathBuf {
    if let Some(x) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(x);
    }
    home().join(".local").join("share")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // `HOME` is process-global; gate tests that mutate it.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_temp_home<F: FnOnce(&std::path::Path)>(f: F) {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let prev_home = std::env::var_os("HOME");
        let prev_cfg = std::env::var_os("XDG_CONFIG_HOME");
        let prev_state = std::env::var_os("XDG_STATE_HOME");
        let prev_data = std::env::var_os("XDG_DATA_HOME");
        std::env::set_var("HOME", dir.path());
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("XDG_STATE_HOME");
        std::env::remove_var("XDG_DATA_HOME");
        f(dir.path());
        if let Some(h) = prev_home {
            std::env::set_var("HOME", h);
        }
        if let Some(v) = prev_cfg {
            std::env::set_var("XDG_CONFIG_HOME", v);
        }
        if let Some(v) = prev_state {
            std::env::set_var("XDG_STATE_HOME", v);
        }
        if let Some(v) = prev_data {
            std::env::set_var("XDG_DATA_HOME", v);
        }
    }

    #[test]
    fn paths_resolve_under_home() {
        with_temp_home(|home| {
            assert_eq!(config_dir(), home.join(".config/libre-cr"));
            assert_eq!(state_dir(), home.join(".local/state/libre-cr"));
            assert_eq!(data_dir(), home.join(".local/share/libre-cr"));
            assert_eq!(log_dir(), home.join(".local/state/libre-cr/log"));
            assert_eq!(run_dir(), home.join(".local/state/libre-cr/run"));
            assert_eq!(
                pid_file(),
                home.join(".local/state/libre-cr/run/review.pid")
            );
        });
    }

    #[test]
    fn ensure_dirs_creates_tree() {
        with_temp_home(|home| {
            ensure_dirs().unwrap();
            assert!(home.join(".config/libre-cr").is_dir());
            assert!(home.join(".local/state/libre-cr/log").is_dir());
            assert!(home.join(".local/state/libre-cr/run").is_dir());
            assert!(home.join(".local/share/libre-cr").is_dir());
        });
    }

    #[test]
    #[cfg(unix)]
    fn ensure_dirs_sets_0700_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        with_temp_home(|home| {
            ensure_dirs().unwrap();
            let mode = std::fs::metadata(home.join(".config/libre-cr"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o700);
        });
    }
}
