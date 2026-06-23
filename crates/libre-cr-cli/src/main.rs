//! `libre-cr` — wrapper CLI that supervises both daemons.
//!
//! See `specs/08-distribution.md` and `specs/plan.md` Phase 7.

fn main() -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(libre_cr_cli::run())
}
