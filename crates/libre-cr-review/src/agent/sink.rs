//! `FrameSink`: streams WS / MCP frames to the caller during a turn.

use std::sync::Arc;

use async_trait::async_trait;
use libre_cr_common::ws_frames::{ServerFrame, UsageTally};
use tokio::sync::Mutex;

use crate::error::Result;

#[async_trait]
pub trait FrameSink: Send + Sync {
    async fn send(&self, frame: ServerFrame) -> Result<()>;

    async fn text_delta(&self, text: &str) -> Result<()> {
        self.send(ServerFrame::TextDelta {
            text: text.to_string(),
        })
        .await
    }

    async fn tool_call(&self, call_id: &str, name: &str, input: serde_json::Value) -> Result<()> {
        self.send(ServerFrame::ToolCall {
            call_id: call_id.to_string(),
            name: name.to_string(),
            input,
        })
        .await
    }

    async fn tool_result(&self, call_id: &str, result_preview: serde_json::Value) -> Result<()> {
        self.send(ServerFrame::ToolResult {
            call_id: call_id.to_string(),
            result_preview,
        })
        .await
    }

    async fn done(&self, turn_id: &str, usage: UsageTally) -> Result<()> {
        self.send(ServerFrame::Done {
            turn_id: turn_id.to_string(),
            usage,
        })
        .await
    }

    async fn error(&self, message: &str, recoverable: bool) -> Result<()> {
        self.send(ServerFrame::Error {
            message: message.to_string(),
            recoverable,
        })
        .await
    }
}

/// Test-only sink that records frames in memory.
#[derive(Default, Clone)]
pub struct RecordingSink {
    pub frames: Arc<Mutex<Vec<ServerFrame>>>,
}

impl RecordingSink {
    pub fn new() -> Self {
        Self::default()
    }
    pub async fn snapshot(&self) -> Vec<ServerFrame> {
        self.frames.lock().await.clone()
    }
}

#[async_trait]
impl FrameSink for RecordingSink {
    async fn send(&self, frame: ServerFrame) -> Result<()> {
        self.frames.lock().await.push(frame);
        Ok(())
    }
}
