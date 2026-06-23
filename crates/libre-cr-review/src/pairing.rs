//! One-time pairing code generation + redemption.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rand::Rng;
use tokio::sync::Mutex;

/// Per-source-IP failure tracking: redeem(...) records a failure on any
/// wrong code; after `MAX_FAILURES` within the failure window the store rate-
/// limits further attempts from that IP. Successful redemption resets the
/// counter for the source.
const MAX_FAILURES: u32 = 5;
const FAILURE_WINDOW: Duration = Duration::from_secs(60);

/// Default per-code lifetime when the issuer doesn't ask for one.
const DEFAULT_CODE_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Copy, Default)]
struct FailureWindow {
    count: u32,
    first_failure: Option<Instant>,
}

impl FailureWindow {
    fn record(&mut self, window: Duration) {
        let now = Instant::now();
        match self.first_failure {
            Some(t) if now.duration_since(t) <= window => {
                self.count += 1;
            }
            _ => {
                self.first_failure = Some(now);
                self.count = 1;
            }
        }
    }

    fn over_limit(&self, window: Duration) -> bool {
        match self.first_failure {
            Some(t) => self.count >= MAX_FAILURES && t.elapsed() <= window,
            None => false,
        }
    }

    /// True once the window since the first recorded failure has fully
    /// lapsed — the entry carries no information any more and can be pruned.
    fn expired(&self, window: Duration) -> bool {
        match self.first_failure {
            Some(t) => t.elapsed() > window,
            None => true,
        }
    }
}

#[derive(Clone)]
pub struct PairingStore {
    /// code → expiry instant (per-code TTL; see [`PairingStore::issue_with_ttl`]).
    inner: Arc<Mutex<HashMap<String, Instant>>>,
    /// Failure ledger keyed by source IP. Reset on first successful redeem
    /// from that IP; entries whose window has fully lapsed are pruned on
    /// every redeem attempt (REVIEW/round2 I11).
    failures: Arc<Mutex<HashMap<IpAddr, FailureWindow>>>,
    default_ttl: Duration,
    failure_window: Duration,
}

/// `Default` delegates to [`PairingStore::new`]. A derived `Default` would
/// ship a zero TTL, making every issued code dead on arrival (round-1 I9).
impl Default for PairingStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Outcome of a redeem attempt. `Allowed` is the only success path; the
/// other two are 401 vs 429 on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedeemOutcome {
    /// Code valid, single-use consumed.
    Ok,
    /// Code unknown / expired / reused — caller should respond 401.
    Invalid,
    /// Source over the per-IP failure threshold — caller should respond 429
    /// with `Retry-After: 60`.
    RateLimited,
}

impl PairingStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            failures: Arc::new(Mutex::new(HashMap::new())),
            default_ttl: DEFAULT_CODE_TTL,
            failure_window: FAILURE_WINDOW,
        }
    }

    /// Test hook: shrink the failure window so pruning is observable
    /// without waiting out the production 60 s.
    #[cfg(test)]
    fn with_failure_window(mut self, window: Duration) -> Self {
        self.failure_window = window;
        self
    }

    /// Generate and store a 6-byte hex pairing code with the default TTL.
    pub async fn issue(&self) -> String {
        self.issue_with_ttl(self.default_ttl).await
    }

    /// Generate and store a code that expires after `ttl`. The HTTP issuer
    /// clamps the caller-supplied TTL; the store applies whatever it's given
    /// so tests can use millisecond lifetimes.
    pub async fn issue_with_ttl(&self, ttl: Duration) -> String {
        let code = gen_code();
        let mut g = self.inner.lock().await;
        prune(&mut g);
        g.insert(code.clone(), Instant::now() + ttl);
        code
    }

    /// Returns true on first successful redeem, false on unknown/expired/reused.
    ///
    /// Use [`Self::redeem_from`] in production paths so per-IP failure
    /// tracking kicks in. This bare variant is kept for tests that don't
    /// model a source address.
    pub async fn redeem(&self, code: &str) -> bool {
        let mut g = self.inner.lock().await;
        prune(&mut g);
        matches!(g.remove(code), Some(expires_at) if Instant::now() <= expires_at)
    }

    /// Source-aware redemption. Rate-limits a single IP that floods us with
    /// wrong codes — see `REVIEW/00-certification.md` I11.
    pub async fn redeem_from(&self, code: &str, source: IpAddr) -> RedeemOutcome {
        {
            let mut failures = self.failures.lock().await;
            // Drop entries whose window has fully lapsed so the map stays
            // bounded under rotating-IP failure sweeps.
            failures.retain(|_, w| !w.expired(self.failure_window));
            if let Some(w) = failures.get(&source) {
                if w.over_limit(self.failure_window) {
                    return RedeemOutcome::RateLimited;
                }
            }
        }
        let ok = self.redeem(code).await;
        let mut failures = self.failures.lock().await;
        if ok {
            // Successful redemption resets the counter for this source.
            failures.remove(&source);
            RedeemOutcome::Ok
        } else {
            let entry = failures.entry(source).or_default();
            entry.record(self.failure_window);
            // If this very failure tipped them over, escalate.
            if entry.over_limit(self.failure_window) {
                RedeemOutcome::RateLimited
            } else {
                RedeemOutcome::Invalid
            }
        }
    }

    pub async fn len(&self) -> usize {
        self.inner.lock().await.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.inner.lock().await.is_empty()
    }
}

fn gen_code() -> String {
    let mut buf = [0u8; 6];
    rand::thread_rng().fill(&mut buf);
    hex::encode(buf)
}

fn prune(map: &mut HashMap<String, Instant>) {
    let now = Instant::now();
    map.retain(|_, expires_at| *expires_at >= now);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn issue_and_redeem_once() {
        let s = PairingStore::new();
        let code = s.issue().await;
        assert!(s.redeem(&code).await);
        assert!(!s.redeem(&code).await);
    }

    #[tokio::test]
    async fn unknown_code_fails() {
        let s = PairingStore::new();
        assert!(!s.redeem("deadbeef").await);
    }

    #[tokio::test]
    async fn default_delegates_to_new() {
        // I9: a derived Default shipped ttl = 0, killing codes at issue time.
        let s = PairingStore::default();
        assert_eq!(s.default_ttl, DEFAULT_CODE_TTL);
        let code = s.issue().await;
        assert!(s.redeem(&code).await);
    }

    #[tokio::test]
    async fn issue_with_ttl_expires_per_code() {
        // N2: the TTL is per-code, not store-global.
        let s = PairingStore::new();
        let short = s.issue_with_ttl(Duration::from_millis(30)).await;
        let long = s.issue_with_ttl(Duration::from_secs(60)).await;
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(!s.redeem(&short).await, "30ms code must be expired");
        assert!(s.redeem(&long).await, "60s code must still redeem");
    }

    #[tokio::test]
    async fn redeem_from_rate_limits_after_threshold() {
        let s = PairingStore::new();
        let ip: std::net::IpAddr = "127.0.0.1".parse().unwrap();
        // First five wrong codes are 401 (Invalid).
        for _ in 0..(MAX_FAILURES - 1) {
            assert_eq!(s.redeem_from("nope", ip).await, RedeemOutcome::Invalid);
        }
        // The MAX_FAILURES-th wrong attempt trips the limit.
        assert_eq!(s.redeem_from("nope", ip).await, RedeemOutcome::RateLimited);
        // Subsequent attempts are also 429.
        assert_eq!(s.redeem_from("nope", ip).await, RedeemOutcome::RateLimited);
    }

    #[tokio::test]
    async fn successful_redeem_resets_failure_counter() {
        let s = PairingStore::new();
        let ip: std::net::IpAddr = "10.0.0.5".parse().unwrap();
        // A few wrong attempts, then a successful one.
        for _ in 0..3 {
            assert_eq!(s.redeem_from("bad", ip).await, RedeemOutcome::Invalid);
        }
        let code = s.issue().await;
        assert_eq!(s.redeem_from(&code, ip).await, RedeemOutcome::Ok);
        // After the reset, the counter starts fresh.
        for _ in 0..(MAX_FAILURES - 1) {
            assert_eq!(s.redeem_from("bad", ip).await, RedeemOutcome::Invalid);
        }
    }

    #[tokio::test]
    async fn rate_limit_is_per_source_ip() {
        let s = PairingStore::new();
        let ip1: std::net::IpAddr = "127.0.0.1".parse().unwrap();
        let ip2: std::net::IpAddr = "127.0.0.2".parse().unwrap();
        for _ in 0..MAX_FAILURES {
            let _ = s.redeem_from("nope", ip1).await;
        }
        // ip1 is now rate-limited; ip2 still gets Invalid.
        assert_eq!(s.redeem_from("nope", ip1).await, RedeemOutcome::RateLimited);
        assert_eq!(s.redeem_from("nope", ip2).await, RedeemOutcome::Invalid);
    }

    #[tokio::test]
    async fn failures_map_pruned_once_windows_lapse() {
        // I11: rotating-IP failures must not grow the map without bound.
        let s = PairingStore::new().with_failure_window(Duration::from_millis(20));
        for i in 0..50u32 {
            let ip: std::net::IpAddr = format!("10.1.{}.{}", i / 256, i % 256).parse().unwrap();
            let _ = s.redeem_from("wrong", ip).await;
        }
        assert!(s.failures.lock().await.len() <= 50);
        tokio::time::sleep(Duration::from_millis(40)).await;
        // Any redeem attempt prunes the lapsed windows.
        let probe: std::net::IpAddr = "10.9.9.9".parse().unwrap();
        let _ = s.redeem_from("wrong", probe).await;
        let len = s.failures.lock().await.len();
        assert!(len <= 1, "expected only the probe entry, got {len}");
    }
}
