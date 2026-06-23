//! `libre-cr doctor` — print a diagnostic checklist.

use anyhow::Result;

use crate::doctor;

pub async fn run() -> Result<()> {
    let results = doctor::run_checks();
    print!("{}", doctor::format_report(&results));
    let failed = results
        .iter()
        .any(|r| matches!(r.status, doctor::CheckStatus::Fail));
    if failed {
        anyhow::bail!("one or more doctor checks failed; see output above");
    }
    Ok(())
}
