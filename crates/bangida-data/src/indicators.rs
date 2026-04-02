use crate::ringbuffer::RingBuffer;

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Common interface for incremental technical indicators.
pub trait Indicator {
    /// Feed a new value into the indicator.
    fn update(&mut self, value: f64);
    /// Current indicator value.
    fn value(&self) -> f64;
    /// Whether enough data points have been received for a valid reading.
    fn is_ready(&self) -> bool;
}

// ---------------------------------------------------------------------------
// EMA - Exponential Moving Average
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct EMA {
    period: usize,
    multiplier: f64,
    current: f64,
    count: usize,
    /// Accumulator used for the initial SMA seed.
    sum: f64,
}

impl EMA {
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "EMA period must be > 0");
        Self {
            period,
            multiplier: 2.0 / (period as f64 + 1.0),
            current: 0.0,
            count: 0,
            sum: 0.0,
        }
    }

    /// Expose the period for external consumers.
    pub fn period(&self) -> usize {
        self.period
    }
}

impl Indicator for EMA {
    fn update(&mut self, value: f64) {
        self.count += 1;
        if self.count <= self.period {
            self.sum += value;
            if self.count == self.period {
                // Seed with SMA.
                self.current = self.sum / self.period as f64;
            }
        } else {
            self.current = (value - self.current) * self.multiplier + self.current;
        }
    }

    #[inline]
    fn value(&self) -> f64 {
        self.current
    }

    #[inline]
    fn is_ready(&self) -> bool {
        self.count >= self.period
    }
}

// ---------------------------------------------------------------------------
// RSI - Relative Strength Index (Wilder's smoothing)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct RSI {
    period: usize,
    avg_gain: f64,
    avg_loss: f64,
    prev_value: f64,
    count: usize,
    /// Accumulators for the initial seed period.
    init_gains: f64,
    init_losses: f64,
}

impl RSI {
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "RSI period must be > 0");
        Self {
            period,
            avg_gain: 0.0,
            avg_loss: 0.0,
            prev_value: 0.0,
            count: 0,
            init_gains: 0.0,
            init_losses: 0.0,
        }
    }

    /// Convenience constructor for the standard 14-period RSI.
    pub fn default_period() -> Self {
        Self::new(14)
    }
}

impl Indicator for RSI {
    fn update(&mut self, value: f64) {
        self.count += 1;

        if self.count == 1 {
            // First value - no delta yet.
            self.prev_value = value;
            return;
        }

        let change = value - self.prev_value;
        self.prev_value = value;

        let gain = if change > 0.0 { change } else { 0.0 };
        let loss = if change < 0.0 { -change } else { 0.0 };

        let n = self.period;

        if self.count <= n + 1 {
            // Accumulating the initial seed window (we need `period` deltas,
            // which means `period + 1` data points).
            self.init_gains += gain;
            self.init_losses += loss;

            if self.count == n + 1 {
                self.avg_gain = self.init_gains / n as f64;
                self.avg_loss = self.init_losses / n as f64;
            }
        } else {
            // Wilder's smoothing.
            self.avg_gain = (self.avg_gain * (n as f64 - 1.0) + gain) / n as f64;
            self.avg_loss = (self.avg_loss * (n as f64 - 1.0) + loss) / n as f64;
        }
    }

    fn value(&self) -> f64 {
        if !self.is_ready() {
            return 50.0; // neutral default
        }
        if self.avg_loss == 0.0 {
            return 100.0;
        }
        let rs = self.avg_gain / self.avg_loss;
        100.0 - (100.0 / (1.0 + rs))
    }

    #[inline]
    fn is_ready(&self) -> bool {
        self.count > self.period
    }
}

// ---------------------------------------------------------------------------
// Bollinger Bands
// ---------------------------------------------------------------------------

/// Bollinger Bands using a ring buffer for incremental standard-deviation
/// computation over a rolling window.
#[derive(Debug, Clone)]
pub struct BollingerBands {
    period: usize,
    num_std: f64,
    buffer: RingBuffer<f64>,
    sum: f64,
    sum_sq: f64,
}

impl BollingerBands {
    /// Create Bollinger Bands with the given period and number of standard
    /// deviations (commonly 2.0).
    pub fn new(period: usize, num_std: f64) -> Self {
        assert!(period > 0, "BollingerBands period must be > 0");
        Self {
            period,
            num_std,
            buffer: RingBuffer::new(period),
            sum: 0.0,
            sum_sq: 0.0,
        }
    }

    /// Returns (upper, middle, lower).
    pub fn bands(&self) -> (f64, f64, f64) {
        if !self.is_ready() {
            let mid = if self.buffer.len() > 0 {
                self.sum / self.buffer.len() as f64
            } else {
                0.0
            };
            return (mid, mid, mid);
        }
        let n = self.period as f64;
        let mean = self.sum / n;
        let variance = (self.sum_sq / n) - (mean * mean);
        let std = if variance > 0.0 { variance.sqrt() } else { 0.0 };
        (
            mean + self.num_std * std,
            mean,
            mean - self.num_std * std,
        )
    }
}

impl Indicator for BollingerBands {
    fn update(&mut self, value: f64) {
        // If the buffer is full, subtract the oldest value that will be evicted.
        if self.buffer.is_full() {
            if let Some(&oldest) = self.buffer.get(0) {
                self.sum -= oldest;
                self.sum_sq -= oldest * oldest;
            }
        }
        self.buffer.push(value);
        self.sum += value;
        self.sum_sq += value * value;
    }

    /// Returns the middle band (SMA).
    fn value(&self) -> f64 {
        self.bands().1
    }

    fn is_ready(&self) -> bool {
        self.buffer.is_full()
    }
}

// ---------------------------------------------------------------------------
// VWAP - Volume Weighted Average Price
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct VWAP {
    cumulative_pv: f64,
    cumulative_vol: f64,
    count: usize,
}

impl VWAP {
    pub fn new() -> Self {
        Self {
            cumulative_pv: 0.0,
            cumulative_vol: 0.0,
            count: 0,
        }
    }

    /// Feed a (price, volume) pair.
    pub fn update_with_volume(&mut self, price: f64, volume: f64) {
        self.cumulative_pv += price * volume;
        self.cumulative_vol += volume;
        self.count += 1;
    }

    /// Reset for a new session / day.
    pub fn reset(&mut self) {
        self.cumulative_pv = 0.0;
        self.cumulative_vol = 0.0;
        self.count = 0;
    }
}

impl Default for VWAP {
    fn default() -> Self {
        Self::new()
    }
}

impl Indicator for VWAP {
    /// Convenience: treats `value` as price with volume = 1.
    fn update(&mut self, value: f64) {
        self.update_with_volume(value, 1.0);
    }

    fn value(&self) -> f64 {
        if self.cumulative_vol == 0.0 {
            return 0.0;
        }
        self.cumulative_pv / self.cumulative_vol
    }

    fn is_ready(&self) -> bool {
        self.count > 0
    }
}

// ---------------------------------------------------------------------------
// MACD - Moving Average Convergence Divergence
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct MACD {
    fast_ema: EMA,
    slow_ema: EMA,
    signal_ema: EMA,
    count: usize,
}

impl MACD {
    /// Create MACD with custom periods.
    pub fn new(fast: usize, slow: usize, signal: usize) -> Self {
        assert!(slow > fast, "MACD slow period must be > fast period");
        Self {
            fast_ema: EMA::new(fast),
            slow_ema: EMA::new(slow),
            signal_ema: EMA::new(signal),
            count: 0,
        }
    }

    /// Convenience constructor for the standard 12/26/9 configuration.
    pub fn default_periods() -> Self {
        Self::new(12, 26, 9)
    }

    /// Returns (macd_line, signal_line, histogram).
    pub fn lines(&self) -> (f64, f64, f64) {
        let macd_line = self.fast_ema.value() - self.slow_ema.value();
        let signal_line = self.signal_ema.value();
        (macd_line, signal_line, macd_line - signal_line)
    }
}

impl Indicator for MACD {
    fn update(&mut self, value: f64) {
        self.fast_ema.update(value);
        self.slow_ema.update(value);
        self.count += 1;

        // Only start feeding the signal EMA once both component EMAs are ready.
        if self.slow_ema.is_ready() {
            let macd_line = self.fast_ema.value() - self.slow_ema.value();
            self.signal_ema.update(macd_line);
        }
    }

    /// Returns the MACD line (fast EMA - slow EMA).
    fn value(&self) -> f64 {
        self.fast_ema.value() - self.slow_ema.value()
    }

    fn is_ready(&self) -> bool {
        self.slow_ema.is_ready() && self.signal_ema.is_ready()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ema_basic() {
        let mut ema = EMA::new(3);
        ema.update(2.0);
        ema.update(4.0);
        ema.update(6.0);
        assert!(ema.is_ready());
        // SMA seed = (2+4+6)/3 = 4.0
        assert!((ema.value() - 4.0).abs() < 1e-10);
        ema.update(8.0);
        // EMA = (8 - 4) * 0.5 + 4 = 6.0
        assert!((ema.value() - 6.0).abs() < 1e-10);
    }

    #[test]
    fn rsi_extreme_gains() {
        let mut rsi = RSI::new(5);
        // Feed 6 monotonically increasing values (5 deltas, all gains).
        for i in 0..6 {
            rsi.update(i as f64 * 10.0);
        }
        assert!(rsi.is_ready());
        assert!((rsi.value() - 100.0).abs() < 1e-10);
    }

    #[test]
    fn rsi_all_losses() {
        let mut rsi = RSI::new(5);
        // Feed 6 monotonically decreasing values (5 deltas, all losses).
        for i in (0..6).rev() {
            rsi.update(i as f64 * 10.0);
        }
        assert!(rsi.is_ready());
        assert!(rsi.value() < 1.0);
    }

    #[test]
    fn bollinger_bands_stable() {
        let mut bb = BollingerBands::new(3, 2.0);
        bb.update(10.0);
        bb.update(10.0);
        bb.update(10.0);
        assert!(bb.is_ready());
        let (upper, mid, lower) = bb.bands();
        assert!((mid - 10.0).abs() < 1e-10);
        assert!((upper - 10.0).abs() < 1e-10);
        assert!((lower - 10.0).abs() < 1e-10);
    }

    #[test]
    fn bollinger_bands_spread() {
        let mut bb = BollingerBands::new(3, 1.0);
        bb.update(10.0);
        bb.update(20.0);
        bb.update(30.0);
        let (upper, mid, lower) = bb.bands();
        // mean = 20, std = sqrt((100+400+900)/3 - 400) = sqrt(66.67) ~ 8.165
        assert!((mid - 20.0).abs() < 1e-10);
        assert!(upper > mid);
        assert!(lower < mid);
    }

    #[test]
    fn vwap_basic() {
        let mut vwap = VWAP::new();
        vwap.update_with_volume(100.0, 10.0);
        vwap.update_with_volume(110.0, 20.0);
        // (100*10 + 110*20) / (10+20) = 3200/30 = 106.667
        assert!((vwap.value() - 106.666_666_666_666_66).abs() < 1e-6);
    }

    #[test]
    fn macd_readiness() {
        let mut macd = MACD::new(3, 5, 3);
        for i in 0..5 {
            macd.update(i as f64);
            assert!(!macd.is_ready());
        }
        // After 5 values the slow EMA is ready, but we still need 3 signal
        // values (including the one just fed).
        macd.update(5.0);
        macd.update(6.0);
        assert!(macd.is_ready());
    }
}
