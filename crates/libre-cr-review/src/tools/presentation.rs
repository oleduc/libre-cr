//! Presentation tools — routed *back* through the active WS to the extension.
//! Spec: `09-presentation-tools.md`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{oneshot, Mutex};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::provider::ToolSchema;

/// Names registered with the LLM, only when there's an active WS.
pub const PRESENTATION_TOOL_NAMES: &[&str] = &[
    "highlight_lines",
    "annotate_line",
    "scroll_to",
    "open_link",
    "clear_presentation",
];

pub fn presentation_tool_schemas() -> Vec<ToolSchema> {
    vec![
        ToolSchema {
            name: "highlight_lines".into(),
            description: "Highlight a range of lines in the PR diff shown in the browser. Use this whenever the reviewer asks you to point out, show, mark or highlight code. ALWAYS pass `label`: it is rendered as a caption next to the code and is how the reviewer ties the highlight to your answer — make it the short heading of the part you are describing (≤ 8 words). Line numbers are NEW-side (right/after) numbers of the PR head; for deleted lines use the OLD-side number. Works for any file in the PR diff (the browser scrolls the file into view if needed).".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file": {"type":"string"},
                    "start_line": {"type":"integer"},
                    "end_line": {"type":"integer"},
                    "color": {"type":"string",
                              "enum":["red","yellow","green","blue","purple"]},
                    "label": {"type":"string"}
                },
                "required": ["file","start_line","end_line"]
            }),
        },
        ToolSchema {
            name: "annotate_line".into(),
            description: "Insert a short inline note under a diff line in the browser (severity-colored). Good for flagging a specific finding at its location.".into(),
            input_schema: serde_json::json!({
                "type":"object",
                "properties": {
                    "file":{"type":"string"},
                    "line":{"type":"integer"},
                    "summary":{"type":"string"},
                    "detail":{"type":"string"},
                    "severity":{"type":"string",
                                "enum":["info","suggestion","warning","critical"]}
                },
                "required":["file","line","summary"]
            }),
        },
        ToolSchema {
            name: "scroll_to".into(),
            description: "Scroll the browser diff to a file, or to a NEW-side line number within it (flashing the row). Without a line it scrolls to the file header.".into(),
            input_schema: serde_json::json!({
                "type":"object",
                "properties": {
                    "file":{"type":"string"},
                    "line":{"type":"integer"}
                },
                "required":["file"]
            }),
        },
        ToolSchema {
            name: "open_link".into(),
            description: "Open an http(s) URL in a new tab. Not for local files.".into(),
            input_schema: serde_json::json!({
                "type":"object",
                "properties": {
                    "url":{"type":"string"},
                    "target":{"type":"string","enum":["tab","panel"]}
                },
                "required":["url"]
            }),
        },
        ToolSchema {
            name: "clear_presentation".into(),
            description: "Clear previously-placed presentation effects.".into(),
            input_schema: serde_json::json!({
                "type":"object",
                "properties": {
                    "scope":{"type":"string","enum":["all","highlights","annotations"]}
                }
            }),
        },
    ]
}

/// Outcome from a presentation call: either success with a result payload or
/// a structured failure the LLM can see.
#[derive(Debug, Clone)]
pub struct PresentationOutcome {
    pub ok: bool,
    pub value: serde_json::Value,
}

/// What the dispatcher sends out the WS: a JSON value the WS sink emits.
#[derive(Debug, Clone)]
pub struct PresentationCallFrame {
    pub call_id: String,
    pub tool: String,
    pub input: serde_json::Value,
}

/// Trait for "send a frame on the active WS." Decouples the dispatcher from
/// the concrete WS sink so tests can plug a recorder.
#[async_trait::async_trait]
pub trait FrameOut: Send + Sync {
    async fn send_presentation_call(&self, frame: PresentationCallFrame) -> Result<()>;
}

/// Per-turn map of call_id → oneshot waiting for the matching
/// `presentation_result`.
#[derive(Clone, Default)]
pub struct PendingCalls {
    inner: Arc<Mutex<HashMap<String, oneshot::Sender<PresentationOutcome>>>>,
}

impl PendingCalls {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn deliver(&self, call_id: &str, outcome: PresentationOutcome) -> bool {
        let mut g = self.inner.lock().await;
        if let Some(tx) = g.remove(call_id) {
            let _ = tx.send(outcome);
            true
        } else {
            false
        }
    }

    pub async fn register(&self, call_id: &str) -> oneshot::Receiver<PresentationOutcome> {
        let (tx, rx) = oneshot::channel();
        let mut g = self.inner.lock().await;
        g.insert(call_id.to_string(), tx);
        rx
    }

    /// Cancel all pending: each receiver gets a `dropped` error so callers
    /// surface `extension_unavailable`.
    pub async fn cancel_all(&self) {
        let mut g = self.inner.lock().await;
        g.clear();
    }
}

pub struct PresentationDispatcher {
    out: Arc<dyn FrameOut>,
    pending: PendingCalls,
    timeout: Duration,
}

impl PresentationDispatcher {
    pub fn new(out: Arc<dyn FrameOut>, pending: PendingCalls) -> Self {
        Self {
            out,
            pending,
            timeout: Duration::from_secs(5),
        }
    }

    pub fn with_timeout(mut self, d: Duration) -> Self {
        self.timeout = d;
        self
    }

    pub fn pending(&self) -> PendingCalls {
        self.pending.clone()
    }

    pub async fn dispatch(
        &self,
        tool: &str,
        input: serde_json::Value,
    ) -> Result<PresentationOutcome> {
        let call_id = format!("p_{}", Uuid::new_v4().simple());
        let rx = self.pending.register(&call_id).await;
        self.out
            .send_presentation_call(PresentationCallFrame {
                call_id: call_id.clone(),
                tool: tool.to_string(),
                input,
            })
            .await?;
        match tokio::time::timeout(self.timeout, rx).await {
            Ok(Ok(outcome)) => Ok(outcome),
            Ok(Err(_)) => Ok(PresentationOutcome {
                ok: false,
                value: serde_json::json!({"error":"extension_unavailable"}),
            }),
            Err(_) => {
                self.pending.inner.lock().await.remove(&call_id);
                Ok(PresentationOutcome {
                    ok: false,
                    value: serde_json::json!({"error":"timeout"}),
                })
            }
        }
    }
}

// Required by axum/tower bounds.
impl std::fmt::Debug for PresentationDispatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PresentationDispatcher").finish()
    }
}

// Silence unused-import lint if Error not used directly anywhere.
#[allow(dead_code)]
fn _ensure_error_in_use(_e: Error) {}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::Mutex as TokioMutex;

    struct Recorder {
        last: TokioMutex<Option<PresentationCallFrame>>,
    }
    #[async_trait::async_trait]
    impl FrameOut for Recorder {
        async fn send_presentation_call(&self, frame: PresentationCallFrame) -> Result<()> {
            *self.last.lock().await = Some(frame);
            Ok(())
        }
    }

    #[tokio::test]
    async fn dispatch_resolves_via_pending_map() {
        let rec = Arc::new(Recorder {
            last: TokioMutex::new(None),
        });
        let pending = PendingCalls::new();
        let disp = PresentationDispatcher::new(rec.clone(), pending.clone());
        let task = tokio::spawn(async move {
            disp.dispatch("highlight_lines", serde_json::json!({"file":"a"}))
                .await
        });
        // Wait a moment for the dispatcher to register the call_id.
        for _ in 0..50 {
            if rec.last.lock().await.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        let call_id = rec.last.lock().await.as_ref().unwrap().call_id.clone();
        let delivered = pending
            .deliver(
                &call_id,
                PresentationOutcome {
                    ok: true,
                    value: serde_json::json!({"effect_id":"h1"}),
                },
            )
            .await;
        assert!(delivered);
        let outcome = task.await.unwrap().unwrap();
        assert!(outcome.ok);
    }

    #[tokio::test]
    async fn dispatch_times_out() {
        let rec = Arc::new(Recorder {
            last: TokioMutex::new(None),
        });
        let pending = PendingCalls::new();
        let disp =
            PresentationDispatcher::new(rec, pending).with_timeout(Duration::from_millis(10));
        let outcome = disp
            .dispatch("scroll_to", serde_json::json!({"file":"a"}))
            .await
            .unwrap();
        assert!(!outcome.ok);
        assert_eq!(outcome.value["error"], "timeout");
    }

    #[tokio::test]
    async fn unknown_call_id_drops() {
        let pending = PendingCalls::new();
        let delivered = pending
            .deliver(
                "p_unknown",
                PresentationOutcome {
                    ok: true,
                    value: serde_json::json!({}),
                },
            )
            .await;
        assert!(!delivered);
    }
}
