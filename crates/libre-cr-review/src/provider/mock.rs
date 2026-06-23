//! Deterministic scripted provider — replays `Vec<ScriptedEvent>` for tests.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use tokio::sync::Mutex;

use crate::config::ScriptedEvent;
use crate::error::Result;

use super::{Message, Provider, StreamEvent, ToolSchema};

/// Provider that replays a script. Each call to `stream` consumes the next
/// "burst" — events up to and including the next `Done` (or `Error`).
///
/// This lets the agent loop drive several LLM rounds within one test scenario:
/// burst 1 emits a `tool_use` then `Done`, the agent dispatches the tool,
/// loops, and we replay burst 2 with the final `text_delta` + `Done`.
#[derive(Clone)]
pub struct MockProvider {
    id: String,
    queue: Arc<Mutex<Vec<ScriptedEvent>>>,
}

impl MockProvider {
    pub fn new(events: Vec<ScriptedEvent>) -> Self {
        Self {
            id: "mock".into(),
            queue: Arc::new(Mutex::new(events)),
        }
    }
}

#[async_trait]
impl Provider for MockProvider {
    fn id(&self) -> &str {
        &self.id
    }

    async fn stream(
        &self,
        _messages: &[Message],
        _tools: &[ToolSchema],
    ) -> Result<BoxStream<'static, Result<StreamEvent>>> {
        // Pull events for this turn: everything until (and including) the
        // next terminator (`Done` or `Error`). If the queue is empty, the
        // stream is empty (the agent loop will error out cleanly).
        let mut burst = Vec::new();
        {
            let mut q = self.queue.lock().await;
            while let Some(ev) = q.first().cloned() {
                q.remove(0);
                let is_terminator = matches!(
                    ev.event,
                    StreamEvent::Done { .. } | StreamEvent::Error { .. }
                );
                burst.push(ev);
                if is_terminator {
                    break;
                }
            }
        }
        let s = futures::stream::iter(burst).then(|ev| async move {
            if ev.delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(ev.delay_ms)).await;
            }
            Ok::<StreamEvent, crate::error::Error>(ev.event)
        });
        Ok(s.boxed())
    }

    async fn validate(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn replays_in_order() {
        let p = MockProvider::new(vec![
            ScriptedEvent {
                delay_ms: 0,
                event: StreamEvent::TextDelta { text: "a".into() },
            },
            ScriptedEvent {
                delay_ms: 0,
                event: StreamEvent::TextDelta { text: "b".into() },
            },
            ScriptedEvent {
                delay_ms: 0,
                event: StreamEvent::Done {
                    input_tokens: 1,
                    output_tokens: 2,
                    stop_reason: "end_turn".into(),
                },
            },
        ]);
        let mut s = p.stream(&[], &[]).await.unwrap();
        let mut texts = Vec::new();
        while let Some(ev) = s.next().await {
            let ev = ev.unwrap();
            match ev {
                StreamEvent::TextDelta { text } => texts.push(text),
                StreamEvent::Done { .. } => break,
                _ => {}
            }
        }
        assert_eq!(texts, vec!["a".to_string(), "b".into()]);
    }

    #[tokio::test]
    async fn second_burst_after_tool() {
        let p = MockProvider::new(vec![
            ScriptedEvent {
                delay_ms: 0,
                event: StreamEvent::ToolUse {
                    id: "t1".into(),
                    name: "grep".into(),
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
        ]);
        // First burst
        let mut s1 = p.stream(&[], &[]).await.unwrap();
        let mut saw_tool = false;
        let mut saw_done1 = false;
        while let Some(ev) = s1.next().await {
            match ev.unwrap() {
                StreamEvent::ToolUse { .. } => saw_tool = true,
                StreamEvent::Done { .. } => {
                    saw_done1 = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(saw_tool && saw_done1);
        // Second burst
        let mut s2 = p.stream(&[], &[]).await.unwrap();
        let mut saw_text = false;
        while let Some(ev) = s2.next().await {
            if let StreamEvent::TextDelta { text } = ev.unwrap() {
                if text == "ok" {
                    saw_text = true;
                }
            }
        }
        assert!(saw_text);
    }
}
