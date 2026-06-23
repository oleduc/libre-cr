//! `libre-cr version` — print the wrapper version.

use anyhow::Result;

pub async fn run() -> Result<()> {
    println!("libre-cr {}", env!("CARGO_PKG_VERSION"));
    Ok(())
}
