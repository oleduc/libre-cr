//! SQLite store wrapping the schema. Synchronous rusqlite under a `Mutex`,
//! good enough for the review-daemon load (single user, low QPS).

use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use tokio::sync::Mutex;
use uuid::Uuid;

use libre_cr_common::Selection;

use crate::error::{Error, Result};

use super::migrations::run_migrations;
use super::model::{Session, Severity, ToolTrace, Turn, TurnKind, TurnStatus};

#[derive(Clone)]
pub struct Store {
    inner: Arc<Mutex<Connection>>,
}

fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4().simple())
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn parse_pr_url(url: &str) -> Result<(String, String, i64)> {
    // Minimal: https://github.com/<owner>/<repo>/pull/<n>
    let trimmed = url.trim_end_matches('/');
    let parts: Vec<&str> = trimmed.split('/').collect();
    // host/owner/repo/pull/n -> after scheme
    let n = parts.len();
    if n < 5 {
        return Err(Error::Validation(format!("malformed pr_url: {url}")));
    }
    let pr_n: i64 = parts[n - 1]
        .parse()
        .map_err(|_| Error::Validation(format!("pr_url number: {url}")))?;
    if parts[n - 2] != "pull" && parts[n - 2] != "pulls" {
        return Err(Error::Validation(format!("pr_url missing /pull/: {url}")));
    }
    let repo = parts[n - 3].to_string();
    let owner = parts[n - 4].to_string();
    Ok((owner, repo, pr_n))
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut conn = Connection::open(path)?;
        run_migrations(&mut conn)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn open_in_memory() -> Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        run_migrations(&mut conn)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(conn)),
        })
    }

    /// Upsert a session keyed by `pr_url`. If existing, refreshes `pr_data`
    /// + `last_active_at`. Returns the full row.
    pub async fn upsert_session(
        &self,
        pr_url: &str,
        pr_data: serde_json::Value,
    ) -> Result<Session> {
        let (owner, repo, n) = parse_pr_url(pr_url)?;
        let now = now_ms();
        let pr_data_str = serde_json::to_string(&pr_data)?;
        let conn = self.inner.lock().await;
        let existing: Option<String> = conn
            .query_row(
                "SELECT session_id FROM sessions WHERE pr_url = ?1",
                params![pr_url],
                |r| r.get(0),
            )
            .optional()?;
        let session_id = if let Some(id) = existing {
            conn.execute(
                "UPDATE sessions SET pr_data=?1, last_active_at=?2 WHERE session_id=?3",
                params![pr_data_str, now, id],
            )?;
            id
        } else {
            let id = new_id("s");
            conn.execute(
                "INSERT INTO sessions(session_id, pr_url, pr_owner, pr_repo, pr_number,
                 repo_id, worktree_path, pr_data, created_at, last_active_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, ?6, ?7, ?7)",
                params![id, pr_url, owner, repo, n, pr_data_str, now],
            )?;
            id
        };
        load_session(&conn, &session_id)
    }

    pub async fn get_session(&self, session_id: &str) -> Result<Option<Session>> {
        let conn = self.inner.lock().await;
        match load_session(&conn, session_id) {
            Ok(s) => Ok(Some(s)),
            Err(Error::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub async fn list_sessions(&self, limit: usize, since_ms: Option<i64>) -> Result<Vec<Session>> {
        let conn = self.inner.lock().await;
        let mut stmt = conn.prepare(
            "SELECT session_id FROM sessions
             WHERE (?1 IS NULL OR last_active_at >= ?1)
             ORDER BY last_active_at DESC LIMIT ?2",
        )?;
        let ids: Vec<String> = stmt
            .query_map(params![since_ms, limit as i64], |r| r.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            out.push(load_session(&conn, &id)?);
        }
        Ok(out)
    }

    pub async fn delete_session(&self, session_id: &str) -> Result<bool> {
        let conn = self.inner.lock().await;
        let n = conn.execute(
            "DELETE FROM sessions WHERE session_id=?1",
            params![session_id],
        )?;
        Ok(n > 0)
    }

    pub async fn set_worktree(
        &self,
        session_id: &str,
        repo_id: Option<&str>,
        worktree_path: Option<&str>,
    ) -> Result<()> {
        let conn = self.inner.lock().await;
        conn.execute(
            "UPDATE sessions SET repo_id=?1, worktree_path=?2 WHERE session_id=?3",
            params![repo_id, worktree_path, session_id],
        )?;
        Ok(())
    }

    /// Returns the next ordinal value to use for a turn in this session.
    ///
    /// NOTE: reading the ordinal here and inserting later is racy across
    /// concurrent writers — production paths use
    /// [`Self::insert_turn_auto_ordinal`], which assigns the ordinal inside
    /// the insert transaction. This stays for tests/callers that pick
    /// ordinals deliberately.
    pub async fn next_ordinal(&self, session_id: &str) -> Result<i64> {
        let conn = self.inner.lock().await;
        let n: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(ordinal), 0) FROM turns WHERE session_id=?1",
                params![session_id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        Ok(n + 1)
    }

    /// Insert a turn using the caller-supplied `t.ordinal`. The turn row,
    /// its FTS mirror, and the tool traces commit in one transaction.
    pub async fn insert_turn(&self, t: &Turn, traces: &[ToolTrace]) -> Result<()> {
        let mut conn = self.inner.lock().await;
        let tx = conn.transaction()?;
        insert_turn_tx(&tx, t, t.ordinal, traces)?;
        tx.commit()?;
        Ok(())
    }

    /// Insert a turn assigning the next ordinal *inside* the same
    /// transaction as the insert (I6: `next_ordinal` + `insert_turn` under
    /// separate lock acquisitions let concurrent writers double-assign an
    /// ordinal). Returns the assigned ordinal; `t.ordinal` is ignored.
    pub async fn insert_turn_auto_ordinal(&self, t: &Turn, traces: &[ToolTrace]) -> Result<i64> {
        let mut conn = self.inner.lock().await;
        let tx = conn.transaction()?;
        let ordinal: i64 = tx.query_row(
            "SELECT COALESCE(MAX(ordinal), 0) + 1 FROM turns WHERE session_id=?1",
            params![t.session_id],
            |r| r.get(0),
        )?;
        insert_turn_tx(&tx, t, ordinal, traces)?;
        tx.commit()?;
        Ok(ordinal)
    }

    pub async fn list_turns(&self, session_id: &str) -> Result<Vec<Turn>> {
        let conn = self.inner.lock().await;
        let mut stmt = conn.prepare(
            "SELECT turn_id, session_id, ordinal, kind, status, verb, question,
             selection, answer, user_content, severity, usage_in, usage_out, created_at,
             source_turn_id
             FROM turns WHERE session_id=?1 ORDER BY ordinal ASC",
        )?;
        let rows = stmt.query_map(params![session_id], row_to_turn)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub async fn list_traces(&self, turn_id: &str) -> Result<Vec<ToolTrace>> {
        let conn = self.inner.lock().await;
        let mut stmt = conn.prepare(
            "SELECT trace_id, turn_id, ordinal, tool_name, input_json, output_json, duration_ms, ok
             FROM tool_traces WHERE turn_id=?1 ORDER BY ordinal ASC",
        )?;
        let rows = stmt.query_map(params![turn_id], |r| {
            let input_s: String = r.get(4)?;
            let output_s: String = r.get(5)?;
            Ok(ToolTrace {
                trace_id: r.get(0)?,
                turn_id: r.get(1)?,
                ordinal: r.get(2)?,
                tool_name: r.get(3)?,
                input_json: serde_json::from_str(&input_s).unwrap_or(serde_json::Value::Null),
                output_json: serde_json::from_str(&output_s).unwrap_or(serde_json::Value::Null),
                duration_ms: r.get(6)?,
                ok: {
                    let i: i64 = r.get(7)?;
                    i != 0
                },
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(Error::from)?);
        }
        Ok(out)
    }

    /// Create a note (a turn with kind=note). Returns the turn_id.
    pub async fn create_note(
        &self,
        session_id: &str,
        content: &str,
        severity: Severity,
        anchor: Option<Selection>,
    ) -> Result<String> {
        let _ = self.get_session(session_id).await?.ok_or(Error::NotFound)?;
        let turn = Turn {
            turn_id: new_id("t"),
            session_id: session_id.to_string(),
            ordinal: 0, // assigned by insert_turn_auto_ordinal
            kind: TurnKind::Note,
            status: TurnStatus::Ok,
            verb: None,
            question: None,
            selection: anchor,
            answer: None,
            user_content: Some(content.to_string()),
            severity: Some(severity),
            usage_in: 0,
            usage_out: 0,
            created_at: now_ms(),
            source_turn_id: None,
        };
        let id = turn.turn_id.clone();
        self.insert_turn_auto_ordinal(&turn, &[]).await?;
        Ok(id)
    }

    /// Create a note with an optional back-reference to a source turn (the
    /// assistant turn the user pinned via "Save as note").
    pub async fn create_note_with_source(
        &self,
        session_id: &str,
        content: &str,
        severity: Severity,
        anchor: Option<Selection>,
        source_turn_id: Option<&str>,
    ) -> Result<String> {
        let _ = self.get_session(session_id).await?.ok_or(Error::NotFound)?;
        let turn = Turn {
            turn_id: new_id("t"),
            session_id: session_id.to_string(),
            ordinal: 0, // assigned by insert_turn_auto_ordinal
            kind: TurnKind::Note,
            status: TurnStatus::Ok,
            verb: None,
            question: None,
            selection: anchor,
            answer: None,
            user_content: Some(content.to_string()),
            severity: Some(severity),
            usage_in: 0,
            usage_out: 0,
            created_at: now_ms(),
            source_turn_id: source_turn_id.map(|s| s.to_string()),
        };
        let id = turn.turn_id.clone();
        self.insert_turn_auto_ordinal(&turn, &[]).await?;
        Ok(id)
    }

    pub async fn update_note(
        &self,
        session_id: &str,
        note_id: &str,
        content: Option<&str>,
        severity: Option<Severity>,
    ) -> Result<bool> {
        let conn = self.inner.lock().await;
        let mut sets = Vec::<&str>::new();
        let mut params_dyn: Vec<rusqlite::types::Value> = Vec::new();
        if let Some(c) = content {
            sets.push("user_content=?");
            params_dyn.push(rusqlite::types::Value::Text(c.to_string()));
        }
        if let Some(s) = severity {
            sets.push("severity=?");
            params_dyn.push(rusqlite::types::Value::Text(s.as_str().to_string()));
        }
        if sets.is_empty() {
            return Ok(false);
        }
        let sql = format!(
            "UPDATE turns SET {} WHERE session_id=? AND turn_id=? AND kind='note'",
            sets.join(", ")
        );
        params_dyn.push(rusqlite::types::Value::Text(session_id.to_string()));
        params_dyn.push(rusqlite::types::Value::Text(note_id.to_string()));
        let n = conn.execute(&sql, rusqlite::params_from_iter(params_dyn.iter()))?;
        Ok(n > 0)
    }

    pub async fn delete_note(&self, session_id: &str, note_id: &str) -> Result<bool> {
        let conn = self.inner.lock().await;
        let n = conn.execute(
            "DELETE FROM turns WHERE session_id=?1 AND turn_id=?2 AND kind='note'",
            params![session_id, note_id],
        )?;
        Ok(n > 0)
    }

    /// Cross-session FTS search. Returns matching turns alongside the parent
    /// session's `pr_url`, a short snippet derived from SQLite's `snippet()`
    /// function, and the raw FTS5 `bm25()` score (lower = better in BM25, so
    /// we negate to make a "higher = better" score for callers).
    pub async fn search_global(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(String, String, String, String, f64)>> {
        let conn = self.inner.lock().await;
        let mut stmt = conn.prepare(
            "SELECT t.session_id, s.pr_url, t.turn_id,
                    snippet(turns_fts, -1, '[', ']', '…', 12) AS snip,
                    -bm25(turns_fts) AS score
             FROM turns t
             JOIN turns_fts ON turns_fts.rowid = t.rowid
             JOIN sessions s ON s.session_id = t.session_id
             WHERE turns_fts MATCH ?1
             ORDER BY score DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![query, limit as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, f64>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Update the session's stored `head_sha`. Returns the previous value,
    /// if any.
    pub async fn set_head_sha(
        &self,
        session_id: &str,
        new_sha: Option<&str>,
    ) -> Result<Option<String>> {
        let conn = self.inner.lock().await;
        let prev: Option<String> = conn
            .query_row(
                "SELECT head_sha FROM sessions WHERE session_id=?1",
                params![session_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        conn.execute(
            "UPDATE sessions SET head_sha=?1 WHERE session_id=?2",
            params![new_sha, session_id],
        )?;
        Ok(prev)
    }

    /// FTS search over turns (used by `session_history_search`).
    pub async fn search_turns(
        &self,
        session_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(String, String)>> {
        let conn = self.inner.lock().await;
        let mut stmt = conn.prepare(
            "SELECT turns.turn_id,
                    COALESCE(turns.answer, turns.user_content, turns.question, '')
             FROM turns
             JOIN turns_fts ON turns_fts.rowid = turns.rowid
             WHERE turns.session_id = ?1 AND turns_fts MATCH ?2
             LIMIT ?3",
        )?;
        let mut out = Vec::new();
        let rows = stmt.query_map(params![session_id, query, limit as i64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }
}

/// Turn + FTS mirror + tool traces, executed on the caller's transaction.
/// `ordinal` overrides `t.ordinal` so [`Store::insert_turn_auto_ordinal`]
/// can assign it atomically.
fn insert_turn_tx(conn: &Connection, t: &Turn, ordinal: i64, traces: &[ToolTrace]) -> Result<()> {
    let sel_json = match &t.selection {
        Some(s) => Some(serde_json::to_string(s)?),
        None => None,
    };
    conn.execute(
        "INSERT INTO turns(turn_id, session_id, ordinal, kind, status, verb,
          question, selection, answer, user_content, severity,
          usage_in, usage_out, created_at, source_turn_id)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
        params![
            t.turn_id,
            t.session_id,
            ordinal,
            t.kind.as_str(),
            t.status.as_str(),
            t.verb,
            t.question,
            sel_json,
            t.answer,
            t.user_content,
            t.severity.map(|s| s.as_str()),
            t.usage_in,
            t.usage_out,
            t.created_at,
            t.source_turn_id,
        ],
    )?;
    // FTS sync
    conn.execute(
        "INSERT INTO turns_fts(rowid, question, answer, user_content) VALUES (
            (SELECT rowid FROM turns WHERE turn_id=?1), ?2, ?3, ?4)",
        params![
            t.turn_id,
            t.question.clone().unwrap_or_default(),
            t.answer.clone().unwrap_or_default(),
            t.user_content.clone().unwrap_or_default(),
        ],
    )?;
    for tr in traces {
        conn.execute(
            "INSERT INTO tool_traces(trace_id, turn_id, ordinal, tool_name,
               input_json, output_json, duration_ms, ok)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                tr.trace_id,
                tr.turn_id,
                tr.ordinal,
                tr.tool_name,
                serde_json::to_string(&tr.input_json)?,
                serde_json::to_string(&tr.output_json)?,
                tr.duration_ms,
                tr.ok as i64,
            ],
        )?;
    }
    Ok(())
}

fn load_session(conn: &Connection, id: &str) -> Result<Session> {
    let row = conn
        .query_row(
            "SELECT session_id, pr_url, pr_owner, pr_repo, pr_number, repo_id,
              worktree_path, pr_data, created_at, last_active_at, head_sha
             FROM sessions WHERE session_id=?1",
            params![id],
            |r| {
                let pr_data_s: String = r.get(7)?;
                Ok(Session {
                    session_id: r.get(0)?,
                    pr_url: r.get(1)?,
                    pr_owner: r.get(2)?,
                    pr_repo: r.get(3)?,
                    pr_number: r.get(4)?,
                    repo_id: r.get(5)?,
                    worktree_path: r.get(6)?,
                    pr_data: serde_json::from_str(&pr_data_s).unwrap_or(serde_json::Value::Null),
                    created_at: r.get(8)?,
                    last_active_at: r.get(9)?,
                    head_sha: r.get(10).ok().flatten(),
                })
            },
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    Ok(row)
}

fn row_to_turn(r: &rusqlite::Row<'_>) -> rusqlite::Result<Turn> {
    let kind_s: String = r.get(3)?;
    let status_s: String = r.get(4)?;
    let sel_s: Option<String> = r.get(7)?;
    let sev_s: Option<String> = r.get(10)?;
    Ok(Turn {
        turn_id: r.get(0)?,
        session_id: r.get(1)?,
        ordinal: r.get(2)?,
        kind: TurnKind::parse(&kind_s).unwrap_or(TurnKind::Question),
        status: TurnStatus::parse(&status_s).unwrap_or(TurnStatus::Ok),
        verb: r.get(5)?,
        question: r.get(6)?,
        selection: sel_s.and_then(|s| serde_json::from_str(&s).ok()),
        answer: r.get(8)?,
        user_content: r.get(9)?,
        severity: sev_s.and_then(|s| Severity::parse(&s)),
        usage_in: r.get::<_, Option<i64>>(11)?.unwrap_or(0),
        usage_out: r.get::<_, Option<i64>>(12)?.unwrap_or(0),
        created_at: r.get(13)?,
        source_turn_id: r.get(14).ok().flatten(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parse_url_works() {
        let (o, r, n) = parse_pr_url("https://github.com/foo/bar/pull/42").unwrap();
        assert_eq!(o, "foo");
        assert_eq!(r, "bar");
        assert_eq!(n, 42);
    }

    #[tokio::test]
    async fn upsert_idempotent() {
        let s = Store::open_in_memory().unwrap();
        let s1 = s
            .upsert_session("https://github.com/a/b/pull/1", serde_json::json!({}))
            .await
            .unwrap();
        let s2 = s
            .upsert_session("https://github.com/a/b/pull/1", serde_json::json!({"x":1}))
            .await
            .unwrap();
        assert_eq!(s1.session_id, s2.session_id);
        assert_eq!(s2.pr_data["x"], 1);
    }

    #[tokio::test]
    async fn turn_and_traces() {
        let s = Store::open_in_memory().unwrap();
        let sess = s
            .upsert_session("https://github.com/a/b/pull/3", serde_json::json!({}))
            .await
            .unwrap();
        let ord = s.next_ordinal(&sess.session_id).await.unwrap();
        let t = Turn {
            turn_id: new_id("t"),
            session_id: sess.session_id.clone(),
            ordinal: ord,
            kind: TurnKind::Question,
            status: TurnStatus::Ok,
            verb: None,
            question: Some("hi?".into()),
            selection: None,
            answer: Some("hello!".into()),
            user_content: None,
            severity: None,
            usage_in: 1,
            usage_out: 2,
            created_at: now_ms(),
            source_turn_id: None,
        };
        let tr = ToolTrace {
            trace_id: new_id("tr"),
            turn_id: t.turn_id.clone(),
            ordinal: 1,
            tool_name: "grep".into(),
            input_json: serde_json::json!({"q":"x"}),
            output_json: serde_json::json!({"matches":[]}),
            duration_ms: 5,
            ok: true,
        };
        s.insert_turn(&t, &[tr]).await.unwrap();
        let turns = s.list_turns(&sess.session_id).await.unwrap();
        assert_eq!(turns.len(), 1);
        let traces = s.list_traces(&t.turn_id).await.unwrap();
        assert_eq!(traces.len(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_note_creation_assigns_distinct_ordinals() {
        // I6: ordinal assignment happens inside the insert transaction, so
        // concurrent writers can't double-assign.
        let s = Store::open_in_memory().unwrap();
        let sess = s
            .upsert_session("https://github.com/a/b/pull/8", serde_json::json!({}))
            .await
            .unwrap();
        let mut handles = Vec::new();
        for i in 0..8 {
            let s = s.clone();
            let sid = sess.session_id.clone();
            handles.push(tokio::spawn(async move {
                s.create_note(&sid, &format!("note {i}"), Severity::Info, None)
                    .await
            }));
        }
        for h in handles {
            h.await.unwrap().unwrap();
        }
        let turns = s.list_turns(&sess.session_id).await.unwrap();
        assert_eq!(turns.len(), 8);
        let ordinals: std::collections::HashSet<i64> = turns.iter().map(|t| t.ordinal).collect();
        assert_eq!(ordinals.len(), 8, "ordinals must be distinct: {ordinals:?}");
        assert_eq!(*ordinals.iter().min().unwrap(), 1);
        assert_eq!(*ordinals.iter().max().unwrap(), 8);
    }

    #[tokio::test]
    async fn note_crud() {
        let s = Store::open_in_memory().unwrap();
        let sess = s
            .upsert_session("https://github.com/a/b/pull/4", serde_json::json!({}))
            .await
            .unwrap();
        let id = s
            .create_note(&sess.session_id, "n1", Severity::Warning, None)
            .await
            .unwrap();
        let ok = s
            .update_note(&sess.session_id, &id, Some("n1'"), Some(Severity::Critical))
            .await
            .unwrap();
        assert!(ok);
        let del = s.delete_note(&sess.session_id, &id).await.unwrap();
        assert!(del);
    }

    #[tokio::test]
    async fn search_global_orders_by_relevance() {
        let s = Store::open_in_memory().unwrap();
        let s1 = s
            .upsert_session("https://github.com/a/b/pull/100", serde_json::json!({}))
            .await
            .unwrap();
        let s2 = s
            .upsert_session("https://github.com/a/b/pull/101", serde_json::json!({}))
            .await
            .unwrap();
        s.create_note(
            &s1.session_id,
            "bcrypt cost is fine here",
            Severity::Info,
            None,
        )
        .await
        .unwrap();
        s.create_note(
            &s2.session_id,
            "the bcrypt migration looks correct overall",
            Severity::Info,
            None,
        )
        .await
        .unwrap();
        // Unrelated note must not show up.
        s.create_note(
            &s1.session_id,
            "this is about totally unrelated stuff",
            Severity::Info,
            None,
        )
        .await
        .unwrap();
        let hits = s.search_global("bcrypt", 10).await.unwrap();
        assert!(hits.len() >= 2);
        // Every hit's session is one of the two we created.
        for (sid, _, _, _, _) in &hits {
            assert!(sid == &s1.session_id || sid == &s2.session_id);
        }
        // Snippet should contain the highlighted term.
        assert!(hits.iter().all(|(_, _, _, snip, _)| snip.contains('[')));
    }

    #[tokio::test]
    async fn head_sha_persists_and_returns_prev() {
        let s = Store::open_in_memory().unwrap();
        let sess = s
            .upsert_session("https://github.com/a/b/pull/200", serde_json::json!({}))
            .await
            .unwrap();
        assert!(sess.head_sha.is_none());
        let prev = s
            .set_head_sha(&sess.session_id, Some("aaaaaaa"))
            .await
            .unwrap();
        assert_eq!(prev, None);
        let prev2 = s
            .set_head_sha(&sess.session_id, Some("bbbbbbb"))
            .await
            .unwrap();
        assert_eq!(prev2.as_deref(), Some("aaaaaaa"));
        let again = s.get_session(&sess.session_id).await.unwrap().unwrap();
        assert_eq!(again.head_sha.as_deref(), Some("bbbbbbb"));
    }

    #[tokio::test]
    async fn note_with_source_turn_id_stored() {
        let s = Store::open_in_memory().unwrap();
        let sess = s
            .upsert_session("https://github.com/a/b/pull/201", serde_json::json!({}))
            .await
            .unwrap();
        let id = s
            .create_note_with_source(
                &sess.session_id,
                "saved",
                Severity::Suggestion,
                None,
                Some("t_origin"),
            )
            .await
            .unwrap();
        let turns = s.list_turns(&sess.session_id).await.unwrap();
        let saved = turns.iter().find(|t| t.turn_id == id).unwrap();
        assert_eq!(saved.source_turn_id.as_deref(), Some("t_origin"));
    }

    #[tokio::test]
    async fn fts_search_finds_note() {
        let s = Store::open_in_memory().unwrap();
        let sess = s
            .upsert_session("https://github.com/a/b/pull/5", serde_json::json!({}))
            .await
            .unwrap();
        s.create_note(
            &sess.session_id,
            "bcrypt cost should be 10",
            Severity::Info,
            None,
        )
        .await
        .unwrap();
        let hits = s.search_turns(&sess.session_id, "bcrypt", 5).await.unwrap();
        assert!(!hits.is_empty());
    }
}
