use rust_decimal::Decimal;
use scalper_core::types::{Exchange, Signal, Trend, VolatilityRegime};

/// Aggregated market data snapshot passed to each strategy for evaluation.
#[derive(Debug, Clone)]
pub struct MarketContext {
    pub symbol: String,
    pub exchange: Exchange,
    pub last_price: Decimal,
    pub best_bid: Decimal,
    pub best_ask: Decimal,
    pub spread: Decimal,
    pub tick_size: Decimal,

    // Order book
    pub imbalance_ratio: f64,
    pub bid_depth_10: Decimal,
    pub ask_depth_10: Decimal,

    // Indicators
    pub rsi_14: f64,
    pub ema_9: f64,
    pub ema_21: f64,
    pub macd_histogram: f64,
    pub bollinger_upper: f64,
    pub bollinger_lower: f64,
    pub bollinger_middle: f64,
    pub vwap: f64,
    pub atr_14: f64,
    pub obv: f64,

    // Order flow
    pub cvd: f64,
    pub volume_ratio: f64,
    pub liquidation_volume_1m: f64,

    // Multi-timeframe
    pub tf_5m_trend: Trend,
    pub tf_15m_trend: Trend,

    // Volatility regime
    pub volatility_regime: VolatilityRegime,

    // Candle data
    pub highest_high_60s: f64,
    pub lowest_low_60s: f64,
    pub avg_volume_60s: f64,
    pub current_volume: f64,

    // Funding
    pub funding_rate: f64,
    pub funding_rate_secondary: f64,

    // Open interest (if available)
    pub open_interest: Option<f64>,

    // Price velocity (pct change per 30s)
    pub price_velocity_30s: f64,

    pub timestamp_ms: u64,
}

/// Trait that all trading strategies must implement.
pub trait Strategy: Send + Sync {
    /// Human-readable strategy name.
    fn name(&self) -> &str;

    /// Evaluate the current market snapshot and optionally produce a trading signal.
    fn evaluate(&self, ctx: &MarketContext) -> Option<Signal>;

    /// Weight of this strategy in the ensemble (0.0 to 1.0).
    fn weight(&self) -> f64;
}
