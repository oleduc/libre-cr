//! Export logic per `07-conversation-and-notes.md` § Export.

use serde::{Deserialize, Serialize};

// Wire shapes live in `libre-cr-common` so the extension mirror has a single
// Rust source of truth; re-exported here for existing call sites.
pub use libre_cr_common::http_api::{ExportResponse, GithubInlineComment, GithubReviewStructure};

use crate::error::Result;
use crate::storage::{Session, Severity, Store, ToolTrace, Turn, TurnKind};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Markdown,
    GithubReview,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExportFilter {
    #[serde(default)]
    pub include_thinking: bool,
    #[serde(default)]
    pub severity_min: Option<Severity>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExportRequest {
    pub format: ExportFormat,
    #[serde(default)]
    pub filter: Option<ExportFilter>,
}

pub async fn build_export(
    store: &Store,
    session: &Session,
    req: &ExportRequest,
) -> Result<ExportResponse> {
    let turns = store.list_turns(&session.session_id).await?;
    let filter = req.filter.clone().unwrap_or_default();
    let notes: Vec<&Turn> = turns
        .iter()
        .filter(|t| t.kind == TurnKind::Note)
        .filter(|t| match (filter.severity_min, t.severity) {
            (Some(min), Some(s)) => s >= min,
            (Some(_), None) => false,
            _ => true,
        })
        .collect();

    let title = session
        .pr_data
        .get("metadata")
        .and_then(|m| m.get("title"))
        .and_then(|s| s.as_str())
        .unwrap_or(session.pr_url.as_str());
    let pr_num = session.pr_number;

    match req.format {
        ExportFormat::Markdown => {
            let mut md = String::new();
            md.push_str(&format!("# Review: PR #{pr_num} — {title}\n\n"));
            for sev in Severity::export_order() {
                let group: Vec<&&Turn> = notes.iter().filter(|t| t.severity == Some(sev)).collect();
                if group.is_empty() {
                    continue;
                }
                md.push_str(&format!("## {}\n\n", sev.group_heading()));
                let mut sorted = group.clone();
                sorted.sort_by_key(|t| t.created_at);
                for n in sorted {
                    let content = n
                        .user_content
                        .clone()
                        .unwrap_or_default()
                        .replace('\n', "\n  ");
                    let anchor = n.selection.as_ref().map(format_anchor).unwrap_or_default();
                    if anchor.is_empty() {
                        md.push_str(&format!("- {content}\n"));
                    } else {
                        md.push_str(&format!("- `{anchor}` — {content}\n"));
                    }
                }
                md.push('\n');
            }
            if filter.include_thinking {
                md.push_str("\n## Investigation context\n\n");
                for t in &turns {
                    if t.kind != TurnKind::Question {
                        continue;
                    }
                    md.push_str(&format!(
                        "### Q: {}\n\nA: {}\n\n",
                        t.question.clone().unwrap_or_default(),
                        t.answer.clone().unwrap_or_default()
                    ));
                    let traces = store.list_traces(&t.turn_id).await?;
                    if !traces.is_empty() {
                        md.push_str("<details><summary>tools</summary>\n\n");
                        for tr in traces {
                            push_trace(&mut md, &tr);
                        }
                        md.push_str("</details>\n\n");
                    }
                }
            }
            md.push_str("---\nReviewed with libre-cr.\n");
            Ok(ExportResponse {
                content: md,
                structure: None,
            })
        }
        ExportFormat::GithubReview => {
            let mut event = "COMMENT";
            let body = {
                let mut out = String::new();
                for sev in Severity::export_order() {
                    let group: Vec<&&Turn> =
                        notes.iter().filter(|t| t.severity == Some(sev)).collect();
                    if group.is_empty() {
                        continue;
                    }
                    out.push_str(&format!("**{}**\n\n", sev.group_heading()));
                    let mut sorted = group.clone();
                    sorted.sort_by_key(|t| t.created_at);
                    for n in sorted {
                        if sev == Severity::Critical {
                            event = "REQUEST_CHANGES";
                        }
                        out.push_str(&format!(
                            "- {}\n",
                            n.user_content.clone().unwrap_or_default()
                        ));
                    }
                    out.push('\n');
                }
                out
            };
            let mut comments = Vec::new();
            for n in &notes {
                if let Some(anchor) = n.selection.as_ref() {
                    let (path, line) = anchor_path_line(anchor);
                    comments.push(GithubInlineComment {
                        path,
                        line,
                        body: n.user_content.clone().unwrap_or_default(),
                    });
                }
            }
            let structure = GithubReviewStructure {
                body: body.clone(),
                event: event.to_string(),
                comments,
            };
            Ok(ExportResponse {
                content: body,
                structure: Some(structure),
            })
        }
    }
}

fn format_anchor(sel: &libre_cr_common::Selection) -> String {
    use libre_cr_common::Selection::*;
    match sel {
        Line { file, line } => format!("{file}:{line}"),
        Range {
            file,
            start_line,
            end_line,
        } => format!("{file}:{start_line}-{end_line}"),
        Symbol {
            file,
            line,
            identifier,
            ..
        } => format!("{file}:{line} ({identifier})"),
    }
}

fn anchor_path_line(sel: &libre_cr_common::Selection) -> (String, Option<u32>) {
    use libre_cr_common::Selection::*;
    match sel {
        Line { file, line } => (file.clone(), Some(*line)),
        Range {
            file, start_line, ..
        } => (file.clone(), Some(*start_line)),
        Symbol { file, line, .. } => (file.clone(), Some(*line)),
    }
}

fn push_trace(md: &mut String, tr: &ToolTrace) {
    md.push_str(&format!(
        "- {} ({} ms, {})\n",
        tr.tool_name,
        tr.duration_ms,
        if tr.ok { "ok" } else { "err" }
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    use crate::storage::{TurnKind, TurnStatus};

    async fn seed_session(store: &Store) -> Session {
        store
            .upsert_session(
                "https://github.com/a/b/pull/1",
                serde_json::json!({"metadata":{"title":"feat: bcrypt"}}),
            )
            .await
            .unwrap()
    }

    async fn add_note(
        store: &Store,
        sid: &str,
        sev: Severity,
        content: &str,
        anchor: Option<libre_cr_common::Selection>,
    ) {
        let _ = store.create_note(sid, content, sev, anchor).await.unwrap();
    }

    #[tokio::test]
    async fn markdown_groups_by_severity() {
        let store = Store::open_in_memory().unwrap();
        let s = seed_session(&store).await;
        add_note(&store, &s.session_id, Severity::Info, "an info", None).await;
        add_note(&store, &s.session_id, Severity::Critical, "a crit", None).await;
        add_note(&store, &s.session_id, Severity::Warning, "a warn", None).await;
        add_note(&store, &s.session_id, Severity::Suggestion, "a sugg", None).await;

        let r = build_export(
            &store,
            &s,
            &ExportRequest {
                format: ExportFormat::Markdown,
                filter: None,
            },
        )
        .await
        .unwrap();
        let pos_crit = r.content.find("## Critical").unwrap();
        let pos_warn = r.content.find("## Warning").unwrap();
        let pos_sugg = r.content.find("## Suggestions").unwrap();
        let pos_info = r.content.find("## Info").unwrap();
        assert!(pos_crit < pos_warn);
        assert!(pos_warn < pos_sugg);
        assert!(pos_sugg < pos_info);
    }

    #[tokio::test]
    async fn github_review_event_on_critical() {
        let store = Store::open_in_memory().unwrap();
        let s = seed_session(&store).await;
        add_note(&store, &s.session_id, Severity::Critical, "x", None).await;
        let r = build_export(
            &store,
            &s,
            &ExportRequest {
                format: ExportFormat::GithubReview,
                filter: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(r.structure.as_ref().unwrap().event, "REQUEST_CHANGES");
    }

    #[tokio::test]
    async fn markdown_include_thinking_appends_traces() {
        let store = Store::open_in_memory().unwrap();
        let s = seed_session(&store).await;
        // Add one Q&A turn
        let ord = store.next_ordinal(&s.session_id).await.unwrap();
        let t = Turn {
            turn_id: format!("t_{}", Uuid::new_v4().simple()),
            session_id: s.session_id.clone(),
            ordinal: ord,
            kind: TurnKind::Question,
            status: TurnStatus::Ok,
            verb: None,
            question: Some("hi?".into()),
            selection: None,
            answer: Some("hello.".into()),
            user_content: None,
            severity: None,
            usage_in: 0,
            usage_out: 0,
            created_at: Utc::now().timestamp_millis(),
            source_turn_id: None,
        };
        store.insert_turn(&t, &[]).await.unwrap();
        let r = build_export(
            &store,
            &s,
            &ExportRequest {
                format: ExportFormat::Markdown,
                filter: Some(ExportFilter {
                    include_thinking: true,
                    severity_min: None,
                }),
            },
        )
        .await
        .unwrap();
        assert!(r.content.contains("Investigation context"));
        assert!(r.content.contains("hi?"));
    }
}
