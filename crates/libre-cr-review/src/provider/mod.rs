//! Provider abstraction. Mirrors `specs/04-review-daemon.md` § LLM Provider Layer.

mod anthropic;
mod mock;
mod openai_compat;

pub use anthropic::AnthropicProvider;
pub use mock::MockProvider;
pub use openai_compat::OpenAICompatProvider;

use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::error::{Error, Result};
use crate::storage::{decrypt_value, InstallKey};

/// One step in the streaming response from an LLM. The provider trait yields
/// these as a stream; the agent loop consumes them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// Incremental text from the assistant.
    TextDelta { text: String },
    /// The assistant decided to call a tool.
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    /// End of message.
    Done {
        #[serde(default)]
        input_tokens: u64,
        #[serde(default)]
        output_tokens: u64,
        #[serde(default)]
        stop_reason: String,
    },
    /// Provider-level error.
    Error { message: String },
}

/// LLM chat message — provider-neutral shape, modeled on Anthropic's roles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
}

/// JSON-schema description of a tool exposed to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> &str;

    async fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
    ) -> Result<BoxStream<'static, Result<StreamEvent>>>;

    async fn validate(&self) -> Result<()>;
}

/// Construct a provider from a config snapshot. Used at startup and on
/// every accepted `POST /v1/config` so the running daemon picks up provider
/// changes without a restart (RC1).
pub fn build_provider(cfg: &Config, install_key: &InstallKey) -> Result<Arc<dyn Provider>> {
    match cfg.provider.kind.as_str() {
        "mock" => Ok(Arc::new(MockProvider::new(
            cfg.mock.provider_script.clone(),
        ))),
        "anthropic" => {
            let key = decrypt_value(install_key, &cfg.provider.api_key_enc).unwrap_or_default();
            let p = AnthropicProvider::new(
                key,
                cfg.provider.model.clone(),
                cfg.provider.max_tokens,
                cfg.provider.temperature,
            )
            .with_endpoint(cfg.provider.endpoint.clone());
            Ok(Arc::new(p))
        }
        "openai_compat" => {
            let key = decrypt_value(install_key, &cfg.provider.api_key_enc).unwrap_or_default();
            let p = OpenAICompatProvider::new(
                key,
                cfg.provider.model.clone(),
                cfg.provider.max_tokens,
                cfg.provider.temperature,
            )
            .with_endpoint(cfg.provider.endpoint.clone());
            Ok(Arc::new(p))
        }
        other => Err(Error::Validation(format!("unknown provider kind: {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_event_round_trip() {
        let e = StreamEvent::TextDelta { text: "hi".into() };
        let s = serde_json::to_string(&e).unwrap();
        let _back: StreamEvent = serde_json::from_str(&s).unwrap();
    }

    #[test]
    fn tool_schema_serializes() {
        let t = ToolSchema {
            name: "grep".into(),
            description: "search".into(),
            input_schema: serde_json::json!({"type":"object"}),
        };
        let s = serde_json::to_string(&t).unwrap();
        assert!(s.contains("\"grep\""));
    }
}
