//! `libre-cr-review` library surface — exposes the building blocks (config,
//! storage, agent loop, provider, tools, server) so integration tests can
//! reach in.
//!
//! See `specs/04-review-daemon.md`.

pub mod agent;
pub mod cli;
pub mod code_daemon;
pub mod config;
pub mod error;
pub mod pairing;
pub mod provider;
pub mod server;
pub mod storage;
pub mod tools;
pub mod verbs;
pub mod worktree;
