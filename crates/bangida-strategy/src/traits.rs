use bangida_core::{Signal, Symbol};
use rust_decimal::Decimal;

/// All the market data a strategy needs to make a decision in a single evaluation tick.
#[derive(Debug, Clone)]
pub struct MarketContext {
    pub symbol: Symbol,
    /// Order-book imbalance ratio, range -1.0 (all asks) to +1.0 (all bids).
    pub orderbook_imbalance: f64,
    pub spread: Decimal,
    pub mid_price: Decimal,
    pub microprice: Decimal,
    pub bid_depth: Decimal,
    pub ask_depth: Decimal,
    pub last_price: Decimal,
    // --- Technical indicators ---
    pub rsi: f64,
    pub ema_fast: f64,
    pub ema_slow: f64,
    pub bb_upper: f64,
    pub bb_lower: f64,
    pub bb_middle: f64,
    pub macd_line: f64,
    pub macd_signal: f64,
    pub macd_histogram: f64,
    pub vwap: f64,
    /// Cumulative volume delta – positive means net buying.
    pub cvd: f64,
    pub volume_1s: f64,
    pub avg_volume_60s: f64,
    pub funding_rate: f64,
    pub highest_high_60s: Decimal,
    pub lowest_low_60s: Decimal,
    pub timestamp_ms: u64,
}

/// Core trait that every strategy must implement.
pub trait Strategy: Send + Sync {
    /// Human-readable name used in logs and signal attribution.
    fn name(&self) -> &str;

    /// Evaluate the current market context and optionally produce a signal.
    fn evaluate(&self, ctx: &MarketContext) -> Option<Signal>;

    /// Relative weight of this strategy in the ensemble (0.0 – 1.0).
    fn weight(&self) -> f64;
}
