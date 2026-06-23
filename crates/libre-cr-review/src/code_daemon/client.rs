//! Spawned/external MCP client. Implements `CodeDaemonClient`.
//!
//! Architecture:
//!
//! - An actor task owns the [`Connection`] and serializes writes + dispatches
//!   reads. It maintains a `HashMap<id, oneshot::Sender<Value>>` of pending
//!   calls; when a response line arrives it routes it back.
//! - The public client holds an `mpsc::Sender<Request>`. `call()` and
//!   `list_tools()` enqueue requests, await a oneshot reply, and bubble
//!   errors / timeouts as `Error::CodeDaemonUnavailable` when the budget is
//!   exhausted.
//! - On connection failure (EOF, write error, timeout) the actor closes the
//!   connection, consults the [`RestartBudget`], optionally reopens, and
//!   resumes serving requests. Any in-flight oneshots are answered with a
//!   "code daemon unavailable" error so callers don't hang.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time::timeout;

use crate::config::{expand_path, CodeDaemonConfig};
use crate::error::{Error, Result};
use crate::provider::ToolSchema;
use crate::tools::code_daemon::CodeDaemonClient;

use super::budget::RestartBudget;
use super::transport::{Connection, TransportSpec};

const CALL_TIMEOUT: Duration = Duration::from_secs(10);
const INITIAL_BACKOFF: Duration = Duration::from_millis(100);
const MAX_BACKOFF: Duration = Duration::from_secs(5);

/// Outbound request to the actor.
struct PendingCall {
    method: &'static str,
    params: Value,
    reply: oneshot::Sender<Result<Value>>,
}

#[derive(Debug, Clone, Default)]
pub struct HealthSnapshot {
    pub connected: bool,
    pub restart_count: u32,
    pub last_error: Option<String>,
    pub version: Option<String>,
}

/// The real MCP client.
pub struct SpawnedClient {
    tx: mpsc::Sender<PendingCall>,
    schemas: Arc<Mutex<Vec<ToolSchema>>>,
    health: Arc<Mutex<HealthSnapshot>>,
}

impl SpawnedClient {
    /// Build from review-daemon config. Connects + caches schemas synchronously
    /// during construction so callers see a ready client on `Ok(...)`.
    pub async fn from_config(cfg: &CodeDaemonConfig) -> Result<Self> {
        let spec = match cfg.mode.as_str() {
            "spawn" => TransportSpec::Spawn {
                binary: resolve_binary(&cfg.binary),
            },
            "external" => {
                if cfg.external_socket.trim().is_empty() {
                    return Err(Error::Validation(
                        "code_daemon.mode = \"external\" requires external_socket".into(),
                    ));
                }
                TransportSpec::ExternalSocket {
                    path: expand_path(&cfg.external_socket),
                }
            }
            other => {
                return Err(Error::Validation(format!(
                    "unknown code_daemon.mode: {other}"
                )))
            }
        };

        let budget = RestartBudget::new(cfg.max_restarts_per_hour.max(1));
        let restart_on_failure = cfg.restart_on_failure;
        Self::start(spec, budget, restart_on_failure).await
    }

    pub async fn start(
        spec: TransportSpec,
        budget: RestartBudget,
        restart_on_failure: bool,
    ) -> Result<Self> {
        let (tx, rx) = mpsc::channel::<PendingCall>(64);
        let schemas = Arc::new(Mutex::new(Vec::new()));
        let health = Arc::new(Mutex::new(HealthSnapshot::default()));
        let actor = Actor {
            spec,
            budget,
            restart_on_failure,
            rx,
            health: health.clone(),
        };
        // Open + initial handshake on the calling task so we surface fatal
        // failures synchronously.
        let mut conn = Connection::open(&actor.spec).await?;
        let init = handshake(&mut conn).await?;
        let tools = list_tools_initial(&mut conn).await?;
        {
            let mut g = schemas.lock().await;
            *g = tools.clone();
        }
        {
            let mut g = health.lock().await;
            g.connected = true;
            g.version = init.server_version;
        }
        tokio::spawn(actor.run(conn));
        Ok(Self {
            tx,
            schemas,
            health,
        })
    }

    pub async fn health(&self) -> HealthSnapshot {
        self.health.lock().await.clone()
    }

    async fn send(&self, method: &'static str, params: Value) -> Result<Value> {
        let (reply, rx) = oneshot::channel();
        let pending = PendingCall {
            method,
            params,
            reply,
        };
        self.tx
            .send(pending)
            .await
            .map_err(|_| Error::CodeDaemonUnavailable)?;
        match timeout(CALL_TIMEOUT, rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err(Error::CodeDaemonUnavailable),
            Err(_) => Err(Error::Internal("code daemon call timeout".into())),
        }
    }
}

#[async_trait]
impl CodeDaemonClient for SpawnedClient {
    async fn list_tools(&self) -> Result<Vec<ToolSchema>> {
        Ok(self.schemas.lock().await.clone())
    }

    async fn call(&self, name: &str, input: Value) -> Result<Value> {
        let params = serde_json::json!({
            "name": name,
            "arguments": input,
        });
        let v = self.send("tools/call", params).await?;
        // Code daemon returns `{ content: [{type:"text", text: "<json>"}], isError }`.
        // Unwrap into the inner envelope for the agent loop.
        Ok(unwrap_mcp_content(v))
    }
}

fn resolve_binary(path: &str) -> PathBuf {
    if path.contains('/') || path.contains('\\') {
        expand_path(path)
    } else {
        // Just a bare command — leave it for the OS to resolve via PATH.
        PathBuf::from(path)
    }
}

fn unwrap_mcp_content(v: Value) -> Value {
    // Best-effort: pull the first text part and parse it as JSON; on failure,
    // return the wrapper as-is so the agent still sees something.
    let Some(content) = v.get("content").and_then(|c| c.as_array()) else {
        return v;
    };
    for part in content {
        if part.get("type").and_then(|t| t.as_str()) == Some("text") {
            if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                if let Ok(inner) = serde_json::from_str::<Value>(text) {
                    return inner;
                }
            }
        }
    }
    v
}

struct InitInfo {
    server_version: Option<String>,
}

async fn handshake(conn: &mut Connection) -> Result<InitInfo> {
    // initialize
    let id = 1u64;
    let req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "libre-cr-review", "version": env!("CARGO_PKG_VERSION") }
        }
    });
    conn.write_line(&req.to_string()).await?;
    let resp = read_response(conn, id).await?;
    let server_version = resp
        .get("serverInfo")
        .and_then(|s| s.get("version"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    // notifications/initialized (no reply expected)
    let notif = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    conn.write_line(&notif.to_string()).await?;
    Ok(InitInfo { server_version })
}

async fn list_tools_initial(conn: &mut Connection) -> Result<Vec<ToolSchema>> {
    let id = 2u64;
    let req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/list",
        "params": {}
    });
    conn.write_line(&req.to_string()).await?;
    let resp = read_response(conn, id).await?;
    let mut out = Vec::new();
    if let Some(arr) = resp.get("tools").and_then(|t| t.as_array()) {
        for t in arr {
            let name = t
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if name.is_empty() {
                continue;
            }
            let description = t
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let input_schema = t
                .get("inputSchema")
                .cloned()
                .unwrap_or(serde_json::json!({"type":"object"}));
            out.push(ToolSchema {
                name,
                description,
                input_schema,
            });
        }
    }
    Ok(out)
}

/// Read responses until one matches `want_id`. Skips notifications.
async fn read_response(conn: &mut Connection, want_id: u64) -> Result<Value> {
    loop {
        let line = conn
            .read_line()
            .await?
            .ok_or_else(|| Error::Internal("code daemon EOF during handshake".into()))?;
        let v: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        // Only handle responses (have id + result/error).
        let id = v.get("id").and_then(|x| x.as_u64()).unwrap_or(u64::MAX);
        if id != want_id {
            continue;
        }
        if let Some(err) = v.get("error") {
            let msg = err
                .get("message")
                .and_then(|x| x.as_str())
                .unwrap_or("unknown")
                .to_string();
            return Err(Error::Internal(format!("code daemon error: {msg}")));
        }
        return Ok(v.get("result").cloned().unwrap_or(Value::Null));
    }
}

struct Actor {
    spec: TransportSpec,
    budget: RestartBudget,
    restart_on_failure: bool,
    rx: mpsc::Receiver<PendingCall>,
    health: Arc<Mutex<HealthSnapshot>>,
}

impl Actor {
    async fn run(mut self, conn: Connection) {
        // Wrap in an Option so we can take it out for restart.
        let mut conn = Some(conn);
        let mut next_id: u64 = 100;
        let mut pending: HashMap<u64, oneshot::Sender<Result<Value>>> = HashMap::new();
        let mut backoff = INITIAL_BACKOFF;

        loop {
            // Make sure we have a live connection.
            if conn.is_none() {
                if !self.restart_on_failure {
                    self.mark_disconnected("connection lost; restart disabled")
                        .await;
                    self.fail_pending_and_drain(&mut pending).await;
                    return;
                }
                if self.budget.would_exceed(Instant::now()) {
                    self.mark_disconnected("restart budget exhausted").await;
                    // Drain incoming requests with an error so callers don't hang.
                    self.fail_pending_and_drain(&mut pending).await;
                    return;
                }
                let _ = self.budget.record(Instant::now());
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(MAX_BACKOFF);
                match Connection::open(&self.spec).await {
                    Ok(mut c) => match handshake(&mut c).await {
                        Ok(init) => {
                            {
                                let mut g = self.health.lock().await;
                                g.connected = true;
                                g.restart_count = self.budget.count(Instant::now());
                                g.version = init.server_version;
                                g.last_error = None;
                            }
                            backoff = INITIAL_BACKOFF;
                            conn = Some(c);
                        }
                        Err(e) => {
                            self.mark_disconnected(&format!("handshake: {e}")).await;
                            continue;
                        }
                    },
                    Err(e) => {
                        self.mark_disconnected(&format!("reconnect: {e}")).await;
                        continue;
                    }
                }
            }

            let c = conn.as_mut().expect("connection present");

            tokio::select! {
                pcall = self.rx.recv() => {
                    let Some(pcall) = pcall else { break; };
                    next_id += 1;
                    let id = next_id;
                    let req = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "method": pcall.method,
                        "params": pcall.params,
                    });
                    if let Err(e) = c.write_line(&req.to_string()).await {
                        let _ = pcall.reply.send(Err(Error::CodeDaemonUnavailable));
                        self.mark_disconnected(&format!("write: {e}")).await;
                        let dropped = conn.take().unwrap();
                        dropped.shutdown().await;
                        self.fail_pending(&mut pending).await;
                        continue;
                    }
                    pending.insert(id, pcall.reply);
                }
                read = c.read_line() => {
                    match read {
                        Ok(Some(line)) => {
                            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                                if let Some(id) = v.get("id").and_then(|x| x.as_u64()) {
                                    if let Some(reply) = pending.remove(&id) {
                                        if let Some(err) = v.get("error") {
                                            let msg = err.get("message").and_then(|x|x.as_str()).unwrap_or("unknown").to_string();
                                            let _ = reply.send(Err(Error::Internal(format!("code daemon: {msg}"))));
                                        } else {
                                            let _ = reply.send(Ok(v.get("result").cloned().unwrap_or(Value::Null)));
                                        }
                                    }
                                }
                            }
                        }
                        Ok(None) => {
                            // EOF
                            self.mark_disconnected("eof").await;
                            let dropped = conn.take().unwrap();
                            dropped.shutdown().await;
                            self.fail_pending(&mut pending).await;
                        }
                        Err(e) => {
                            self.mark_disconnected(&format!("read: {e}")).await;
                            let dropped = conn.take().unwrap();
                            dropped.shutdown().await;
                            self.fail_pending(&mut pending).await;
                        }
                    }
                }
            }
        }
        if let Some(c) = conn.take() {
            c.shutdown().await;
        }
    }

    async fn mark_disconnected(&self, why: &str) {
        let mut g = self.health.lock().await;
        g.connected = false;
        g.last_error = Some(why.to_string());
    }

    async fn fail_pending(&self, pending: &mut HashMap<u64, oneshot::Sender<Result<Value>>>) {
        for (_id, reply) in pending.drain() {
            let _ = reply.send(Err(Error::CodeDaemonUnavailable));
        }
    }

    /// Called when we've decided to give up. Drains the queue too so callers
    /// see `CodeDaemonUnavailable` instead of hanging.
    async fn fail_pending_and_drain(
        &mut self,
        pending: &mut HashMap<u64, oneshot::Sender<Result<Value>>>,
    ) {
        self.fail_pending(pending).await;
        while let Ok(pcall) = self.rx.try_recv() {
            let _ = pcall.reply.send(Err(Error::CodeDaemonUnavailable));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwrap_mcp_content_parses_inner_json() {
        let v = serde_json::json!({
            "content": [{ "type": "text", "text": "{\"ok\":true,\"matches\":[1,2,3]}" }],
            "isError": false
        });
        let r = unwrap_mcp_content(v);
        assert_eq!(r["ok"], true);
        assert_eq!(r["matches"][0], 1);
    }

    #[test]
    fn unwrap_mcp_content_passes_unwrapped_through() {
        let v = serde_json::json!({"already": "unwrapped"});
        let r = unwrap_mcp_content(v.clone());
        assert_eq!(r, v);
    }

    #[test]
    fn unwrap_mcp_content_returns_original_when_text_isnt_json() {
        let v = serde_json::json!({
            "content": [{ "type": "text", "text": "plain text" }]
        });
        let r = unwrap_mcp_content(v.clone());
        assert_eq!(r, v);
    }
}
