//! Shared helpers for the E2E test crate.
//!
//! Each test file in `tests/*.rs` declares `mod common;` to pull these in.
//! See:
//!   - [`mcp_client`] for spawning + framing JSON-RPC against `libre-cr-code`
//!   - [`spawned_daemon`] for spawning `libre-cr-review` against a temp `HOME`
//!   - [`git_fixture`] for building a small on-disk git repo

#![allow(dead_code)]

pub mod git_fixture;
pub mod mcp_client;
pub mod spawned_daemon;
