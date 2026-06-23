//! `libre-cr-code` — standalone MCP server for repo-aware code intelligence.
//!
//! See `specs/03-code-daemon.md` for the binding spec.

mod cli;
mod config;
mod error;
mod git;
mod languages;
mod mcp;
mod repo;
mod search;
mod tools;
mod treesitter;
mod util;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    cli::run().await
}
