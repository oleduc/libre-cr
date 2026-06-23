//! Worktree management — fetch + `git worktree add --detach`, idempotent,
//! single-flighted per `(repo_id, ref)`, with LRU eviction.

use crate::config::WorktreeConfig;
use crate::error::{ErrorCode, ToolError};
use crate::repo::registry::RepoRegistry;
use crate::util::{sanitize_ref, validate_ref};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use tokio::process::Command;
use tokio::sync::Mutex;

/// Per-(repo, ref) single-flight locks. Values are `Weak` so an entry dies
/// with its last holder; dead entries are pruned on every acquire, keeping
/// the map bounded by the number of *in-flight* prepares (I7).
type FlightMap = std::sync::Mutex<HashMap<String, Weak<Mutex<()>>>>;

pub struct WorktreeManager {
    pub root: PathBuf,
    pub config: WorktreeConfig,
    pub registry: Arc<RepoRegistry>,
    flight: FlightMap,
}

impl WorktreeManager {
    pub fn new(root: PathBuf, config: WorktreeConfig, registry: Arc<RepoRegistry>) -> Self {
        Self {
            root,
            config,
            registry,
            flight: std::sync::Mutex::new(HashMap::new()),
        }
    }

    fn lock_for(&self, key: &str) -> Arc<Mutex<()>> {
        let mut map = self.flight.lock().unwrap();
        // Prune entries whose last holder has dropped.
        map.retain(|_, w| w.strong_count() > 0);
        if let Some(existing) = map.get(key).and_then(|w| w.upgrade()) {
            return existing;
        }
        let lock = Arc::new(Mutex::new(()));
        map.insert(key.to_string(), Arc::downgrade(&lock));
        lock
    }

    #[cfg(test)]
    fn flight_len(&self) -> usize {
        self.flight.lock().unwrap().len()
    }

    /// Prepare a worktree for the given (repo_id, ref). Idempotent.
    ///
    /// If the worktree already exists, the ref is re-fetched and the worktree
    /// is reset to the fetched tip when it diverged (force-push safety, I8).
    /// The fetch is skipped only when the caller passes `expected_sha` and it
    /// already matches the worktree's `HEAD`.
    pub async fn prepare(
        &self,
        repo_id: &str,
        ref_name: &str,
        name: Option<&str>,
        expected_sha: Option<&str>,
    ) -> Result<PathBuf, ToolError> {
        validate_ref(ref_name)?;
        if let Some(sha) = expected_sha {
            validate_ref(sha)?;
        }
        let repo = self.registry.require_repo(repo_id)?;
        let key = format!("{}::{}", repo_id, ref_name);
        let lock = self.lock_for(&key);
        let _guard = lock.lock().await;

        let folder = name
            .map(|n| n.to_string())
            .unwrap_or_else(|| sanitize_ref(ref_name));
        let wt_path = self.root.join(sanitize_ref(repo_id)).join(&folder);

        std::fs::create_dir_all(self.root.join(sanitize_ref(repo_id)))?;

        // If already exists as a git worktree, refresh it instead of trusting
        // a possibly-stale checkout.
        if wt_path.join(".git").exists() {
            // Fast path: caller already knows the tip and the worktree is on
            // it — no network round-trip needed.
            if let Some(expected) = expected_sha {
                if let Ok(head) = rev_parse(&wt_path, "HEAD").await {
                    if head == expected {
                        self.registry.upsert_worktree(&wt_path, repo_id, ref_name)?;
                        return Ok(wt_path);
                    }
                }
            }
            self.fetch_ref(&repo.local_path, ref_name).await?;
            let fetched = rev_parse(&repo.local_path, "FETCH_HEAD").await?;
            let head = rev_parse(&wt_path, "HEAD").await.unwrap_or_default();
            if head != fetched {
                let reset = Command::new("git")
                    .arg("-C")
                    .arg(&wt_path)
                    .args(["reset", "--hard", &fetched])
                    .output()
                    .await?;
                if !reset.status.success() {
                    let stderr = String::from_utf8_lossy(&reset.stderr).into_owned();
                    return Err(ToolError::internal(format!(
                        "git reset --hard failed: {stderr}"
                    )));
                }
            }
            self.registry.upsert_worktree(&wt_path, repo_id, ref_name)?;
            return Ok(wt_path);
        }

        // Fetch the ref from origin.
        self.fetch_ref(&repo.local_path, ref_name).await?;

        // git worktree add --detach <path> FETCH_HEAD
        let add = Command::new("git")
            .arg("-C")
            .arg(&repo.local_path)
            .args(["worktree", "add", "--detach"])
            .arg(&wt_path)
            .arg("FETCH_HEAD")
            .output()
            .await?;
        if !add.status.success() {
            let stderr = String::from_utf8_lossy(&add.stderr).into_owned();
            return Err(ToolError::internal(format!(
                "git worktree add failed: {stderr}"
            )));
        }

        self.registry.upsert_worktree(&wt_path, repo_id, ref_name)?;
        Ok(wt_path)
    }

    async fn fetch_ref(&self, repo_path: &Path, ref_name: &str) -> Result<(), ToolError> {
        let fetch_status = Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .args(["fetch", "origin"])
            .arg(ref_name)
            .output()
            .await;

        match fetch_status {
            Ok(o) if o.status.success() => Ok(()),
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr).into_owned();
                Err(ToolError::new(
                    ErrorCode::UnknownRef,
                    format!("git fetch failed: {stderr}"),
                ))
            }
            Err(e) => Err(ToolError::new(
                ErrorCode::UnknownRef,
                format!("git fetch could not run: {e}"),
            )),
        }
    }

    /// Remove a worktree (force).
    pub async fn remove(&self, worktree_path: &Path) -> Result<(), ToolError> {
        // Find owning repo to invoke `git worktree remove`.
        let list = self.registry.list_worktrees(None)?;
        let record = list.into_iter().find(|r| r.worktree_path == worktree_path);
        if let Some(rec) = &record {
            if let Some(repo) = self.registry.get_repo(&rec.repo_id)? {
                let _ = Command::new("git")
                    .arg("-C")
                    .arg(&repo.local_path)
                    .args(["worktree", "remove", "--force"])
                    .arg(worktree_path)
                    .output()
                    .await;
            }
        }
        // Best-effort filesystem cleanup.
        if worktree_path.exists() {
            let _ = tokio::fs::remove_dir_all(worktree_path).await;
        }
        self.registry.remove_worktree(worktree_path)?;
        Ok(())
    }

    /// LRU eviction. Returns the paths that would be (or were) removed.
    /// `dry_run = true` does not actually evict.
    pub async fn evict(&self, dry_run: bool) -> Result<Vec<PathBuf>, ToolError> {
        let mut worktrees = self.registry.list_worktrees(None)?;
        // ascending by last_used_at => oldest first
        worktrees.sort_by_key(|w| w.last_used_at);

        // Directory sizing walks the filesystem — keep it off the async workers.
        let paths: Vec<PathBuf> = worktrees.iter().map(|w| w.worktree_path.clone()).collect();
        let sizes = tokio::task::spawn_blocking(move || {
            paths.iter().map(|p| dir_size(p)).collect::<Vec<u64>>()
        })
        .await
        .map_err(|e| ToolError::internal(format!("join: {e}")))?;

        let mut total: u64 = sizes.iter().sum();
        let threshold = self.config.max_total_bytes;

        let mut evicted = Vec::new();
        for (w, sz) in worktrees.into_iter().zip(sizes) {
            if total <= threshold {
                break;
            }
            evicted.push(w.worktree_path.clone());
            if !dry_run {
                let _ = self.remove(&w.worktree_path).await;
            }
            total = total.saturating_sub(sz);
        }
        Ok(evicted)
    }

    /// Background eviction task.
    pub fn spawn_eviction(self: Arc<Self>) {
        let interval = self.config.eviction_check_interval_secs;
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
                if let Err(e) = self.evict(false).await {
                    tracing::warn!(error = %e, "eviction failed");
                }
            }
        });
    }
}

async fn rev_parse(dir: &Path, refspec: &str) -> Result<String, ToolError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", refspec])
        .output()
        .await?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        return Err(ToolError::internal(format!(
            "git rev-parse {refspec} failed: {stderr}"
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn dir_size(path: &Path) -> u64 {
    walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::now_secs;
    use tempfile::TempDir;

    #[test]
    fn dir_size_basic() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a"), vec![0u8; 100]).unwrap();
        std::fs::write(tmp.path().join("b"), vec![0u8; 50]).unwrap();
        assert_eq!(dir_size(tmp.path()), 150);
    }

    #[test]
    fn now_secs_monotonic() {
        let t1 = now_secs();
        std::thread::sleep(std::time::Duration::from_millis(10));
        let t2 = now_secs();
        assert!(t2 >= t1);
    }

    #[test]
    fn flight_map_prunes_dead_entries() {
        let registry = Arc::new(RepoRegistry::open_in_memory().unwrap());
        let tmp = TempDir::new().unwrap();
        let mgr = WorktreeManager::new(
            tmp.path().to_path_buf(),
            WorktreeConfig::default(),
            registry,
        );
        for i in 0..32 {
            let lock = mgr.lock_for(&format!("repo::ref-{i}"));
            drop(lock);
        }
        // Each acquire prunes dead Weaks first, so at most the latest entry
        // (whose Arc just dropped, but which hasn't been pruned yet) lingers.
        assert!(
            mgr.flight_len() <= 1,
            "flight map grew unbounded: {}",
            mgr.flight_len()
        );
    }

    /// End-to-end git fixture: an "origin" repo and a clone the manager
    /// fetches through, the way the daemon uses registered repos.
    struct GitFixture {
        _tmp: TempDir,
        origin: PathBuf,
        manager: WorktreeManager,
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "Tester")
            .env("GIT_AUTHOR_EMAIL", "tester@example.com")
            .env("GIT_COMMITTER_NAME", "Tester")
            .env("GIT_COMMITTER_EMAIL", "tester@example.com")
            .output()
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn fixture() -> GitFixture {
        let tmp = TempDir::new().unwrap();
        let origin = tmp.path().join("origin");
        std::fs::create_dir_all(&origin).unwrap();
        git(&origin, &["init", "-q", "-b", "main"]);
        std::fs::write(origin.join("file.txt"), "v1\n").unwrap();
        git(&origin, &["add", "."]);
        git(&origin, &["commit", "-q", "-m", "v1"]);

        let clone = tmp.path().join("clone");
        git(
            tmp.path(),
            &["clone", "-q", origin.to_str().unwrap(), "clone"],
        );

        let registry = Arc::new(RepoRegistry::open_in_memory().unwrap());
        registry
            .upsert_repo("example.com/t/r", &clone, &[])
            .unwrap();
        let manager = WorktreeManager::new(
            tmp.path().join("worktrees"),
            WorktreeConfig::default(),
            registry,
        );
        GitFixture {
            _tmp: tmp,
            origin,
            manager,
        }
    }

    #[tokio::test]
    async fn prepare_refetches_when_branch_advances() {
        let fx = fixture();
        let wt = fx
            .manager
            .prepare("example.com/t/r", "main", None, None)
            .await
            .unwrap();
        let first_head = rev_parse(&wt, "HEAD").await.unwrap();
        assert_eq!(first_head, rev_parse(&fx.origin, "HEAD").await.unwrap());

        // Advance the branch on origin (same shape as a force-push: the
        // worktree's HEAD no longer matches the remote tip).
        std::fs::write(fx.origin.join("file.txt"), "v2\n").unwrap();
        git(&fx.origin, &["add", "."]);
        git(&fx.origin, &["commit", "-q", "-m", "v2"]);
        let new_tip = rev_parse(&fx.origin, "HEAD").await.unwrap();
        assert_ne!(new_tip, first_head);

        let wt2 = fx
            .manager
            .prepare("example.com/t/r", "main", None, None)
            .await
            .unwrap();
        assert_eq!(wt, wt2, "prepare must stay idempotent on the path");
        let second_head = rev_parse(&wt2, "HEAD").await.unwrap();
        assert_eq!(second_head, new_tip, "worktree HEAD must follow the ref");
        assert_eq!(
            std::fs::read_to_string(wt2.join("file.txt")).unwrap(),
            "v2\n"
        );
    }

    #[tokio::test]
    async fn prepare_skips_fetch_when_expected_sha_matches() {
        let fx = fixture();
        let wt = fx
            .manager
            .prepare("example.com/t/r", "main", None, None)
            .await
            .unwrap();
        let head = rev_parse(&wt, "HEAD").await.unwrap();

        // Advance origin; with a matching expected_sha the fast path must
        // serve the existing checkout without fetching.
        std::fs::write(fx.origin.join("file.txt"), "v2\n").unwrap();
        git(&fx.origin, &["add", "."]);
        git(&fx.origin, &["commit", "-q", "-m", "v2"]);

        let wt2 = fx
            .manager
            .prepare("example.com/t/r", "main", None, Some(&head))
            .await
            .unwrap();
        assert_eq!(wt, wt2);
        assert_eq!(rev_parse(&wt2, "HEAD").await.unwrap(), head);

        // A stale expected_sha falls through to the re-fetch path.
        let stale = "0123456789012345678901234567890123456789";
        let wt3 = fx
            .manager
            .prepare("example.com/t/r", "main", None, Some(stale))
            .await
            .unwrap();
        let new_tip = rev_parse(&fx.origin, "HEAD").await.unwrap();
        assert_eq!(rev_parse(&wt3, "HEAD").await.unwrap(), new_tip);
    }
}
