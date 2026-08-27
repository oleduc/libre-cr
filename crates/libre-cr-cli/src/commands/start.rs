//! `libre-cr start` — bring up the review daemon under supervision.
//!
//! The wrapper supervises `libre-cr-review`; the review daemon supervises
//! `libre-cr-code` itself (Phase 3 work). See `specs/08-distribution.md`
//! § Supervision Model.
//!
//! In the production layout this command spawns the supervisor in the
//! foreground — `brew services` / `systemd --user` / Task Scheduler is what
//! turns it into a background service. Detaching ourselves (double-fork,
//! `setsid`, …) is out of scope for this phase.

use std::time::Duration;

use anyhow::{Context, Result};

use crate::{logs, paths, proc, supervisor};

pub async fn run(autostart: bool) -> Result<()> {
    paths::ensure_dirs().context("ensure libre-cr directories")?;
    let pid_file = paths::pid_file();

    if let Some(pid) = proc::read_pid_file(&pid_file)? {
        if proc::is_alive(pid) {
            println!("libre-cr: already running (PID {pid}).");
            return Ok(());
        } else {
            println!("libre-cr: clearing stale PID file (PID {pid} is dead).");
            proc::remove_pid_file(&pid_file).ok();
        }
    }

    if autostart {
        println!(
            "libre-cr: --autostart noted; OS-level autostart hook lands in Phase 7.5.\n\
             For now use `brew services start libre-cr` (macOS), `systemctl --user enable\n\
             --now libre-cr` (Linux), or a Task Scheduler entry (Windows)."
        );
    }

    logs::supervisor_event("start-requested").await.ok();

    // Remove any endpoint file left by a previous run *before* spawning, so
    // the watcher below announces the fresh daemon's port rather than a stale
    // one (the daemon uses an ephemeral port by default, so the old contents
    // are almost always wrong). Found by manual testing: the banner printed a
    // dead endpoint from the prior session.
    if let Err(e) = std::fs::remove_file(paths::endpoint_file()) {
        if e.kind() != std::io::ErrorKind::NotFound {
            eprintln!(
                "libre-cr: warning: could not remove stale endpoint file {}: {e}",
                paths::endpoint_file().display()
            );
        }
    }

    // First-run summary, printed *before* we hand the foreground to the
    // supervisor. We can't know the daemon's chosen port until it writes
    // the endpoint file, so we poll for that with a 5 s budget on a side
    // task while the supervisor runs.
    print_first_run_banner();

    let spec = supervisor::SpawnSpec::review_daemon();
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();

    // Cancellation: SIGINT/SIGTERM trigger the supervisor's graceful path.
    spawn_signal_handler(cancel_tx);

    // Endpoint-watcher: announce the URL once the daemon writes it.
    let watcher = tokio::spawn(async {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline {
            if let Ok(Some(endpoint)) = crate::config::read_endpoint() {
                println!("  endpoint: {endpoint}");
                println!("  token:    {}", paths::token_file().display());
                println!();
                println!("Run `libre-cr pair` to pair the browser extension.");
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        eprintln!(
            "libre-cr: daemon didn't write an endpoint file within 5s. \
             Check `libre-cr logs`."
        );
    });

    let outcome = supervisor::Supervisor::new(spec).run(cancel_rx).await?;
    watcher.abort();
    match outcome {
        supervisor::SupervisorOutcome::Clean | supervisor::SupervisorOutcome::ShutdownRequested => {
            Ok(())
        }
        supervisor::SupervisorOutcome::BudgetExhausted { restarts } => {
            anyhow::bail!(
                "libre-cr-review crashed {restarts} times within 60s. \
                 See `libre-cr logs` for details."
            )
        }
    }
}

fn print_first_run_banner() {
    println!("libre-cr {}", env!("CARGO_PKG_VERSION"));
    println!("  starting review daemon under supervision…");
    println!("  logs:   {}", paths::log_dir().display());
    println!("  config: {}", paths::config_dir().display());
}

#[cfg(unix)]
fn spawn_signal_handler(cancel: tokio::sync::oneshot::Sender<()>) {
    tokio::spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => return,
        };
        let mut int = match signal(SignalKind::interrupt()) {
            Ok(s) => s,
            Err(_) => return,
        };
        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
        }
        let _ = cancel.send(());
    });
}

#[cfg(windows)]
fn spawn_signal_handler(cancel: tokio::sync::oneshot::Sender<()>) {
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = cancel.send(());
    });
}
