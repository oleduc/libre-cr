//! SQLite-backed repo registry. Schema per `03-code-daemon.md` § Repo registry.

use crate::error::{ErrorCode, ToolError};
use crate::repo::remote_url::canonicalize_remote_url;
use crate::util::now_secs;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Current schema version. The daemon refuses to open a database stamped
/// with a newer version than it knows about.
pub const SCHEMA_VERSION: i64 = 1;

const M0001: &str = r#"
CREATE TABLE IF NOT EXISTS migrations (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    applied_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS repos (
    repo_id      TEXT PRIMARY KEY,
    local_path   TEXT NOT NULL,
    registered_at INTEGER NOT NULL,
    last_used_at  INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS repo_remotes (
    repo_id      TEXT NOT NULL,
    remote_url   TEXT NOT NULL,
    PRIMARY KEY (repo_id, remote_url),
    FOREIGN KEY (repo_id) REFERENCES repos(repo_id)
);
CREATE TABLE IF NOT EXISTS worktrees (
    worktree_path TEXT PRIMARY KEY,
    repo_id       TEXT NOT NULL,
    ref_name      TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    last_used_at  INTEGER NOT NULL,
    FOREIGN KEY (repo_id) REFERENCES repos(repo_id)
);
INSERT OR IGNORE INTO migrations (id, name, applied_at)
    VALUES (1, '0001_initial', strftime('%s','now'));
"#;

pub struct RepoRegistry {
    conn: Mutex<Connection>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct RepoRecord {
    pub repo_id: String,
    pub local_path: PathBuf,
    pub registered_at: i64,
    pub last_used_at: i64,
    pub remotes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct WorktreeRecord {
    pub worktree_path: PathBuf,
    pub repo_id: String,
    pub ref_name: String,
    pub created_at: i64,
    pub last_used_at: i64,
}

impl RepoRegistry {
    pub fn open(db_path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)?;
        let reg = Self {
            conn: Mutex::new(conn),
        };
        reg.migrate()?;
        Ok(reg)
    }

    /// In-memory registry — used by tests.
    #[allow(dead_code)]
    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        let reg = Self {
            conn: Mutex::new(conn),
        };
        reg.migrate()?;
        Ok(reg)
    }

    fn migrate(&self) -> anyhow::Result<()> {
        let mut conn = self.conn.lock().unwrap();
        // Versioned, forward-refusing migrations — same pattern as the
        // review daemon's `storage::migrations`.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS _schema_version (version INTEGER NOT NULL PRIMARY KEY);",
        )?;
        let current: i64 = conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM _schema_version",
            [],
            |r| r.get(0),
        )?;
        if current > SCHEMA_VERSION {
            anyhow::bail!(
                "repo registry schema {current} is newer than this binary supports ({SCHEMA_VERSION})"
            );
        }
        if current < 1 {
            let tx = conn.transaction()?;
            tx.execute_batch(M0001)?;
            tx.execute("INSERT INTO _schema_version (version) VALUES (?1)", [1])?;
            tx.commit()?;
        }
        Ok(())
    }

    /// Register or update a repo. Returns the canonical `repo_id`.
    pub fn upsert_repo(
        &self,
        repo_id: &str,
        local_path: &Path,
        remotes: &[String],
    ) -> Result<(), ToolError> {
        let now = now_secs();
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO repos (repo_id, local_path, registered_at, last_used_at)
             VALUES (?1, ?2, ?3, ?3)
             ON CONFLICT(repo_id) DO UPDATE SET local_path = excluded.local_path, last_used_at = excluded.last_used_at",
            params![repo_id, local_path.to_string_lossy().to_string(), now],
        )?;
        for raw in remotes {
            if let Some(canon) = canonicalize_remote_url(raw) {
                tx.execute(
                    "INSERT OR IGNORE INTO repo_remotes (repo_id, remote_url) VALUES (?1, ?2)",
                    params![repo_id, canon],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Look up a repo by remote URL (canonicalized).
    pub fn find_by_remote(&self, remote_url: &str) -> Result<Option<RepoRecord>, ToolError> {
        let canon = canonicalize_remote_url(remote_url).ok_or_else(|| {
            ToolError::invalid(format!("could not parse remote url: {remote_url}"))
        })?;
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT r.repo_id, r.local_path, r.registered_at, r.last_used_at
             FROM repos r
             JOIN repo_remotes m ON m.repo_id = r.repo_id
             WHERE m.remote_url = ?1
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![canon])?;
        if let Some(row) = rows.next()? {
            let repo_id: String = row.get(0)?;
            let local_path: String = row.get(1)?;
            let registered_at: i64 = row.get(2)?;
            let last_used_at: i64 = row.get(3)?;
            drop(rows);
            drop(stmt);
            let remotes = remotes_for(&conn, &repo_id)?;
            return Ok(Some(RepoRecord {
                repo_id,
                local_path: PathBuf::from(local_path),
                registered_at,
                last_used_at,
                remotes,
            }));
        }
        Ok(None)
    }

    pub fn get_repo(&self, repo_id: &str) -> Result<Option<RepoRecord>, ToolError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT repo_id, local_path, registered_at, last_used_at FROM repos WHERE repo_id = ?1",
        )?;
        let mut rows = stmt.query(params![repo_id])?;
        if let Some(row) = rows.next()? {
            let repo_id: String = row.get(0)?;
            let local_path: String = row.get(1)?;
            let registered_at: i64 = row.get(2)?;
            let last_used_at: i64 = row.get(3)?;
            drop(rows);
            drop(stmt);
            let remotes = remotes_for(&conn, &repo_id)?;
            return Ok(Some(RepoRecord {
                repo_id,
                local_path: PathBuf::from(local_path),
                registered_at,
                last_used_at,
                remotes,
            }));
        }
        Ok(None)
    }

    #[allow(dead_code)]
    pub fn touch_repo(&self, repo_id: &str) -> Result<(), ToolError> {
        let now = now_secs();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE repos SET last_used_at = ?1 WHERE repo_id = ?2",
            params![now, repo_id],
        )?;
        Ok(())
    }

    pub fn list_repos(&self) -> Result<Vec<RepoRecord>, ToolError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT repo_id, local_path, registered_at, last_used_at FROM repos ORDER BY repo_id",
        )?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let repo_id: String = row.get(0)?;
            let local_path: String = row.get(1)?;
            let registered_at: i64 = row.get(2)?;
            let last_used_at: i64 = row.get(3)?;
            let remotes = remotes_for(&conn, &repo_id)?;
            out.push(RepoRecord {
                repo_id,
                local_path: PathBuf::from(local_path),
                registered_at,
                last_used_at,
                remotes,
            });
        }
        Ok(out)
    }

    pub fn upsert_worktree(
        &self,
        worktree_path: &Path,
        repo_id: &str,
        ref_name: &str,
    ) -> Result<(), ToolError> {
        let now = now_secs();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO worktrees (worktree_path, repo_id, ref_name, created_at, last_used_at)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(worktree_path) DO UPDATE SET last_used_at = excluded.last_used_at",
            params![
                worktree_path.to_string_lossy().to_string(),
                repo_id,
                ref_name,
                now
            ],
        )?;
        Ok(())
    }

    pub fn list_worktrees(&self, repo_id: Option<&str>) -> Result<Vec<WorktreeRecord>, ToolError> {
        let conn = self.conn.lock().unwrap();
        let (query, has_filter) = if repo_id.is_some() {
            (
                "SELECT worktree_path, repo_id, ref_name, created_at, last_used_at
                 FROM worktrees WHERE repo_id = ?1 ORDER BY last_used_at DESC",
                true,
            )
        } else {
            (
                "SELECT worktree_path, repo_id, ref_name, created_at, last_used_at
                 FROM worktrees ORDER BY last_used_at DESC",
                false,
            )
        };
        let mut stmt = conn.prepare(query)?;
        let mut out = Vec::new();
        let mut rows = if has_filter {
            stmt.query(params![repo_id.unwrap()])?
        } else {
            stmt.query([])?
        };
        while let Some(row) = rows.next()? {
            let worktree_path: String = row.get(0)?;
            let repo_id: String = row.get(1)?;
            let ref_name: String = row.get(2)?;
            let created_at: i64 = row.get(3)?;
            let last_used_at: i64 = row.get(4)?;
            out.push(WorktreeRecord {
                worktree_path: PathBuf::from(worktree_path),
                repo_id,
                ref_name,
                created_at,
                last_used_at,
            });
        }
        Ok(out)
    }

    pub fn remove_worktree(&self, worktree_path: &Path) -> Result<(), ToolError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM worktrees WHERE worktree_path = ?1",
            params![worktree_path.to_string_lossy().to_string()],
        )?;
        Ok(())
    }

    pub fn require_repo(&self, repo_id: &str) -> Result<RepoRecord, ToolError> {
        self.get_repo(repo_id)?.ok_or_else(|| {
            ToolError::new(
                ErrorCode::UnknownRepo,
                format!("repo not registered: {repo_id}"),
            )
        })
    }
}

fn remotes_for(conn: &Connection, repo_id: &str) -> Result<Vec<String>, ToolError> {
    let mut stmt =
        conn.prepare("SELECT remote_url FROM repo_remotes WHERE repo_id = ?1 ORDER BY remote_url")?;
    let mut rows = stmt.query(params![repo_id])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let url: String = row.get(0)?;
        out.push(url);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn upsert_and_find() {
        let reg = RepoRegistry::open_in_memory().unwrap();
        let tmp = TempDir::new().unwrap();
        reg.upsert_repo(
            "github.com/foo/bar",
            tmp.path(),
            &["git@github.com:foo/bar.git".to_string()],
        )
        .unwrap();
        let r = reg.find_by_remote("https://github.com/foo/bar").unwrap();
        assert!(r.is_some());
        assert_eq!(r.unwrap().repo_id, "github.com/foo/bar");
    }

    #[test]
    fn require_repo_errors_on_missing() {
        let reg = RepoRegistry::open_in_memory().unwrap();
        let err = reg.require_repo("github.com/missing/repo").unwrap_err();
        assert_eq!(err.code, ErrorCode::UnknownRepo);
    }

    #[test]
    fn migrates_from_empty_and_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("state.db");
        {
            let reg = RepoRegistry::open(&db).unwrap();
            drop(reg);
        }
        let reg = RepoRegistry::open(&db).unwrap();
        let conn = reg.conn.lock().unwrap();
        let v: i64 = conn
            .query_row("SELECT MAX(version) FROM _schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
    }

    #[test]
    fn refuses_newer_schema() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("state.db");
        {
            let c = Connection::open(&db).unwrap();
            c.execute_batch("CREATE TABLE _schema_version (version INTEGER NOT NULL PRIMARY KEY);")
                .unwrap();
            c.execute("INSERT INTO _schema_version (version) VALUES (?1)", [99])
                .unwrap();
        }
        let err = match RepoRegistry::open(&db) {
            Ok(_) => panic!("opening a v99 database must fail"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("newer"), "unexpected error: {err}");
    }

    #[test]
    fn list_worktrees() {
        let reg = RepoRegistry::open_in_memory().unwrap();
        let tmp = TempDir::new().unwrap();
        reg.upsert_repo("github.com/foo/bar", tmp.path(), &[])
            .unwrap();
        let wt = tmp.path().join("wt-1");
        reg.upsert_worktree(&wt, "github.com/foo/bar", "main")
            .unwrap();
        let list = reg.list_worktrees(Some("github.com/foo/bar")).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].ref_name, "main");
    }
}
