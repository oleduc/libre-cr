//! Integration tests: spawn `libre-cr-code mcp-stdio` and exercise the
//! `tools/list` and `tools/call` flow.

mod fixtures;

use serde_json::{json, Value};
use std::process::Stdio;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

fn bin_path() -> std::path::PathBuf {
    // CARGO_BIN_EXE_<name> is set for integration tests.
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_libre-cr-code"))
}

async fn rpc_session(req_lines: Vec<Value>) -> Vec<Value> {
    let mut child = Command::new(bin_path())
        .arg("mcp-stdio")
        .env("HOME", std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);

    let total = req_lines.iter().filter(|v| v.get("id").is_some()).count();

    for r in req_lines {
        let mut line = serde_json::to_string(&r).unwrap();
        line.push('\n');
        stdin.write_all(line.as_bytes()).await.unwrap();
        stdin.flush().await.unwrap();
    }

    let mut out = Vec::new();
    while out.len() < total {
        let mut buf = String::new();
        let n = reader.read_line(&mut buf).await.unwrap();
        if n == 0 {
            break;
        }
        if buf.trim().is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(buf.trim()).unwrap();
        out.push(v);
    }

    drop(stdin);
    let _ = child.kill().await;
    out
}

#[tokio::test]
async fn initialize_and_tools_list() {
    let responses = rpc_session(vec![
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
    ])
    .await;
    assert_eq!(responses.len(), 2);
    let init = &responses[0];
    assert_eq!(init["jsonrpc"], "2.0");
    assert!(init["result"]["serverInfo"]["name"].as_str().unwrap() == "libre-cr-code");

    let list = &responses[1];
    let tools = list["result"]["tools"].as_array().unwrap();
    let names: Vec<_> = tools
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    for expected in [
        "discover_repo",
        "scan_for_repos",
        "clone_repo",
        "prepare_worktree",
        "list_worktrees",
        "remove_worktree",
        "read_file",
        "list_dir",
        "stat_file",
        "grep",
        "ast_search",
        "list_symbols",
        "find_definition",
        "find_references",
        "git_log",
        "git_blame",
        "git_show",
        "git_diff",
        "detect_languages",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "missing tool: {expected}"
        );
    }
}

#[tokio::test]
async fn tools_call_read_file() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("fixture");
    std::fs::create_dir_all(&repo).unwrap();
    fixtures::build_fixture_repo(&repo);

    let responses = rpc_session(vec![
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
            "name": "read_file",
            "arguments": {
                "repo_path": repo.to_string_lossy(),
                "file": "README.md"
            }
        }}),
    ])
    .await;
    assert_eq!(responses.len(), 2);
    let call = &responses[1];
    let text = call["result"]["content"][0]["text"].as_str().unwrap();
    let envelope: Value = serde_json::from_str(text).unwrap();
    assert_eq!(envelope["ok"], true);
    assert!(envelope["content"]
        .as_str()
        .unwrap()
        .contains("Fixture repo"));
}

#[tokio::test]
async fn tools_call_grep_finds_needle() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("fixture");
    std::fs::create_dir_all(&repo).unwrap();
    fixtures::build_fixture_repo(&repo);

    let responses = rpc_session(vec![
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
            "name": "grep",
            "arguments": {
                "repo_path": repo.to_string_lossy(),
                "pattern": "needle",
                "fixed_string": true
            }
        }}),
    ])
    .await;
    let call = &responses[1];
    let text = call["result"]["content"][0]["text"].as_str().unwrap();
    let envelope: Value = serde_json::from_str(text).unwrap();
    assert_eq!(envelope["ok"], true);
    let matches = envelope["matches"].as_array().unwrap();
    assert!(!matches.is_empty(), "expected at least one grep match");
}

#[tokio::test]
async fn tools_call_git_log_works() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("fixture");
    std::fs::create_dir_all(&repo).unwrap();
    fixtures::build_fixture_repo(&repo);

    let responses = rpc_session(vec![
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
            "name": "git_log",
            "arguments": { "repo_path": repo.to_string_lossy() }
        }}),
    ])
    .await;
    let call = &responses[1];
    let text = call["result"]["content"][0]["text"].as_str().unwrap();
    let envelope: Value = serde_json::from_str(text).unwrap();
    assert_eq!(envelope["ok"], true);
    let commits = envelope["commits"].as_array().unwrap();
    assert!(commits.len() >= 2);
    assert_eq!(commits[0]["summary"], "punctuate greeting");
}

#[tokio::test]
async fn unsupported_language_envelope_for_stubs() {
    let responses = rpc_session(vec![
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
            "name": "list_symbols",
            "arguments": { "repo_path": ".", "file": "foo.rs" }
        }}),
    ])
    .await;
    let call = &responses[1];
    let text = call["result"]["content"][0]["text"].as_str().unwrap();
    let envelope: Value = serde_json::from_str(text).unwrap();
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["error"], "unsupported_language");
}
