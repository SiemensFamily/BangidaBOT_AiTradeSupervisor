use crate::ringbuffer::RingBuffer;

/// Common trait for all technical indicators.
pub trait Indicator {
    fn update(&mut self, value: f64);
    fn value(&self) -> f64;
    fn is_ready(&self) -> bool;
}

// ---------------------------------------------------------------------------
// EMA — Exponential Moving Average
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct EMA {
    period: usize,
    k: f64,
    current: f64,
    count: usize,
    sum: f64, // used for SMA seed
}

impl EMA {
    pub fn new(period: usize) -> Self {
        assert!(period > 0);
        Self {
            period,
            k: 2.0 / (period as f64 + 1.0),
            current: 0.0,
            count: 0,
            sum: 0.0,
        }
    }
}

impl Indicator for EMA {
    fn update(&mut self, value: f64) {
        self.count += 1;
        if self.count <= self.period {
            self.sum += value;
            if self.count == self.period {
                self.current = self.sum / self.period as f64;
            }
        } else {
            self.current = value * self.k + self.current * (1.0 - self.k);
        }
    }

    fn value(&self) -> f64 {
        self.current
    }

    fn is_ready(&self) -> bool {
        self.count >= self.period
    }
}

// ---------------------------------------------------------------------------
// RSI — Relative Strength Index (Wilder's smoothing)
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct RSI {
    period: usize,
    avg_gain: f64,
    avg_loss: f64,
    prev_value: f64,
    count: usize,
    gains: Vec<f64>,
    losses: Vec<f64>,
}

impl RSI {
    pub fn new(period: usize) -> Self {
        assert!(period > 0);
        Self {
            period,
            avg_gain: 0.0,
            avg_loss: 0.0,
            prev_value: 0.0,
            count: 0,
            gains: Vec::with_capacity(period),
            losses: Vec::with_capacity(period),
        }
    }
}

impl Indicator for RSI {
    fn update(&mut self, value: f64) {
        self.count += 1;

        if self.count == 1 {
            self.prev_value = value;
            return;
        }

        let change = value - self.prev_value;
        self.prev_value = value;
        let gain = if change > 0.0 { change } else { 0.0 };
        let loss = if change < 0.0 { -change } else { 0.0 };

        if self.count <= self.period + 1 {
            // Collecting initial period
            self.gains.push(gain);
            self.losses.push(loss);

            if self.count == self.period + 1 {
                self.avg_gain = self.gains.iter().sum::<f64>() / self.period as f64;
                self.avg_loss = self.losses.iter().sum::<f64>() / self.period as f64;
            }
        } else {
            // Wilder's smoothing
            self.avg_gain = (self.avg_gain * (self.period as f64 - 1.0) + gain) / self.period as f64;
            self.avg_loss = (self.avg_loss * (self.period as f64 - 1.0) + loss) / self.period as f64;
        }
    }

    fn value(&self) -> f64 {
        if !self.is_ready() {
            return 50.0;
        }
        if self.avg_loss == 0.0 {
            return 100.0;
        }
        let rs = self.avg_gain / self.avg_loss;
        100.0 - (100.0 / (1.0 + rs))
    }

    fn is_ready(&self) -> bool {
        self.count > self.period
    }
}

// ---------------------------------------------------------------------------
// BollingerBands
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct BollingerBands {
    period: usize,
    num_std: f64,
    buffer: RingBuffer<f64>,
}

impl BollingerBands {
    pub fn new(period: usize, num_std: f64) -> Self {
        Self {
            period,
            num_std,
            buffer: RingBuffer::new(period),
        }
    }

    /// Returns (upper, middle, lower).
    pub fn bands(&self) -> (f64, f64, f64) {
        if !self.is_ready() {
            return (0.0, 0.0, 0.0);
        }
        let mean = self.buffer.iter().sum::<f64>() / self.period as f64;
        let variance = self.buffer.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / self.period as f64;
        let std = variance.sqrt();
        (mean + self.num_std * std, mean, mean - self.num_std * std)
    }
}

impl Indicator for BollingerBands {
    fn update(&mut self, value: f64) {
        self.buffer.push(value);
    }

    fn value(&self) -> f64 {
        self.bands().1
    }

    fn is_ready(&self) -> bool {
        self.buffer.is_full()
    }
}

// ---------------------------------------------------------------------------
// MACD
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct MACD {
    fast_ema: EMA,
    slow_ema: EMA,
    signal_ema: EMA,
    macd_line: f64,
    signal_line: f64,
    histogram: f64,
    count: usize,
    #[allow(dead_code)]
    slow_period: usize,
    #[allow(dead_code)]
    signal_period: usize,
}

impl MACD {
    pub fn new(fast: usize, slow: usize, signal: usize) -> Self {
        Self {
            fast_ema: EMA::new(fast),
            slow_ema: EMA::new(slow),
            signal_ema: EMA::new(signal),
            macd_line: 0.0,
            signal_line: 0.0,
            histogram: 0.0,
            count: 0,
            slow_period: slow,
            signal_period: signal,
        }
    }

    /// Returns (macd_line, signal_line, histogram).
    pub fn lines(&self) -> (f64, f64, f64) {
        (self.macd_line, self.signal_line, self.histogram)
    }
}

impl Indicator for MACD {
    fn update(&mut self, value: f64) {
        self.fast_ema.update(value);
        self.slow_ema.update(value);
        self.count += 1;

        if self.slow_ema.is_ready() {
            self.macd_line = self.fast_ema.value() - self.slow_ema.value();
            self.signal_ema.update(self.macd_line);
            self.signal_line = self.signal_ema.value();
            self.histogram = self.macd_line - self.signal_line;
        }
    }

    fn value(&self) -> f64 {
        self.macd_line
    }

    fn is_ready(&self) -> bool {
        self.slow_ema.is_ready() && self.signal_ema.is_ready()
    }
}

// ---------------------------------------------------------------------------
// VWAP — Volume Weighted Average Price
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct VWAP {
    cum_pv: f64,
    cum_vol: f64,
}

impl VWAP {
    pub fn new() -> Self {
        Self {
            cum_pv: 0.0,
            cum_vol: 0.0,
        }
    }

    pub fn update_with_volume(&mut self, price: f64, volume: f64) {
        self.cum_pv += price * volume;
        self.cum_vol += volume;
    }

    pub fn reset(&mut self) {
        self.cum_pv = 0.0;
        self.cum_vol = 0.0;
    }
}

impl Default for VWAP {
    fn default() -> Self {
        Self::new()
    }
}

impl Indicator for VWAP {
    fn update(&mut self, value: f64) {
        // Without volume info, treat each tick as volume=1
        self.update_with_volume(value, 1.0);
    }

    fn value(&self) -> f64 {
        if self.cum_vol == 0.0 {
            0.0
        } else {
            self.cum_pv / self.cum_vol
        }
    }

    fn is_ready(&self) -> bool {
        self.cum_vol > 0.0
    }
}

// ---------------------------------------------------------------------------
// ATR — Average True Range
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct ATR {
    ema: EMA,
    ready: bool,
}

impl ATR {
    pub fn new(period: usize) -> Self {
        Self {
            ema: EMA::new(period),
            ready: false,
        }
    }

    /// Main update method: feeds true range into an EMA.
    pub fn update_ohlc(&mut self, high: f64, low: f64, prev_close: f64) {
        let tr = (high - low)
            .max((high - prev_close).abs())
            .max((low - prev_close).abs());
        self.ema.update(tr);
        if self.ema.is_ready() {
            self.ready = true;
        }
    }
}

impl Indicator for ATR {
    fn update(&mut self, value: f64) {
        // Treat raw value as a true-range input directly
        self.ema.update(value);
        if self.ema.is_ready() {
            self.ready = true;
        }
    }

    fn value(&self) -> f64 {
        self.ema.value()
    }

    fn is_ready(&self) -> bool {
        self.ready
    }
}

// ---------------------------------------------------------------------------
// OBV — On Balance Volume
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct OBV {
    obv: f64,
    prev_price: Option<f64>,
    count: usize,
}

impl OBV {
    pub fn new() -> Self {
        Self {
            obv: 0.0,
            prev_price: None,
            count: 0,
        }
    }

    pub fn update_with_price(&mut self, price: f64, volume: f64) {
        if let Some(prev) = self.prev_price {
            if price > prev {
                self.obv += volume;
            } else if price < prev {
                self.obv -= volume;
            }
            // equal => no change
        }
        self.prev_price = Some(price);
        self.count += 1;
    }
}

impl Default for OBV {
    fn default() -> Self {
        Self::new()
    }
}

impl Indicator for OBV {
    fn update(&mut self, value: f64) {
        // Without explicit volume, treat as volume=1
        self.update_with_price(value, 1.0);
    }

    fn value(&self) -> f64 {
        self.obv
    }

    fn is_ready(&self) -> bool {
        self.count >= 2
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    // -- EMA --
    #[test]
    fn test_ema_sma_seed() {
        let mut ema = EMA::new(3);
        ema.update(2.0);
        ema.update(4.0);
        ema.update(6.0);
        assert!(ema.is_ready());
        // SMA of first 3 values = 4.0
        assert!((ema.value() - 4.0).abs() < 1e-10);
    }

    #[test]
    fn test_ema_after_seed() {
        let mut ema = EMA::new(3);
        ema.update(2.0);
        ema.update(4.0);
        ema.update(6.0); // seed = 4.0
        ema.update(8.0); // k=0.5: 8*0.5 + 4*0.5 = 6.0
        assert!((ema.value() - 6.0).abs() < 1e-10);
    }

    // -- RSI --
    #[test]
    fn test_rsi_all_gains() {
        let mut rsi = RSI::new(5);
        for i in 0..10 {
            rsi.update(i as f64);
        }
        assert!(rsi.is_ready());
        // All positive changes => RSI near 100
        assert!(rsi.value() > 95.0);
    }

    #[test]
    fn test_rsi_all_losses() {
        let mut rsi = RSI::new(5);
        for i in (0..10).rev() {
            rsi.update(i as f64);
        }
        assert!(rsi.is_ready());
        assert!(rsi.value() < 5.0);
    }

    // -- BollingerBands --
    #[test]
    fn test_bb_constant_values() {
        let mut bb = BollingerBands::new(5, 2.0);
        for _ in 0..5 {
            bb.update(100.0);
        }
        let (upper, mid, lower) = bb.bands();
        assert!((mid - 100.0).abs() < 1e-10);
        // stddev = 0, so bands collapse
        assert!((upper - 100.0).abs() < 1e-10);
        assert!((lower - 100.0).abs() < 1e-10);
    }

    #[test]
    fn test_bb_spread() {
        let mut bb = BollingerBands::new(3, 1.0);
        bb.update(10.0);
        bb.update(20.0);
        bb.update(30.0);
        let (upper, mid, lower) = bb.bands();
        assert!((mid - 20.0).abs() < 1e-10);
        assert!(upper > mid);
        assert!(lower < mid);
    }

    // -- MACD --
    #[test]
    fn test_macd_converges() {
        let mut macd = MACD::new(3, 5, 3);
        for i in 0..20 {
            macd.update(100.0 + i as f64);
        }
        assert!(macd.is_ready());
        let (line, signal, hist) = macd.lines();
        // Fast EMA > Slow EMA in uptrend => positive MACD
        assert!(line > 0.0);
        // Histogram = line - signal
        assert!((hist - (line - signal)).abs() < 1e-10);
    }

    #[test]
    fn test_macd_flat() {
        let mut macd = MACD::new(3, 5, 3);
        for _ in 0..20 {
            macd.update(50.0);
        }
        assert!(macd.is_ready());
        let (line, _signal, _hist) = macd.lines();
        assert!(line.abs() < 1e-10);
    }

    // -- VWAP --
    #[test]
    fn test_vwap_simple() {
        let mut vwap = VWAP::new();
        vwap.update_with_volume(100.0, 10.0);
        vwap.update_with_volume(110.0, 20.0);
        // (100*10 + 110*20) / (10+20) = 3200/30 = 106.666...
        assert!((vwap.value() - 106.666666666).abs() < 0.001);
    }

    #[test]
    fn test_vwap_reset() {
        let mut vwap = VWAP::new();
        vwap.update_with_volume(100.0, 10.0);
        vwap.reset();
        assert!(!vwap.is_ready());
        assert_eq!(vwap.value(), 0.0);
    }

    // -- ATR --
    #[test]
    fn test_atr_simple() {
        let mut atr = ATR::new(3);
        atr.update_ohlc(12.0, 10.0, 11.0); // TR = max(2, 1, 1) = 2
        atr.update_ohlc(14.0, 11.0, 12.0); // TR = max(3, 2, 1) = 3
        atr.update_ohlc(13.0, 10.0, 14.0); // TR = max(3, 1, 4) = 4
        assert!(atr.is_ready());
        // SMA seed = (2+3+4)/3 = 3.0
        assert!((atr.value() - 3.0).abs() < 1e-10);
    }

    #[test]
    fn test_atr_after_seed() {
        let mut atr = ATR::new(3);
        atr.update_ohlc(12.0, 10.0, 11.0); // TR=2
        atr.update_ohlc(14.0, 11.0, 12.0); // TR=3
        atr.update_ohlc(13.0, 10.0, 14.0); // TR=4, seed=3.0
        atr.update_ohlc(15.0, 12.0, 13.0); // TR=3, k=0.5: 3*0.5+3*0.5=3.0
        assert!((atr.value() - 3.0).abs() < 1e-10);
    }

    // -- OBV --
    #[test]
    fn test_obv_up_down() {
        let mut obv = OBV::new();
        obv.update_with_price(100.0, 1000.0); // first, no prev
        obv.update_with_price(105.0, 2000.0); // up => +2000
        obv.update_with_price(103.0, 500.0);  // down => -500
        assert!((obv.value() - 1500.0).abs() < 1e-10);
    }

    #[test]
    fn test_obv_equal_price() {
        let mut obv = OBV::new();
        obv.update_with_price(100.0, 1000.0);
        obv.update_with_price(100.0, 5000.0); // equal => no change
        assert!((obv.value() - 0.0).abs() < 1e-10);
    }
}
