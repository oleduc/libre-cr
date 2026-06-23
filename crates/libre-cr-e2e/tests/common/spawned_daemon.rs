//! Spawn `libre-cr-review` (and, when available, `libre-cr-code`) as real
//! subprocesses under a sandboxed `$HOME`. Used by the
//! `e2e_http_consumer.rs` suite to exercise the daemon end-to-end over the
//! wire, the way the browser extension does.
//!
//! Pattern mirrors `tests/spawned_daemon.rs`:
//!   1. Lazily run `cargo build -p libre-cr-{review,code} --bin libre-cr-{review,code}`
//!      once per test process.
//!   2. Locate each binary at `<target>/<profile>/<name>` via the test
//!      executable's own location.
//!   3. Write a `review.toml` configured for the mock provider and a known
//!      token, then start the daemon as a tokio `Child`. Read the endpoint
//!      file (with timeout) to discover the URL.
//!
//! Drop kills the child. The tempdir keeps the daemon's `$HOME` alive for
//! the lifetime of the `SpawnedDaemon`.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use tempfile::TempDir;
use tokio::process::{Child, Command};

static BUILD: Once = Once::new();
static BUILT_OK: AtomicBool = AtomicBool::new(false);

/// In CI (`LIBRE_CR_E2E_REQUIRED=1`) a missing binary is a hard failure,
/// never a silent skip — otherwise a broken Rust build shows green E2E.
fn e2e_required() -> bool {
    std::env::var("LIBRE_CR_E2E_REQUIRED").as_deref() == Ok("1")
}

/// Lazily build both binaries once per test process. Errors are recorded so
/// follow-up callers can `?` skip rather than panic with "poisoned Once".
fn ensure_built() -> bool {
    BUILD.call_once(|| {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let mut ok = true;
        for bin in ["libre-cr-review", "libre-cr-code"] {
            let out = std::process::Command::new(&cargo)
                .args(["build", "-p", bin, "--bin", bin])
                .output();
            match out {
                Ok(o) if o.status.success() => {}
                Ok(o) => {
                    eprintln!(
                        "[spawned_daemon] cargo build {bin} failed:\n\
                         --- stdout ---\n{}\n--- stderr ---\n{}",
                        String::from_utf8_lossy(&o.stdout),
                        String::from_utf8_lossy(&o.stderr),
                    );
                    ok = false;
                }
                Err(e) => {
                    eprintln!("[spawned_daemon] spawn cargo {bin}: {e}");
                    ok = false;
                }
            }
        }
        BUILT_OK.store(ok, Ordering::SeqCst);
    });
    BUILT_OK.load(Ordering::SeqCst)
}

/// Locate a built binary relative to the test executable.
fn locate_bin(name: &str) -> Option<PathBuf> {
    let mut p = std::env::current_exe().ok()?;
    p.pop();
    p.pop();
    p.push(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    });
    if p.exists() {
        Some(p)
    } else {
        eprintln!("[spawned_daemon] binary not found at {p:?}");
        None
    }
}

/// One spawned daemon under a tempdir-rooted `$HOME`.
pub struct SpawnedDaemon {
    pub endpoint: String,
    pub token: String,
    pub home: PathBuf,
    pub config_path: PathBuf,
    child: Option<Child>,
    /// Held to keep the tempdir alive. Dropped after `child` so the kill
    /// signal lands before files vanish.
    _tempdir: TempDir,
}

impl SpawnedDaemon {
    /// Build the daemon, write a default `review.toml`, start it. Returns
    /// `None` if the binaries can't be built (CI without Rust, etc.).
    pub async fn start() -> Result<Option<Self>> {
        Self::start_with(|_| {}).await
    }

    /// Variant that lets the caller mutate the `Config` before it's written
    /// to disk. Use this to enable `mock.code_intel`, register scripted
    /// provider events, swap to a real code daemon, etc.
    pub async fn start_with<F>(mutate: F) -> Result<Option<Self>>
    where
        F: FnOnce(&mut libre_cr_review::config::Config),
    {
        if !ensure_built() {
            if e2e_required() {
                panic!(
                    "[spawned_daemon] LIBRE_CR_E2E_REQUIRED=1 but the daemon \
                     binaries could not be built — failing instead of skipping"
                );
            }
            return Ok(None);
        }
        let review_bin = match locate_bin("libre-cr-review") {
            Some(p) => p,
            None => {
                if e2e_required() {
                    panic!(
                        "[spawned_daemon] LIBRE_CR_E2E_REQUIRED=1 but the \
                         libre-cr-review binary could not be located — \
                         failing instead of skipping"
                    );
                }
                return Ok(None);
            }
        };
        let code_bin = locate_bin("libre-cr-code");

        let home_dir = tempfile::tempdir().context("tempdir")?;
        let home = home_dir.path().to_path_buf();
        // `Config::default_path()` resolves via `dirs::config_dir()`, which
        // on macOS lands under `Library/Application Support` rather than
        // `~/.config`. Sidestep the divergence by writing the config to a
        // known location and passing `--config` explicitly.
        let config_path = home.join("review.toml");
        // `endpoint_file` / `token_file` still default to `~/.config/...`
        // and `shellexpand` reads `$HOME`, so those land predictably inside
        // the sandbox.
        let endpoint_path = home.join(".config/libre-cr/endpoint");
        let token_path = home.join(".config/libre-cr/token");

        // Write a known token up-front; the daemon will reuse the file rather
        // than mint a fresh one. Saves us from having to read it back.
        let token = "e2e-token-deadbeef".to_string();
        std::fs::create_dir_all(home.join(".config/libre-cr"))?;
        std::fs::write(&token_path, &token)?;

        let mut cfg = libre_cr_review::config::Config::default();
        // The default ServerConfig already binds 127.0.0.1:0; just confirm.
        cfg.server.bind = "127.0.0.1".into();
        cfg.server.port = 0;
        // Use the mock provider so we can run hermetic tests with no network.
        cfg.provider.kind = "mock".into();
        cfg.mock.code_intel = true; // skip real worktree spawn by default
        if let Some(bin) = &code_bin {
            cfg.code_daemon.binary = bin.to_string_lossy().into_owned();
        }
        mutate(&mut cfg);
        cfg.save(&config_path)?;

        // Make sure no stale endpoint file is mistaken for the new daemon's.
        let _ = std::fs::remove_file(&endpoint_path);

        let child = Command::new(&review_bin)
            .arg("--config")
            .arg(&config_path)
            .arg("serve")
            .env("HOME", &home)
            // Mute axum logging to keep test stderr clean.
            .env("RUST_LOG", "warn")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("spawn libre-cr-review serve")?;

        let endpoint = wait_for_endpoint(&endpoint_path, Duration::from_secs(10))
            .await
            .context("daemon did not publish endpoint")?;

        Ok(Some(Self {
            endpoint,
            token,
            home,
            config_path,
            child: Some(child),
            _tempdir: home_dir,
        }))
    }

    /// Path to the daemon's `~/.config/libre-cr/endpoint` file.
    pub fn endpoint_file(&self) -> PathBuf {
        self.home.join(".config/libre-cr/endpoint")
    }

    /// Kill the daemon explicitly. Drop does this too; some tests need to
    /// kill then restart with the same `$HOME`.
    pub async fn shutdown(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.start_kill();
            let _ = c.wait().await;
        }
    }

    /// Restart with the same `$HOME` and `review.toml`. Useful for
    /// "config persists across restart" assertions.
    pub async fn restart(&mut self) -> Result<()> {
        self.shutdown().await;
        let endpoint_path = self.endpoint_file();
        let _ = std::fs::remove_file(&endpoint_path);

        let review_bin = locate_bin("libre-cr-review")
            .ok_or_else(|| anyhow!("review bin missing on restart"))?;
        let child = Command::new(&review_bin)
            .arg("--config")
            .arg(&self.config_path)
            .arg("serve")
            .env("HOME", &self.home)
            .env("RUST_LOG", "warn")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("respawn libre-cr-review serve")?;
        self.child = Some(child);
        self.endpoint = wait_for_endpoint(&endpoint_path, Duration::from_secs(10))
            .await
            .context("daemon did not republish endpoint")?;
        Ok(())
    }

    /// Convenience for HTTP clients.
    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.endpoint.trim_end_matches('/'), path)
    }

    /// WS URL (token is appended as query — daemon accepts both header and
    /// `?token=`).
    pub fn ws_url(&self, path: &str) -> String {
        let base = self
            .endpoint
            .replace("http://", "ws://")
            .replace("https://", "wss://");
        format!("{}{}", base.trim_end_matches('/'), path)
    }
}

impl Drop for SpawnedDaemon {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            // kill_on_drop already requested a SIGKILL when the Child is
            // dropped; we still try `start_kill` to be explicit.
            let _ = c.start_kill();
        }
    }
}

async fn wait_for_endpoint(path: &Path, total: Duration) -> Result<String> {
    let deadline = std::time::Instant::now() + total;
    let mut last_err = String::new();
    while std::time::Instant::now() < deadline {
        match std::fs::read_to_string(path) {
            Ok(s) => {
                let t = s.trim().to_string();
                if !t.is_empty() {
                    return Ok(t);
                }
            }
            Err(e) => {
                last_err = e.to_string();
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(anyhow!(
        "endpoint file {} never appeared (last error: {})",
        path.display(),
        last_err,
    ))
}

/// Quick HTTP-client convenience for tests.
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .expect("reqwest client")
}
