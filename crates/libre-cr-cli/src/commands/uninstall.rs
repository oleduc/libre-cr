//! `libre-cr uninstall` — stop daemons, optionally remove data + logs.
//!
//! Destructive actions are gated behind both `--force` and the
//! `LIBRE_CR_UNINSTALL_DRY_RUN=1` env var (set by tests) so the integration
//! suite never wipes a real `$HOME`.

use std::io::{self, Write};

use anyhow::Result;

use crate::{commands::stop, paths};

pub async fn run(force: bool) -> Result<()> {
    if !force {
        if !confirm("This will stop libre-cr and remove its config/data/logs. Continue? [y/N] ")? {
            println!("Aborted.");
            return Ok(());
        }
        if !confirm("Are you sure? This cannot be undone. [y/N] ")? {
            println!("Aborted.");
            return Ok(());
        }
    }

    // Always try to stop first.
    stop::run().await.ok();

    let dry_run = std::env::var("LIBRE_CR_UNINSTALL_DRY_RUN").is_ok();

    let targets = [
        ("config", paths::config_dir()),
        ("data", paths::data_dir()),
        ("state", paths::state_dir()),
    ];

    for (label, dir) in targets.iter() {
        if !dir.exists() {
            continue;
        }
        if dry_run {
            println!("(dry-run) would remove {label}: {}", dir.display());
        } else {
            match std::fs::remove_dir_all(dir) {
                Ok(()) => println!("removed {label}: {}", dir.display()),
                Err(e) => eprintln!("failed to remove {}: {e}", dir.display()),
            }
        }
    }

    println!("libre-cr: uninstall complete.");
    Ok(())
}

fn confirm(prompt: &str) -> Result<bool> {
    print!("{prompt}");
    io::stdout().flush().ok();
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim().to_lowercase().as_str(), "y" | "yes"))
}
