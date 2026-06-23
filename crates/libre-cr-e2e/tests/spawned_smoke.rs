//! Integration test that exercises the **real** `SpawnedClient` against the
//! workspace-built `libre-cr-code` binary. The other integration tests use
//! `MockCodeDaemonClient`; this one closes the loop on Phase 3 wiring.
//!
//! Strategy:
//!   1. Lazily run `cargo build --bin libre-cr-code` once per test process.
//!   2. Locate the binary at `<target_dir>/libre-cr-code` via the test
//!      executable's own location (`current_exe()` lives in
//!      `<target>/<profile>/deps/`, two parents up is `<target>/<profile>`).
//!   3. Build a `SpawnedClient` pointed at it, list tools, and call one against
//!      a temp-dir fixture repo to confirm tools/call round-trips.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Once;

use libre_cr_review::code_daemon::SpawnedClient;
use libre_cr_review::config::CodeDaemonConfig;
use libre_cr_review::tools::code_daemon::CodeDaemonClient;
use tempfile::TempDir;

static BUILD: Once = Once::new();
static BUILT_OK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Locate (and lazily build) the `libre-cr-code` binary.
///
/// Returns `None` if the binary can't be produced — in that case the test
/// emits a skip message and returns early instead of failing. Concretely the
/// inner `cargo build` runs once per process; if it succeeds we use the
/// resulting binary, otherwise subsequent calls all return `None`. We *don't*
/// poison the `Once` with a panic — that would make follow-up tests fail with
/// "previously poisoned" instead of skipping cleanly.
fn libre_cr_code_bin() -> Option<PathBuf> {
    BUILD.call_once(|| {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let output = Command::new(&cargo)
            .args(["build", "-p", "libre-cr-code", "--bin", "libre-cr-code"])
            .output();
        match output {
            Ok(o) if o.status.success() => {
                BUILT_OK.store(true, std::sync::atomic::Ordering::SeqCst)
            }
            Ok(o) => {
                eprintln!(
                    "[spawned_daemon] cargo build failed:\n--- stdout ---\n{}\n--- stderr ---\n{}",
                    String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr),
                );
            }
            Err(e) => {
                eprintln!("[spawned_daemon] could not spawn cargo: {e}");
            }
        }
    });

    if !BUILT_OK.load(std::sync::atomic::Ordering::SeqCst) {
        return None;
    }

    let test_exe = std::env::current_exe().expect("current_exe");
    let mut p = test_exe;
    p.pop();
    p.pop();
    p.push(if cfg!(windows) {
        "libre-cr-code.exe"
    } else {
        "libre-cr-code"
    });
    if !p.exists() {
        eprintln!("[spawned_daemon] binary not found at {p:?}, skipping");
        return None;
    }
    Some(p)
}

/// Skip-or-go helper — returns the config if the binary is available, prints
/// a skip message and returns `None` otherwise. Tests using this should
/// `return` on `None` so they're recorded as passing-but-no-op.
fn spawn_config_or_skip(test_name: &str) -> Option<CodeDaemonConfig> {
    let bin = match libre_cr_code_bin() {
        Some(b) => b,
        None => {
            // In CI (`LIBRE_CR_E2E_REQUIRED=1`) a missing binary is a hard
            // failure, never a silent skip.
            if std::env::var("LIBRE_CR_E2E_REQUIRED").as_deref() == Ok("1") {
                panic!(
                    "[spawned_daemon::{test_name}] LIBRE_CR_E2E_REQUIRED=1 but \
                     the libre-cr-code binary could not be built/located — \
                     failing instead of skipping"
                );
            }
            return None;
        }
    };
    eprintln!("[spawned_daemon::{test_name}] using {bin:?}");
    Some(CodeDaemonConfig {
        mode: "spawn".into(),
        binary: bin.to_string_lossy().into_owned(),
        external_socket: String::new(),
        restart_on_failure: false,
        max_restarts_per_hour: 1,
    })
}

/// Build a tiny git repo on disk: one file, one commit. Pure-Rust git would
/// be nicer but the `git` CLI is required by the daemon anyway, so use it.
fn fixture_repo() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();

    let run = |args: &[&str]| {
        let status = Command::new("git")
            .args(args)
            .current_dir(path)
            .env("GIT_AUTHOR_NAME", "Tester")
            .env("GIT_AUTHOR_EMAIL", "tester@example.com")
            .env("GIT_COMMITTER_NAME", "Tester")
            .env("GIT_COMMITTER_EMAIL", "tester@example.com")
            .status()
            .expect("spawn git");
        assert!(status.success(), "git {args:?} failed");
    };

    run(&["init", "--quiet", "--initial-branch=main"]);
    std::fs::write(path.join("hello.txt"), "hello bcryptHash world\n").unwrap();
    run(&["add", "hello.txt"]);
    run(&["commit", "--quiet", "-m", "init"]);

    dir
}

#[tokio::test]
async fn spawned_client_lists_real_tools() {
    let Some(cfg) = spawn_config_or_skip("client") else {
        return;
    };
    let client = SpawnedClient::from_config(&cfg)
        .await
        .expect("connect spawned daemon");

    let tools = client.list_tools().await.expect("list_tools");
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();

    // A representative subset of Phase B tools. If any of these are missing
    // the wire-format is broken on one side.
    for expected in [
        "read_file",
        "list_dir",
        "stat_file",
        "grep",
        "git_log",
        "git_blame",
        "discover_repo",
        "scan_for_repos",
        "prepare_worktree",
        "detect_languages",
    ] {
        assert!(
            names.contains(&expected),
            "expected tool {expected:?} in {names:?}"
        );
    }
}

#[tokio::test]
async fn spawned_client_reads_file_from_fixture() {
    let repo = fixture_repo();
    let Some(cfg) = spawn_config_or_skip("client") else {
        return;
    };
    let client = SpawnedClient::from_config(&cfg)
        .await
        .expect("connect spawned daemon");

    let result = client
        .call(
            "read_file",
            serde_json::json!({
                "repo_path": repo.path().to_string_lossy(),
                "file": "hello.txt",
            }),
        )
        .await
        .expect("read_file call");

    let content = result
        .get("content")
        .and_then(|v| v.as_str())
        .expect("read_file should return a `content` string field");
    assert!(
        content.contains("bcryptHash"),
        "unexpected file body: {content:?}"
    );
}

#[tokio::test]
async fn spawned_client_greps_fixture_repo() {
    let repo = fixture_repo();
    let Some(cfg) = spawn_config_or_skip("client") else {
        return;
    };
    let client = SpawnedClient::from_config(&cfg)
        .await
        .expect("connect spawned daemon");

    let result = client
        .call(
            "grep",
            serde_json::json!({
                "repo_path": repo.path().to_string_lossy(),
                "pattern": "bcryptHash",
            }),
        )
        .await
        .expect("grep call");

    let matches = result
        .get("matches")
        .and_then(|v| v.as_array())
        .expect("grep should return a `matches` array");
    assert_eq!(matches.len(), 1, "expected one match, got: {matches:?}");
    assert_eq!(
        matches[0].get("file").and_then(|v| v.as_str()),
        Some("hello.txt")
    );
}
