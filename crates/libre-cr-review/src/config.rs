//! Review daemon configuration (`~/.config/libre-cr/review.toml`).
//!
//! Layout mirrors `specs/04-review-daemon.md` § Configuration.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::provider::StreamEvent;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub provider: ProviderConfig,
    #[serde(default)]
    pub code_daemon: CodeDaemonConfig,
    #[serde(default)]
    pub mcp_server: McpServerConfig,
    #[serde(default)]
    pub global_instructions: GlobalInstructions,
    #[serde(default)]
    pub limits: Limits,
    /// Mock-mode toggles, only used in tests / Phase 2 verification.
    #[serde(default)]
    pub mock: MockConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub bind: String,
    pub port: u16,
    pub endpoint_file: String,
    pub token_file: String,
    pub install_key_file: String,
    pub extension_origin: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".into(),
            port: 0,
            endpoint_file: "~/.config/libre-cr/endpoint".into(),
            token_file: "~/.config/libre-cr/token".into(),
            install_key_file: "~/.config/libre-cr/install_key".into(),
            extension_origin: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub data_dir: String,
    pub db: String,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            data_dir: "~/.local/share/libre-cr-review".into(),
            db: "~/.local/share/libre-cr-review/state.db".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub kind: String,
    #[serde(default)]
    pub api_key_enc: String,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    #[serde(default)]
    pub endpoint: String,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            kind: "mock".into(),
            api_key_enc: String::new(),
            model: "mock-model".into(),
            max_tokens: 4096,
            temperature: 0.0,
            endpoint: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeDaemonConfig {
    pub mode: String,
    pub binary: String,
    #[serde(default)]
    pub external_socket: String,
    pub restart_on_failure: bool,
    pub max_restarts_per_hour: u32,
}

impl Default for CodeDaemonConfig {
    fn default() -> Self {
        Self {
            mode: "spawn".into(),
            binary: "libre-cr-code".into(),
            external_socket: String::new(),
            restart_on_failure: true,
            max_restarts_per_hour: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub enabled: bool,
    pub stdio: bool,
    pub sse: bool,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            stdio: true,
            sse: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GlobalInstructions {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Limits {
    pub max_tool_turns: u32,
    pub max_history_messages: u32,
    pub session_idle_evict_days: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_tool_turns: 25,
            max_history_messages: 30,
            session_idle_evict_days: 90,
        }
    }
}

/// Mock toggles used for the Phase 2 smoke flow. Not persisted by default —
/// tests and the integration harness set these in memory.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MockConfig {
    /// If true, sessions get a fake worktree path immediately.
    #[serde(default)]
    pub code_intel: bool,
    /// Script the mock provider replays, in order. Each event optionally
    /// delayed by `delay_ms`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_script: Vec<ScriptedEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptedEvent {
    #[serde(default)]
    pub delay_ms: u64,
    pub event: StreamEvent,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let s = std::fs::read_to_string(path)?;
        let cfg: Config =
            toml::from_str(&s).map_err(|e| Error::Internal(format!("parse review.toml: {e}")))?;
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let s = toml::to_string_pretty(self)
            .map_err(|e| Error::Internal(format!("write review.toml: {e}")))?;
        std::fs::write(path, s)?;
        Ok(())
    }

    /// Default config path under `$HOME/.config/libre-cr/review.toml`.
    pub fn default_path() -> PathBuf {
        let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        base.join("libre-cr").join("review.toml")
    }
}

/// Expand `~/` and environment variables in a path string.
pub fn expand_path(s: &str) -> PathBuf {
    let expanded = shellexpand::tilde(s).to_string();
    PathBuf::from(expanded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_stable() {
        let c = Config::default();
        assert_eq!(c.server.bind, "127.0.0.1");
        assert_eq!(c.limits.max_tool_turns, 25);
        assert_eq!(c.limits.max_history_messages, 30);
        assert_eq!(c.provider.kind, "mock");
    }

    #[test]
    fn round_trip_toml() {
        let c = Config::default();
        let s = toml::to_string(&c).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back.server.bind, c.server.bind);
        assert_eq!(back.limits.max_tool_turns, c.limits.max_tool_turns);
    }

    #[test]
    fn expand_tilde() {
        let p = expand_path("~/foo");
        assert!(!p.to_string_lossy().starts_with('~'));
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("review.toml");
        let mut c = Config::default();
        c.server.port = 1234;
        c.save(&p).unwrap();
        let back = Config::load(&p).unwrap();
        assert_eq!(back.server.port, 1234);
    }
}
