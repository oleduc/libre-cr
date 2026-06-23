//! Minimal MCP server over stdio / Unix socket. JSON-RPC 2.0, line-delimited.

pub mod server;
pub mod types;

pub use server::{run_socket, run_stdio};
