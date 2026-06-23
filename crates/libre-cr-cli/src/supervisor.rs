//! Process supervisor for `libre-cr-review`.
//!
//! Behavior (spec § Supervision Model):
//!
//! - Spawn the daemon as a child process; redirect stdout/stderr to a log
//!   file.
//! - On unexpected exit, restart up to 5 times within a 60-second sliding
//!   window. Exceeding the budget surfaces a hard failure to the caller.
//! - Write `~/.local/state/libre-cr/run/review.pid` while running; remove
//!   it on clean shutdown.
//! - Honor `SIGTERM` / `SIGINT` for graceful shutdown: signal the daemon,
//!   wait up to 5 s, then force-kill if it's still alive.
//!
//! The supervisor itself is a `tokio` task. Tests inject a custom command
//! (e.g. `sleep`, `false`) via [`SpawnSpec`] to exercise the restart loop.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::process::Command;

use crate::{logs, paths, proc};

/// How the supervisor finished. Useful for tests; production callers only
/// care about the `Err` case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupervisorOutcome {
    /// The child exited cleanly and we did not restart.
    Clean,
    /// We exhausted the restart budget.
    BudgetExhausted { restarts: usize },
    /// A graceful shutdown was requested via [`Supervisor::shutdown`].
    ShutdownRequested,
}

/// What to launch. Production passes `program = "libre-cr-review"`,
/// `args = []`. Tests pass `program = "sleep"`, etc.
#[derive(Debug, Clone)]
pub struct SpawnSpec {
    pub program: String,
    pub args: Vec<String>,
    /// Where stdout + stderr are appended. Created if missing.
    pub log_file: PathBuf,
    /// Where the live PID is recorded. Removed on clean stop.
    pub pid_file: PathBuf,
}

impl SpawnSpec {
    /// Production default: spawn `libre-cr-review` with the wrapper's
    /// canonical paths.
    pub fn review_daemon() -> Self {
        Self {
            program: "libre-cr-review".into(),
            args: vec!["serve".into()],
            log_file: paths::review_log_file(),
            pid_file: paths::pid_file(),
        }
    }
}

/// Tunable restart budget. Defaults match the spec (5 in 60 s).
#[derive(Debug, Clone, Copy)]
pub struct RestartPolicy {
    pub max_restarts: usize,
    pub window: Duration,
    /// How long to wait between restarts. Bounded by spec implicitly via the
    /// 60-second window; we add a small floor to avoid pid-loop hammering.
    pub backoff: Duration,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            max_restarts: 5,
            window: Duration::from_secs(60),
            backoff: Duration::from_millis(250),
        }
    }
}

/// Tracks restart timestamps in a sliding window. Returns `true` if a new
/// restart would exceed the budget.
#[derive(Debug, Default)]
pub struct RestartTracker {
    events: VecDeque<Instant>,
}

impl RestartTracker {
    /// Record a restart. Returns the total count inside the policy window.
    pub fn record(&mut self, now: Instant, policy: &RestartPolicy) -> usize {
        self.events.push_back(now);
        self.prune(now, policy);
        self.events.len()
    }

    /// Would the next restart bust the budget?
    pub fn would_exceed(&mut self, now: Instant, policy: &RestartPolicy) -> bool {
        self.prune(now, policy);
        self.events.len() + 1 > policy.max_restarts
    }

    fn prune(&mut self, now: Instant, policy: &RestartPolicy) {
        while let Some(front) = self.events.front().copied() {
            if now.duration_since(front) > policy.window {
                self.events.pop_front();
            } else {
                break;
            }
        }
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// One supervised run. The caller owns the future; cancelling the future
/// stops supervision but leaves any live child in place — call
/// [`Supervisor::shutdown_blocking`] for cleanup. In practice the wrapper
/// installs a signal handler that drives the shutdown.
pub struct Supervisor {
    spec: SpawnSpec,
    policy: RestartPolicy,
}

impl Supervisor {
    pub fn new(spec: SpawnSpec) -> Self {
        Self {
            spec,
            policy: RestartPolicy::default(),
        }
    }

    pub fn with_policy(mut self, policy: RestartPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Run the supervision loop until the child exits cleanly, the budget
    /// is exhausted, or `cancel` fires.
    pub async fn run(
        self,
        mut cancel: tokio::sync::oneshot::Receiver<()>,
    ) -> Result<SupervisorOutcome> {
        let mut tracker = RestartTracker::default();
        loop {
            // Ensure the log dir exists before redirecting fds into it.
            if let Some(parent) = self.spec.log_file.parent() {
                tokio::fs::create_dir_all(parent).await.ok();
            }
            let log = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.spec.log_file)
                .with_context(|| format!("open log {}", self.spec.log_file.display()))?;
            let log_err = log
                .try_clone()
                .with_context(|| "clone log handle for stderr")?;

            let mut cmd = Command::new(&self.spec.program);
            cmd.args(&self.spec.args)
                .stdin(Stdio::null())
                .stdout(Stdio::from(log))
                .stderr(Stdio::from(log_err))
                .kill_on_drop(true);

            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    logs::supervisor_event(format!(
                        "spawn-failed program={} error={e}",
                        self.spec.program
                    ))
                    .await
                    .ok();
                    return Err(anyhow::anyhow!(
                        "failed to spawn {}: {e}",
                        self.spec.program
                    ));
                }
            };
            let pid = child.id().unwrap_or(0);
            proc::write_pid_file(&self.spec.pid_file, pid).ok();
            logs::supervisor_event(format!("spawned program={} pid={pid}", self.spec.program))
                .await
                .ok();

            // Wait for either the child to exit or a cancel signal.
            let status = tokio::select! {
                s = child.wait() => s,
                _ = &mut cancel => {
                    // Graceful: SIGTERM, wait up to 5s, then SIGKILL.
                    if pid > 0 {
                        let _ = proc::send_term(pid);
                    }
                    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
                    let waited = tokio::time::timeout_at(deadline, child.wait()).await;
                    let _ = match waited {
                        Ok(s) => s,
                        Err(_) => {
                            let _ = child.start_kill();
                            child.wait().await
                        }
                    };
                    logs::supervisor_event(format!("graceful-stop pid={pid}")).await.ok();
                    proc::remove_pid_file(&self.spec.pid_file).ok();
                    return Ok(SupervisorOutcome::ShutdownRequested);
                }
            };

            proc::remove_pid_file(&self.spec.pid_file).ok();

            match status {
                Ok(st) if st.success() => {
                    logs::supervisor_event(format!("clean-exit pid={pid}"))
                        .await
                        .ok();
                    return Ok(SupervisorOutcome::Clean);
                }
                Ok(st) => {
                    logs::supervisor_event(format!("unclean-exit pid={pid} code={:?}", st.code()))
                        .await
                        .ok();
                }
                Err(e) => {
                    logs::supervisor_event(format!("wait-failed pid={pid} error={e}"))
                        .await
                        .ok();
                }
            }

            let now = Instant::now();
            if tracker.would_exceed(now, &self.policy) {
                let total = tracker.len();
                logs::supervisor_event(format!(
                    "budget-exhausted restarts={total} window_s={}",
                    self.policy.window.as_secs()
                ))
                .await
                .ok();
                return Ok(SupervisorOutcome::BudgetExhausted { restarts: total });
            }
            tracker.record(now, &self.policy);
            tokio::time::sleep(self.policy.backoff).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unix_only_skip() -> bool {
        cfg!(not(unix))
    }

    #[test]
    fn tracker_records_within_window() {
        let policy = RestartPolicy {
            max_restarts: 3,
            window: Duration::from_secs(60),
            backoff: Duration::from_millis(0),
        };
        let mut t = RestartTracker::default();
        let now = Instant::now();
        t.record(now, &policy);
        t.record(now, &policy);
        assert_eq!(t.len(), 2);
        assert!(!t.would_exceed(now, &policy)); // 2 + 1 = 3, max is 3
        t.record(now, &policy);
        assert!(t.would_exceed(now, &policy));
    }

    #[test]
    fn tracker_prunes_old_events() {
        let policy = RestartPolicy {
            max_restarts: 3,
            window: Duration::from_millis(50),
            backoff: Duration::from_millis(0),
        };
        let mut t = RestartTracker::default();
        let earlier = Instant::now() - Duration::from_secs(1);
        t.record(earlier, &policy);
        let now = Instant::now();
        t.record(now, &policy);
        // The earlier event should have been pruned at the second record.
        assert_eq!(t.len(), 1);
    }

    #[tokio::test]
    async fn supervises_a_short_lived_clean_child() {
        if unix_only_skip() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let spec = SpawnSpec {
            program: "true".into(),
            args: vec![],
            log_file: dir.path().join("child.log"),
            pid_file: dir.path().join("child.pid"),
        };
        let (_tx, rx) = tokio::sync::oneshot::channel();
        let outcome = Supervisor::new(spec).run(rx).await.unwrap();
        assert_eq!(outcome, SupervisorOutcome::Clean);
    }

    #[tokio::test]
    async fn busts_the_restart_budget_on_a_flapping_child() {
        if unix_only_skip() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let spec = SpawnSpec {
            program: "false".into(),
            args: vec![],
            log_file: dir.path().join("child.log"),
            pid_file: dir.path().join("child.pid"),
        };
        let policy = RestartPolicy {
            max_restarts: 2,
            window: Duration::from_secs(60),
            backoff: Duration::from_millis(10),
        };
        let (_tx, rx) = tokio::sync::oneshot::channel();
        let outcome = Supervisor::new(spec)
            .with_policy(policy)
            .run(rx)
            .await
            .unwrap();
        match outcome {
            SupervisorOutcome::BudgetExhausted { restarts } => {
                assert!(restarts >= 2, "got {restarts}");
            }
            other => panic!("expected BudgetExhausted, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn shutdown_signal_stops_a_long_running_child() {
        if unix_only_skip() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let spec = SpawnSpec {
            program: "sleep".into(),
            args: vec!["30".into()],
            log_file: dir.path().join("child.log"),
            pid_file: dir.path().join("child.pid"),
        };
        let (tx, rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move { Supervisor::new(spec).run(rx).await });
        tokio::time::sleep(Duration::from_millis(100)).await;
        tx.send(()).unwrap();
        let outcome = handle.await.unwrap().unwrap();
        assert_eq!(outcome, SupervisorOutcome::ShutdownRequested);
    }
}
