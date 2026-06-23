//! Shared handles passed into every tool.

use crate::config::Config;
use crate::repo::{RepoRegistry, WorktreeManager};
use std::path::PathBuf;
use std::sync::Arc;

pub struct ToolContext {
    pub config: Config,
    pub data_dir: PathBuf,
    pub registry: Arc<RepoRegistry>,
    pub worktrees: Arc<WorktreeManager>,
}

impl ToolContext {
    pub fn new(config: Config) -> anyhow::Result<Arc<Self>> {
        let data_dir = config.storage.data_dir_path();
        std::fs::create_dir_all(&data_dir)?;
        let db_path = config.storage.state_db_path();
        let registry = Arc::new(RepoRegistry::open(&db_path)?);
        let worktree_root = data_dir.join("worktrees");
        std::fs::create_dir_all(&worktree_root)?;
        let wt = Arc::new(WorktreeManager::new(
            worktree_root,
            config.worktrees.clone(),
            registry.clone(),
        ));
        Ok(Arc::new(Self {
            config,
            data_dir,
            registry,
            worktrees: wt,
        }))
    }
}
