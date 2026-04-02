use std::collections::VecDeque;

/// Tracks recent latency measurements (in microseconds) and provides
/// percentile, mean, and max statistics for monitoring execution quality.
#[derive(Debug)]
pub struct LatencyTracker {
    /// Ring buffer of recent latency samples (microseconds).
    samples: VecDeque<u64>,
    /// Maximum number of samples to retain.
    max_samples: usize,
}

impl LatencyTracker {
    /// Create a new latency tracker retaining up to `max_samples` measurements.
    pub fn new(max_samples: usize) -> Self {
        Self {
            samples: VecDeque::with_capacity(max_samples),
            max_samples,
        }
    }

    /// Record a latency measurement in microseconds.
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

    /// Mean latency.
    pub fn mean(&self) -> u64 {
        if self.samples.is_empty() {
            return 0;
        }
        let sum: u64 = self.samples.iter().sum();
        sum / self.samples.len() as u64
    }

    /// Maximum observed latency in the window.
    pub fn max(&self) -> u64 {
        self.samples.iter().copied().max().unwrap_or(0)
    }

    /// Minimum observed latency in the window.
    pub fn min(&self) -> u64 {
        self.samples.iter().copied().min().unwrap_or(0)
    }

    /// Number of samples currently stored.
    pub fn count(&self) -> usize {
        self.samples.len()
    }

    /// Compute an arbitrary percentile (0-100).
    fn percentile(&self, pct: u32) -> u64 {
        if self.samples.is_empty() {
            return 0;
        }
        let mut sorted: Vec<u64> = self.samples.iter().copied().collect();
        sorted.sort_unstable();
        let idx = ((pct as f64 / 100.0) * (sorted.len() - 1) as f64).round() as usize;
        sorted[idx.min(sorted.len() - 1)]
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
    fn test_basic_stats() {
        let mut lt = LatencyTracker::new(100);
        for i in 1..=100 {
            lt.record(i);
        }
        assert_eq!(lt.count(), 100);
        assert_eq!(lt.mean(), 50); // (1+100)/2 = 50.5, integer division = 50
        assert_eq!(lt.min(), 1);
        assert_eq!(lt.max(), 100);
    }

    #[test]
    fn test_percentiles() {
        let mut lt = LatencyTracker::new(100);
        for i in 1..=100 {
            lt.record(i);
        }
        // p50: idx = round(0.50 * 99) = 50 => sorted[50] = 51
        assert_eq!(lt.p50(), 51);
        // p99: idx = round(0.99 * 99) = 98 => sorted[98] = 99
        assert_eq!(lt.p99(), 99);
    }

    #[test]
    fn test_ring_buffer_eviction() {
        let mut lt = LatencyTracker::new(5);
        for i in 1..=10 {
            lt.record(i);
        }
        assert_eq!(lt.count(), 5);
        // Only 6,7,8,9,10 should remain
        assert_eq!(lt.min(), 6);
    }

    #[test]
    fn test_empty() {
        let lt = LatencyTracker::new(100);
        assert_eq!(lt.mean(), 0);
        assert_eq!(lt.p50(), 0);
        assert_eq!(lt.p99(), 0);
    }
}
