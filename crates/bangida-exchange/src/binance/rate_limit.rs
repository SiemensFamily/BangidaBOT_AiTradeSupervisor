use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::time::{Duration, Instant};
use parking_lot::Mutex;
use tracing::warn;

/// Token-bucket rate limiter for Binance Futures API.
///
/// Binance allows 2400 request weight per minute. Each request type consumes
/// a certain weight (most endpoints cost 1, some cost 5-20).
/// The bucket refills completely every 60 seconds.
#[derive(Debug, Clone)]
pub struct BinanceRateLimiter {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    /// Accumulated weight used in the current window.
    used_weight: AtomicU32,
    /// Maximum weight per window.
    max_weight: u32,
    /// Start of the current window.
    window_start: Mutex<Instant>,
    /// Duration of one window.
    window_duration: Duration,
}

impl BinanceRateLimiter {
    /// Create a new rate limiter with the standard Binance limit of 2400 weight / 60s.
    pub fn new() -> Self {
        Self::with_limit(2400, Duration::from_secs(60))
    }

    /// Create a rate limiter with custom limits.
    pub fn with_limit(max_weight: u32, window: Duration) -> Self {
        Self {
            inner: Arc::new(Inner {
                used_weight: AtomicU32::new(0),
                max_weight,
                window_start: Mutex::new(Instant::now()),
                window_duration: window,
            }),
        }
    }

    /// Check whether we can spend `weight` request weight.
    /// If the window has expired, reset the counter.
    /// Returns `true` if the request can proceed immediately.
    pub fn check_and_consume(&self, weight: u32) -> bool {
        self.maybe_reset_window();
        let prev = self.inner.used_weight.fetch_add(weight, Ordering::SeqCst);
        if prev + weight <= self.inner.max_weight {
            true
        } else {
            // roll back
            self.inner.used_weight.fetch_sub(weight, Ordering::SeqCst);
            false
        }
    }

    /// Wait until enough capacity is available, then consume `weight`.
    pub async fn acquire(&self, weight: u32) {
        loop {
            if self.check_and_consume(weight) {
                return;
            }
            let sleep_dur = self.time_until_reset();
            warn!(
                weight,
                sleep_ms = sleep_dur.as_millis() as u64,
                used = self.inner.used_weight.load(Ordering::SeqCst),
                max = self.inner.max_weight,
                "Rate limit approaching, sleeping"
            );
            tokio::time::sleep(sleep_dur).await;
        }
    }

    /// Return current used weight.
    pub fn used_weight(&self) -> u32 {
        self.inner.used_weight.load(Ordering::SeqCst)
    }

    fn maybe_reset_window(&self) {
        let now = Instant::now();
        let mut start = self.inner.window_start.lock();
        if now.duration_since(*start) >= self.inner.window_duration {
            *start = now;
            self.inner.used_weight.store(0, Ordering::SeqCst);
        }
    }

    fn time_until_reset(&self) -> Duration {
        let start = *self.inner.window_start.lock();
        let elapsed = Instant::now().duration_since(start);
        if elapsed >= self.inner.window_duration {
            Duration::ZERO
        } else {
            self.inner.window_duration - elapsed
        }
    }
}

impl Default for BinanceRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_consume_within_limit() {
        let rl = BinanceRateLimiter::with_limit(10, Duration::from_secs(60));
        assert!(rl.check_and_consume(5));
        assert!(rl.check_and_consume(5));
        assert!(!rl.check_and_consume(1));
    }
}
