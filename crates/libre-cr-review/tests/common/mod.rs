//! Shared test harness: spin up the server **in-process** on an ephemeral
//! port. This is fine for integration coverage of the axum router.
//!
//! Real-binary E2E suites live in the `libre-cr-e2e` crate, not here.

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;

use libre_cr_review::config::{Config, ScriptedEvent};
use libre_cr_review::pairing::PairingStore;
use libre_cr_review::provider::{MockProvider, Provider, StreamEvent};
use libre_cr_review::server::{serve, AppStateBuilder, ConfigStore, ListenInfo};
use libre_cr_review::storage::{InstallKey, Store};
use libre_cr_review::tools::code_daemon::{CodeDaemonClient, MockCodeDaemonClient};

pub struct Harness {
    pub addr: SocketAddr,
    pub token: String,
    pub pairing: PairingStore,
    pub store: Store,
    pub _task: tokio::task::JoinHandle<std::io::Result<()>>,
}

pub async fn start_server_with_script(script: Vec<ScriptedEvent>) -> Harness {
    start_server_with_provider(Arc::new(MockProvider::new(script))).await
}

pub async fn start_server_default() -> Harness {
    start_server_with_provider(Arc::new(MockProvider::new(vec![]))).await
}

pub async fn start_server_with_provider(provider: Arc<dyn Provider>) -> Harness {
    let mut cfg = Config::default();
    cfg.mock.code_intel = true;
    start_server_with(cfg, provider, None).await
}

/// Fully-parameterized harness: custom config (e.g. a scripted
/// `mock.provider_script` the daemon rebuilds from on `POST /v1/config`)
/// and an optional on-disk `review.toml` path for persistence assertions.
pub async fn start_server_with(
    cfg: Config,
    provider: Arc<dyn Provider>,
    config_path: Option<std::path::PathBuf>,
) -> Harness {
    let store = Store::open_in_memory().unwrap();
    let install_key = Arc::new(InstallKey::from_bytes([0u8; 32]));
    let code_daemon: Arc<dyn CodeDaemonClient> = Arc::new(MockCodeDaemonClient);
    let state = AppStateBuilder {
        store: store.clone(),
        config: ConfigStore::new(cfg),
        provider,
        code_daemon,
        token: "test-token".into(),
        extension_origin: String::new(),
        install_key,
        health_hook: None,
        config_path,
    }
    .build();
    let pairing = state.pairing.clone();
    let bind: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let ListenInfo { addr, task } = serve(state, bind).await.unwrap();
    Harness {
        addr,
        token: "test-token".into(),
        pairing,
        store,
        _task: task,
    }
}

pub fn happy_two_round_script() -> Vec<ScriptedEvent> {
    vec![
        ScriptedEvent {
            delay_ms: 0,
            event: StreamEvent::TextDelta {
                text: "Looking…".into(),
            },
        },
        ScriptedEvent {
            delay_ms: 0,
            event: StreamEvent::ToolUse {
                id: "t1".into(),
                name: "grep".into(),
                input: serde_json::json!({"query":"bcryptHash"}),
            },
        },
        ScriptedEvent {
            delay_ms: 0,
            event: StreamEvent::Done {
                input_tokens: 1,
                output_tokens: 2,
                stop_reason: "tool_use".into(),
            },
        },
        ScriptedEvent {
            delay_ms: 0,
            event: StreamEvent::TextDelta {
                text: "Found 1 match.".into(),
            },
        },
        ScriptedEvent {
            delay_ms: 0,
            event: StreamEvent::Done {
                input_tokens: 3,
                output_tokens: 4,
                stop_reason: "end_turn".into(),
            },
        },
    ]
}
