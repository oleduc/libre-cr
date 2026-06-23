//! Real MCP client to `libre-cr-code`.
//!
//! Two transport modes per `specs/04-review-daemon.md` § Configuration
//! `[code_daemon]`:
//!
//! - `mode = "spawn"` — fork `libre-cr-code mcp-stdio` as a tokio child and
//!   speak line-delimited JSON-RPC 2.0 over stdin/stdout.
//! - `mode = "external"` — connect to an existing `libre-cr-code mcp-socket`
//!   instance over a Unix domain socket.
//!
//! Health: on child death / EOF / call timeout the connection is restarted
//! with exponential backoff, bounded by `max_restarts_per_hour`. When the
//! budget is exhausted, dispatches surface as `ErrorCategory::CodeDaemonUnavailable`.

pub mod budget;
pub mod client;
pub mod transport;

pub use budget::RestartBudget;
pub use client::{HealthSnapshot, SpawnedClient};
