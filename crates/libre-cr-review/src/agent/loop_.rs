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
         code. Prefer concise, structured answers.",
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
             open_link, clear_presentation. Use them sparingly to amplify your \
             answer when they help the reviewer follow it. The answer text \
             remains primary.",
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

    // Pull recent history. Phase 4 will integrate verb addenda; here we only
    // load the prior text exchange so the model has continuity.
    let mut messages = vec![system];
    let history = ctx.store.list_turns(&ctx.session_id).await?;
    let take = (ctx.max_history_messages as usize).saturating_sub(1);
    for turn in history.iter().rev().take(take).rev() {
        if matches!(turn.kind, TurnKind::Question) && turn.status == TurnStatus::Ok {
            if let Some(q) = &turn.question {
                messages.push(Message {
                    role: Role::User,
                    content: vec![ContentBlock::Text { text: q.clone() }],
                });
            }
            if let Some(a) = &turn.answer {
                messages.push(Message {
                    role: Role::Assistant,
                    content: vec![ContentBlock::Text { text: a.clone() }],
                });
            }
        }
    }
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
            },
            &sink,
        )
        .await;
        assert!(matches!(r, Err(Error::TooManyToolTurns)));
    }
}
