//! Restart-budget tracker: at most N restarts per hour, per spec.
//!
//! Pure synchronous logic — easy to test without spawning subprocesses.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct RestartBudget {
    max_per_window: u32,
    window: Duration,
    events: VecDeque<Instant>,
}

impl RestartBudget {
    pub fn new(max_per_window: u32) -> Self {
        Self::with_window(max_per_window, Duration::from_secs(3600))
    }

    pub fn with_window(max_per_window: u32, window: Duration) -> Self {
        Self {
            max_per_window,
            window,
            events: VecDeque::new(),
        }
    }

    fn prune(&mut self, now: Instant) {
        while let Some(&front) = self.events.front() {
            if now.duration_since(front) > self.window {
                self.events.pop_front();
            } else {
                break;
            }
        }
    }

    /// Record a restart attempt at `now`. Returns `false` if the budget is
    /// exceeded (the attempt is still recorded so subsequent calls keep
    /// failing until the window slides).
    pub fn record(&mut self, now: Instant) -> bool {
        self.prune(now);
        self.events.push_back(now);
        (self.events.len() as u32) <= self.max_per_window
    }

    /// Snapshot — number of restart events still inside the window.
    pub fn count(&mut self, now: Instant) -> u32 {
        self.prune(now);
        self.events.len() as u32
    }

    /// Whether any further restart would exceed the budget.
    pub fn would_exceed(&mut self, now: Instant) -> bool {
        self.count(now) >= self.max_per_window
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_budget_is_accepted() {
        let mut b = RestartBudget::with_window(3, Duration::from_secs(60));
        let t0 = Instant::now();
        assert!(b.record(t0));
        assert!(b.record(t0));
        assert!(b.record(t0));
    }

    #[test]
    fn over_budget_is_rejected() {
        let mut b = RestartBudget::with_window(3, Duration::from_secs(60));
        let t0 = Instant::now();
        b.record(t0);
        b.record(t0);
        b.record(t0);
        assert!(!b.record(t0));
    }

    #[test]
    fn window_slides() {
        let mut b = RestartBudget::with_window(2, Duration::from_millis(50));
        let t0 = Instant::now();
        b.record(t0);
        b.record(t0);
        assert!(b.would_exceed(t0));
        // After the window passes, prior events fall off.
        let later = t0 + Duration::from_millis(200);
        assert!(!b.would_exceed(later));
        assert!(b.record(later));
    }

    #[test]
    fn count_prunes_events() {
        let mut b = RestartBudget::with_window(5, Duration::from_millis(10));
        let t0 = Instant::now();
        b.record(t0);
        b.record(t0);
        let later = t0 + Duration::from_millis(50);
        assert_eq!(b.count(later), 0);
    }
}
