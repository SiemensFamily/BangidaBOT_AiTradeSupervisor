use rust_decimal::Decimal;
use scalper_core::config::ObImbalanceConfig;
use scalper_core::types::{Side, Signal};
use tracing::debug;

use crate::traits::{MarketContext, Strategy};

/// Order Book Imbalance Strategy — SECONDARY (25% default weight).
///
/// Monitors bid/ask volume imbalance in the top levels of the order book.
/// Confirmed by Cumulative Volume Delta (CVD). Uses tick-based TP/SL
/// which is tight — best for BTC/ETH on liquid exchanges.
/// Uses PostOnly orders exclusively to capture maker rebates.
pub struct ObImbalanceStrategy {
    config: ObImbalanceConfig,
}

impl ObImbalanceStrategy {
    pub fn new(config: ObImbalanceConfig) -> Self {
        Self { config }
    }
}

impl Strategy for ObImbalanceStrategy {
    fn name(&self) -> &str {
        "ob_imbalance"
    }

    fn evaluate(&self, ctx: &MarketContext) -> Option<Signal> {
        if !self.config.enabled {
            return None;
        }

        // Spread guard: skip if spread is too wide (> 2x tick size)
        if ctx.spread > ctx.tick_size * Decimal::from(2) {
            return None;
        }

        let threshold = self.config.imbalance_threshold;

        // Check for bullish imbalance: bids dominate
        if ctx.imbalance_ratio > threshold && ctx.cvd > 0.0 {
            let excess = ctx.imbalance_ratio - threshold;
            let strength = (excess / (1.0 - threshold)).min(1.0);

            let tp_offset = ctx.tick_size * Decimal::from(self.config.take_profit_ticks);
            let sl_offset = ctx.tick_size * Decimal::from(self.config.stop_loss_ticks);

            let mid = ctx.mid_price();
            let tp = mid + tp_offset;
            let sl = mid - sl_offset;

            debug!("OB imbalance BUY: ratio={:.3} cvd={:.1} strength={:.2}", ctx.imbalance_ratio, ctx.cvd, strength);

            return Some(Signal {
                strategy_name: self.name().to_string(),
                symbol: ctx.symbol.clone(),
                exchange: ctx.exchange,
                side: Side::Buy,
                strength,
                confidence: strength * 0.85,
                take_profit: Some(tp),
                stop_loss: Some(sl),
                timestamp_ms: ctx.timestamp_ms,
            });
        }

        // Check for bearish imbalance: asks dominate
        if ctx.imbalance_ratio < -threshold && ctx.cvd < 0.0 {
            let excess = (-ctx.imbalance_ratio) - threshold;
            let strength = (excess / (1.0 - threshold)).min(1.0);

            let tp_offset = ctx.tick_size * Decimal::from(self.config.take_profit_ticks);
            let sl_offset = ctx.tick_size * Decimal::from(self.config.stop_loss_ticks);

            let mid = ctx.mid_price();
            let tp = mid - tp_offset;
            let sl = mid + sl_offset;

            debug!("OB imbalance SELL: ratio={:.3} cvd={:.1} strength={:.2}", ctx.imbalance_ratio, ctx.cvd, strength);

            return Some(Signal {
                strategy_name: self.name().to_string(),
                symbol: ctx.symbol.clone(),
                exchange: ctx.exchange,
                side: Side::Sell,
                strength,
                confidence: strength * 0.85,
                take_profit: Some(tp),
                stop_loss: Some(sl),
                timestamp_ms: ctx.timestamp_ms,
            });
        }

        None
    }

    fn weight(&self) -> f64 {
        self.config.weight
    }
}

impl MarketContext {
    pub fn mid_price(&self) -> Decimal {
        (self.best_bid + self.best_ask) / Decimal::from(2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use scalper_core::types::*;

    fn default_config() -> ObImbalanceConfig {
        ObImbalanceConfig {
            enabled: true,
            weight: 0.25,
            min_imbalance_ratio: 0.20,
            imbalance_threshold: 0.30,
            take_profit_ticks: 3,
            stop_loss_ticks: 2,
        }
    }

    fn base_ctx() -> MarketContext {
        MarketContext {
            symbol: "BTCUSDT".into(),
            exchange: Exchange::Binance,
            last_price: dec!(50000),
            best_bid: dec!(49999),
            best_ask: dec!(50001),
            spread: dec!(0.1),
            tick_size: dec!(0.1),
            imbalance_ratio: 0.5,
            bid_depth_10: dec!(200),
            ask_depth_10: dec!(100),
            rsi_14: 50.0,
            ema_9: 50000.0,
            ema_21: 50000.0,
            ema_50: 50000.0,
            ema_200: 50000.0,
            macd_histogram: 0.0,
            bollinger_upper: 51000.0,
            bollinger_lower: 49000.0,
            bollinger_middle: 50000.0,
            vwap: 50000.0,
            atr_14: 200.0,
            obv: 0.0,
            cvd: 100.0,
            volume_ratio: 1.0,
            liquidation_volume_1m: 0.0,
            tf_5m_trend: Trend::Neutral,
            tf_15m_trend: Trend::Neutral,
            volatility_regime: VolatilityRegime::Normal,
            highest_high_60s: 50100.0,
            lowest_low_60s: 49900.0,
            avg_volume_60s: 100.0,
            current_volume: 100.0,
            funding_rate: 0.001,
            funding_rate_secondary: 0.001,
            open_interest: None,
            price_velocity_30s: 0.0,
            stoch_k: 50.0,
            stoch_d: 50.0,
            stoch_rsi: 50.0,
            cci_20: 0.0,
            adx_14: 20.0,
            psar: 0.0,
            psar_long: true,
            supertrend: 0.0,
            supertrend_up: true,
            donchian: Default::default(),
            timestamp_ms: 1000000,
        }
    }

    #[test]
    fn buy_on_bid_imbalance() {
        let strategy = ObImbalanceStrategy::new(default_config());
        let ctx = base_ctx();
        let signal = strategy.evaluate(&ctx);
        assert!(signal.is_some());
        assert_eq!(signal.unwrap().side, Side::Buy);
    }

    #[test]
    fn sell_on_ask_imbalance() {
        let strategy = ObImbalanceStrategy::new(default_config());
        let mut ctx = base_ctx();
        ctx.imbalance_ratio = -0.5;
        ctx.cvd = -100.0;
        let signal = strategy.evaluate(&ctx);
        assert!(signal.is_some());
        assert_eq!(signal.unwrap().side, Side::Sell);
    }

    #[test]
    fn no_signal_wide_spread() {
        let strategy = ObImbalanceStrategy::new(default_config());
        let mut ctx = base_ctx();
        ctx.spread = dec!(1); // 10x tick size, too wide
        ctx.tick_size = dec!(0.1);
        let signal = strategy.evaluate(&ctx);
        assert!(signal.is_none());
    }

    #[test]
    fn no_signal_weak_imbalance() {
        let strategy = ObImbalanceStrategy::new(default_config());
        let mut ctx = base_ctx();
        ctx.imbalance_ratio = 0.15; // below 0.30 threshold
        let signal = strategy.evaluate(&ctx);
        assert!(signal.is_none());
    }
}
