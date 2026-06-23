//! `WS /v1/sessions/:id/ask` handler.

use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    extract::{
        ws::{Message as WsMessage, WebSocket},
        Path, State, WebSocketUpgrade,
    },
    response::Response,
};
use futures::{stream::SplitSink, SinkExt, StreamExt};
use libre_cr_common::ws_frames::{AskInit, ClientFrame, ServerFrame};
use tokio::sync::Mutex;

use crate::agent::{persist_cancelled, run_turn, FrameSink, TurnContext, TurnInput};
use crate::error::{Error, Result};
use crate::tools::code_daemon::CodeDaemonClient;
use crate::tools::internal::InternalContext;
use crate::tools::presentation::{
    FrameOut, PendingCalls, PresentationCallFrame, PresentationDispatcher, PresentationOutcome,
};
use crate::tools::ToolRouter;

use super::state::{AppState, BusySessions};

/// RAII claim on a session's single-flight ask slot (N4). The claim is a
/// single check-and-insert under one lock acquisition (no TOCTOU), and the
/// release runs in `Drop`, so failed upgrades, early handler errors, and
/// panics all free the slot instead of 409-ing the session forever.
pub(crate) struct BusyGuard {
    set: BusySessions,
    session_id: String,
}

impl BusyGuard {
    /// Atomically claim `session_id`. `None` when an ask is already in
    /// flight for it.
    pub(crate) fn try_claim(set: &BusySessions, session_id: &str) -> Option<Self> {
        let mut g = set.lock().expect("busy_sessions lock poisoned");
        if !g.insert(session_id.to_string()) {
            return None;
        }
        Some(Self {
            set: set.clone(),
            session_id: session_id.to_string(),
        })
    }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        if let Ok(mut g) = self.set.lock() {
            g.remove(&self.session_id);
        }
    }
}

pub async fn ws_ask_handler(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    ws: WebSocketUpgrade,
) -> std::result::Result<Response, Error> {
    let Some(guard) = BusyGuard::try_claim(&state.busy_sessions, &session_id) else {
        return Err(Error::Conflict(
            "session already has an in-flight ask".into(),
        ));
    };
    // The guard moves into the upgrade future: if the upgrade never
    // completes, axum drops the future (and the guard) instead of running
    // it — the slot is still released.
    Ok(ws.on_upgrade(move |socket| async move {
        let _guard = guard;
        let _ = handle_ws(state, session_id, socket).await;
    }))
}

type WsSink = Arc<Mutex<SplitSink<WebSocket, WsMessage>>>;

/// Server-side accumulator for the assistant's text deltas during a turn.
/// Used by the disconnect handler so a cancelled turn can be persisted with
/// the partial answer the user already saw.
type PartialAnswer = Arc<Mutex<String>>;

struct WebSocketFrameSink {
    sink: WsSink,
    /// Mirrors every `TextDelta` we emit to the client. The WS handler
    /// reads this on cancellation to persist the partial answer.
    partial: PartialAnswer,
}

#[async_trait]
impl FrameSink for WebSocketFrameSink {
    async fn send(&self, frame: ServerFrame) -> Result<()> {
        if let ServerFrame::TextDelta { text } = &frame {
            self.partial.lock().await.push_str(text);
        }
        let s = serde_json::to_string(&frame)?;
        self.sink
            .lock()
            .await
            .send(WsMessage::Text(s))
            .await
            .map_err(|e| Error::Internal(format!("ws send: {e}")))?;
        Ok(())
    }
}

struct WsPresentationOut {
    sink: WsSink,
}

#[async_trait]
impl FrameOut for WsPresentationOut {
    async fn send_presentation_call(&self, frame: PresentationCallFrame) -> Result<()> {
        let f = ServerFrame::PresentationCall {
            call_id: frame.call_id,
            tool: frame.tool,
            input: frame.input,
        };
        let s = serde_json::to_string(&f)?;
        self.sink
            .lock()
            .await
            .send(WsMessage::Text(s))
            .await
            .map_err(|e| Error::Internal(format!("ws send: {e}")))?;
        Ok(())
    }
}

async fn handle_ws(state: AppState, session_id: String, ws: WebSocket) -> Result<()> {
    let (writer, mut reader) = ws.split();
    let writer: WsSink = Arc::new(Mutex::new(writer));
    let partial: PartialAnswer = Arc::new(Mutex::new(String::new()));
    let sink = Arc::new(WebSocketFrameSink {
        sink: writer.clone(),
        partial: partial.clone(),
    });

    // Read first frame
    let first = match reader.next().await {
        Some(Ok(WsMessage::Text(s))) => s,
        Some(Ok(WsMessage::Binary(b))) => String::from_utf8_lossy(&b).to_string(),
        _ => return Err(Error::Validation("ws closed before init".into())),
    };
    let init: AskInit = serde_json::from_str(&first).map_err(Error::from)?;

    // Selection enforcement for verbs (defensive — extension grays out invalid
    // combos but the daemon refuses them anyway). Send the message as an
    // `error` frame and close — by then we're past HTTP upgrade.
    if let Some(verb_id) = &init.verb {
        if let Err(e) = crate::verbs::validate_selection(verb_id, init.selection.as_ref()) {
            let _ = sink.error(&e.to_string(), false).await;
            let _ = writer.lock().await.send(WsMessage::Close(None)).await;
            return Err(e);
        }
    }

    let sess = state
        .store
        .get_session(&session_id)
        .await?
        .ok_or(Error::NotFound)?;

    // Presentation dispatcher with WS sink.
    let pending = PendingCalls::new();
    let presentation = Arc::new(PresentationDispatcher::new(
        Arc::new(WsPresentationOut {
            sink: writer.clone(),
        }),
        pending.clone(),
    ));

    let code_schemas = state.code_daemon.list_tools().await?;
    let internal = InternalContext {
        session_id: session_id.clone(),
        pr_data: sess.pr_data.clone(),
        selection: init.selection.clone(),
        store: state.store.clone(),
    };
    let mut router: ToolRouter = ToolRouter::new(
        Arc::clone(&state.code_daemon as &Arc<dyn CodeDaemonClient>),
        code_schemas,
        internal,
        sess.worktree_path.clone(),
    )
    .with_repo_id(sess.repo_id.clone());
    // E1: a muted turn never registers the presentation tools, so the model
    // cannot emit presentation_call frames at all (same mechanism as the
    // config-level disable: no dispatcher attached → tools not offered).
    if !init.mute_presentations {
        router = router.with_presentation(presentation.clone());
    }

    let cfg = state.config.snapshot().await;
    let ctx = TurnContext {
        session_id: session_id.clone(),
        // RC1: pick up the provider as of this turn, not the startup one.
        provider: state.provider.get().await,
        router,
        store: state.store.clone(),
        max_tool_turns: cfg.limits.max_tool_turns,
        max_history_messages: cfg.limits.max_history_messages,
        global_instructions: cfg.global_instructions.text.clone(),
    };

    // Spawn a reader task that forwards presentation_result frames into the
    // pending map, and watches for client disconnect.
    let pending_for_reader = pending.clone();
    let mut reader_task = tokio::spawn(async move {
        while let Some(msg) = reader.next().await {
            let Ok(msg) = msg else {
                break;
            };
            let s = match msg {
                WsMessage::Text(s) => s,
                WsMessage::Binary(b) => String::from_utf8_lossy(&b).to_string(),
                WsMessage::Close(_) => break,
                _ => continue,
            };
            if let Ok(ClientFrame::PresentationResult {
                call_id,
                ok,
                result,
                error,
                message,
            }) = serde_json::from_str::<ClientFrame>(&s)
            {
                let value = if ok {
                    result.unwrap_or(serde_json::json!({}))
                } else {
                    serde_json::json!({
                        "error": error.unwrap_or_default(),
                        "message": message.unwrap_or_default(),
                    })
                };
                pending_for_reader
                    .deliver(&call_id, PresentationOutcome { ok, value })
                    .await;
            }
        }
        pending_for_reader.cancel_all().await;
    });

    let input = TurnInput {
        question: init.question.clone(),
        selection: init.selection.clone(),
        verb: init.verb.clone(),
    };
    // Clone the bits we'll need on the cancellation arm before run_turn
    // borrows them.
    let cancel_input = TurnInput {
        question: init.question,
        selection: init.selection,
        verb: init.verb,
    };

    let agent_fut = run_turn(&ctx, input, sink.as_ref());

    // tokio::select! consumes the futures by value; pin agent_fut, and
    // borrow the reader_task JoinHandle so we don't double-spawn it.
    tokio::pin!(agent_fut);
    let outcome = tokio::select! {
        r = &mut agent_fut => r,
        _ = &mut reader_task => {
            // Client dropped the connection mid-turn. Persist whatever
            // partial answer we've already streamed so the session history
            // doesn't lose the turn. See REVIEW/00-certification.md I3.
            let partial_text = partial.lock().await.clone();
            if let Err(e) = persist_cancelled(&ctx, &cancel_input, partial_text).await {
                tracing::warn!(error = %e, "persist cancelled turn");
            }
            Err(Error::Internal("client disconnected".into()))
        }
    };
    if let Err(e) = outcome {
        // Best-effort: the client may already be gone (notably on the
        // cancellation arm, where the WS is already closed).
        let _ = sink.error(&e.to_string(), false).await;
    }
    let _ = writer.lock().await.send(WsMessage::Close(None)).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn set() -> BusySessions {
        Arc::new(std::sync::Mutex::new(HashSet::new()))
    }

    #[test]
    fn busy_guard_claims_once_and_releases_on_drop() {
        let s = set();
        let g = BusyGuard::try_claim(&s, "s_1").expect("first claim");
        assert!(
            BusyGuard::try_claim(&s, "s_1").is_none(),
            "second claim must fail while the first is live"
        );
        // Different session unaffected.
        assert!(BusyGuard::try_claim(&s, "s_2").is_some());
        drop(g);
        assert!(
            BusyGuard::try_claim(&s, "s_1").is_some(),
            "slot must be free after drop"
        );
    }

    #[test]
    fn busy_guard_releases_on_panic() {
        let s = set();
        let s2 = s.clone();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _g = BusyGuard::try_claim(&s2, "s_1").unwrap();
            panic!("handler blew up after claiming");
        }));
        assert!(r.is_err());
        assert!(
            BusyGuard::try_claim(&s, "s_1").is_some(),
            "slot must be free after a panic released the guard"
        );
    }
}
