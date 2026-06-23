//! Stdio + Unix-socket transports. Line-delimited JSON-RPC 2.0.
//!
//! Requests are dispatched concurrently (bounded by a semaphore) so a slow
//! tool call — e.g. a network-bound `prepare_worktree` — does not
//! head-of-line-block fast calls on the same connection. JSON-RPC permits
//! out-of-order responses keyed by `id`; a single writer task serializes the
//! response lines so framing stays atomic. The `initialize` handshake is the
//! exception: it is answered inline, before any later request is spawned, so
//! no tool call can be admitted ahead of it.

use crate::mcp::types::{initialize_result, JsonRpcRequest, JsonRpcResponse};
use crate::tools::{ToolContext, ToolRegistry};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, Semaphore};

/// Maximum number of requests being handled at once per connection.
const MAX_CONCURRENT_REQUESTS: usize = 16;

/// Run an MCP server reading line-delimited JSON-RPC from stdin and writing
/// to stdout. Returns when stdin is closed.
pub async fn run_stdio(ctx: Arc<ToolContext>, registry: Arc<ToolRegistry>) -> anyhow::Result<()> {
    serve_lines(tokio::io::stdin(), tokio::io::stdout(), ctx, registry).await
}

/// Run an MCP server on a Unix socket. Accepts multiple connections.
pub async fn run_socket(
    path: &std::path::Path,
    ctx: Arc<ToolContext>,
    registry: Arc<ToolRegistry>,
) -> anyhow::Result<()> {
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    let listener = tokio::net::UnixListener::bind(path)?;
    // Lock down to 0600.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        let _ = std::fs::set_permissions(path, perms);
    }
    tracing::info!(path = %path.display(), "mcp-socket listening");

    loop {
        let (stream, _addr) = listener.accept().await?;
        let ctx = ctx.clone();
        let registry = registry.clone();
        tokio::spawn(async move {
            let (read_half, write_half) = stream.into_split();
            if let Err(e) = serve_lines(read_half, write_half, ctx, registry).await {
                tracing::warn!(error = %e, "socket connection ended");
            }
        });
    }
}

/// Serve one line-delimited JSON-RPC connection. Generic over the transport
/// so stdio, Unix sockets, and in-memory test duplexes share one code path.
async fn serve_lines<R, W>(
    reader: R,
    writer: W,
    ctx: Arc<ToolContext>,
    registry: Arc<ToolRegistry>,
) -> anyhow::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let mut reader = BufReader::new(reader);

    // Single writer task: response lines flow through an mpsc channel so
    // concurrent handlers never interleave bytes.
    let (tx, mut rx) = mpsc::channel::<String>(64);
    let writer_task = tokio::spawn(async move {
        let mut writer = writer;
        while let Some(mut line) = rx.recv().await {
            line.push('\n');
            if writer.write_all(line.as_bytes()).await.is_err() {
                break;
            }
            if writer.flush().await.is_err() {
                break;
            }
        }
    });

    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS));
    let mut in_flight = tokio::task::JoinSet::new();
    let mut line = String::new();

    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req: JsonRpcRequest = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                let resp = JsonRpcResponse::err(Value::Null, -32700, format!("parse error: {e}"));
                let _ = tx.send(serde_json::to_string(&resp)?).await;
                continue;
            }
        };

        // `initialize` completes inline: nothing read after it can be
        // spawned (let alone answered) before the handshake response is
        // queued, so tool calls are never admitted ahead of it.
        if req.method == "initialize" {
            if let Some(resp) = handle_request(req, &ctx, &registry).await {
                let _ = tx.send(serde_json::to_string(&resp)?).await;
            }
            continue;
        }

        let ctx = ctx.clone();
        let registry = registry.clone();
        let tx = tx.clone();
        let semaphore = semaphore.clone();
        in_flight.spawn(async move {
            let _permit = semaphore.acquire_owned().await;
            if let Some(resp) = handle_request(req, &ctx, &registry).await {
                if let Ok(text) = serde_json::to_string(&resp) {
                    let _ = tx.send(text).await;
                }
            }
        });
    }

    // EOF: let in-flight requests finish, then drain the writer.
    while in_flight.join_next().await.is_some() {}
    drop(tx);
    let _ = writer_task.await;
    Ok(())
}

async fn handle_request(
    req: JsonRpcRequest,
    ctx: &Arc<ToolContext>,
    registry: &Arc<ToolRegistry>,
) -> Option<JsonRpcResponse> {
    let id = req.id.clone().unwrap_or(Value::Null);
    let is_notification = req.id.is_none();

    let result = dispatch(req.method.as_str(), req.params, ctx, registry).await;

    if is_notification {
        return None;
    }

    match result {
        Ok(v) => Some(JsonRpcResponse::ok(id, v)),
        Err((code, msg)) => Some(JsonRpcResponse::err(id, code, msg)),
    }
}

async fn dispatch(
    method: &str,
    params: Value,
    ctx: &Arc<ToolContext>,
    registry: &Arc<ToolRegistry>,
) -> Result<Value, (i64, String)> {
    match method {
        "initialize" => Ok(initialize_result()),
        "notifications/initialized" | "initialized" => Ok(Value::Null),
        "tools/list" => {
            let tools: Vec<Value> = registry
                .all()
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name(),
                        "description": t.description(),
                        "inputSchema": t.input_schema(),
                    })
                })
                .collect();
            Ok(json!({ "tools": tools }))
        }
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or((-32602, "missing 'name'".to_string()))?;
            let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
            let started = std::time::Instant::now();
            let tool_result = registry.call(name, ctx.clone(), arguments).await;
            let latency_ms = started.elapsed().as_millis() as u64;
            let (envelope, ok) = match tool_result {
                Ok(v) => {
                    let v = ensure_ok_envelope(v);
                    let was_ok = v.get("ok").and_then(|x| x.as_bool()).unwrap_or(true);
                    (v, was_ok)
                }
                Err(e) => (e.to_envelope(), false),
            };
            tracing::info!(tool_name = %name, latency_ms, result = if ok { "ok" } else { "err" }, "tool call");
            // MCP content envelope: text part containing our JSON envelope.
            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": serde_json::to_string(&envelope).unwrap_or_default(),
                    }
                ],
                "isError": !ok,
            }))
        }
        // Ignore prompts/resources methods — we don't expose any.
        "prompts/list" => Ok(json!({ "prompts": [] })),
        "resources/list" => Ok(json!({ "resources": [] })),
        "ping" => Ok(Value::Null),
        other => Err((-32601, format!("unknown method: {other}"))),
    }
}

fn ensure_ok_envelope(v: Value) -> Value {
    match v {
        Value::Object(mut m) => {
            m.entry("ok".to_string()).or_insert(Value::Bool(true));
            Value::Object(m)
        }
        other => json!({ "ok": true, "result": other }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::repo::{RepoRegistry, WorktreeManager};
    use crate::tools::registry::{Tool, ToolFuture};
    use tempfile::TempDir;

    struct SlowTool;
    impl Tool for SlowTool {
        fn name(&self) -> &'static str {
            "slow_tool"
        }
        fn description(&self) -> &'static str {
            "sleeps, then answers"
        }
        fn input_schema(&self) -> Value {
            json!({ "type": "object", "properties": {} })
        }
        fn call<'a>(&'a self, _ctx: Arc<ToolContext>, _input: Value) -> ToolFuture<'a> {
            Box::pin(async {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                Ok(json!({ "ok": true, "speed": "slow" }))
            })
        }
    }

    struct FastTool;
    impl Tool for FastTool {
        fn name(&self) -> &'static str {
            "fast_tool"
        }
        fn description(&self) -> &'static str {
            "answers immediately"
        }
        fn input_schema(&self) -> Value {
            json!({ "type": "object", "properties": {} })
        }
        fn call<'a>(&'a self, _ctx: Arc<ToolContext>, _input: Value) -> ToolFuture<'a> {
            Box::pin(async { Ok(json!({ "ok": true, "speed": "fast" })) })
        }
    }

    fn test_ctx(tmp: &TempDir) -> Arc<ToolContext> {
        let registry = Arc::new(RepoRegistry::open_in_memory().unwrap());
        let cfg = Config::default();
        Arc::new(ToolContext {
            data_dir: tmp.path().to_path_buf(),
            registry: registry.clone(),
            worktrees: Arc::new(WorktreeManager::new(
                tmp.path().join("worktrees"),
                cfg.worktrees.clone(),
                registry,
            )),
            config: cfg,
        })
    }

    /// A slow tool call must not head-of-line-block a fast one issued after
    /// it on the same connection (N6): the fast response arrives first.
    #[tokio::test]
    async fn slow_call_does_not_block_fast_call() {
        let tmp = TempDir::new().unwrap();
        let ctx = test_ctx(&tmp);
        let mut tools = ToolRegistry::new();
        tools.register(SlowTool);
        tools.register(FastTool);
        let tools = Arc::new(tools);

        let (mut client_w, server_r) = tokio::io::duplex(64 * 1024);
        let (server_w, client_r) = tokio::io::duplex(64 * 1024);
        let server = tokio::spawn(serve_lines(server_r, server_w, ctx, tools));

        let mut lines = BufReader::new(client_r).lines();

        // Handshake first; it must complete before tool calls are admitted.
        client_w
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n")
            .await
            .unwrap();
        let init = lines.next_line().await.unwrap().unwrap();
        let init: Value = serde_json::from_str(&init).unwrap();
        assert_eq!(init["id"], 1);

        // Slow call (id 2) then fast call (id 3), written back-to-back.
        client_w
            .write_all(
                b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"slow_tool\",\"arguments\":{}}}\n\
                  {\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"fast_tool\",\"arguments\":{}}}\n",
            )
            .await
            .unwrap();

        let first: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        let second: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(
            first["id"], 3,
            "fast call must finish before the slow one: {first}"
        );
        assert_eq!(second["id"], 2, "slow call answers later: {second}");

        drop(client_w);
        server.await.unwrap().unwrap();
    }
}
