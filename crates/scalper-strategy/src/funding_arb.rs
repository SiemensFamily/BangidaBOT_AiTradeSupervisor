use scalper_core::config::FundingBiasConfig;
use scalper_core::types::{Side, Signal};
use tracing::debug;

use crate::traits::{MarketContext, Strategy};

/// Funding Rate Bias Strategy — SUPPLEMENTARY (15% default weight).
///
/// Adds directional bias when funding rates are extreme. Also detects
/// cross-exchange funding rate divergence. Does NOT generate standalone
/// signals — only boosts existing signals from other strategies.
pub struct FundingBiasStrategy {
    config: FundingBiasConfig,
}

impl FundingBiasStrategy {
    pub fn new(config: FundingBiasConfig) -> Self {
        Self { config }
    }
}

impl Strategy for FundingBiasStrategy {
    fn name(&self) -> &str {
        "funding_bias"
    }

    fn evaluate(&self, ctx: &MarketContext) -> Option<Signal> {
        if !self.config.enabled {
            return None;
        }

        let threshold = self.config.funding_threshold;
        let boost = self.config.strength_boost;

        // Check for extreme funding rate on primary exchange
        let primary_extreme = ctx.funding_rate.abs() > threshold;

        // Check for cross-exchange divergence
        let divergence = (ctx.funding_rate - ctx.funding_rate_secondary).abs();
        let cross_exchange_signal = divergence > threshold / 2.0;

        if !primary_extreme && !cross_exchange_signal {
            return None;
        }

        // Direction: short when funding is high positive (longs overpaying),
        // long when funding is very negative (shorts overpaying)
        let side = if ctx.funding_rate > threshold {
            Side::Sell
        } else if ctx.funding_rate < -threshold {
            Side::Buy
        } else if cross_exchange_signal {
            // If primary has higher funding than secondary, short primary
            if ctx.funding_rate > ctx.funding_rate_secondary {
                Side::Sell
            } else {
                Side::Buy
            }
        } else {
            return None;
        };

        let strength = boost;

        debug!(
            "Funding bias {:?}: rate={:.4} secondary={:.4} divergence={:.4}",
            side, ctx.funding_rate, ctx.funding_rate_secondary, divergence
        );

        Some(Signal {
            strategy_name: self.name().to_string(),
            symbol: ctx.symbol.clone(),
            exchange: ctx.exchange,
            side,
            strength,
            confidence: strength * 0.70,
            take_profit: None, // TP/SL comes from the primary strategy in ensemble
            stop_loss: None,
            timestamp_ms: ctx.timestamp_ms,
        })
    }

    fn weight(&self) -> f64 {
        self.config.weight
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use scalper_core::types::*;
    use crate::traits::MarketContext;

    fn default_config() -> FundingBiasConfig {
        FundingBiasConfig {
            enabled: true,
            weight: 0.15,
            funding_threshold: 0.05,
            strength_boost: 0.1,
        }
    }

    fn base_ctx() -> MarketContext {
        MarketContext {
            symbol: "BTCUSDT".into(),
            exchange: Exchange::Binance,
            last_price: dec!(50000),
            best_bid: dec!(49999),
            best_ask: dec!(50001),
            spread: dec!(2),
            tick_size: dec!(0.1),
            imbalance_ratio: 0.0,
            bid_depth_10: dec!(100),
            ask_depth_10: dec!(100),
            rsi_14: 50.0,
            ema_9: 50000.0,
            ema_21: 50000.0,
            macd_histogram: 0.0,
            bollinger_upper: 51000.0,
            bollinger_lower: 49000.0,
            bollinger_middle: 50000.0,
            vwap: 50000.0,
            atr_14: 200.0,
            obv: 0.0,
            cvd: 0.0,
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
            timestamp_ms: 1000000,
        }
    }

    #[test]
    fn sell_on_high_funding() {
        let strategy = FundingBiasStrategy::new(default_config());
        let mut ctx = base_ctx();
        ctx.funding_rate = 0.08; // above 0.05 threshold
        let signal = strategy.evaluate(&ctx);
        assert!(signal.is_some());
        assert_eq!(signal.unwrap().side, Side::Sell);
    }

    #[test]
    fn buy_on_negative_funding() {
        let strategy = FundingBiasStrategy::new(default_config());
        let mut ctx = base_ctx();
        ctx.funding_rate = -0.08;
        let signal = strategy.evaluate(&ctx);
        assert!(signal.is_some());
        assert_eq!(signal.unwrap().side, Side::Buy);
    }

    #[test]
    fn no_signal_normal_funding() {
        let strategy = FundingBiasStrategy::new(default_config());
        let signal = strategy.evaluate(&base_ctx());
        assert!(signal.is_none());
    }

    #[test]
    fn signal_on_cross_exchange_divergence() {
        let strategy = FundingBiasStrategy::new(default_config());
        let mut ctx = base_ctx();
        ctx.funding_rate = 0.04; // below threshold alone
        ctx.funding_rate_secondary = -0.01; // divergence = 0.05 > threshold/2
        let signal = strategy.evaluate(&ctx);
        assert!(signal.is_some());
        assert_eq!(signal.unwrap().side, Side::Sell); // short higher-funding exchange
    }
}
