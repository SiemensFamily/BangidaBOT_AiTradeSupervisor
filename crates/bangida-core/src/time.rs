use chrono::{DateTime, Utc};

/// Returns the current UTC timestamp in milliseconds.
pub fn now_ms() -> u64 {
    Utc::now().timestamp_millis() as u64
}

/// Returns the current UTC timestamp in nanoseconds.
pub fn now_ns() -> u64 {
    let now = Utc::now();
    (now.timestamp() as u64) * 1_000_000_000 + (now.timestamp_subsec_nanos() as u64)
}

/// Converts milliseconds to a `DateTime<Utc>`.
pub fn ms_to_datetime(ms: u64) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp_millis(ms as i64)
}

/// Formats a millisecond timestamp as a human-readable string.
pub fn format_ms(ms: u64) -> String {
    ms_to_datetime(ms)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S%.3f UTC").to_string())
        .unwrap_or_else(|| format!("{}ms", ms))
}

/// A monotonic clock for measuring elapsed time with nanosecond precision.
/// Uses `std::time::Instant` which is not affected by system clock changes.
#[derive(Debug, Clone)]
pub struct MonoClock {
    start: std::time::Instant,
}

impl MonoClock {
    pub fn new() -> Self {
        Self {
            start: std::time::Instant::now(),
        }
    }

    /// Returns elapsed nanoseconds since this clock was created.
    pub fn elapsed_ns(&self) -> u64 {
        self.start.elapsed().as_nanos() as u64
    }

    /// Returns elapsed microseconds since this clock was created.
    pub fn elapsed_us(&self) -> u64 {
        self.start.elapsed().as_micros() as u64
    }

    /// Resets the clock and returns elapsed nanoseconds since last reset.
    pub fn lap_ns(&mut self) -> u64 {
        let elapsed = self.elapsed_ns();
        self.start = std::time::Instant::now();
        elapsed
    }
}

impl Default for MonoClock {
    fn default() -> Self {
        Self::new()
    }
}
