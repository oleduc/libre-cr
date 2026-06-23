//! `libre-cr update` — placeholder (Phase 7.5).

use anyhow::Result;

use crate::update;

pub async fn run() -> Result<()> {
    println!("{}", update::stub_message());
    Ok(())
}
