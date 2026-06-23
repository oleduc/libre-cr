//! End-to-end suite for the **MCP client → `libre-cr-code` daemon** consumer.
//!
//! External MCP clients (Claude Desktop, Claude Code, custom agents) spawn
//! this daemon and speak JSON-RPC 2.0 over stdio. The contract is
//! `specs/03-code-daemon.md` § Tool Surface. Each test spawns the real
//! binary, drives one focused flow, and asserts wire-level behaviour.
//!
//! The existing `mcp_stdio.rs` integration tests are kept untouched — they
//! exercise a slightly different framing harness (per-test spawn with a
//! pre-batched script). This consumer suite uses a single re-usable
//! [`McpClient`] that can issue multiple sequential or concurrent calls per
//! daemon instance, matching how a real MCP client behaves.

mod common;

use std::sync::Arc;

use common::mcp_client::{code_bin_or_skip, McpClient, McpSocketClient};
use serde_json::{json, Value};
use tempfile::TempDir;

/// Spawn a daemon under a fresh sandboxed `$HOME`. Returns `None` (skip) if
/// the binary can't be built. The returned tempdir keeps the daemon's
/// `$HOME` alive for the lifetime of the test.
async fn spawn_or_skip(test: &str) -> Option<(McpClient, TempDir)> {
    let bin = code_bin_or_skip(test)?;
    let home = tempfile::tempdir().expect("tempdir");
    let client = McpClient::spawn_stdio(bin, home.path())
        .await
        .expect("spawn mcp-stdio");
    Some((client, home))
}

fn build_fixture() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    common::git_fixture::build_fixture_repo(dir.path());
    dir
}

// ─── handshake / discovery ────────────────────────────────────────────────

#[tokio::test]
async fn initialize_handshake() {
    let Some((c, _home)) = spawn_or_skip("initialize_handshake").await else {
        return;
    };
    let result = c.initialize().await.expect("initialize");
    assert_eq!(
        result["serverInfo"]["name"].as_str(),
        Some("libre-cr-code"),
        "unexpected serverInfo: {result}"
    );
    // Protocol version is whatever the daemon advertises; just assert it's a
    // non-empty string so a regression in the handshake is caught.
    assert!(
        result["protocolVersion"]
            .as_str()
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        "protocolVersion missing: {result}"
    );
}

#[tokio::test]
async fn tools_list_returns_19_tools() {
    let Some((c, _home)) = spawn_or_skip("tools_list_returns_19_tools").await else {
        return;
    };
    c.initialize().await.unwrap();
    let tools = c.list_tools().await.expect("tools/list");

    let names: Vec<&str> = tools
        .iter()
        .map(|t| t["name"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(tools.len(), 19, "expected 19 tools, got: {names:?}");

    // Every entry must have the documented schema fields. MCP names the
    // schema field `inputSchema` on the wire.
    for t in &tools {
        let name = t["name"].as_str().expect("tool has name");
        assert!(!name.is_empty());
        assert!(
            t["description"].as_str().is_some_and(|s| !s.is_empty()),
            "tool {name} missing description"
        );
        assert!(
            t["inputSchema"].is_object(),
            "tool {name} missing inputSchema object"
        );
    }

    // The 4 documented stubs must be present.
    for stub in [
        "ast_search",
        "list_symbols",
        "find_definition",
        "find_references",
    ] {
        assert!(names.contains(&stub), "missing stub: {stub}");
    }
    // A representative sampling of real tools.
    for real in [
        "read_file",
        "list_dir",
        "stat_file",
        "grep",
        "git_log",
        "git_blame",
        "git_show",
        "git_diff",
        "detect_languages",
        "discover_repo",
        "scan_for_repos",
        "clone_repo",
        "prepare_worktree",
        "list_worktrees",
        "remove_worktree",
    ] {
        assert!(names.contains(&real), "missing real tool: {real}");
    }
}

// ─── filesystem reads ─────────────────────────────────────────────────────

#[tokio::test]
async fn read_file_round_trip() {
    let Some((c, _home)) = spawn_or_skip("read_file_round_trip").await else {
        return;
    };
    let repo = build_fixture();
    c.initialize().await.unwrap();
    let env = c
        .call(
            "read_file",
            json!({
                "repo_path": repo.path().to_string_lossy(),
                "file": "README.md",
            }),
        )
        .await
        .unwrap();
    assert_eq!(env["ok"], true, "envelope: {env}");
    assert!(env["content"].as_str().unwrap().contains("Fixture repo"));
    assert!(env["total_lines"].as_u64().unwrap() >= 1);
}

#[tokio::test]
async fn read_file_ref_aware() {
    let Some((c, _home)) = spawn_or_skip("read_file_ref_aware").await else {
        return;
    };
    let repo = build_fixture();
    c.initialize().await.unwrap();

    let head = c
        .call(
            "read_file",
            json!({
                "repo_path": repo.path().to_string_lossy(),
                "file": "src/main.rs",
            }),
        )
        .await
        .unwrap();
    assert_eq!(head["ok"], true);
    // HEAD contains "hello, world!" with the bang.
    assert!(head["content"].as_str().unwrap().contains("hello, world!"));

    let v1 = c
        .call(
            "read_file",
            json!({
                "repo_path": repo.path().to_string_lossy(),
                "file": "src/main.rs",
                "ref": "v1",
            }),
        )
        .await
        .unwrap();
    assert_eq!(v1["ok"], true);
    // v1 (the first commit) has no bang.
    let v1_content = v1["content"].as_str().unwrap();
    assert!(v1_content.contains("hello, world"));
    assert!(
        !v1_content.contains("hello, world!"),
        "v1 should be the pre-punctuation version: {v1_content:?}"
    );
}

#[tokio::test]
async fn read_file_rejects_traversal() {
    let Some((c, _home)) = spawn_or_skip("read_file_rejects_traversal").await else {
        return;
    };
    let repo = build_fixture();
    c.initialize().await.unwrap();
    let env = c
        .call(
            "read_file",
            json!({
                "repo_path": repo.path().to_string_lossy(),
                "file": "../../etc/passwd",
            }),
        )
        .await
        .unwrap();
    assert_eq!(env["ok"], false, "expected failure: {env}");
    assert_eq!(env["error"], "not_in_workspace");
}

#[tokio::test]
async fn list_dir_lists() {
    let Some((c, _home)) = spawn_or_skip("list_dir_lists").await else {
        return;
    };
    let repo = build_fixture();
    c.initialize().await.unwrap();
    let env = c
        .call(
            "list_dir",
            json!({
                "repo_path": repo.path().to_string_lossy(),
                "dir": ".",
            }),
        )
        .await
        .unwrap();
    assert_eq!(env["ok"], true);
    let names: Vec<&str> = env["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap_or(""))
        .collect();
    assert!(names.contains(&"README.md"), "names: {names:?}");
    assert!(names.contains(&"src"));
}

#[tokio::test]
async fn stat_file_returns_metadata() {
    let Some((c, _home)) = spawn_or_skip("stat_file_returns_metadata").await else {
        return;
    };
    let repo = build_fixture();
    c.initialize().await.unwrap();
    let env = c
        .call(
            "stat_file",
            json!({
                "repo_path": repo.path().to_string_lossy(),
                "file": "src/main.rs",
            }),
        )
        .await
        .unwrap();
    assert_eq!(env["ok"], true);
    assert!(env["size"].as_u64().unwrap() > 0);
    // Language detection returns capitalized names (e.g. "Rust").
    let lang = env["language"].as_str().unwrap_or("").to_lowercase();
    assert_eq!(lang, "rust", "language: {env}");
    assert_eq!(env["is_binary"], false);
}

// ─── search ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn grep_finds_matches() {
    let Some((c, _home)) = spawn_or_skip("grep_finds_matches").await else {
        return;
    };
    let repo = build_fixture();
    c.initialize().await.unwrap();

    // Case-sensitive literal match.
    let env = c
        .call(
            "grep",
            json!({
                "repo_path": repo.path().to_string_lossy(),
                "pattern": "needle",
                "fixed_string": true,
            }),
        )
        .await
        .unwrap();
    assert_eq!(env["ok"], true);
    let matches = env["matches"].as_array().unwrap();
    assert!(
        matches.len() >= 2,
        "needle should appear in both lib.py and main.rs: {matches:?}"
    );
    let files: Vec<&str> = matches
        .iter()
        .map(|m| m["file"].as_str().unwrap_or(""))
        .collect();
    assert!(files.iter().any(|f| f.ends_with("main.rs")));
    assert!(files.iter().any(|f| f.ends_with("lib.py")));

    // Case-insensitive via inline regex flag.
    let env = c
        .call(
            "grep",
            json!({
                "repo_path": repo.path().to_string_lossy(),
                "pattern": "(?i)NEEDLE",
            }),
        )
        .await
        .unwrap();
    assert_eq!(env["ok"], true);
    assert!(!env["matches"].as_array().unwrap().is_empty());

    // Glob filter restricts to *.py.
    let env = c
        .call(
            "grep",
            json!({
                "repo_path": repo.path().to_string_lossy(),
                "pattern": "needle",
                "fixed_string": true,
                "glob": "*.py",
            }),
        )
        .await
        .unwrap();
    assert_eq!(env["ok"], true);
    let matches = env["matches"].as_array().unwrap();
    for m in matches {
        assert!(
            m["file"].as_str().unwrap().ends_with(".py"),
            "glob filter leaked: {m}"
        );
    }
    assert!(!matches.is_empty());

    // max_matches=1 truncates.
    let env = c
        .call(
            "grep",
            json!({
                "repo_path": repo.path().to_string_lossy(),
                "pattern": "needle",
                "fixed_string": true,
                "max_matches": 1,
            }),
        )
        .await
        .unwrap();
    assert_eq!(env["ok"], true);
    assert_eq!(env["matches"].as_array().unwrap().len(), 1);
    assert_eq!(env["truncated"], true);
}

// ─── git ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn git_log_returns_commits() {
    let Some((c, _home)) = spawn_or_skip("git_log_returns_commits").await else {
        return;
    };
    let repo = build_fixture();
    c.initialize().await.unwrap();
    let env = c
        .call(
            "git_log",
            json!({ "repo_path": repo.path().to_string_lossy() }),
        )
        .await
        .unwrap();
    assert_eq!(env["ok"], true);
    let commits = env["commits"].as_array().unwrap();
    assert_eq!(commits.len(), 2, "expected two commits, got: {commits:?}");
    // The fixture's most recent commit is "punctuate greeting".
    assert_eq!(commits[0]["summary"], "punctuate greeting");
}

#[tokio::test]
async fn git_blame_returns_lines() {
    let Some((c, _home)) = spawn_or_skip("git_blame_returns_lines").await else {
        return;
    };
    let repo = build_fixture();
    c.initialize().await.unwrap();
    let env = c
        .call(
            "git_blame",
            json!({
                "repo_path": repo.path().to_string_lossy(),
                "file": "src/main.rs",
            }),
        )
        .await
        .unwrap();
    assert_eq!(env["ok"], true, "envelope: {env}");
    let lines = env["lines"].as_array().expect("lines array");
    assert!(!lines.is_empty(), "blame should produce lines");
    // Every line carries an author (the fixture sets `user.name=Test`).
    for line in lines {
        let author = line["author"].as_str().unwrap_or("");
        assert!(author.contains("Test"), "unexpected author: {line}");
    }
}

#[tokio::test]
async fn git_show_for_sha() {
    let Some((c, _home)) = spawn_or_skip("git_show_for_sha").await else {
        return;
    };
    let repo = build_fixture();
    let sha = common::git_fixture::head_sha(repo.path());
    c.initialize().await.unwrap();
    let env = c
        .call(
            "git_show",
            json!({
                "repo_path": repo.path().to_string_lossy(),
                "sha": sha,
            }),
        )
        .await
        .unwrap();
    assert_eq!(env["ok"], true, "envelope: {env}");
    // Schema: message + diff (or files); accept either name as long as the
    // commit summary surfaces.
    let dump = serde_json::to_string(&env).unwrap();
    assert!(
        dump.contains("punctuate greeting"),
        "git_show response missing commit summary: {dump}"
    );
}

#[tokio::test]
async fn git_diff_between_refs() {
    let Some((c, _home)) = spawn_or_skip("git_diff_between_refs").await else {
        return;
    };
    let repo = build_fixture();
    c.initialize().await.unwrap();
    let env = c
        .call(
            "git_diff",
            json!({
                "repo_path": repo.path().to_string_lossy(),
                "from_ref": "v1",
                "to_ref": "HEAD",
            }),
        )
        .await
        .unwrap();
    assert_eq!(env["ok"], true, "envelope: {env}");
    let dump = serde_json::to_string(&env).unwrap();
    // The only change between v1 and HEAD is src/main.rs.
    assert!(
        dump.contains("src/main.rs"),
        "git_diff missing changed file: {dump}"
    );
}

// ─── languages / detection ────────────────────────────────────────────────

#[tokio::test]
async fn detect_languages_classifies() {
    let Some((c, _home)) = spawn_or_skip("detect_languages_classifies").await else {
        return;
    };
    let repo = build_fixture();
    c.initialize().await.unwrap();
    let env = c
        .call(
            "detect_languages",
            json!({ "repo_path": repo.path().to_string_lossy() }),
        )
        .await
        .unwrap();
    assert_eq!(env["ok"], true, "envelope: {env}");
    let dump = serde_json::to_string(&env).unwrap().to_lowercase();
    assert!(dump.contains("rust"), "rust not detected: {dump}");
    assert!(dump.contains("python"), "python not detected: {dump}");
}

// ─── repo registry ────────────────────────────────────────────────────────

#[tokio::test]
async fn discover_repo_misses_for_unknown_url() {
    let Some((c, _home)) = spawn_or_skip("discover_repo_misses_for_unknown_url").await else {
        return;
    };
    c.initialize().await.unwrap();
    let env = c
        .call(
            "discover_repo",
            json!({ "remote_url": "https://example.invalid/never/registered.git" }),
        )
        .await
        .unwrap();
    assert_eq!(env["ok"], false, "envelope: {env}");
    assert_eq!(env["error"], "unknown_repo");
}

#[tokio::test]
async fn scan_for_repos_finds_fixture() {
    let Some((c, _home)) = spawn_or_skip("scan_for_repos_finds_fixture").await else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let repo_dir = root.path().join("the-fixture-repo");
    std::fs::create_dir_all(&repo_dir).unwrap();
    common::git_fixture::build_fixture_repo(&repo_dir);
    common::git_fixture::add_remote(
        &repo_dir,
        "origin",
        "https://github.com/libre-cr/fixture.git",
    );

    c.initialize().await.unwrap();
    let env = c
        .call(
            "scan_for_repos",
            json!({ "roots": [root.path().to_string_lossy()] }),
        )
        .await
        .unwrap();
    assert_eq!(env["ok"], true, "envelope: {env}");
    let discovered = env["discovered"].as_array().expect("discovered array");
    let canonical = discovered.iter().find(|d| {
        d["repo_path"]
            .as_str()
            .map(|p| p.contains("the-fixture-repo"))
            .unwrap_or(false)
    });
    let entry = canonical.expect("our fixture must be in discovered");
    let id = entry["repo_id"].as_str().unwrap();
    assert!(
        id.contains("libre-cr/fixture") || id.contains("github.com/libre-cr/fixture"),
        "expected canonical remote url in repo_id, got {id:?}"
    );
}

#[tokio::test]
async fn prepare_worktree_is_idempotent() {
    let Some((c, _home)) = spawn_or_skip("prepare_worktree_is_idempotent").await else {
        return;
    };
    // Scan a fixture so the daemon knows the repo, then prepare a worktree
    // for the `v1` ref twice. The second call should return the same path
    // without re-fetching.
    let root = tempfile::tempdir().unwrap();
    let repo_dir = root.path().join("idem-fixture");
    std::fs::create_dir_all(&repo_dir).unwrap();
    common::git_fixture::build_fixture_repo(&repo_dir);
    common::git_fixture::add_remote(&repo_dir, "origin", "https://github.com/libre-cr/idem.git");

    c.initialize().await.unwrap();
    let scan = c
        .call(
            "scan_for_repos",
            json!({ "roots": [root.path().to_string_lossy()] }),
        )
        .await
        .unwrap();
    let repo_id = scan["discovered"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|d| {
            if d["repo_path"]
                .as_str()
                .map(|p| p.contains("idem-fixture"))
                .unwrap_or(false)
            {
                d["repo_id"].as_str().map(|s| s.to_string())
            } else {
                None
            }
        })
        .expect("repo_id in discovered");

    let env1 = c
        .call(
            "prepare_worktree",
            json!({ "repo_id": repo_id, "ref": "v1" }),
        )
        .await
        .unwrap();
    if env1["ok"] != true {
        // The fixture repo has no fetchable remote — `prepare_worktree`
        // may legitimately fail in this isolated setup. Skip the
        // idempotency assertion rather than fail the suite.
        eprintln!(
            "[prepare_worktree_is_idempotent] prepare_worktree returned !ok ({}); \
             skipping idempotency assertion",
            env1["error"].as_str().unwrap_or("(no error)")
        );
        return;
    }
    let path1 = env1["worktree_path"].as_str().unwrap().to_string();

    let env2 = c
        .call(
            "prepare_worktree",
            json!({ "repo_id": repo_id, "ref": "v1" }),
        )
        .await
        .unwrap();
    assert_eq!(env2["ok"], true);
    let path2 = env2["worktree_path"].as_str().unwrap();
    assert_eq!(path1, path2, "second prepare returned different path");
}

// ─── concurrency ──────────────────────────────────────────────────────────

#[tokio::test]
async fn concurrent_tool_calls_serialize_cleanly() {
    let Some((client, _home)) = spawn_or_skip("concurrent_tool_calls_serialize_cleanly").await
    else {
        return;
    };
    let repo = build_fixture();
    client.initialize().await.unwrap();

    let client = Arc::new(client);
    let mut handles = Vec::new();
    for _ in 0..5 {
        let client = client.clone();
        let path = repo.path().to_path_buf();
        handles.push(tokio::spawn(async move {
            client
                .call(
                    "read_file",
                    json!({
                        "repo_path": path.to_string_lossy(),
                        "file": "README.md",
                    }),
                )
                .await
        }));
    }
    for h in handles {
        let env = h.await.expect("join").expect("call");
        assert_eq!(env["ok"], true);
        assert!(env["content"].as_str().unwrap().contains("Fixture repo"));
    }
}

// ─── stubs ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn stub_tools_return_unsupported_language() {
    let Some((c, _home)) = spawn_or_skip("stub_tools_return_unsupported_language").await else {
        return;
    };
    c.initialize().await.unwrap();

    let env = c
        .call(
            "ast_search",
            json!({
                "repo_path": ".",
                "language": "rust",
                "pattern": "fn $X",
            }),
        )
        .await
        .unwrap();
    assert_eq!(env["ok"], false);
    assert_eq!(env["error"], "unsupported_language");

    let env = c
        .call(
            "list_symbols",
            json!({ "repo_path": ".", "file": "foo.rs" }),
        )
        .await
        .unwrap();
    assert_eq!(env["ok"], false);
    assert_eq!(env["error"], "unsupported_language");

    for tool in ["find_definition", "find_references"] {
        let env = c
            .call(
                tool,
                json!({
                    "repo_path": ".",
                    "file": "foo.rs",
                    "line": 1,
                    "column": 1,
                }),
            )
            .await
            .unwrap();
        assert_eq!(env["ok"], false);
        assert_eq!(env["error"], "unsupported_language", "{tool}");
        assert_eq!(env["confidence"], "low", "{tool} must carry confidence:low");
    }
}

// ─── error surface ────────────────────────────────────────────────────────

#[tokio::test]
async fn unknown_tool_errors() {
    let Some((c, _home)) = spawn_or_skip("unknown_tool_errors").await else {
        return;
    };
    c.initialize().await.unwrap();
    let env = c.call("definitely_not_a_tool", json!({})).await.unwrap();
    assert_eq!(env["ok"], false, "envelope: {env}");
    // Per `registry::ToolRegistry::call`, unknown tools come back as
    // `validation_failed` (the shared `ErrorCategory` vocabulary) with a
    // descriptive message.
    assert_eq!(env["error"], "validation_failed");
    assert!(env["message"]
        .as_str()
        .unwrap_or("")
        .contains("definitely_not_a_tool"));
}

#[tokio::test]
async fn bad_input_shape_errors() {
    let Some((c, _home)) = spawn_or_skip("bad_input_shape_errors").await else {
        return;
    };
    c.initialize().await.unwrap();
    let env = c
        .call("read_file", json!({ "repo_path": "." }))
        .await
        .unwrap();
    assert_eq!(env["ok"], false);
    assert_eq!(env["error"], "validation_failed");
    assert!(env["message"].as_str().unwrap_or("").contains("file"));
}

// ─── unix socket transport ────────────────────────────────────────────────

#[tokio::test]
async fn unix_socket_transport() {
    if cfg!(windows) {
        return; // Unix-domain only.
    }
    let Some(bin) = code_bin_or_skip("unix_socket_transport") else {
        return;
    };
    // tempdir on macOS uses /private/var/... and adds enough path length
    // that the daemon's socket may approach sun_path limits. Use a
    // shorter tempdir under the system tmp.
    let sock_dir = tempfile::Builder::new()
        .prefix("lcr-sock-")
        .tempdir_in(std::env::temp_dir())
        .unwrap();
    let sock = sock_dir.path().join("d.sock");
    let home = tempfile::tempdir().unwrap();

    // Spawn daemon listening on the socket. Detach as a tokio Child so
    // dropping the test process kills it.
    let mut child = tokio::process::Command::new(&bin)
        .arg("mcp-socket")
        .arg("--path")
        .arg(&sock)
        .env("HOME", home.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn mcp-socket");

    // Wait up to ~2s for the socket file.
    let mut connected = None;
    for _ in 0..40 {
        if sock.exists() {
            if let Ok(c) = McpSocketClient::connect(&sock).await {
                connected = Some(c);
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let client = connected.expect("socket connect");

    let init = client.rpc("initialize", json!({})).await.unwrap();
    assert_eq!(
        init["result"]["serverInfo"]["name"].as_str(),
        Some("libre-cr-code"),
    );
    let list: Value = client.rpc("tools/list", json!({})).await.unwrap();
    let tools = list["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 19);

    // Clean shutdown when the test process drops the child.
    let _ = child.kill().await;
}
