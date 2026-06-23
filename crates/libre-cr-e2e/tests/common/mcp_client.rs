//! Minimal MCP client used by the E2E consumer suite.
//!
//! Spawns `libre-cr-code` (built lazily via `cargo build`) as a child process
//! and frames JSON-RPC 2.0 over its stdio (or, in the socket case, over a
//! `tokio::net::UnixStream`). Tests use the wrappers to issue `initialize`,
//! `tools/list`, and `tools/call` without re-implementing the framing.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Once;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;
use tokio::time::timeout;

static BUILD: Once = Once::new();
static BUILT_OK: AtomicBool = AtomicBool::new(false);

/// In CI (`LIBRE_CR_E2E_REQUIRED=1`) a missing binary is a hard failure,
/// never a silent skip — otherwise a broken Rust build shows green E2E.
pub fn e2e_required() -> bool {
    std::env::var("LIBRE_CR_E2E_REQUIRED").as_deref() == Ok("1")
}

/// Locate (and lazily build) the `libre-cr-code` binary. Returns `None` if
/// the binary can't be produced — tests that get `None` from
/// [`code_bin_or_skip`] should `return` and let themselves pass as a no-op,
/// matching the pattern in `crates/libre-cr-review/tests/spawned_daemon.rs`.
pub fn code_bin() -> Option<PathBuf> {
    BUILD.call_once(|| {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let output = std::process::Command::new(&cargo)
            .args(["build", "-p", "libre-cr-code", "--bin", "libre-cr-code"])
            .output();
        match output {
            Ok(o) if o.status.success() => BUILT_OK.store(true, Ordering::SeqCst),
            Ok(o) => {
                eprintln!(
                    "[mcp_client] cargo build failed:\n--- stdout ---\n{}\n--- stderr ---\n{}",
                    String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr),
                );
            }
            Err(e) => eprintln!("[mcp_client] could not spawn cargo: {e}"),
        }
    });

    if !BUILT_OK.load(Ordering::SeqCst) {
        return None;
    }

    // current_exe() is .../target/debug/deps/<test>. Two parents up is the
    // profile directory (debug or release).
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    p.pop();
    p.push(if cfg!(windows) {
        "libre-cr-code.exe"
    } else {
        "libre-cr-code"
    });
    if !p.exists() {
        eprintln!("[mcp_client] binary not found at {p:?}");
        return None;
    }
    Some(p)
}

/// `None` here means "skip" — the test prints a diagnostic and `return`s.
/// With `LIBRE_CR_E2E_REQUIRED=1` (set in CI) the skip becomes a panic.
pub fn code_bin_or_skip(test: &str) -> Option<PathBuf> {
    let b = code_bin();
    if b.is_none() {
        if e2e_required() {
            panic!(
                "[mcp_client::{test}] LIBRE_CR_E2E_REQUIRED=1 but the \
                 libre-cr-code binary could not be built/located — failing \
                 instead of skipping (see build diagnostics above)"
            );
        }
        eprintln!("[mcp_client::{test}] libre-cr-code binary unavailable, skipping");
    }
    b
}

/// Default per-call read timeout. Each tool call should be well under this.
const READ_TIMEOUT: Duration = Duration::from_secs(8);

/// Minimal MCP client. One per spawned daemon. The reader is a `Mutex` so
/// concurrent callers don't interleave bytes on stdout — calls are still
/// fully concurrent at the protocol level (each carries a unique `id`).
pub struct McpClient {
    next_id: AtomicU64,
    inner: Mutex<Inner>,
    _child: Child,
}

struct Inner {
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
}

impl McpClient {
    /// Spawn `<bin> mcp-stdio` under a sandboxed `$HOME` and return a client.
    pub async fn spawn_stdio(bin: PathBuf, home: &std::path::Path) -> Result<Self> {
        let mut child = Command::new(&bin)
            .arg("mcp-stdio")
            .env("HOME", home)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .with_context(|| format!("spawn {bin:?}"))?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        Ok(Self {
            next_id: AtomicU64::new(1),
            inner: Mutex::new(Inner {
                stdin,
                reader: BufReader::new(stdout),
            }),
            _child: child,
        })
    }

    /// Send a request and read the next response line. The protocol is
    /// stateful enough that we serialize per-call under a mutex.
    pub async fn rpc(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let mut guard = self.inner.lock().await;
        let mut line = serde_json::to_string(&req)?;
        line.push('\n');
        guard.stdin.write_all(line.as_bytes()).await?;
        guard.stdin.flush().await?;
        let mut buf = String::new();
        loop {
            buf.clear();
            let n = timeout(READ_TIMEOUT, guard.reader.read_line(&mut buf))
                .await
                .with_context(|| format!("timeout reading response for {method}"))??;
            if n == 0 {
                return Err(anyhow!("daemon stdout closed"));
            }
            if buf.trim().is_empty() {
                continue;
            }
            let v: Value =
                serde_json::from_str(buf.trim()).with_context(|| format!("parse: {buf:?}"))?;
            // Sanity check the id; the server is request/response only on
            // this transport.
            if v.get("id").and_then(|x| x.as_u64()) != Some(id) {
                continue;
            }
            return Ok(v);
        }
    }

    pub async fn initialize(&self) -> Result<Value> {
        let resp = self.rpc("initialize", json!({})).await?;
        if let Some(err) = resp.get("error") {
            return Err(anyhow!("initialize error: {err}"));
        }
        resp.get("result")
            .cloned()
            .ok_or_else(|| anyhow!("no result"))
    }

    pub async fn list_tools(&self) -> Result<Vec<Value>> {
        let resp = self.rpc("tools/list", json!({})).await?;
        let result = resp
            .get("result")
            .ok_or_else(|| anyhow!("tools/list: no result: {resp}"))?;
        let arr = result
            .get("tools")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow!("tools/list: missing tools[]"))?;
        Ok(arr.clone())
    }

    /// Call a tool. Returns the parsed envelope (the inner `text` content,
    /// JSON-decoded). `tools/call` always returns 200 — semantic failure
    /// shows up as `ok: false` inside the envelope.
    pub async fn call(&self, name: &str, args: Value) -> Result<Value> {
        let resp = self
            .rpc(
                "tools/call",
                json!({
                    "name": name,
                    "arguments": args,
                }),
            )
            .await?;
        if let Some(err) = resp.get("error") {
            // JSON-RPC level error — not the same as `ok: false`. Bubble up.
            return Err(anyhow!("tools/call error: {err}"));
        }
        let result = resp
            .get("result")
            .ok_or_else(|| anyhow!("tools/call: no result"))?;
        let text = result
            .pointer("/content/0/text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("tools/call: missing content[0].text"))?;
        serde_json::from_str(text).with_context(|| format!("decode envelope: {text:?}"))
    }

    /// Issue a JSON-RPC method directly. Returns the *full* response
    /// envelope so the caller can read `error` for protocol-level failures.
    pub async fn rpc_raw(&self, method: &str, params: Value) -> Result<Value> {
        self.rpc(method, params).await
    }
}

/// MCP client speaking over a Unix socket. Used by the
/// `unix_socket_transport` test. The daemon owns the socket file lifecycle.
pub struct McpSocketClient {
    next_id: AtomicU64,
    inner: Mutex<SocketInner>,
}

struct SocketInner {
    writer: tokio::net::unix::OwnedWriteHalf,
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
}

impl McpSocketClient {
    pub async fn connect(path: &std::path::Path) -> Result<Self> {
        let stream = UnixStream::connect(path)
            .await
            .with_context(|| format!("connect {path:?}"))?;
        let (r, w) = stream.into_split();
        Ok(Self {
            next_id: AtomicU64::new(1),
            inner: Mutex::new(SocketInner {
                writer: w,
                reader: BufReader::new(r),
            }),
        })
    }

    pub async fn rpc(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let mut guard = self.inner.lock().await;
        let mut line = serde_json::to_string(&req)?;
        line.push('\n');
        guard.writer.write_all(line.as_bytes()).await?;
        guard.writer.flush().await?;
        let mut buf = String::new();
        loop {
            buf.clear();
            let n = timeout(READ_TIMEOUT, guard.reader.read_line(&mut buf))
                .await
                .with_context(|| format!("timeout reading socket response for {method}"))??;
            if n == 0 {
                return Err(anyhow!("socket closed"));
            }
            if buf.trim().is_empty() {
                continue;
            }
            let v: Value = serde_json::from_str(buf.trim())?;
            if v.get("id").and_then(|x| x.as_u64()) != Some(id) {
                continue;
            }
            return Ok(v);
        }
    }
}
