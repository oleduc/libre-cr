//! The agent loop. Pseudo-code in `04-review-daemon.md` § Agent Loop is the
//! contract.

use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use futures::StreamExt;
use libre_cr_common::ws_frames::UsageTally;
use libre_cr_common::{Selection, Side};
use uuid::Uuid;

use crate::config::Limits;
use crate::error::{Error, Result};
use crate::provider::{ContentBlock, Message, Provider, Role, StreamEvent};
use crate::storage::{Severity, Store, ToolTrace, Turn, TurnKind, TurnStatus};
use crate::tools::{ToolCall, ToolRouter};

use super::sink::FrameSink;

/// Inputs to one `ask` turn.
pub struct TurnInput {
    pub question: String,
    pub selection: Option<Selection>,
    pub verb: Option<String>,
    /// Turn ids the client asked to replay at full fidelity (expanded in the
    /// panel). Ids not belonging to the session are ignored.
    pub context_turn_ids: Vec<String>,
}

/// Context the loop needs from the caller. Built fresh per turn.
pub struct TurnContext {
    pub session_id: String,
    pub provider: Arc<dyn Provider>,
    pub router: ToolRouter,
    pub store: Store,
    /// Every tunable that shapes the turn, straight from `[limits]` in
    /// review.toml (editable from the extension's options page).
    pub limits: Limits,
    pub global_instructions: String,
}

/// Outcome of a turn.
#[derive(Debug, Clone)]
pub struct TurnResult {
    pub turn_id: String,
    pub answer: String,
    pub status: TurnStatus,
    pub usage: UsageTally,
    pub tool_call_count: usize,
    pub wall_ms: u64,
}

fn build_system_prompt(
    global: &str,
    verb: Option<&str>,
    has_presentation: bool,
    worktree: Option<&str>,
    base_ref: Option<&str>,
) -> Message {
    let mut s = String::new();
    s.push_str(
        "You are libre-cr's review assistant. The reviewer asks questions about \
         a specific pull request; you answer with grounded references to the \
         code. Prefer concise, structured answers. Earlier answers in this \
         conversation may rest on tool results that were wrong or incomplete at \
         the time; when a fresh tool result disagrees with something said \
         before, the tool result wins — re-derive from it rather than repeating \
         the earlier claim. Ground every statement about the code in what you \
         actually read this turn: before describing, naming or proposing changes \
         to functions, classes or files, read them (get_pr_diff, read_file, \
         grep). Never invent identifiers — if you have not read it, say so \
         instead of guessing. Tool outputs from earlier turns are replayed \
         only for the most recent turns; anything older is gone from this \
         conversation. Before citing an identifier, line number or code \
         detail first seen in an earlier turn, re-read it — every identifier \
         and file:line you state must appear verbatim in a tool result you \
         can currently see.",
    );
    if let Some(path) = worktree {
        s.push_str(&format!(
            "\n\nThe PR head is already checked out at `{path}`{}. All code tools \
             (read_file, grep, git_diff, git_log, list_symbols, …) operate on that \
             checkout automatically — you never need to clone, discover or prepare \
             a repository. Use get_pr_diff for the PR's changes (pass `paths` to \
             narrow it) and read_file/grep for surrounding context. Line numbers \
             in the diff are the PR head's.",
            base_ref
                .map(|b| format!(" (base branch: `{b}`)"))
                .unwrap_or_default()
        ));
    }
    if has_presentation {
        s.push_str(
            "\n\nYou have presentation tools that affect what the reviewer sees \
             in the browser: highlight_lines, annotate_line, scroll_to, \
             open_link, clear_presentation. When the reviewer asks you to walk \
             through, point out, show or highlight parts of the PR, highlight \
             each part you describe (highlight_lines with `label` = the heading \
             you use in the answer and `detail` = your explanation of that part, \
             in the order you present them) — that is the expected deliverable, \
             not an extra; the reviewer steps through these highlights in a tour \
             widget that shows label and detail beside the code. Finish a \
             walkthrough with scroll_to on the first highlighted part so the \
             reviewer starts where you started. For plain questions, use them only when they help. The \
             answer text remains primary. If a presentation call fails for one \
             file (e.g. file_not_in_view), say so briefly and continue with the \
             others — never abandon the rest of the walkthrough over it.",
        );
    }
    if let Some(v) = verb {
        // Append the verb's addendum verbatim. Per spec § Tool Composition
        // Per Verb, tools are *suggested* not enforced — the addendum text
        // names them and the LLM decides what to call.
        if let Some(vdef) = crate::verbs::find(v) {
            s.push_str("\n\n");
            s.push_str(vdef.system_prompt);
            if !vdef.suggested_tools.is_empty() {
                s.push_str("\n\nSuggested tools for this verb: ");
                s.push_str(&vdef.suggested_tools.join(", "));
                s.push('.');
            }
        } else {
            s.push_str(&format!("\n\nVerb in use: {v}."));
        }
    }
    if !global.trim().is_empty() {
        s.push_str("\n\n");
        s.push_str(global.trim());
    }
    Message {
        role: Role::System,
        content: vec![ContentBlock::Text { text: s }],
    }
}

fn build_user_message(question: &str, selection: Option<&Selection>) -> Message {
    let mut text = String::new();
    if let Some(s) = selection {
        text.push_str(&format!(
            "[Selection: {} in {}]\n",
            match s {
                Selection::Line { line, .. } => format!("line {line}"),
                Selection::Range {
                    start_line,
                    end_line,
                    ..
                } => format!("lines {start_line}-{end_line}"),
                Selection::Symbol {
                    identifier, line, ..
                } => format!("symbol `{identifier}` (line {line})"),
                Selection::Comment { line, side, .. } => format!(
                    "review comment on line {line} ({} side)",
                    match side {
                        Side::Left => "old",
                        Side::Right => "new",
                    }
                ),
            },
            s.file()
        ));
        // The exact selected text, so "this line" can never mean another line.
        // A comment selection quotes the thread instead — the concern itself
        // is the evidence, and the anchor above says where it points.
        let snippet = match s {
            Selection::Line { text, .. }
            | Selection::Range { text, .. }
            | Selection::Symbol { text, .. } => text.clone(),
            Selection::Comment { comments, .. } => Some(
                comments
                    .iter()
                    .map(|c| format!("@{}: {}", c.author, c.body))
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            ),
        };
        let label = match s {
            Selection::Comment { .. } => "The selected review thread is",
            _ => "The selected text is",
        };
        if let Some(snippet) = snippet.filter(|t| !t.trim().is_empty()) {
            let mut clipped: String = snippet.chars().take(2000).collect();
            if snippet.chars().count() > 2000 {
                clipped.push('…');
            }
            text.push_str(&format!("{label}:\n```\n{clipped}\n```\n"));
        }
    }
    text.push_str(question);
    Message {
        role: Role::User,
        content: vec![ContentBlock::Text { text }],
    }
}

fn make_assistant_message(
    text: String,
    tool_uses: &[(String, String, serde_json::Value)],
) -> Message {
    let mut blocks = Vec::new();
    if !text.is_empty() {
        blocks.push(ContentBlock::Text { text });
    }
    for (id, name, input) in tool_uses {
        blocks.push(ContentBlock::ToolUse {
            id: id.clone(),
            name: name.clone(),
            input: input.clone(),
        });
    }
    Message {
        role: Role::Assistant,
        content: blocks,
    }
}

fn make_user_tool_results(blocks: Vec<ContentBlock>) -> Message {
    Message {
        role: Role::User,
        content: blocks,
    }
}

/// Run one turn end-to-end. Streams frames to `sink`, persists the turn,
/// returns the result. Cancellation is performed by the caller dropping the
/// returned future.
pub async fn run_turn(
    ctx: &TurnContext,
    input: TurnInput,
    sink: &dyn FrameSink,
) -> Result<TurnResult> {
    let started = Instant::now();
    let turn_id = format!("t_{}", Uuid::new_v4().simple());

    let has_presentation = ctx
        .router
        .tools_for_verb(input.verb.as_deref())
        .iter()
        .any(|t| crate::tools::presentation::PRESENTATION_TOOL_NAMES.contains(&t.name.as_str()));

    let base_ref = ctx.router.base_ref();
    let system = build_system_prompt(
        &ctx.global_instructions,
        input.verb.as_deref(),
        has_presentation,
        ctx.router.worktree_path(),
        base_ref.as_deref(),
    );
    let user_msg = build_user_message(&input.question, input.selection.as_ref());

    // Pull recent history: prose for every prior turn, plus the tool results
    // of the most recent turns verbatim so a follow-up question still has its
    // evidence in context. Older turns keep a stub naming the tools used, so
    // the model re-reads instead of reciting from memory.
    let mut messages = vec![system];
    messages.extend(
        build_history_messages(
            &ctx.store,
            &ctx.session_id,
            &ctx.limits,
            &input.context_turn_ids,
        )
        .await?,
    );
    messages.push(user_msg);

    let tools = ctx.router.tools_for_verb(input.verb.as_deref());

    let mut traces: Vec<ToolTrace> = Vec::new();
    let mut answer = String::new();
    let mut usage = UsageTally::default();
    let mut tool_call_count = 0usize;
    // Shared across every round of this turn: the per-result cap alone still
    // permits `max_tool_turns` × that cap of tool output in one request.
    let mut tool_chars_spent = 0usize;

    for _ in 0..ctx.limits.max_tool_turns {
        let mut text_buf = String::new();
        let mut tool_uses: Vec<(String, String, serde_json::Value)> = Vec::new();
        let mut stream = ctx.provider.stream(&messages, &tools).await?;
        let mut got_done = false;
        let mut last_usage = UsageTally::default();
        while let Some(ev) = stream.next().await {
            match ev? {
                StreamEvent::TextDelta { text } => {
                    sink.text_delta(&text).await?;
                    text_buf.push_str(&text);
                }
                StreamEvent::ToolUse { id, name, input } => {
                    tool_uses.push((id, name, input));
                }
                StreamEvent::Done {
                    input_tokens,
                    output_tokens,
                    ..
                } => {
                    last_usage = UsageTally {
                        input_tokens: usage.input_tokens + input_tokens,
                        output_tokens: usage.output_tokens + output_tokens,
                    };
                    got_done = true;
                    break;
                }
                StreamEvent::Error { message } => {
                    sink.error(&message, false).await?;
                    return Err(Error::Internal(message));
                }
            }
        }
        if !got_done {
            return Err(Error::Internal("provider stream ended without done".into()));
        }
        usage = last_usage;

        if tool_uses.is_empty() {
            // Final answer.
            answer.push_str(&text_buf);
            let result = persist(
                ctx,
                &turn_id,
                &input,
                &answer,
                TurnStatus::Ok,
                &traces,
                usage.clone(),
            )
            .await?;
            sink.done(&turn_id, usage.clone()).await?;
            return Ok(TurnResult {
                turn_id: result,
                answer,
                status: TurnStatus::Ok,
                usage,
                tool_call_count,
                wall_ms: started.elapsed().as_millis() as u64,
            });
        }

        // Accumulate into transcript for next provider round.
        if !text_buf.is_empty() {
            answer.push_str(&text_buf);
        }
        let assistant = make_assistant_message(text_buf, &tool_uses);
        messages.push(assistant);

        // I2: dispatch every tool call of this LLM turn concurrently.
        // `tool_call` frames go out first (original order), and the result
        // blocks fed back to the model preserve the original call order
        // (`join_all` returns outputs in input order).
        for (id, name, input_json) in &tool_uses {
            sink.tool_call(id, name, input_json.clone()).await?;
        }
        let outcomes = futures::future::join_all(tool_uses.iter().map(|(id, name, input_json)| {
            let call = ToolCall {
                id: id.clone(),
                name: name.clone(),
                input: input_json.clone(),
            };
            async move { ctx.router.dispatch(&call).await }
        }))
        .await;
        let mut tool_result_blocks = Vec::new();
        for ((id, name, input_json), outcome) in tool_uses.into_iter().zip(outcomes) {
            // Capping happens here, not in the concurrent dispatch above: the
            // turn's budget is shared state, and the frame has to report the
            // same size the model actually received.
            let (content, truncated_from) = clip_tool_result(
                &outcome.value.to_string(),
                &ctx.limits,
                &mut tool_chars_spent,
            );
            sink.tool_result(&id, outcome.value.clone(), truncated_from)
                .await?;
            traces.push(ToolTrace {
                trace_id: format!("tr_{}", Uuid::new_v4().simple()),
                turn_id: turn_id.clone(),
                ordinal: (traces.len() as i64) + 1,
                tool_name: name,
                input_json,
                // The store keeps the whole result: the cap is about what the
                // model can hold, not about what the export may show.
                output_json: outcome.value.clone(),
                duration_ms: outcome.duration_ms,
                ok: outcome.ok,
            });
            tool_call_count += 1;
            tool_result_blocks.push(ContentBlock::ToolResult {
                tool_use_id: id,
                content,
                is_error: !outcome.ok,
            });
        }
        messages.push(make_user_tool_results(tool_result_blocks));
    }
    Err(Error::TooManyToolTurns)
}

#[allow(clippy::too_many_arguments)]
async fn persist(
    ctx: &TurnContext,
    turn_id: &str,
    input: &TurnInput,
    answer: &str,
    status: TurnStatus,
    traces: &[ToolTrace],
    usage: UsageTally,
) -> Result<String> {
    let turn = Turn {
        turn_id: turn_id.to_string(),
        session_id: ctx.session_id.clone(),
        ordinal: 0, // assigned by insert_turn_auto_ordinal (I6)
        kind: TurnKind::Question,
        status,
        verb: input.verb.clone(),
        question: Some(input.question.clone()),
        selection: input.selection.clone(),
        answer: Some(answer.to_string()),
        user_content: None,
        severity: None::<Severity>,
        usage_in: usage.input_tokens as i64,
        usage_out: usage.output_tokens as i64,
        created_at: Utc::now().timestamp_millis(),
        source_turn_id: None,
    };
    ctx.store.insert_turn_auto_ordinal(&turn, traces).await?;
    Ok(turn.turn_id)
}

/// Bound one live tool result: the per-result cap first, then whatever is left
/// of the turn's shared budget. Returns what the model receives and, when
/// shortened, the original char count for the panel.
///
/// Truncating rather than dropping the result or failing the turn is the whole
/// point: the answer still goes through, and the note tells the model how to
/// fetch the rest on its next call — which fixes the overflow instead of
/// merely surviving it.
fn clip_tool_result(serialized: &str, limits: &Limits, spent: &mut usize) -> (String, Option<u64>) {
    let total = serialized.chars().count();
    let remaining = limits.max_turn_tool_chars.saturating_sub(*spent);
    let allowance = limits
        .max_tool_result_chars
        .min(remaining.max(TOOL_RESULT_FLOOR_CHARS));
    if total <= allowance {
        *spent += total;
        return (serialized.to_string(), None);
    }
    let mut content: String = serialized.chars().take(allowance).collect();
    *spent += allowance;
    if remaining <= allowance {
        content.push_str(&format!(
            "\n…[truncated: {total} chars. This turn's tool-output budget ({} chars) is spent — \
             answer from what you already have, or narrow your requests.]",
            limits.max_turn_tool_chars
        ));
    } else {
        content.push_str(&format!(
            "\n…[truncated: {total} → {allowance} chars. Narrow the request to see more: `paths` \
             on get_pr_diff, `start_line`/`end_line` on read_file.]"
        ));
    }
    (content, Some(total as u64))
}

/// The smallest head of a tool result the model still receives once a turn's
/// tool-output budget is spent. Not configurable: it exists so the loop can
/// always make progress instead of handing the model nothing, and it bounds
/// the overrun at `max_tool_turns` × this.
const TOOL_RESULT_FLOOR_CHARS: usize = 2_000;

fn clip(s: &str, max_chars: usize) -> (String, bool) {
    if s.chars().count() <= max_chars {
        (s.to_string(), false)
    } else {
        (s.chars().take(max_chars).collect(), true)
    }
}

/// One-line description of a trace for the stub on older turns.
fn tool_stub(t: &ToolTrace) -> String {
    let arg = t
        .input_json
        .get("file")
        .or_else(|| t.input_json.get("pattern"))
        .or_else(|| t.input_json.get("dir"))
        .and_then(|v| v.as_str());
    match arg {
        Some(a) => format!("{} {a}", t.tool_name),
        None => t.tool_name.clone(),
    }
}

/// History replay: every prior ok question turn contributes its Q/A prose;
/// the last `limits.replay_full_turns` of them also carry their tool results
/// verbatim (capped), and older turns carry a one-line stub naming the tools
/// used, so the model knows those outputs are gone and re-reads instead of
/// citing from memory.
async fn build_history_messages(
    store: &Store,
    session_id: &str,
    limits: &Limits,
    context_turn_ids: &[String],
) -> Result<Vec<Message>> {
    let max_history_messages = limits.max_history_messages as usize;
    let history = store.list_turns(session_id).await?;
    // Each replayed turn contributes two messages (user + assistant); budget
    // by messages so the configured limit is what actually reaches the
    // provider (minus one slot for the current question).
    let take = max_history_messages.saturating_sub(1) / 2;
    let turns: Vec<&Turn> = history
        .iter()
        .filter(|t| matches!(t.kind, TurnKind::Question) && t.status == TurnStatus::Ok)
        .collect();
    let turns = &turns[turns.len().saturating_sub(take)..];
    // Full fidelity: the recency floor plus any turn the client expanded,
    // capped at `replay_max_full_turns` (oldest demoted first). Matching against
    // this session's own turns is also what scopes client-sent ids: a foreign
    // turn_id matches nothing.
    let full_from = turns.len().saturating_sub(limits.replay_full_turns);
    let mut full: Vec<bool> = turns
        .iter()
        .enumerate()
        .map(|(i, t)| i >= full_from || context_turn_ids.iter().any(|id| id == &t.turn_id))
        .collect();
    let mut remaining = limits.replay_max_full_turns;
    for flag in full.iter_mut().rev() {
        if *flag {
            if remaining == 0 {
                *flag = false;
            } else {
                remaining -= 1;
            }
        }
    }
    let mut messages = Vec::new();
    for (i, turn) in turns.iter().enumerate() {
        if let Some(q) = &turn.question {
            messages.push(Message {
                role: Role::User,
                content: vec![ContentBlock::Text { text: q.clone() }],
            });
        }
        if let Some(a) = &turn.answer {
            let mut text = a.clone();
            let traces = store.list_traces(&turn.turn_id).await?;
            if !traces.is_empty() {
                if full[i] {
                    text.push_str(
                        "\n\n[Tool results gathered during this turn — reference \
                         material, not part of the shown answer:]",
                    );
                    let mut budget = limits.replay_turn_chars;
                    for tr in &traces {
                        let (input, _) = clip(&tr.input_json.to_string(), 300);
                        if budget == 0 {
                            text.push_str(&format!(
                                "\n- {} {input} → [omitted — re-read to cite]",
                                tr.tool_name
                            ));
                            continue;
                        }
                        let cap = limits.replay_result_chars.min(budget);
                        let (out, clipped) = clip(&tr.output_json.to_string(), cap);
                        budget = budget.saturating_sub(out.chars().count());
                        text.push_str(&format!(
                            "\n- {} {input} →\n{out}{}",
                            tr.tool_name,
                            if clipped {
                                "\n…[truncated — re-read to cite the rest]"
                            } else {
                                ""
                            }
                        ));
                    }
                } else {
                    let names: Vec<String> = traces.iter().map(tool_stub).collect();
                    text.push_str(&format!(
                        "\n\n[Tools used this turn: {} — outputs no longer in \
                         context; re-read before citing specifics.]",
                        names.join(", ")
                    ));
                }
            }
            messages.push(Message {
                role: Role::Assistant,
                content: vec![ContentBlock::Text { text }],
            });
        }
    }
    Ok(messages)
}

/// Persist a turn as `cancelled` (used by the WS handler on disconnect).
/// Persist a turn as `error` (used by the WS handler when the turn failed).
///
/// Without this a failed turn left no trace at all: no log line and no row, so
/// the only evidence was the error frame in the browser, and diagnosing one
/// meant reconstructing it from stored sizes.
pub async fn persist_failed(
    ctx: &TurnContext,
    input: &TurnInput,
    partial_answer: String,
) -> Result<()> {
    let turn_id = format!("t_{}", Uuid::new_v4().simple());
    persist(
        ctx,
        &turn_id,
        input,
        &partial_answer,
        TurnStatus::Error,
        &[],
        UsageTally::default(),
    )
    .await?;
    Ok(())
}

pub async fn persist_cancelled(
    ctx: &TurnContext,
    input: &TurnInput,
    partial_answer: String,
) -> Result<()> {
    let turn_id = format!("t_{}", Uuid::new_v4().simple());
    persist(
        ctx,
        &turn_id,
        input,
        &partial_answer,
        TurnStatus::Cancelled,
        &[],
        UsageTally::default(),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn tool_result_under_the_cap_passes_through_whole() {
        let limits = Limits::default();
        let mut spent = 0usize;
        let result = "{\"ok\":true}";
        let (content, truncated) = clip_tool_result(result, &limits, &mut spent);
        assert_eq!(content, result);
        assert_eq!(truncated, None);
        assert_eq!(spent, result.len(), "a small result still costs its budget");
    }

    #[test]
    fn an_oversized_tool_result_is_truncated_and_says_how_to_narrow() {
        // The shape that overflowed a 262k-token context: one `get_pr_diff`
        // without `paths` measured 589,499 chars on a real PR.
        let limits = Limits {
            max_tool_result_chars: 100,
            ..Limits::default()
        };
        let huge = "x".repeat(589_499);
        let mut spent = 0usize;
        let (content, truncated) = clip_tool_result(&huge, &limits, &mut spent);
        assert_eq!(
            truncated,
            Some(589_499),
            "the panel needs the original size"
        );
        assert!(content.starts_with(&"x".repeat(100)));
        assert!(content.contains("truncated: 589499 → 100 chars"));
        // The note has to be actionable, or the model just calls it again.
        assert!(content.contains("`paths` on get_pr_diff"));
        assert_eq!(spent, 100);
    }

    #[test]
    fn a_spent_turn_budget_still_yields_a_readable_head() {
        // Answers must go through: a turn that has used its whole budget gets
        // the floor plus a note, never an empty result and never an error.
        let limits = Limits {
            max_tool_result_chars: 20_000,
            max_turn_tool_chars: 1_000,
            ..Limits::default()
        };
        let mut spent = limits.max_turn_tool_chars; // budget already gone
        let huge = "y".repeat(50_000);
        let (content, truncated) = clip_tool_result(&huge, &limits, &mut spent);
        assert_eq!(truncated, Some(50_000));
        assert!(
            content.starts_with(&"y".repeat(TOOL_RESULT_FLOOR_CHARS)),
            "the model still gets the floor"
        );
        assert!(content.contains("tool-output budget (1000 chars) is spent"));
    }

    #[test]
    fn the_turn_budget_shrinks_the_allowance_across_rounds() {
        let limits = Limits {
            max_tool_result_chars: 10_000,
            max_turn_tool_chars: 12_000,
            ..Limits::default()
        };
        let mut spent = 0usize;
        let big = "z".repeat(30_000);
        let (first, _) = clip_tool_result(&big, &limits, &mut spent);
        assert!(first.starts_with(&"z".repeat(10_000)));
        assert_eq!(spent, 10_000);
        // 2,000 chars of budget remain, so the next result is cut to that.
        let (second, truncated) = clip_tool_result(&big, &limits, &mut spent);
        assert_eq!(truncated, Some(30_000));
        assert!(second.starts_with(&"z".repeat(2_000)));
        assert!(!second.starts_with(&"z".repeat(2_001)));
    }

    #[tokio::test]
    async fn history_replay_caps_come_from_config() {
        let store = Store::open_in_memory().unwrap();
        let sess = store
            .upsert_session("https://github.com/a/b/pull/9", serde_json::json!({}))
            .await
            .unwrap();
        let t = Turn {
            turn_id: "t1".into(),
            session_id: sess.session_id.clone(),
            ordinal: 1,
            kind: TurnKind::Question,
            status: TurnStatus::Ok,
            verb: None,
            question: Some("q".into()),
            selection: None,
            answer: Some("a".into()),
            user_content: None,
            severity: None,
            usage_in: 0,
            usage_out: 0,
            created_at: 1,
            source_turn_id: None,
        };
        let tr = ToolTrace {
            trace_id: "tr1".into(),
            turn_id: "t1".into(),
            ordinal: 1,
            tool_name: "read_file".into(),
            input_json: serde_json::json!({"file": "src/x.py"}),
            output_json: serde_json::json!({"content": "A".repeat(5_000)}),
            duration_ms: 1,
            ok: true,
        };
        store.insert_turn(&t, &[tr]).await.unwrap();

        let text = |msgs: Vec<Message>| -> String {
            msgs.iter()
                .flat_map(|m| m.content.iter())
                .map(|b| match b {
                    ContentBlock::Text { text } => text.clone(),
                    _ => String::new(),
                })
                .collect()
        };

        // A tight per-result cap truncates the replayed evidence...
        let tight = Limits {
            replay_result_chars: 200,
            ..Limits::default()
        };
        let clipped = text(
            build_history_messages(&store, &sess.session_id, &tight, &[])
                .await
                .unwrap(),
        );
        assert!(clipped.contains("re-read to cite the rest"));

        // ...and turning the recency floor off stubs it entirely.
        let none = Limits {
            replay_full_turns: 0,
            replay_max_full_turns: 0,
            ..Limits::default()
        };
        let stubbed = text(
            build_history_messages(&store, &sess.session_id, &none, &[])
                .await
                .unwrap(),
        );
        assert!(stubbed.contains("re-read before citing"));
        assert!(!stubbed.contains("AAAA"));
    }

    #[tokio::test]
    async fn history_replays_recent_tool_results_and_stubs_older() {
        let store = Store::open_in_memory().unwrap();
        let sess = store
            .upsert_session("https://github.com/a/b/pull/9", serde_json::json!({}))
            .await
            .unwrap();
        for (n, marker) in [(1i64, "OLD_OUTPUT"), (2, "MID_OUTPUT"), (3, "NEW_OUTPUT")] {
            let turn_id = format!("t{n}");
            let t = Turn {
                turn_id: turn_id.clone(),
                session_id: sess.session_id.clone(),
                ordinal: n,
                kind: TurnKind::Question,
                status: TurnStatus::Ok,
                verb: None,
                question: Some(format!("q{n}")),
                selection: None,
                answer: Some(format!("a{n}")),
                user_content: None,
                severity: None,
                usage_in: 0,
                usage_out: 0,
                created_at: n,
                source_turn_id: None,
            };
            let tr = ToolTrace {
                trace_id: format!("tr{n}"),
                turn_id,
                ordinal: 1,
                tool_name: "read_file".into(),
                input_json: serde_json::json!({"file": "src/x.py"}),
                output_json: serde_json::json!({"content": marker}),
                duration_ms: 1,
                ok: true,
            };
            store.insert_turn(&t, &[tr]).await.unwrap();
        }
        let msgs = build_history_messages(&store, &sess.session_id, &Limits::default(), &[])
            .await
            .unwrap();
        let all: String = msgs
            .iter()
            .flat_map(|m| m.content.iter())
            .map(|b| match b {
                ContentBlock::Text { text } => text.clone(),
                _ => String::new(),
            })
            .collect();
        // The two most recent turns carry their tool outputs verbatim.
        assert!(all.contains("NEW_OUTPUT"));
        assert!(all.contains("MID_OUTPUT"));
        // The oldest turn is stubbed: tool + file named, output dropped.
        assert!(!all.contains("OLD_OUTPUT"));
        assert!(all.contains("read_file src/x.py"));
        assert!(all.contains("re-read before citing"));
    }

    #[tokio::test]
    async fn context_turn_ids_promote_old_turns_and_ignore_foreign_ids() {
        let store = Store::open_in_memory().unwrap();
        let sess = store
            .upsert_session("https://github.com/a/b/pull/9", serde_json::json!({}))
            .await
            .unwrap();
        for (n, marker) in [(1i64, "OLD_OUTPUT"), (2, "MID_OUTPUT"), (3, "NEW_OUTPUT")] {
            let turn_id = format!("t{n}");
            let t = Turn {
                turn_id: turn_id.clone(),
                session_id: sess.session_id.clone(),
                ordinal: n,
                kind: TurnKind::Question,
                status: TurnStatus::Ok,
                verb: None,
                question: Some(format!("q{n}")),
                selection: None,
                answer: Some(format!("a{n}")),
                user_content: None,
                severity: None,
                usage_in: 0,
                usage_out: 0,
                created_at: n,
                source_turn_id: None,
            };
            let tr = ToolTrace {
                trace_id: format!("tr{n}"),
                turn_id,
                ordinal: 1,
                tool_name: "read_file".into(),
                input_json: serde_json::json!({"file": "src/x.py"}),
                output_json: serde_json::json!({"content": marker}),
                duration_ms: 1,
                ok: true,
            };
            store.insert_turn(&t, &[tr]).await.unwrap();
        }
        // Expanding turn t1 promotes it to full fidelity; a foreign id is a no-op.
        let ids = vec!["t1".to_string(), "t_other_session".to_string()];
        let msgs = build_history_messages(&store, &sess.session_id, &Limits::default(), &ids)
            .await
            .unwrap();
        let all: String = msgs
            .iter()
            .flat_map(|m| m.content.iter())
            .map(|b| match b {
                ContentBlock::Text { text } => text.clone(),
                _ => String::new(),
            })
            .collect();
        assert!(all.contains("OLD_OUTPUT"));
        assert!(all.contains("MID_OUTPUT"));
        assert!(all.contains("NEW_OUTPUT"));
    }

    #[test]
    fn user_message_quotes_the_selected_text() {
        let sel = libre_cr_common::Selection::Line {
            file: "src/a.rs".into(),
            line: 38,
            text: Some("_CONDITIONAL_CHECK_FAILED = \"ConditionalCheckFailedException\"".into()),
        };
        let msg = build_user_message("What is this for?", Some(&sel));
        let ContentBlock::Text { text } = &msg.content[0] else {
            panic!("expected text");
        };
        assert!(text.contains("[Selection: line 38 in src/a.rs]"));
        assert!(text.contains("ConditionalCheckFailedException"));
        assert!(text.contains("What is this for?"));
    }

    #[test]
    fn user_message_quotes_the_whole_review_thread() {
        let sel = libre_cr_common::Selection::Comment {
            comment_id: "3872880867".into(),
            file: "src/a.rs".into(),
            line: 36,
            side: libre_cr_common::Side::Left,
            comments: vec![
                libre_cr_common::ThreadComment {
                    author: "coderabbitai[bot]".into(),
                    body: "This retries forever.".into(),
                },
                libre_cr_common::ThreadComment {
                    author: "oleduc".into(),
                    body: "Intentional — the caller bounds it.".into(),
                },
            ],
        };
        let msg = build_user_message("Is this concern valid?", Some(&sel));
        let ContentBlock::Text { text } = &msg.content[0] else {
            panic!("expected text");
        };
        // The old side must say so: resolving line 36 against the new file
        // would read a different line.
        assert!(text.contains("[Selection: review comment on line 36 (old side) in src/a.rs]"));
        // The reply is where the answer usually lives — it must survive.
        assert!(text.contains("@coderabbitai[bot]: This retries forever."));
        assert!(text.contains("@oleduc: Intentional — the caller bounds it."));
        assert!(text.contains("Is this concern valid?"));
    }

    use super::*;
    use crate::agent::RecordingSink;
    use crate::config::ScriptedEvent;
    use crate::provider::MockProvider;
    use crate::tools::code_daemon::{CodeDaemonClient, MockCodeDaemonClient};
    use crate::tools::internal::InternalContext;

    async fn ctx_with_script(script: Vec<ScriptedEvent>) -> (TurnContext, RecordingSink) {
        let store = Store::open_in_memory().unwrap();
        let sess = store
            .upsert_session("https://github.com/a/b/pull/9", serde_json::json!({}))
            .await
            .unwrap();
        let mc = std::sync::Arc::new(MockCodeDaemonClient);
        let schemas = mc.list_tools().await.unwrap();
        let internal = InternalContext {
            session_id: sess.session_id.clone(),
            pr_data: serde_json::json!({"metadata":{"title":"t"}}),
            selection: None,
            store: store.clone(),
        };
        let router = ToolRouter::new(mc, schemas, internal, Some("/tmp/w".into()));
        let provider = std::sync::Arc::new(MockProvider::new(script));
        (
            TurnContext {
                session_id: sess.session_id,
                provider,
                router,
                store,
                limits: Limits {
                    max_tool_turns: 5,
                    ..Limits::default()
                },
                global_instructions: String::new(),
            },
            RecordingSink::new(),
        )
    }

    #[tokio::test]
    async fn single_text_turn() {
        let script = vec![
            ScriptedEvent {
                delay_ms: 0,
                event: StreamEvent::TextDelta {
                    text: "hello".into(),
                },
            },
            ScriptedEvent {
                delay_ms: 0,
                event: StreamEvent::Done {
                    input_tokens: 1,
                    output_tokens: 2,
                    stop_reason: "end_turn".into(),
                },
            },
        ];
        let (ctx, sink) = ctx_with_script(script).await;
        let r = run_turn(
            &ctx,
            TurnInput {
                question: "hi?".into(),
                selection: None,
                verb: None,
                context_turn_ids: vec![],
            },
            &sink,
        )
        .await
        .unwrap();
        assert_eq!(r.answer, "hello");
        assert_eq!(r.status, TurnStatus::Ok);
        let frames = sink.snapshot().await;
        // 1 text_delta + 1 done
        assert!(frames
            .iter()
            .any(|f| matches!(f, libre_cr_common::ws_frames::ServerFrame::Done { .. })));
    }

    #[tokio::test]
    async fn two_round_with_tool() {
        let script = vec![
            ScriptedEvent {
                delay_ms: 0,
                event: StreamEvent::ToolUse {
                    id: "t1".into(),
                    name: "grep".into(),
                    input: serde_json::json!({"query":"x"}),
                },
            },
            ScriptedEvent {
                delay_ms: 0,
                event: StreamEvent::Done {
                    input_tokens: 0,
                    output_tokens: 0,
                    stop_reason: "tool_use".into(),
                },
            },
            ScriptedEvent {
                delay_ms: 0,
                event: StreamEvent::TextDelta {
                    text: "done.".into(),
                },
            },
            ScriptedEvent {
                delay_ms: 0,
                event: StreamEvent::Done {
                    input_tokens: 5,
                    output_tokens: 6,
                    stop_reason: "end_turn".into(),
                },
            },
        ];
        let (ctx, sink) = ctx_with_script(script).await;
        let r = run_turn(
            &ctx,
            TurnInput {
                question: "where?".into(),
                selection: None,
                verb: None,
                context_turn_ids: vec![],
            },
            &sink,
        )
        .await
        .unwrap();
        assert_eq!(r.answer, "done.");
        assert_eq!(r.tool_call_count, 1);
        let frames = sink.snapshot().await;
        assert!(frames
            .iter()
            .any(|f| matches!(f, libre_cr_common::ws_frames::ServerFrame::ToolCall { .. })));
        assert!(frames.iter().any(|f| matches!(
            f,
            libre_cr_common::ws_frames::ServerFrame::ToolResult { .. }
        )));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tool_calls_in_one_turn_dispatch_concurrently() {
        // I2: two tool calls in a single LLM turn must overlap in time, not
        // run back-to-back.
        use crate::provider::ToolSchema;
        use std::time::{Duration, Instant};

        struct SlowClient {
            windows: std::sync::Arc<tokio::sync::Mutex<Vec<(Instant, Instant)>>>,
        }
        #[async_trait::async_trait]
        impl CodeDaemonClient for SlowClient {
            async fn list_tools(&self) -> crate::error::Result<Vec<ToolSchema>> {
                Ok(vec![ToolSchema {
                    name: "slow".into(),
                    description: "".into(),
                    input_schema: serde_json::json!({"type":"object","properties":{}}),
                }])
            }
            async fn call(
                &self,
                _name: &str,
                _input: serde_json::Value,
            ) -> crate::error::Result<serde_json::Value> {
                let start = Instant::now();
                tokio::time::sleep(Duration::from_millis(80)).await;
                self.windows.lock().await.push((start, Instant::now()));
                Ok(serde_json::json!({"ok":true}))
            }
        }

        let windows = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let client = std::sync::Arc::new(SlowClient {
            windows: windows.clone(),
        });
        let store = Store::open_in_memory().unwrap();
        let sess = store
            .upsert_session("https://github.com/a/b/pull/77", serde_json::json!({}))
            .await
            .unwrap();
        let schemas = client.list_tools().await.unwrap();
        let internal = InternalContext {
            session_id: sess.session_id.clone(),
            pr_data: serde_json::json!({}),
            selection: None,
            store: store.clone(),
        };
        let router = ToolRouter::new(client, schemas, internal, Some("/tmp/w".into()));
        let script = vec![
            ScriptedEvent {
                delay_ms: 0,
                event: StreamEvent::ToolUse {
                    id: "t1".into(),
                    name: "slow".into(),
                    input: serde_json::json!({}),
                },
            },
            ScriptedEvent {
                delay_ms: 0,
                event: StreamEvent::ToolUse {
                    id: "t2".into(),
                    name: "slow".into(),
                    input: serde_json::json!({}),
                },
            },
            ScriptedEvent {
                delay_ms: 0,
                event: StreamEvent::Done {
                    input_tokens: 0,
                    output_tokens: 0,
                    stop_reason: "tool_use".into(),
                },
            },
            ScriptedEvent {
                delay_ms: 0,
                event: StreamEvent::TextDelta { text: "ok".into() },
            },
            ScriptedEvent {
                delay_ms: 0,
                event: StreamEvent::Done {
                    input_tokens: 0,
                    output_tokens: 0,
                    stop_reason: "end_turn".into(),
                },
            },
        ];
        let ctx = TurnContext {
            session_id: sess.session_id,
            provider: std::sync::Arc::new(MockProvider::new(script)),
            router,
            store,
            limits: Limits {
                max_tool_turns: 5,
                ..Limits::default()
            },
            global_instructions: String::new(),
        };
        let sink = RecordingSink::new();
        let r = run_turn(
            &ctx,
            TurnInput {
                question: "go".into(),
                selection: None,
                verb: None,
                context_turn_ids: vec![],
            },
            &sink,
        )
        .await
        .unwrap();
        assert_eq!(r.tool_call_count, 2);
        let w = windows.lock().await.clone();
        assert_eq!(w.len(), 2);
        let latest_start = w.iter().map(|(s, _)| *s).max().unwrap();
        let earliest_end = w.iter().map(|(_, e)| *e).min().unwrap();
        assert!(
            latest_start < earliest_end,
            "tool dispatch windows must overlap (serial dispatch detected)"
        );
    }

    #[tokio::test]
    async fn exhausts_budget() {
        // Each round: tool_use + done — agent will keep dispatching forever.
        let mut script = Vec::new();
        for _ in 0..30 {
            script.push(ScriptedEvent {
                delay_ms: 0,
                event: StreamEvent::ToolUse {
                    id: "x".into(),
                    name: "grep".into(),
                    input: serde_json::json!({}),
                },
            });
            script.push(ScriptedEvent {
                delay_ms: 0,
                event: StreamEvent::Done {
                    input_tokens: 0,
                    output_tokens: 0,
                    stop_reason: "tool_use".into(),
                },
            });
        }
        let (ctx, sink) = ctx_with_script(script).await;
        let r = run_turn(
            &ctx,
            TurnInput {
                question: "loop".into(),
                selection: None,
                verb: None,
                context_turn_ids: vec![],
            },
            &sink,
        )
        .await;
        assert!(matches!(r, Err(Error::TooManyToolTurns)));
    }
}
