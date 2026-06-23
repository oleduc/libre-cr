//! End-to-end test crate.
//!
//! This crate has no production code. Every meaningful artifact lives in the
//! integration test files under `tests/`. Each test binary spawns the real
//! `libre-cr-code` and/or `libre-cr-review` binary and acts as an external
//! consumer of the public wire protocol (MCP stdio / Unix socket; HTTP+WS).
//!
//! - `tests/mcp_consumer.rs` — an MCP client driving the code daemon
//! - `tests/http_consumer.rs` — an HTTP/WS client driving the review daemon
//! - `tests/spawned_smoke.rs` — minimal real-daemon smoke kept around as a
//!   fast bisection point when the bigger suites surface a regression
//!
//! Helpers in `tests/common/`:
//!   - `mcp_client` — line-delimited JSON-RPC 2.0 over child stdio + Unix sockets
//!   - `spawned_daemon` — bootstraps the review daemon against a temp `HOME`
//!   - `git_fixture` — builds a small on-disk git repo for the daemon to read
