//! `libre-cr status` — health summary.

use std::time::Duration;

use anyhow::Result;
use nu_ansi_term::Color;

use crate::{config, paths, proc};

pub async fn run() -> Result<()> {
    println!("libre-cr {}", env!("CARGO_PKG_VERSION"));

    // Supervisor and daemon are reported separately: a daemon with no live
    // supervisor is an orphan holding the port, which used to read as a
    // perfectly healthy "running".
    let supervisor_pid_file = paths::supervisor_pid_file();
    let review_pid_file = paths::pid_file();
    let supervisor = proc::read_pid_file(&supervisor_pid_file)?;
    let supervised = supervisor.map(proc::is_alive).unwrap_or(false);
    match supervisor {
        Some(pid) if proc::is_alive(pid) => {
            println!(
                "  {} supervisor: running (PID {pid})",
                Color::Green.paint("✓")
            );
        }
        Some(pid) => {
            println!(
                "  {} supervisor: stale PID {pid} in {}",
                Color::Yellow.paint("!"),
                supervisor_pid_file.display()
            );
        }
        None => {
            println!("  {} supervisor: not running", Color::DarkGray.paint("·"));
        }
    }
    match proc::read_pid_file(&review_pid_file)? {
        Some(pid) if proc::is_alive(pid) && supervised => {
            println!(
                "  {} review daemon: running (PID {pid})",
                Color::Green.paint("✓")
            );
        }
        Some(pid) if proc::is_alive(pid) => {
            println!(
                "  {} review daemon: running unsupervised (PID {pid}) — `libre-cr stop` clears it",
                Color::Yellow.paint("!")
            );
        }
        Some(pid) => {
            println!(
                "  {} review daemon: stale PID {pid} in {}",
                Color::Yellow.paint("!"),
                review_pid_file.display()
            );
        }
        None => {
            println!(
                "  {} review daemon: not running",
                Color::DarkGray.paint("·")
            );
        }
    }

    // Endpoint
    match config::read_endpoint()? {
        Some(endpoint) => {
            println!("  endpoint: {endpoint}");
            // Ping /v1/health
            match ping_health(&endpoint).await {
                Ok(()) => {
                    println!("  {} /v1/health reachable", Color::Green.paint("✓"));
                }
                Err(e) => {
                    println!("  {} /v1/health unreachable: {e}", Color::Yellow.paint("!"));
                }
            }
        }
        None => {
            println!("  endpoint: (none recorded; run `libre-cr start`)");
        }
    }

    Ok(())
}

async fn ping_health(endpoint: &str) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()?;
    let url = format!("{}/v1/health", endpoint.trim_end_matches('/'));
    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {}", resp.status());
    }
    Ok(())
}
