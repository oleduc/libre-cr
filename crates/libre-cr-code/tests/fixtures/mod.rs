//! Pure-Rust fixture-repo builder used by integration tests.

use std::path::Path;
use std::process::Command;

/// Initialize a small git repo at `path` with a few committed files.
/// Returns the SHA of the HEAD commit.
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

    // Second commit modifying main.rs.
    std::fs::write(
        path.join("src").join("main.rs"),
        "fn main() {\n    println!(\"hello, world!\");\n    helper();\n}\n\nfn helper() {\n    let needle = 42;\n    let _ = needle;\n}\n",
    )
    .unwrap();
    run(path, &["add", "src/main.rs"]);
    run(path, &["commit", "-q", "-m", "punctuate greeting"]);

    let out = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
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
