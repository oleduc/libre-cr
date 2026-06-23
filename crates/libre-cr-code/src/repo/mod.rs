//! Repo registry, remote-URL canonicalization, and worktree management.

pub mod registry;
pub mod remote_url;
pub mod worktree;

pub use registry::RepoRegistry;
pub use worktree::WorktreeManager;
