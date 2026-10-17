//! Token-bucket speed limiter. One global bucket plus one per download.
//! A rate of 0 means unlimited. Tokens may go into debt so that chunks larger
//! than one second of budget still pass, while the average rate holds.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Burst allowance as a fraction of one second of rate (1/4 s).
const BURST_DIVISOR: i128 = 4;
/// Longest single sleep, so rate changes take effect quickly.
const MAX_SLEEP_MS: u64 = 250;

/// A token bucket.
#[derive(Debug)]
pub struct Limiter {
    rate: AtomicU64,
    bucket: Mutex<Bucket>,
}

#[derive(Debug)]
struct Bucket {
    /// Available bytes; negative = debt.
    tokens: i128,
    last: Instant,
}

impl Limiter {
    /// Creates a limiter with `bytes_per_sec` (0 = unlimited).
    pub fn new(bytes_per_sec: u64) -> Self {
        Self {
            rate: AtomicU64::new(bytes_per_sec),
            bucket: Mutex::new(Bucket {
                tokens: 0,
                last: Instant::now(),
            }),
        }
    }

    /// Current rate (0 = unlimited).
    pub fn rate(&self) -> u64 {
        self.rate.load(Ordering::Relaxed)
    }

    /// Changes the rate.
    pub fn set_rate(&self, bytes_per_sec: u64) {
        self.rate.store(bytes_per_sec, Ordering::Relaxed);
    }

    /// Takes `n` bytes from the bucket and returns how long the caller must
    /// wait before the bytes may be considered sent (zero when within budget).
    pub fn take(&self, n: u64, now: Instant) -> Duration {
        let rate = self.rate();
        if rate == 0 {
            return Duration::ZERO;
        }
        let Ok(mut b) = self.bucket.lock() else {
            return Duration::ZERO;
        };
        let rate_i = i128::from(rate);
        let elapsed_ns =
            i128::try_from(now.saturating_duration_since(b.last).as_nanos()).unwrap_or(i128::MAX);
        let refill = elapsed_ns.saturating_mul(rate_i) / 1_000_000_000;
        let cap = (rate_i / BURST_DIVISOR).max(1);
        b.tokens = b.tokens.saturating_add(refill).min(cap);
        b.last = now;
        b.tokens = b.tokens.saturating_sub(i128::from(n));
        if b.tokens >= 0 {
            return Duration::ZERO;
        }
        let debt = b.tokens.saturating_neg();
        let wait_ns = debt
            .saturating_mul(1_000_000_000)
            .checked_div(rate_i)
            .unwrap_or(0);
        Duration::from_nanos(u64::try_from(wait_ns).unwrap_or(u64::MAX))
    }

    /// Async form: waits until `n` bytes fit the budget.
    pub async fn acquire(&self, n: u64) {
        let mut wait = self.take(n, Instant::now());
        while !wait.is_zero() {
            let step = wait.min(Duration::from_millis(MAX_SLEEP_MS));
            tokio::time::sleep(step).await;
            wait = wait.saturating_sub(step);
            if self.rate() == 0 {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlimited_never_waits() {
        let l = Limiter::new(0);
        assert_eq!(l.take(u64::MAX, Instant::now()), Duration::ZERO);
    }

    #[test]
    fn limiter_bounds_rate() {
        // 1000 B/s: sending 10 000 B in 1 KB chunks at one instant needs ~10 s of waiting.
        let l = Limiter::new(1_000);
        let t0 = Instant::now();
        let mut total = Duration::ZERO;
        for _ in 0..10 {
            total = l.take(1_000, t0);
        }
        assert!(total >= Duration::from_millis(9_500), "{total:?}");
        // After the debt has elapsed the bucket is usable again.
        let later = t0 + Duration::from_secs(11);
        assert_eq!(l.take(100, later), Duration::ZERO);
    }

    #[test]
    fn hostile_limiter_huge_take_saturates() {
        let l = Limiter::new(1);
        let w = l.take(u64::MAX, Instant::now());
        assert!(w > Duration::from_secs(1_000_000));
    }
}
