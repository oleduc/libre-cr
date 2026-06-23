//! On-disk git fixture builder for E2E tests.
//!
//! Builds a small two-commit repo (one Rust file + one Python file) via the
//! `git` CLI. Used by the MCP-consumer suite to give the spawned code daemon
//! something real to read.

#![allow(dead_code)]

use std::path::Path;
use std::process::Command;

/// Build a small git repo on disk: one Rust file, one Python file, two
/// commits. Returns the SHA of HEAD. We use the `git` CLI rather than `gix`
/// to keep the fixture trivially auditable — the daemon already requires
/// `git` on `$PATH`.
pub fn build_fixture_repo(path: &Path) -> String {
    run(path, &["init", "-q", "-b", "main"]);
    run(path, &["config", "user.email", "test@example.com"]);
    run(path, &["config", "user.name", "Test"]);
    run(path, &["config", "commit.gpgsign", "false"]);

    std::fs::write(path.join("README.md"), "# Fixture repo\n\nHello world.\n").unwrap();
    std::fs::create_dir_all(path.join("src")).unwrap();
    std::fs::write(
        path.join("src").join("main.rs"),
        "fn main() {\n    println!(\"hello, world\");\n    helper();\n}\n\nfn helper() {\n    let needle = 42;\n    let _ = needle;\n}\n",
    )
    .unwrap();
    std::fs::write(
        path.join("src").join("lib.py"),
        "def helper():\n    needle = 42\n    return needle\n",
    )
    .unwrap();
    run(path, &["add", "."]);
    run(path, &["commit", "-q", "-m", "initial commit"]);
    let first_sha = head_sha(path);

    // Second commit modifying main.rs.
    std::fs::write(
        path.join("src").join("main.rs"),
        "fn main() {\n    println!(\"hello, world!\");\n    helper();\n}\n\nfn helper() {\n    let needle = 42;\n    let _ = needle;\n}\n",
    )
    .unwrap();
    run(path, &["add", "src/main.rs"]);
    run(path, &["commit", "-q", "-m", "punctuate greeting"]);

    // Tag the first commit so tests can address it by ref name.
    run(path, &["tag", "v1", &first_sha]);

    head_sha(path)
}

pub fn head_sha(path: &Path) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

pub fn rev_parse(path: &Path, refname: &str) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", refname])
        .output()
        .expect("rev-parse");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

pub fn add_remote(path: &Path, name: &str, url: &str) {
    run(path, &["remote", "add", name, url]);
}

fn run(cwd: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .expect("git command failed to spawn");
    if !status.status.success() {
        panic!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&status.stderr)
        );
    }
}
