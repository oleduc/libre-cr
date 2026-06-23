//! Inline migration definitions. The daemon refuses to start against a
//! newer schema than it knows.

use rusqlite::Connection;

use crate::error::{Error, Result};

pub const SCHEMA_VERSION: i64 = 3;

const M0001: &str = r#"
CREATE TABLE IF NOT EXISTS _schema_version (
  version INTEGER NOT NULL PRIMARY KEY
);

CREATE TABLE IF NOT EXISTS sessions (
  session_id      TEXT PRIMARY KEY,
  pr_url          TEXT NOT NULL UNIQUE,
  pr_owner        TEXT NOT NULL,
  pr_repo         TEXT NOT NULL,
  pr_number       INTEGER NOT NULL,
  repo_id         TEXT,
  worktree_path   TEXT,
  pr_data         TEXT NOT NULL,
  created_at      INTEGER NOT NULL,
  last_active_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS turns (
  turn_id      TEXT PRIMARY KEY,
  session_id   TEXT NOT NULL REFERENCES sessions(session_id) ON DELETE CASCADE,
  ordinal      INTEGER NOT NULL,
  kind         TEXT NOT NULL,
  status       TEXT NOT NULL,
  verb         TEXT,
  question     TEXT,
  selection    TEXT,
  answer       TEXT,
  user_content TEXT,
  severity     TEXT,
  usage_in     INTEGER,
  usage_out    INTEGER,
  created_at   INTEGER NOT NULL,
  UNIQUE (session_id, ordinal)
);

CREATE TABLE IF NOT EXISTS tool_traces (
  trace_id     TEXT PRIMARY KEY,
  turn_id      TEXT NOT NULL REFERENCES turns(turn_id) ON DELETE CASCADE,
  ordinal      INTEGER NOT NULL,
  tool_name    TEXT NOT NULL,
  input_json   TEXT NOT NULL,
  output_json  TEXT NOT NULL,
  duration_ms  INTEGER NOT NULL,
  ok           INTEGER NOT NULL,
  UNIQUE (turn_id, ordinal)
);

CREATE VIRTUAL TABLE IF NOT EXISTS turns_fts USING fts5(
  question, answer, user_content,
  content='turns', content_rowid='rowid'
);
"#;

// 0002 — note `source_turn_id` back-reference, additive.
const M0002: &str = r#"
ALTER TABLE turns ADD COLUMN source_turn_id TEXT;
"#;

// 0003 — session `head_sha` tracking for diff-change detection, additive.
const M0003: &str = r#"
ALTER TABLE sessions ADD COLUMN head_sha TEXT;
"#;

pub fn run_migrations(conn: &mut Connection) -> Result<()> {
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    // Read current version (0 if none)
    let current: i64 = {
        // Ensure _schema_version exists first (idempotent)
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS _schema_version (version INTEGER NOT NULL PRIMARY KEY);",
        )?;
        conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM _schema_version",
            [],
            |r| r.get(0),
        )?
    };
    if current > SCHEMA_VERSION {
        return Err(Error::Internal(format!(
            "database schema {current} is newer than this binary supports ({SCHEMA_VERSION})"
        )));
    }
    if current < 1 {
        let tx = conn.transaction()?;
        tx.execute_batch(M0001)?;
        tx.execute("INSERT INTO _schema_version (version) VALUES (?1)", [1])?;
        tx.commit()?;
    }
    if current < 2 {
        let tx = conn.transaction()?;
        tx.execute_batch(M0002)?;
        tx.execute("INSERT INTO _schema_version (version) VALUES (?1)", [2])?;
        tx.commit()?;
    }
    if current < 3 {
        let tx = conn.transaction()?;
        tx.execute_batch(M0003)?;
        tx.execute("INSERT INTO _schema_version (version) VALUES (?1)", [3])?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_from_empty() {
        let mut c = Connection::open_in_memory().unwrap();
        run_migrations(&mut c).unwrap();
        let v: i64 = c
            .query_row("SELECT MAX(version) FROM _schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
        // Idempotent
        run_migrations(&mut c).unwrap();
    }

    #[test]
    fn refuses_newer_schema() {
        let mut c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE _schema_version (version INTEGER NOT NULL PRIMARY KEY);")
            .unwrap();
        c.execute("INSERT INTO _schema_version (version) VALUES (?1)", [999])
            .unwrap();
        let r = run_migrations(&mut c);
        assert!(r.is_err());
    }
}
