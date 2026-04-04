use std::collections::VecDeque;

/// Ring-buffer latency tracker that stores latency samples in microseconds
/// and provides percentile, mean, min, and max statistics.
pub struct LatencyTracker {
    samples: VecDeque<u64>,
    max_samples: usize,
}

impl LatencyTracker {
    pub fn new(max_samples: usize) -> Self {
        Self {
            samples: VecDeque::with_capacity(max_samples),
            max_samples,
        }
    }

    /// Record a latency sample in microseconds. If the buffer is full,
    /// the oldest sample is evicted.
    pub fn record(&mut self, latency_us: u64) {
        if self.samples.len() >= self.max_samples {
            self.samples.pop_front();
        }
        self.samples.push_back(latency_us);
    }

    /// Median latency (50th percentile).
    pub fn p50(&self) -> u64 {
        self.percentile(50)
    }

    /// 99th percentile latency.
    pub fn p99(&self) -> u64 {
        self.percentile(99)
    }

    /// Arithmetic mean of all recorded samples.
    pub fn mean(&self) -> u64 {
        if self.samples.is_empty() {
            return 0;
        }
        let sum: u64 = self.samples.iter().sum();
        sum / self.samples.len() as u64
    }

    /// Maximum recorded latency.
    pub fn max(&self) -> u64 {
        self.samples.iter().copied().max().unwrap_or(0)
    }

    /// Minimum recorded latency.
    pub fn min(&self) -> u64 {
        self.samples.iter().copied().min().unwrap_or(0)
    }

    /// Number of samples currently stored.
    pub fn count(&self) -> usize {
        self.samples.len()
    }

    /// Compute a percentile value from the stored samples.
    /// Sorts a copy of the samples, then picks the index closest to
    /// `round(pct/100 * (len-1))`.
    fn percentile(&self, pct: u32) -> u64 {
        if self.samples.is_empty() {
            return 0;
        }
        let mut sorted: Vec<u64> = self.samples.iter().copied().collect();
        sorted.sort_unstable();
        let idx = ((pct as f64 / 100.0) * (sorted.len() - 1) as f64).round() as usize;
        sorted[idx]
    }
}

impl Default for LatencyTracker {
    fn default() -> Self {
        Self::new(1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_tracker() {
        let tracker = LatencyTracker::new(100);
        assert_eq!(tracker.count(), 0);
        assert_eq!(tracker.mean(), 0);
        assert_eq!(tracker.min(), 0);
        assert_eq!(tracker.max(), 0);
        assert_eq!(tracker.p50(), 0);
        assert_eq!(tracker.p99(), 0);
    }

    #[test]
    fn test_single_sample() {
        let mut tracker = LatencyTracker::new(100);
        tracker.record(500);
        assert_eq!(tracker.count(), 1);
        assert_eq!(tracker.mean(), 500);
        assert_eq!(tracker.min(), 500);
        assert_eq!(tracker.max(), 500);
        assert_eq!(tracker.p50(), 500);
        assert_eq!(tracker.p99(), 500);
    }

    #[test]
    fn test_multiple_samples() {
        let mut tracker = LatencyTracker::new(100);
        for i in 1..=100 {
            tracker.record(i);
        }
        assert_eq!(tracker.count(), 100);
        assert_eq!(tracker.mean(), 50); // (1+100)/2 = 50.5, integer division = 50
        assert_eq!(tracker.min(), 1);
        assert_eq!(tracker.max(), 100);
        // p50: index = round(0.50 * 99) = round(49.5) = 50 -> sorted[50] = 51
        assert_eq!(tracker.p50(), 51);
        // p99: index = round(0.99 * 99) = round(98.01) = 98 -> sorted[98] = 99
        assert_eq!(tracker.p99(), 99);
    }

    #[test]
    fn test_ring_buffer_eviction() {
        let mut tracker = LatencyTracker::new(5);
        for i in 1..=10 {
            tracker.record(i);
        }
        // Only the last 5 values should remain: 6, 7, 8, 9, 10
        assert_eq!(tracker.count(), 5);
        assert_eq!(tracker.min(), 6);
        assert_eq!(tracker.max(), 10);
    }

    #[test]
    fn test_default() {
        let tracker = LatencyTracker::default();
        assert_eq!(tracker.max_samples, 1000);
        assert_eq!(tracker.count(), 0);
    }

    #[test]
    fn test_percentile_ordering() {
        let mut tracker = LatencyTracker::new(1000);
        // Record values in reverse order to ensure sorting works
        for i in (1..=1000).rev() {
            tracker.record(i);
        }
        assert!(tracker.p50() <= tracker.p99());
        assert!(tracker.min() <= tracker.p50());
        assert!(tracker.p99() <= tracker.max());
    }
}
