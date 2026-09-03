//! Provider abstraction. Mirrors `specs/04-review-daemon.md` § LLM Provider Layer.

mod anthropic;
mod mock;
mod openai_compat;

pub use anthropic::AnthropicProvider;
pub use mock::MockProvider;
pub use openai_compat::OpenAICompatProvider;

/// A model offered by a provider. Single definition lives in
/// `libre-cr-common` so the provider layer and the HTTP wire contract can't
/// drift; we re-export it here for the provider modules.
pub use libre_cr_common::http_api::ModelInfo;

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

    /// List the models this provider offers. Providers opt in; the default is
    /// a validation error so the config UI can show "not supported" without
    /// every provider having to implement it.
    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Err(Error::Validation("model listing not supported".into()))
    }
}

/// Accept either a provider *base* URL (`https://host/v1`, what every
/// OpenAI-compatible service documents and what our docs promise) or the full
/// request URL. A base ending in `/v1` gets `path` appended; anything else is
/// used verbatim.
pub(crate) fn normalize_endpoint(endpoint: &str, path: &str) -> String {
    let e = endpoint.trim().trim_end_matches('/');
    if e.ends_with("/v1") {
        format!("{e}{path}")
    } else {
        e.to_string()
    }
}

/// Resolve the effective API key: the stored (decrypted) key always wins;
/// when it is empty we fall back to the standard ambient environment variable
/// for the provider kind (`ANTHROPIC_API_KEY` / `OPENAI_API_KEY`). This lets a
/// user with the env var set never have to paste a key. Env vars only.
fn resolve_api_key(stored: &str, env_var: &str) -> String {
    if !stored.is_empty() {
        return stored.to_string();
    }
    std::env::var(env_var).unwrap_or_default()
}

/// Decrypt the stored key and apply [`resolve_api_key`]. A *non-empty* stored
/// key that fails to decrypt is an error — falling back to the ambient env var
/// there would silently route reviews through a different account's key.
fn stored_or_env_key(cfg: &Config, install_key: &InstallKey, env_var: &str) -> Result<String> {
    let stored = if cfg.provider.api_key_enc.is_empty() {
        String::new()
    } else {
        decrypt_value(install_key, &cfg.provider.api_key_enc).map_err(|e| {
            Error::Validation(format!(
                "stored API key could not be decrypted ({e}); re-enter it in the config UI"
            ))
        })?
    };
    Ok(resolve_api_key(&stored, env_var))
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
            let key = stored_or_env_key(cfg, install_key, "ANTHROPIC_API_KEY")?;
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
            let key = stored_or_env_key(cfg, install_key, "OPENAI_API_KEY")?;
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

/// Build the concrete Anthropic provider exactly as `build_provider` does,
/// returning it concretely so tests can read the resolved key without
/// downcasting a `dyn Provider`. Mirrors the `anthropic` arm above.
#[cfg(test)]
fn build_anthropic_for_test(cfg: &Config, install_key: &InstallKey) -> AnthropicProvider {
    let key = stored_or_env_key(cfg, install_key, "ANTHROPIC_API_KEY").expect("decrypt");
    AnthropicProvider::new(
        key,
        cfg.provider.model.clone(),
        cfg.provider.max_tokens,
        cfg.provider.temperature,
    )
    .with_endpoint(cfg.provider.endpoint.clone())
}

/// As [`build_anthropic_for_test`] but for the OpenAI-compatible arm.
#[cfg(test)]
fn build_openai_for_test(cfg: &Config, install_key: &InstallKey) -> OpenAICompatProvider {
    let key = stored_or_env_key(cfg, install_key, "OPENAI_API_KEY").expect("decrypt");
    OpenAICompatProvider::new(
        key,
        cfg.provider.model.clone(),
        cfg.provider.max_tokens,
        cfg.provider.temperature,
    )
    .with_endpoint(cfg.provider.endpoint.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::encrypt_value;

    #[test]
    fn undecryptable_stored_key_is_an_error_not_an_env_fallback() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("ANTHROPIC_API_KEY", "sk-ant-env");
        let mut cfg = Config::default();
        cfg.provider.kind = "anthropic".into();
        cfg.provider.api_key_enc = "not-a-ciphertext".into();
        let result = build_provider(&cfg, &InstallKey::from_bytes([7u8; 32]));
        std::env::remove_var("ANTHROPIC_API_KEY");
        match result {
            Err(Error::Validation(msg)) => assert!(msg.contains("could not be decrypted"), "{msg}"),
            other => panic!(
                "expected Validation error, got {:?}",
                other.map(|_| "provider")
            ),
        }
    }

    /// Serializes all tests that mutate process-global environment variables.
    /// `cargo test` runs tests in the same crate concurrently, so two env
    /// tests racing on `ANTHROPIC_API_KEY` would flake without this guard.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn resolve_api_key_prefers_stored_over_env() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("ANTHROPIC_API_KEY", "from-env");
        assert_eq!(
            resolve_api_key("from-stored", "ANTHROPIC_API_KEY"),
            "from-stored"
        );
        std::env::remove_var("ANTHROPIC_API_KEY");
    }

    #[test]
    fn resolve_api_key_falls_back_to_env_when_stored_empty() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("OPENAI_API_KEY", "env-key");
        assert_eq!(resolve_api_key("", "OPENAI_API_KEY"), "env-key");
        std::env::remove_var("OPENAI_API_KEY");
        assert_eq!(resolve_api_key("", "OPENAI_API_KEY"), "");
    }

    #[test]
    fn build_provider_anthropic_uses_env_key_when_stored_empty() {
        let _g = ENV_LOCK.lock().unwrap();
        let key = InstallKey::from_bytes([7u8; 32]);
        let mut cfg = Config::default();
        cfg.provider.kind = "anthropic".into();
        cfg.provider.api_key_enc = String::new(); // nothing stored
        std::env::set_var("ANTHROPIC_API_KEY", "sk-ant-env");
        let p = build_anthropic_for_test(&cfg, &key);
        assert_eq!(p.api_key_for_test(), "sk-ant-env");
        std::env::remove_var("ANTHROPIC_API_KEY");
    }

    #[test]
    fn build_provider_anthropic_stored_key_wins() {
        let _g = ENV_LOCK.lock().unwrap();
        let key = InstallKey::from_bytes([7u8; 32]);
        let mut cfg = Config::default();
        cfg.provider.kind = "anthropic".into();
        cfg.provider.api_key_enc = encrypt_value(&key, "sk-ant-stored").unwrap();
        std::env::set_var("ANTHROPIC_API_KEY", "sk-ant-env");
        let p = build_anthropic_for_test(&cfg, &key);
        assert_eq!(p.api_key_for_test(), "sk-ant-stored");
        std::env::remove_var("ANTHROPIC_API_KEY");
    }

    #[test]
    fn build_provider_openai_uses_env_key_when_stored_empty() {
        let _g = ENV_LOCK.lock().unwrap();
        let key = InstallKey::from_bytes([7u8; 32]);
        let mut cfg = Config::default();
        cfg.provider.kind = "openai_compat".into();
        cfg.provider.api_key_enc = String::new();
        std::env::set_var("OPENAI_API_KEY", "sk-oai-env");
        let p = build_openai_for_test(&cfg, &key);
        assert_eq!(p.api_key_for_test(), "sk-oai-env");
        std::env::remove_var("OPENAI_API_KEY");
    }

    #[test]
    fn build_provider_openai_stored_key_wins() {
        let _g = ENV_LOCK.lock().unwrap();
        let key = InstallKey::from_bytes([7u8; 32]);
        let mut cfg = Config::default();
        cfg.provider.kind = "openai_compat".into();
        cfg.provider.api_key_enc = encrypt_value(&key, "sk-oai-stored").unwrap();
        std::env::set_var("OPENAI_API_KEY", "sk-oai-env");
        let p = build_openai_for_test(&cfg, &key);
        assert_eq!(p.api_key_for_test(), "sk-oai-stored");
        std::env::remove_var("OPENAI_API_KEY");
    }

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
