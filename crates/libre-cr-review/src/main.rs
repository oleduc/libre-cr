//! `libre-cr-review` — review daemon hosting the agent loop and per-PR state.
//!
//! See `specs/04-review-daemon.md`.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    libre_cr_review::cli::run().await
}
