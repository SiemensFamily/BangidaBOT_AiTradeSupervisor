use rust_decimal::Decimal;
use scalper_core::types::{Exchange, Signal, Trend, VolatilityRegime};

/// Rolling Donchian Channel snapshot: highest high and lowest low over
/// the last N bars, for several common N values. The replay engine and
/// live bot precompute these so strategies can reference them without
/// owning their own rolling state.
#[derive(Debug, Clone, Default)]
pub struct DonchianSnapshot {
    pub upper_10: f64,
    pub lower_10: f64,
    pub upper_20: f64,
    pub lower_20: f64,
    pub upper_55: f64,
    pub lower_55: f64,
}

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
    pub ema_50: f64,
    pub ema_200: f64,
    pub macd_histogram: f64,
    pub bollinger_upper: f64,
    pub bollinger_lower: f64,
    pub bollinger_middle: f64,
    pub vwap: f64,
    pub atr_14: f64,
    pub obv: f64,

    // Extended indicators
    pub stoch_k: f64,           // Stochastic %K (0-100)
    pub stoch_d: f64,           // Stochastic %D (0-100)
    pub stoch_rsi: f64,         // StochRSI (0-100)
    pub cci_20: f64,            // Commodity Channel Index (typically -200..+200)
    pub adx_14: f64,            // Average Directional Index (0-100, >25 = strong trend)
    pub psar: f64,              // Parabolic SAR price
    pub psar_long: bool,        // PSAR currently in long mode
    pub supertrend: f64,        // Supertrend line price
    pub supertrend_up: bool,    // Supertrend currently bullish

    // Donchian channels (for swing / breakout strategies)
    pub donchian: DonchianSnapshot,

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
