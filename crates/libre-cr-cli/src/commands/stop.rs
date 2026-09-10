//! `libre-cr stop` — stop the supervisor, which stops the review daemon.
//!
//! The review daemon must never be the target: it is a child inside the
//! supervisor's restart loop, so signalling it directly only makes the
//! supervisor spawn a replacement (on a fresh port), after which `start`
//! reports "already running". The supervisor's own SIGTERM handler stops the
//! daemon gracefully and then leaves the loop.

use std::path::Path;
use std::time::Duration;

use anyhow::Result;

use crate::{logs, paths, proc};

pub async fn run() -> Result<()> {
    stop_with_timeout(Duration::from_secs(5)).await
}

/// The PID in `path`, if it names a process that is actually alive.
fn live_pid(path: &Path) -> Result<Option<u32>> {
    Ok(proc::read_pid_file(path)?.filter(|pid| proc::is_alive(*pid)))
}

/// Exposed for tests so they don't have to wait the full 5 s.
pub async fn stop_with_timeout(timeout: Duration) -> Result<()> {
    stop_files(&paths::supervisor_pid_file(), &paths::pid_file(), timeout).await
}

/// Stop whatever the two given pid files name.
///
/// Takes the paths rather than reading `paths::*` so tests act on their own
/// temp files instead of the ambient `$HOME` — the integration suite
/// re-points `$HOME` concurrently, which made assertions on global paths
/// order-dependent.
pub async fn stop_files(
    supervisor_pid_file: &Path,
    review_pid_file: &Path,
    timeout: Duration,
) -> Result<()> {
    let supervisor = live_pid(supervisor_pid_file)?;
    let review = live_pid(review_pid_file)?;

    if supervisor.is_none() && review.is_none() {
        println!("libre-cr: not running.");
        proc::remove_pid_file(supervisor_pid_file).ok();
        proc::remove_pid_file(review_pid_file).ok();
        return Ok(());
    }

    if let Some(pid) = supervisor {
        logs::supervisor_event(format!("stop-requested supervisor_pid={pid}"))
            .await
            .ok();
        if proc::terminate_and_wait(pid, timeout).await {
            println!("libre-cr: stopped (supervisor PID {pid}).");
        } else {
            println!("libre-cr: supervisor PID {pid} did not exit, even after SIGKILL.");
        }
    }

    // A supervisor that had to be force-killed never got to stop its child, so
    // the daemon can outlive it — still holding the port. Re-read rather than
    // reuse the earlier value: a graceful supervisor has cleared this file.
    if let Some(pid) = live_pid(review_pid_file)? {
        if supervisor.is_none() {
            println!(
                "libre-cr: no supervisor running; stopping orphaned review daemon (PID {pid})."
            );
        } else {
            println!(
                "libre-cr: review daemon (PID {pid}) outlived its supervisor; stopping it too."
            );
        }
        logs::supervisor_event(format!(
            "stop-requested review_pid={pid} orphaned={}",
            supervisor.is_none()
        ))
        .await
        .ok();
        if !proc::terminate_and_wait(pid, timeout).await {
            println!("libre-cr: review daemon PID {pid} did not exit, even after SIGKILL.");
        }
    }

    proc::remove_pid_file(supervisor_pid_file).ok();
    proc::remove_pid_file(review_pid_file).ok();
    Ok(())
}
