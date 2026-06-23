//! Integration tests that exercise the wrapper against a temp `$HOME` and
//! controlled child processes. We avoid spawning the real `libre-cr-review`
//! binary because Phase 7 tests shouldn't depend on Phase 2 being built.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use libre_cr_cli::supervisor::{RestartPolicy, SpawnSpec, Supervisor, SupervisorOutcome};
use libre_cr_cli::{paths, proc};

// Many of these tests mutate `$HOME` and `$PATH`. They must not run in
// parallel with each other or with the unit tests that do the same. We use
// `--test-threads=1` for both the unit and integration suites via the
// `test-threads` line in the report; failing that, this mutex keeps each
// integration test serial.
static ENV_LOCK: Mutex<()> = Mutex::new(());

struct TempHome {
    _dir: tempfile::TempDir,
    path: PathBuf,
    prev_home: Option<std::ffi::OsString>,
    prev_xdg_cfg: Option<std::ffi::OsString>,
    prev_xdg_state: Option<std::ffi::OsString>,
    prev_xdg_data: Option<std::ffi::OsString>,
}

impl TempHome {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let prev_home = std::env::var_os("HOME");
        let prev_xdg_cfg = std::env::var_os("XDG_CONFIG_HOME");
        let prev_xdg_state = std::env::var_os("XDG_STATE_HOME");
        let prev_xdg_data = std::env::var_os("XDG_DATA_HOME");
        std::env::set_var("HOME", &path);
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("XDG_STATE_HOME");
        std::env::remove_var("XDG_DATA_HOME");
        Self {
            _dir: dir,
            path,
            prev_home,
            prev_xdg_cfg,
            prev_xdg_state,
            prev_xdg_data,
        }
    }

    fn root(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        if let Some(v) = &self.prev_home {
            std::env::set_var("HOME", v);
        } else {
            std::env::remove_var("HOME");
        }
        if let Some(v) = &self.prev_xdg_cfg {
            std::env::set_var("XDG_CONFIG_HOME", v);
        }
        if let Some(v) = &self.prev_xdg_state {
            std::env::set_var("XDG_STATE_HOME", v);
        }
        if let Some(v) = &self.prev_xdg_data {
            std::env::set_var("XDG_DATA_HOME", v);
        }
    }
}

#[test]
fn ensure_dirs_in_temp_home_creates_full_tree() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let h = TempHome::new();
    paths::ensure_dirs().unwrap();
    assert!(h.root().join(".config/libre-cr").is_dir());
    assert!(h.root().join(".local/state/libre-cr/log").is_dir());
    assert!(h.root().join(".local/state/libre-cr/run").is_dir());
    assert!(h.root().join(".local/share/libre-cr").is_dir());
}

#[test]
fn pid_file_lifecycle_with_stale_detection() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _h = TempHome::new();
    let pid_file = paths::pid_file();

    // Write a high PID that's certainly not a real process.
    proc::write_pid_file(&pid_file, 0x7fff_fff0).unwrap();
    let pid = proc::read_pid_file(&pid_file).unwrap().unwrap();
    assert_eq!(pid, 0x7fff_fff0);
    assert!(!proc::is_alive(pid), "fake PID should not be alive");

    proc::remove_pid_file(&pid_file).unwrap();
    assert!(proc::read_pid_file(&pid_file).unwrap().is_none());
}

#[cfg(unix)]
#[tokio::test]
async fn supervisor_runs_a_fake_review_daemon_and_publishes_endpoint() {
    // Build a small shell command that:
    //  1. writes the endpoint file (mimicking what the real daemon does)
    //  2. sleeps long enough for the test to observe it
    //
    // We resolve all paths under the env-lock guard and then drop the guard
    // before any `.await` (clippy: await_holding_lock). The `TempHome` and
    // its temp directory are leaked so the paths stay live for the rest of
    // the async work; that's safe for a one-shot test process.
    let endpoint_path;
    let pid_path;
    let log_path;
    {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let h = TempHome::new();
        paths::ensure_dirs().unwrap();
        endpoint_path = paths::endpoint_file();
        pid_path = paths::pid_file();
        log_path = paths::review_log_file();
        // Forget the guard so $HOME stays set for the duration of this test.
        // The next test that takes ENV_LOCK and constructs its own TempHome
        // will overwrite $HOME on `new()`.
        std::mem::forget(h);
    }

    let script = format!(
        "echo http://127.0.0.1:55555 > '{}'; sleep 2",
        endpoint_path.display()
    );

    let spec = SpawnSpec {
        program: "sh".into(),
        args: vec!["-c".into(), script],
        log_file: log_path,
        pid_file: pid_path.clone(),
    };
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    let h = tokio::spawn(async move {
        Supervisor::new(spec)
            .with_policy(RestartPolicy {
                max_restarts: 1,
                window: Duration::from_secs(60),
                backoff: Duration::from_millis(10),
            })
            .run(cancel_rx)
            .await
    });

    // Give the script a moment to write the endpoint.
    for _ in 0..30 {
        if endpoint_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let endpoint = std::fs::read_to_string(&endpoint_path).unwrap();
    assert!(
        endpoint.contains("127.0.0.1:55555"),
        "expected endpoint file to be written; got {endpoint:?}"
    );

    // Now cancel and confirm we get the shutdown outcome.
    cancel_tx.send(()).unwrap();
    let outcome = h.await.unwrap().unwrap();
    assert_eq!(outcome, SupervisorOutcome::ShutdownRequested);
    assert!(
        !pid_path.exists(),
        "pid file should be cleaned up on shutdown"
    );
}
