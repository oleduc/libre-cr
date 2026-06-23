//! `libre-cr restart` — stop, then start.

use anyhow::Result;

use crate::commands::{start, stop};

pub async fn run() -> Result<()> {
    stop::run().await.ok();
    start::run(false).await
}
