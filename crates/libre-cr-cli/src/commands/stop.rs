//! `libre-cr stop` — signal the review daemon, wait for graceful exit,
//! force-kill on timeout.

use std::time::Duration;

use anyhow::Result;

use crate::{logs, paths, proc};

pub async fn run() -> Result<()> {
    stop_with_timeout(Duration::from_secs(5)).await
}

/// Exposed for tests so they don't have to wait the full 5 s.
pub async fn stop_with_timeout(timeout: Duration) -> Result<()> {
    let pid_file = paths::pid_file();
    let pid = match proc::read_pid_file(&pid_file)? {
        Some(p) => p,
        None => {
            println!(
                "libre-cr: no PID file at {}; nothing to stop.",
                pid_file.display()
            );
            return Ok(());
        }
    };

    if !proc::is_alive(pid) {
        println!("libre-cr: PID {pid} not alive; clearing stale PID file.");
        proc::remove_pid_file(&pid_file).ok();
        return Ok(());
    }

    proc::send_term(pid).ok();
    logs::supervisor_event(format!("stop-requested pid={pid}"))
        .await
        .ok();

    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if !proc::is_alive(pid) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    if proc::is_alive(pid) {
        println!(
            "libre-cr: PID {pid} did not exit after {}s; sending SIGKILL.",
            timeout.as_secs()
        );
        proc::send_kill(pid).ok();
        // Brief grace for the OS to reap.
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    proc::remove_pid_file(&pid_file).ok();
    println!("libre-cr: stopped (PID {pid}).");
    Ok(())
}
