//! Transport abstraction for the MCP client.
//!
//! Both `spawn` and `external` modes ultimately give us a writer-half and a
//! reader-half speaking line-delimited JSON-RPC 2.0. We model that uniformly.

use std::path::PathBuf;
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

use crate::error::{Error, Result};

/// How to start (or attach to) a code daemon.
#[derive(Debug, Clone)]
pub enum TransportSpec {
    /// Spawn `binary mcp-stdio` as a child process.
    Spawn { binary: PathBuf },
    /// Connect to a Unix socket served by `binary mcp-socket`.
    ExternalSocket { path: PathBuf },
}

/// A live connection to the code daemon. Owns the I/O halves and (in
/// spawn mode) the child handle so killing it tears the whole thing down.
pub struct Connection {
    pub reader: BufReader<Box<dyn AsyncRead + Send + Unpin>>,
    pub writer: Box<dyn AsyncWrite + Send + Unpin>,
    /// Held so we can kill the child on shutdown. None for external mode.
    pub child: Option<Child>,
}

impl Connection {
    pub async fn open(spec: &TransportSpec) -> Result<Self> {
        match spec {
            TransportSpec::Spawn { binary } => spawn_child(binary).await,
            TransportSpec::ExternalSocket { path } => connect_socket(path).await,
        }
    }

    pub async fn write_line(&mut self, line: &str) -> Result<()> {
        self.writer
            .write_all(line.as_bytes())
            .await
            .map_err(|e| Error::Internal(format!("mcp write: {e}")))?;
        self.writer
            .write_all(b"\n")
            .await
            .map_err(|e| Error::Internal(format!("mcp write: {e}")))?;
        self.writer
            .flush()
            .await
            .map_err(|e| Error::Internal(format!("mcp flush: {e}")))?;
        Ok(())
    }

    pub async fn read_line(&mut self) -> Result<Option<String>> {
        let mut buf = String::new();
        let n = self
            .reader
            .read_line(&mut buf)
            .await
            .map_err(|e| Error::Internal(format!("mcp read: {e}")))?;
        if n == 0 {
            return Ok(None);
        }
        Ok(Some(buf))
    }

    pub async fn shutdown(mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
            // Best-effort wait so we don't leak zombies in tests.
            let _ = tokio::time::timeout(std::time::Duration::from_millis(500), child.wait()).await;
        }
    }
}

async fn spawn_child(binary: &PathBuf) -> Result<Connection> {
    let mut cmd = Command::new(binary);
    cmd.arg("mcp-stdio")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(stderr_log_stdio())
        .kill_on_drop(true);

    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Internal(format!("spawn {}: {e}", binary.display())))?;

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| Error::Internal("code daemon stdin missing".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Internal("code daemon stdout missing".into()))?;

    Ok(Connection {
        reader: BufReader::new(Box::new(stdout)),
        writer: Box::new(stdin),
        child: Some(child),
    })
}

/// Open (or create) the code-daemon log file under the wrapper's state dir
/// and return a stdio that forwards the child's stderr into it. Falls back
/// to `null` if the file can't be opened — we never want to fail the spawn
/// on logging issues.
fn stderr_log_stdio() -> Stdio {
    match open_code_daemon_log() {
        Some(file) => Stdio::from(file),
        None => Stdio::null(),
    }
}

fn open_code_daemon_log() -> Option<std::fs::File> {
    let path = code_daemon_log_path()?;
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!(path = %parent.display(), error = %e, "create log dir");
            return None;
        }
    }
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(f) => Some(f),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "open code daemon log");
            None
        }
    }
}

/// `~/.local/state/libre-cr/log/libre-cr-code.log`, honoring `$XDG_STATE_HOME`
/// and `$HOME` so tests can sandbox the tree.
fn code_daemon_log_path() -> Option<PathBuf> {
    let base = if let Some(x) = std::env::var_os("XDG_STATE_HOME") {
        PathBuf::from(x)
    } else if let Some(h) = std::env::var_os("HOME") {
        PathBuf::from(h).join(".local").join("state")
    } else {
        dirs::state_dir().or_else(|| dirs::home_dir().map(|h| h.join(".local").join("state")))?
    };
    Some(base.join("libre-cr").join("log").join("libre-cr-code.log"))
}

async fn connect_socket(path: &PathBuf) -> Result<Connection> {
    let stream = tokio::net::UnixStream::connect(path)
        .await
        .map_err(|e| Error::Internal(format!("connect {}: {e}", path.display())))?;
    let (r, w) = stream.into_split();
    Ok(Connection {
        reader: BufReader::new(Box::new(r)),
        writer: Box::new(w),
        child: None,
    })
}
