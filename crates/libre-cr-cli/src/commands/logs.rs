//! `libre-cr logs [-f]` — tail the daemon + supervisor logs.

use anyhow::Result;

use crate::{logs, paths};

pub async fn run(follow: bool, lines: usize) -> Result<()> {
    let files = vec![
        paths::review_log_file(),
        paths::supervisor_log_file(),
        paths::code_log_file(),
    ];
    logs::tail(&files, lines, follow).await
}
