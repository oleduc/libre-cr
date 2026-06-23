//! Application state shared across handlers.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

use crate::config::Config;
use crate::pairing::PairingStore;
use crate::provider::Provider;
use crate::storage::{InstallKey, Store};
use crate::tools::code_daemon::CodeDaemonClient;
use crate::worktree::SessionStatusBoard;

/// Wrap the config so it can be mutated at runtime via `POST /v1/config`.
#[derive(Clone)]
pub struct ConfigStore(pub Arc<Mutex<Config>>);

impl ConfigStore {
    pub fn new(cfg: Config) -> Self {
        Self(Arc::new(Mutex::new(cfg)))
    }
    pub async fn snapshot(&self) -> Config {
        self.0.lock().await.clone()
    }
}

/// Swappable provider holder (RC1). `POST /v1/config` rebuilds the provider
/// from the accepted config and swaps it in; the agent loop picks up the
/// current provider at turn start via [`ProviderHandle::get`].
#[derive(Clone)]
pub struct ProviderHandle(Arc<RwLock<Arc<dyn Provider>>>);

impl ProviderHandle {
    pub fn new(provider: Arc<dyn Provider>) -> Self {
        Self(Arc::new(RwLock::new(provider)))
    }

    /// Current provider snapshot. Cheap (one read lock + Arc clone).
    pub async fn get(&self) -> Arc<dyn Provider> {
        self.0.read().await.clone()
    }

    /// Swap in a freshly-built provider. In-flight turns keep their old
    /// Arc; new turns see the replacement.
    pub async fn set(&self, provider: Arc<dyn Provider>) {
        *self.0.write().await = provider;
    }
}

/// Single-flight registry of sessions with an in-flight ask. A `std` mutex
/// (not tokio) so the RAII release guard can run in `Drop`; critical
/// sections are a hash lookup — never held across `.await`.
pub type BusySessions = Arc<std::sync::Mutex<HashSet<String>>>;

/// Optional hook used by `GET /v1/health/code-daemon`. The CLI wires in the
/// real `SpawnedClient`'s health snapshot; tests with the mock client leave
/// this empty.
pub type HealthHook = Arc<
    dyn Fn() -> futures::future::BoxFuture<'static, crate::code_daemon::HealthSnapshot>
        + Send
        + Sync,
>;

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub config: ConfigStore,
    pub provider: ProviderHandle,
    pub code_daemon: Arc<dyn CodeDaemonClient>,
    pub token: String,
    pub extension_origin: String,
    /// Live CORS allowlist: updated when `/v1/pair` learns a new origin so
    /// the `AllowOrigin::predicate` built at startup reads current state.
    pub allowed_origin: Arc<std::sync::RwLock<String>>,
    pub install_key: Arc<InstallKey>,
    pub pairing: PairingStore,
    /// Sessions with an in-flight ask (single-flight per session).
    pub busy_sessions: BusySessions,
    pub session_status: SessionStatusBoard,
    pub health_hook: Option<HealthHook>,
    pub version: String,
    /// On-disk path of `review.toml`. `POST /v1/config` rewrites this file
    /// atomically after each accepted mutation. `None` in tests where the
    /// config is in-memory only.
    pub config_path: Option<PathBuf>,
}

pub struct AppStateBuilder {
    pub store: Store,
    pub config: ConfigStore,
    pub provider: Arc<dyn Provider>,
    pub code_daemon: Arc<dyn CodeDaemonClient>,
    pub token: String,
    pub extension_origin: String,
    pub install_key: Arc<InstallKey>,
    pub health_hook: Option<HealthHook>,
    pub config_path: Option<PathBuf>,
}

impl AppStateBuilder {
    pub fn build(self) -> AppState {
        AppState {
            store: self.store,
            config: self.config,
            provider: ProviderHandle::new(self.provider),
            code_daemon: self.code_daemon,
            token: self.token,
            allowed_origin: Arc::new(std::sync::RwLock::new(self.extension_origin.clone())),
            extension_origin: self.extension_origin,
            install_key: self.install_key,
            pairing: PairingStore::new(),
            busy_sessions: Arc::new(std::sync::Mutex::new(HashSet::new())),
            session_status: SessionStatusBoard::new(),
            health_hook: self.health_hook,
            version: env!("CARGO_PKG_VERSION").to_string(),
            config_path: self.config_path,
        }
    }
}
