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
