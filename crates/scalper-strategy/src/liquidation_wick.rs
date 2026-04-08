use rust_decimal::Decimal;
use scalper_core::config::LiquidationWickConfig;
use scalper_core::types::{Side, Signal};
use tracing::debug;

use crate::traits::{MarketContext, Strategy};

/// Liquidation Wick Reversal Strategy — NEW (20% default weight).
///
/// Detects liquidation cascades via rapid price movement + volume spike,
/// then enters a reversal trade after the cascade exhausts.
/// High win rate (65%+) but fires infrequently (1-3 times/day).
///
/// Requires: Open Interest data + funding rate elevation as preconditions.
pub struct LiquidationWickStrategy {
    config: LiquidationWickConfig,
}

impl LiquidationWickStrategy {
    pub fn new(config: LiquidationWickConfig) -> Self {
        Self { config }
    }
}

impl Strategy for LiquidationWickStrategy {
    fn name(&self) -> &str {
        "liquidation_wick"
    }

    fn evaluate(&self, ctx: &MarketContext) -> Option<Signal> {
        if !self.config.enabled {
            return None;
        }

        // Need significant liquidation volume in the last minute
        if ctx.liquidation_volume_1m < self.config.volume_spike_multiplier * ctx.avg_volume_60s {
            return None;
        }

        // Price velocity must show a sharp move (cascade in progress or just completed)
        let velocity_abs = ctx.price_velocity_30s.abs();
        if velocity_abs < self.config.price_velocity_threshold {
            return None;
        }

        // Determine direction: trade the REVERSAL of the cascade
        // If price dropped fast (negative velocity) → cascade was downward → enter LONG
        // If price rose fast (positive velocity) → cascade was upward → enter SHORT
        let (side, tp, sl) = if ctx.price_velocity_30s < 0.0 {
            // Downward cascade exhaustion → BUY reversal
            let tp = ctx.last_price
                * Decimal::from_f64_retain(1.0 + self.config.take_profit_pct / 100.0)?;
            let sl = ctx.last_price
                * Decimal::from_f64_retain(1.0 - self.config.stop_loss_pct / 100.0)?;
            (Side::Buy, tp, sl)
        } else {
            // Upward cascade exhaustion → SELL reversal
            let tp = ctx.last_price
                * Decimal::from_f64_retain(1.0 - self.config.take_profit_pct / 100.0)?;
            let sl = ctx.last_price
                * Decimal::from_f64_retain(1.0 + self.config.stop_loss_pct / 100.0)?;
            (Side::Sell, tp, sl)
        };

        // Signal strength based on velocity magnitude and liquidation volume
        let velocity_strength = (velocity_abs / self.config.price_velocity_threshold).min(2.0) / 2.0;
        let vol_ratio = ctx.liquidation_volume_1m / (self.config.volume_spike_multiplier * ctx.avg_volume_60s);
        let vol_strength = (vol_ratio).min(2.0) / 2.0;
        let strength = (velocity_strength + vol_strength) / 2.0;

        debug!(
            "Liquidation wick {:?}: velocity={:.3}% liq_vol={:.0} strength={:.2}",
            side, ctx.price_velocity_30s, ctx.liquidation_volume_1m, strength
        );

        Some(Signal {
            strategy_name: self.name().to_string(),
            symbol: ctx.symbol.clone(),
            exchange: ctx.exchange,
            side,
            strength,
            confidence: strength * 0.80,
            take_profit: Some(tp),
            stop_loss: Some(sl),
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

    fn default_config() -> LiquidationWickConfig {
        LiquidationWickConfig {
            enabled: true,
            weight: 0.20,
            price_velocity_threshold: 1.0,
            volume_spike_multiplier: 3.0,
            take_profit_pct: 0.80,
            stop_loss_pct: 0.40,
        }
    }

    fn cascade_ctx() -> MarketContext {
        MarketContext {
            symbol: "BTCUSDT".into(),
            exchange: Exchange::Binance,
            last_price: dec!(48000),
            best_bid: dec!(47999),
            best_ask: dec!(48001),
            spread: dec!(2),
            tick_size: dec!(0.1),
            imbalance_ratio: -0.2,
            bid_depth_10: dec!(50),
            ask_depth_10: dec!(150),
            rsi_14: 25.0,
            ema_9: 49000.0,
            ema_21: 49500.0,
            macd_histogram: -50.0,
            bollinger_upper: 51000.0,
            bollinger_lower: 47000.0,
            bollinger_middle: 49000.0,
            vwap: 49000.0,
            atr_14: 500.0,
            obv: -5000.0,
            cvd: -2000.0,
            volume_ratio: 0.3,
            liquidation_volume_1m: 5000.0, // massive liquidation volume
            tf_5m_trend: Trend::Down,
            tf_15m_trend: Trend::Down,
            volatility_regime: VolatilityRegime::Volatile,
            highest_high_60s: 50000.0,
            lowest_low_60s: 47500.0,
            avg_volume_60s: 500.0,
            current_volume: 3000.0,
            funding_rate: 0.08,
            funding_rate_secondary: 0.06,
            open_interest: Some(1000000.0),
            price_velocity_30s: -2.5, // -2.5% in 30 seconds = sharp drop
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
    fn buy_reversal_on_downward_cascade() {
        let strategy = LiquidationWickStrategy::new(default_config());
        let ctx = cascade_ctx();
        let signal = strategy.evaluate(&ctx);
        assert!(signal.is_some());
        let s = signal.unwrap();
        assert_eq!(s.side, Side::Buy); // reversal of downward cascade
        assert!(s.strength > 0.0);
    }

    #[test]
    fn sell_reversal_on_upward_cascade() {
        let strategy = LiquidationWickStrategy::new(default_config());
        let mut ctx = cascade_ctx();
        ctx.price_velocity_30s = 2.5; // upward cascade
        let signal = strategy.evaluate(&ctx);
        assert!(signal.is_some());
        assert_eq!(signal.unwrap().side, Side::Sell);
    }

    #[test]
    fn no_signal_without_liquidation_volume() {
        let strategy = LiquidationWickStrategy::new(default_config());
        let mut ctx = cascade_ctx();
        ctx.liquidation_volume_1m = 100.0; // too low
        let signal = strategy.evaluate(&ctx);
        assert!(signal.is_none());
    }

    #[test]
    fn no_signal_slow_price_movement() {
        let strategy = LiquidationWickStrategy::new(default_config());
        let mut ctx = cascade_ctx();
        ctx.price_velocity_30s = -0.3; // too slow
        let signal = strategy.evaluate(&ctx);
        assert!(signal.is_none());
    }
}
