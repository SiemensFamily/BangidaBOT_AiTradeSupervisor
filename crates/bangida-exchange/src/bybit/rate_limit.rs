use parking_lot::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::time::{Duration, Instant};
use tracing::warn;

/// Token-bucket rate limiter for Bybit V5 API.
///
/// Bybit allows 120 requests per 5 seconds for order endpoints.
/// Different endpoint categories have different limits; this implementation
/// covers the most restrictive (order) bucket.
#[derive(Debug, Clone)]
pub struct BybitRateLimiter {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    /// Requests consumed in the current window.
    used: AtomicU32,
    /// Maximum requests per window.
    max_requests: u32,
    /// Start of the current window.
    window_start: Mutex<Instant>,
    /// Duration of one window.
    window_duration: Duration,
}

impl BybitRateLimiter {
    /// Create a rate limiter with the standard Bybit order limit (120 / 5s).
    pub fn new() -> Self {
        Self::with_limit(120, Duration::from_secs(5))
    }

    /// Create a rate limiter with custom limits.
    pub fn with_limit(max_requests: u32, window: Duration) -> Self {
        Self {
            inner: Arc::new(Inner {
                used: AtomicU32::new(0),
                max_requests,
                window_start: Mutex::new(Instant::now()),
                window_duration: window,
            }),
        }
    }

    /// Try to consume one request slot. Returns `true` on success.
    pub fn check_and_consume(&self, count: u32) -> bool {
        self.maybe_reset_window();
        let prev = self.inner.used.fetch_add(count, Ordering::SeqCst);
        if prev + count <= self.inner.max_requests {
            true
        } else {
            self.inner.used.fetch_sub(count, Ordering::SeqCst);
            false
        }
    }

    /// Wait until capacity is available, then consume `count` request slots.
    pub async fn acquire(&self, count: u32) {
        loop {
            if self.check_and_consume(count) {
                return;
            }
            let sleep_dur = self.time_until_reset();
            warn!(
                count,
                sleep_ms = sleep_dur.as_millis() as u64,
                used = self.inner.used.load(Ordering::SeqCst),
                max = self.inner.max_requests,
                "Bybit rate limit approaching, sleeping"
            );
            tokio::time::sleep(sleep_dur).await;
        }
    }

    /// Return current usage count.
    pub fn used(&self) -> u32 {
        self.inner.used.load(Ordering::SeqCst)
    }

    fn maybe_reset_window(&self) {
        let now = Instant::now();
        let mut start = self.inner.window_start.lock();
        if now.duration_since(*start) >= self.inner.window_duration {
            *start = now;
            self.inner.used.store(0, Ordering::SeqCst);
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

impl Default for BybitRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_consume_within_limit() {
        let rl = BybitRateLimiter::with_limit(5, Duration::from_secs(5));
        assert!(rl.check_and_consume(3));
        assert!(rl.check_and_consume(2));
        assert!(!rl.check_and_consume(1));
    }
}
