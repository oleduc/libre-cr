//! Tool dispatcher + per-tool registrations.

mod context;
mod fs_tools;
mod git_tools;
mod grep_tools;
mod lang_tools;
pub(crate) mod registry;
mod repo_tools;
mod stubs;
mod worktree_tools;

pub use context::ToolContext;
pub use registry::ToolRegistry;

/// An optional string argument, with `""` read as absent.
///
/// A model that means "no filter" writes the key with an empty value about as
/// often as it omits the key, and the two must not mean different things. They
/// did: `git_log {"file": ""}` filtered every commit against a path that
/// matches nothing and returned an empty history, which an agent read as "this
/// checkout has no commits" while diagnosing a stale worktree.
pub(crate) fn optional_arg(input: &serde_json::Value, key: &str) -> Option<String> {
    input
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Register all Phase 1 tools.
pub fn build_registry() -> ToolRegistry {
    let mut r = ToolRegistry::new();

    // Repo / worktree management
    r.register(repo_tools::DiscoverRepo);
    r.register(repo_tools::ScanForRepos);
    r.register(repo_tools::CloneRepo);
    r.register(worktree_tools::PrepareWorktree);
    r.register(worktree_tools::ListWorktrees);
    r.register(worktree_tools::RemoveWorktree);

    // FS reads
    r.register(fs_tools::ReadFile);
    r.register(fs_tools::ListDir);
    r.register(fs_tools::StatFile);

    // Search
    r.register(grep_tools::Grep);
    r.register(stubs::AstSearch);

    // Symbols (stubs)
    r.register(stubs::ListSymbols);
    r.register(stubs::FindDefinition);
    r.register(stubs::FindReferences);

    // Git
    r.register(git_tools::GitLog);
    r.register(git_tools::GitBlame);
    r.register(git_tools::GitShow);
    r.register(git_tools::GitDiff);

    // Lang
    r.register(lang_tools::DetectLanguages);

    r
}

#[cfg(test)]
mod tests {
    use super::optional_arg;
    use serde_json::json;

    /// `""` and an absent key mean the same thing to a caller, so they must
    /// mean the same thing here. `git_log {"file": ""}` used to filter every
    /// commit against a path matching nothing and answer with an empty
    /// history — read, reasonably, as "this checkout has no commits".
    #[test]
    fn an_empty_argument_is_no_argument() {
        let input = json!({
            "file": "",
            "ref": "   ",
            "glob": "*.rs",
            "max_count": 5,
            "paths": ["src"],
        });
        assert_eq!(optional_arg(&input, "file"), None);
        assert_eq!(optional_arg(&input, "ref"), None, "whitespace is empty too");
        assert_eq!(optional_arg(&input, "absent"), None);
        assert_eq!(optional_arg(&input, "glob"), Some("*.rs".to_string()));
        // A value the caller did provide is untouched apart from the trim.
        assert_eq!(
            optional_arg(&json!({"file": " src/main.rs "}), "file"),
            Some("src/main.rs".to_string())
        );
        // Wrong types are absent, not errors: the schema is the contract.
        assert_eq!(optional_arg(&input, "max_count"), None);
        assert_eq!(optional_arg(&input, "paths"), None);
    }
}
