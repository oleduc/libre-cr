//! `libre-cr status` — health summary.

use std::time::Duration;

use anyhow::Result;
use nu_ansi_term::Color;

use crate::{config, paths, proc};

pub async fn run() -> Result<()> {
    println!("libre-cr {}", env!("CARGO_PKG_VERSION"));

    // PID file
    let pid_file = paths::pid_file();
    match proc::read_pid_file(&pid_file)? {
        Some(pid) if proc::is_alive(pid) => {
            println!(
                "  {} review daemon: running (PID {pid})",
                Color::Green.paint("✓")
            );
        }
        Some(pid) => {
            println!(
                "  {} review daemon: stale PID {pid} in {}",
                Color::Yellow.paint("!"),
                pid_file.display()
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
