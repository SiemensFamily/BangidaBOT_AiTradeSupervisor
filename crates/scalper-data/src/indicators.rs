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
// Stochastic Oscillator (%K, %D)
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct Stochastic {
    period: usize,
    smooth_k: usize,
    smooth_d: usize,
    highs: RingBuffer<f64>,
    lows: RingBuffer<f64>,
    closes: RingBuffer<f64>,
    k_history: RingBuffer<f64>,
    d_history: RingBuffer<f64>,
    raw_k: f64,
    k: f64,
    d: f64,
}

impl Stochastic {
    pub fn new(period: usize, smooth_k: usize, smooth_d: usize) -> Self {
        Self {
            period,
            smooth_k,
            smooth_d,
            highs: RingBuffer::new(period),
            lows: RingBuffer::new(period),
            closes: RingBuffer::new(period),
            k_history: RingBuffer::new(smooth_k),
            d_history: RingBuffer::new(smooth_d),
            raw_k: 50.0,
            k: 50.0,
            d: 50.0,
        }
    }

    pub fn update_ohlc(&mut self, high: f64, low: f64, close: f64) {
        self.highs.push(high);
        self.lows.push(low);
        self.closes.push(close);
        if self.highs.len() < self.period { return; }
        let hh = self.highs.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));
        let ll = self.lows.iter().fold(f64::INFINITY, |a, &b| a.min(b));
        self.raw_k = if hh > ll { 100.0 * (close - ll) / (hh - ll) } else { 50.0 };
        self.k_history.push(self.raw_k);
        if self.k_history.len() >= self.smooth_k {
            let sum: f64 = self.k_history.iter().sum();
            self.k = sum / self.k_history.len() as f64;
            self.d_history.push(self.k);
            if self.d_history.len() >= self.smooth_d {
                let dsum: f64 = self.d_history.iter().sum();
                self.d = dsum / self.d_history.len() as f64;
            }
        }
    }

    pub fn k(&self) -> f64 { self.k }
    pub fn d(&self) -> f64 { self.d }
    pub fn is_ready(&self) -> bool {
        self.k_history.len() >= self.smooth_k && self.d_history.len() >= self.smooth_d
    }
}

// ---------------------------------------------------------------------------
// Stochastic RSI — Stochastic applied to RSI values
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct StochRSI {
    rsi: RSI,
    rsi_history: RingBuffer<f64>,
    period: usize,
    value: f64,
}

impl StochRSI {
    pub fn new(rsi_period: usize, stoch_period: usize) -> Self {
        Self {
            rsi: RSI::new(rsi_period),
            rsi_history: RingBuffer::new(stoch_period),
            period: stoch_period,
            value: 50.0,
        }
    }

    pub fn update(&mut self, price: f64) {
        self.rsi.update(price);
        if !self.rsi.is_ready() { return; }
        let r = self.rsi.value();
        self.rsi_history.push(r);
        if self.rsi_history.len() >= self.period {
            let hh = self.rsi_history.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));
            let ll = self.rsi_history.iter().fold(f64::INFINITY, |a, &b| a.min(b));
            self.value = if hh > ll { 100.0 * (r - ll) / (hh - ll) } else { 50.0 };
        }
    }

    pub fn value(&self) -> f64 { self.value }
    pub fn is_ready(&self) -> bool {
        self.rsi.is_ready() && self.rsi_history.len() >= self.period
    }
}

// ---------------------------------------------------------------------------
// CCI — Commodity Channel Index
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct CCI {
    period: usize,
    tp_history: RingBuffer<f64>,
    value: f64,
}

impl CCI {
    pub fn new(period: usize) -> Self {
        Self { period, tp_history: RingBuffer::new(period), value: 0.0 }
    }

    pub fn update_ohlc(&mut self, high: f64, low: f64, close: f64) {
        let tp = (high + low + close) / 3.0;
        self.tp_history.push(tp);
        if self.tp_history.len() < self.period { return; }
        let sma: f64 = self.tp_history.iter().sum::<f64>() / self.period as f64;
        let mean_dev: f64 = self.tp_history.iter()
            .map(|&v| (v - sma).abs())
            .sum::<f64>() / self.period as f64;
        self.value = if mean_dev > 0.0 { (tp - sma) / (0.015 * mean_dev) } else { 0.0 };
    }

    pub fn value(&self) -> f64 { self.value }
    pub fn is_ready(&self) -> bool { self.tp_history.len() >= self.period }
}

// ---------------------------------------------------------------------------
// ADX — Average Directional Index (Wilder)
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct ADX {
    period: usize,
    prev_high: Option<f64>,
    prev_low: Option<f64>,
    prev_close: Option<f64>,
    smoothed_tr: f64,
    smoothed_plus_dm: f64,
    smoothed_minus_dm: f64,
    dx_history: RingBuffer<f64>,
    adx: f64,
    count: usize,
}

impl ADX {
    pub fn new(period: usize) -> Self {
        Self {
            period,
            prev_high: None,
            prev_low: None,
            prev_close: None,
            smoothed_tr: 0.0,
            smoothed_plus_dm: 0.0,
            smoothed_minus_dm: 0.0,
            dx_history: RingBuffer::new(period),
            adx: 0.0,
            count: 0,
        }
    }

    pub fn update_ohlc(&mut self, high: f64, low: f64, close: f64) {
        if let (Some(ph), Some(pl), Some(pc)) = (self.prev_high, self.prev_low, self.prev_close) {
            let tr = (high - low).max((high - pc).abs()).max((low - pc).abs());
            let up = high - ph;
            let down = pl - low;
            let plus_dm = if up > down && up > 0.0 { up } else { 0.0 };
            let minus_dm = if down > up && down > 0.0 { down } else { 0.0 };

            self.count += 1;
            if self.count <= self.period {
                self.smoothed_tr += tr;
                self.smoothed_plus_dm += plus_dm;
                self.smoothed_minus_dm += minus_dm;
            } else {
                let p = self.period as f64;
                self.smoothed_tr = self.smoothed_tr - (self.smoothed_tr / p) + tr;
                self.smoothed_plus_dm = self.smoothed_plus_dm - (self.smoothed_plus_dm / p) + plus_dm;
                self.smoothed_minus_dm = self.smoothed_minus_dm - (self.smoothed_minus_dm / p) + minus_dm;
            }

            if self.smoothed_tr > 0.0 && self.count >= self.period {
                let plus_di = 100.0 * self.smoothed_plus_dm / self.smoothed_tr;
                let minus_di = 100.0 * self.smoothed_minus_dm / self.smoothed_tr;
                let di_sum = plus_di + minus_di;
                let dx = if di_sum > 0.0 { 100.0 * (plus_di - minus_di).abs() / di_sum } else { 0.0 };
                self.dx_history.push(dx);
                if self.dx_history.len() >= self.period {
                    let avg: f64 = self.dx_history.iter().sum::<f64>() / self.dx_history.len() as f64;
                    self.adx = avg;
                }
            }
        }
        self.prev_high = Some(high);
        self.prev_low = Some(low);
        self.prev_close = Some(close);
    }

    pub fn value(&self) -> f64 { self.adx }
    pub fn is_ready(&self) -> bool { self.dx_history.len() >= self.period }
}

// ---------------------------------------------------------------------------
// Parabolic SAR
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct ParabolicSAR {
    af_start: f64,
    af_step: f64,
    af_max: f64,
    af: f64,
    sar: f64,
    ep: f64,        // extreme point
    is_long: bool,
    initialized: bool,
}

impl ParabolicSAR {
    pub fn new() -> Self {
        Self {
            af_start: 0.02,
            af_step: 0.02,
            af_max: 0.20,
            af: 0.02,
            sar: 0.0,
            ep: 0.0,
            is_long: true,
            initialized: false,
        }
    }

    pub fn update_hl(&mut self, high: f64, low: f64) {
        if !self.initialized {
            self.sar = low;
            self.ep = high;
            self.is_long = true;
            self.af = self.af_start;
            self.initialized = true;
            return;
        }
        let new_sar = self.sar + self.af * (self.ep - self.sar);
        if self.is_long {
            if low < new_sar {
                // flip to short
                self.is_long = false;
                self.sar = self.ep;
                self.ep = low;
                self.af = self.af_start;
            } else {
                self.sar = new_sar;
                if high > self.ep {
                    self.ep = high;
                    self.af = (self.af + self.af_step).min(self.af_max);
                }
            }
        } else {
            if high > new_sar {
                // flip to long
                self.is_long = true;
                self.sar = self.ep;
                self.ep = high;
                self.af = self.af_start;
            } else {
                self.sar = new_sar;
                if low < self.ep {
                    self.ep = low;
                    self.af = (self.af + self.af_step).min(self.af_max);
                }
            }
        }
    }

    pub fn value(&self) -> f64 { self.sar }
    pub fn is_long(&self) -> bool { self.is_long }
    pub fn is_ready(&self) -> bool { self.initialized }
}

impl Default for ParabolicSAR {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// Supertrend (ATR-based trailing trend)
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct Supertrend {
    multiplier: f64,
    atr: ATR,
    upper: f64,
    lower: f64,
    trend_up: bool,
    value: f64,
    initialized: bool,
}

impl Supertrend {
    pub fn new(atr_period: usize, multiplier: f64) -> Self {
        Self {
            multiplier,
            atr: ATR::new(atr_period),
            upper: 0.0,
            lower: 0.0,
            trend_up: true,
            value: 0.0,
            initialized: false,
        }
    }

    pub fn update_ohlc(&mut self, high: f64, low: f64, close: f64, prev_close: f64) {
        self.atr.update_ohlc(high, low, prev_close);
        if !self.atr.is_ready() { return; }
        let mid = (high + low) / 2.0;
        let band = self.multiplier * self.atr.value();
        let basic_upper = mid + band;
        let basic_lower = mid - band;
        if !self.initialized {
            self.upper = basic_upper;
            self.lower = basic_lower;
            self.trend_up = close > basic_upper;
            self.initialized = true;
        } else {
            self.upper = if basic_upper < self.upper || prev_close > self.upper {
                basic_upper
            } else { self.upper };
            self.lower = if basic_lower > self.lower || prev_close < self.lower {
                basic_lower
            } else { self.lower };
            if self.trend_up && close < self.lower {
                self.trend_up = false;
            } else if !self.trend_up && close > self.upper {
                self.trend_up = true;
            }
        }
        self.value = if self.trend_up { self.lower } else { self.upper };
    }

    pub fn value(&self) -> f64 { self.value }
    pub fn trend_up(&self) -> bool { self.trend_up }
    pub fn is_ready(&self) -> bool { self.initialized && self.atr.is_ready() }
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
