//! The agent loop. Pseudo-code in `04-review-daemon.md` § Agent Loop is the
//! contract.

use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use futures::StreamExt;
use libre_cr_common::ws_frames::UsageTally;
use libre_cr_common::Selection;
use uuid::Uuid;

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
    pub max_tool_turns: u32,
    pub max_history_messages: u32,
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
            },
            s.file()
        ));
        // The exact selected text, so "this line" can never mean another line.
        let snippet = match s {
            Selection::Line { text, .. }
            | Selection::Range { text, .. }
            | Selection::Symbol { text, .. } => text.as_deref(),
        };
        if let Some(snippet) = snippet.filter(|t| !t.trim().is_empty()) {
            let mut clipped: String = snippet.chars().take(2000).collect();
            if snippet.chars().count() > 2000 {
                clipped.push('…');
            }
            text.push_str(&format!("The selected text is:\n```\n{clipped}\n```\n"));
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
            ctx.max_history_messages as usize,
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

    for _ in 0..ctx.max_tool_turns {
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
        // `tool_call` frames go out first (original order), each
        // `tool_result` frame is emitted as its dispatch resolves, and the
        // result blocks fed back to the model preserve the original call
        // order (`join_all` returns outputs in input order).
        for (id, name, input_json) in &tool_uses {
            sink.tool_call(id, name, input_json.clone()).await?;
        }
        let outcomes = futures::future::join_all(tool_uses.iter().map(|(id, name, input_json)| {
            let call = ToolCall {
                id: id.clone(),
                name: name.clone(),
                input: input_json.clone(),
            };
            async move {
                let outcome = ctx.router.dispatch(&call).await;
                let sent = sink.tool_result(&call.id, outcome.value.clone()).await;
                (outcome, sent)
            }
        }))
        .await;
        let mut tool_result_blocks = Vec::new();
        for ((id, name, input_json), (outcome, sent)) in tool_uses.into_iter().zip(outcomes) {
            sent?;
            traces.push(ToolTrace {
                trace_id: format!("tr_{}", Uuid::new_v4().simple()),
                turn_id: turn_id.clone(),
                ordinal: (traces.len() as i64) + 1,
                tool_name: name,
                input_json,
                output_json: outcome.value.clone(),
                duration_ms: outcome.duration_ms,
                ok: outcome.ok,
            });
            tool_call_count += 1;
            tool_result_blocks.push(ContentBlock::ToolResult {
                tool_use_id: id,
                content: outcome.value.to_string(),
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

/// How many of the most recent history turns are replayed with their full
/// tool results: a follow-up question almost always targets the last answers,
/// and without the evidence in context the model paraphrases from memory
/// (the fabricated-identifier hallucination class).
const REPLAY_FULL_TURNS: usize = 2;
/// Per tool-result cap when replaying history (a `get_pr_diff` can be huge).
const REPLAY_RESULT_MAX_CHARS: usize = 20_000;
/// Total tool-result budget per replayed turn.
const REPLAY_TURN_MAX_CHARS: usize = 40_000;
/// Hard cap on how many turns replay at full fidelity per ask, however many
/// the client expands; oldest are demoted to stubs first.
const REPLAY_MAX_FULL_TURNS: usize = 5;

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
/// the last `REPLAY_FULL_TURNS` of them also carry their tool results
/// verbatim (capped), and older turns carry a one-line stub naming the tools
/// used, so the model knows those outputs are gone and re-reads instead of
/// citing from memory.
async fn build_history_messages(
    store: &Store,
    session_id: &str,
    max_history_messages: usize,
    context_turn_ids: &[String],
) -> Result<Vec<Message>> {
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
    // capped at REPLAY_MAX_FULL_TURNS (oldest demoted first). Matching against
    // this session's own turns is also what scopes client-sent ids: a foreign
    // turn_id matches nothing.
    let full_from = turns.len().saturating_sub(REPLAY_FULL_TURNS);
    let mut full: Vec<bool> = turns
        .iter()
        .enumerate()
        .map(|(i, t)| i >= full_from || context_turn_ids.iter().any(|id| id == &t.turn_id))
        .collect();
    let mut remaining = REPLAY_MAX_FULL_TURNS;
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
                    let mut budget = REPLAY_TURN_MAX_CHARS;
                    for tr in &traces {
                        let (input, _) = clip(&tr.input_json.to_string(), 300);
                        if budget == 0 {
                            text.push_str(&format!(
                                "\n- {} {input} → [omitted — re-read to cite]",
                                tr.tool_name
                            ));
                            continue;
                        }
                        let cap = REPLAY_RESULT_MAX_CHARS.min(budget);
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
        let msgs = build_history_messages(&store, &sess.session_id, 30, &[])
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
        let msgs = build_history_messages(&store, &sess.session_id, 30, &ids)
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
                max_tool_turns: 5,
                max_history_messages: 30,
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
            max_tool_turns: 5,
            max_history_messages: 30,
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
