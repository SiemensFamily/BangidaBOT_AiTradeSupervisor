use rust_decimal::Decimal;
use scalper_core::config::MomentumConfig;
use scalper_core::types::{Side, Signal, Trend};
use tracing::debug;

use crate::traits::{MarketContext, Strategy};

/// Momentum Breakout Strategy — PRIMARY strategy (40% default weight).
///
/// Detects price breakouts above/below 60-second high/low with volume spike
/// confirmation, RSI filter, OBV confirmation, and multi-timeframe trend alignment.
/// Optimized for $100 accounts with 2:1 risk-reward ratio.
pub struct MomentumStrategy {
    config: MomentumConfig,
}

impl MomentumStrategy {
    pub fn new(config: MomentumConfig) -> Self {
        Self { config }
    }

    fn check_long(&self, ctx: &MarketContext) -> Option<(f64, Decimal, Decimal)> {
        let price_f64 = decimal_to_f64(ctx.last_price);

        // Breakout above 60s high
        if price_f64 <= ctx.highest_high_60s {
            return None;
        }

        // Volume spike confirmation
        if ctx.avg_volume_60s <= 0.0 {
            return None;
        }
        let volume_ratio = ctx.current_volume / ctx.avg_volume_60s;
        if volume_ratio < self.config.volume_spike_multiplier {
            return None;
        }

        // RSI filter: avoid overbought
        if ctx.rsi_14 >= self.config.rsi_overbought {
            return None;
        }

        // OBV confirmation: OBV should be positive (accumulation)
        if ctx.obv < 0.0 {
            return None;
        }

        // Multi-timeframe filter: 5m and 15m should not be downtrend
        if ctx.tf_5m_trend == Trend::Down || ctx.tf_15m_trend == Trend::Down {
            return None;
        }

        // EMA trend alignment: fast EMA above slow EMA
        if ctx.ema_9 < ctx.ema_21 {
            return None;
        }

        // Signal strength based on volume ratio
        let strength = (volume_ratio / self.config.volume_spike_multiplier)
            .min(2.0)
            / 2.0;

        let tp = ctx.last_price * Decimal::from_f64_retain(1.0 + self.config.take_profit_pct / 100.0)?;
        let sl = ctx.last_price * Decimal::from_f64_retain(1.0 - self.config.stop_loss_pct / 100.0)?;

        Some((strength, tp, sl))
    }

    fn check_short(&self, ctx: &MarketContext) -> Option<(f64, Decimal, Decimal)> {
        let price_f64 = decimal_to_f64(ctx.last_price);

        // Breakout below 60s low
        if price_f64 >= ctx.lowest_low_60s {
            return None;
        }

        // Volume spike confirmation
        if ctx.avg_volume_60s <= 0.0 {
            return None;
        }
        let volume_ratio = ctx.current_volume / ctx.avg_volume_60s;
        if volume_ratio < self.config.volume_spike_multiplier {
            return None;
        }

        // RSI filter: avoid oversold
        if ctx.rsi_14 <= self.config.rsi_oversold {
            return None;
        }

        // OBV confirmation: OBV should be negative (distribution)
        if ctx.obv > 0.0 {
            return None;
        }

        // Multi-timeframe filter
        if ctx.tf_5m_trend == Trend::Up || ctx.tf_15m_trend == Trend::Up {
            return None;
        }

        // EMA alignment
        if ctx.ema_9 > ctx.ema_21 {
            return None;
        }

        let strength = (volume_ratio / self.config.volume_spike_multiplier)
            .min(2.0)
            / 2.0;

        let tp = ctx.last_price * Decimal::from_f64_retain(1.0 - self.config.take_profit_pct / 100.0)?;
        let sl = ctx.last_price * Decimal::from_f64_retain(1.0 + self.config.stop_loss_pct / 100.0)?;

        Some((strength, tp, sl))
    }
}

impl Strategy for MomentumStrategy {
    fn name(&self) -> &str {
        "momentum_breakout"
    }

    fn evaluate(&self, ctx: &MarketContext) -> Option<Signal> {
        if !self.config.enabled {
            return None;
        }

        let (side, strength, tp, sl) = if let Some((s, tp, sl)) = self.check_long(ctx) {
            (Side::Buy, s, tp, sl)
        } else if let Some((s, tp, sl)) = self.check_short(ctx) {
            (Side::Sell, s, tp, sl)
        } else {
            return None;
        };

        debug!(
            "Momentum signal: {:?} {} strength={:.2} tp={} sl={}",
            side, ctx.symbol, strength, tp, sl
        );

        Some(Signal {
            strategy_name: self.name().to_string(),
            symbol: ctx.symbol.clone(),
            exchange: ctx.exchange,
            side,
            strength,
            confidence: strength * 0.9,
            take_profit: Some(tp),
            stop_loss: Some(sl),
            timestamp_ms: ctx.timestamp_ms,
        })
    }

    fn weight(&self) -> f64 {
        self.config.weight
    }
}

fn decimal_to_f64(d: Decimal) -> f64 {
    use std::str::FromStr;
    f64::from_str(&d.to_string()).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use scalper_core::types::{Exchange, VolatilityRegime};

    fn default_config() -> MomentumConfig {
        MomentumConfig {
            enabled: true,
            weight: 0.40,
            volume_spike_multiplier: 2.5,
            take_profit_pct: 0.50,
            stop_loss_pct: 0.25,
            trailing_stop_pct: 0.20,
            rsi_overbought: 80.0,
            rsi_oversold: 20.0,
        }
    }

    fn base_context() -> MarketContext {
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
            rsi_14: 55.0,
            ema_9: 50100.0,
            ema_21: 49900.0,
            ema_50: 49800.0,
            ema_200: 49500.0,
            macd_histogram: 10.0,
            bollinger_upper: 51000.0,
            bollinger_lower: 49000.0,
            bollinger_middle: 50000.0,
            vwap: 50000.0,
            atr_14: 200.0,
            obv: 1000.0,
            cvd: 500.0,
            volume_ratio: 1.5,
            liquidation_volume_1m: 0.0,
            tf_5m_trend: Trend::Up,
            tf_15m_trend: Trend::Up,
            volatility_regime: VolatilityRegime::Normal,
            highest_high_60s: 49950.0, // price above this = breakout
            lowest_low_60s: 49800.0,
            avg_volume_60s: 100.0,
            current_volume: 300.0, // 3x = spike
            funding_rate: 0.001,
            funding_rate_secondary: 0.001,
            open_interest: None,
            price_velocity_30s: 0.1,
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
    fn long_signal_on_breakout() {
        let strategy = MomentumStrategy::new(default_config());
        let ctx = base_context();
        let signal = strategy.evaluate(&ctx);
        assert!(signal.is_some());
        let s = signal.unwrap();
        assert_eq!(s.side, Side::Buy);
        assert!(s.strength > 0.0);
        assert!(s.take_profit.is_some());
    }

    #[test]
    fn no_signal_without_volume_spike() {
        let strategy = MomentumStrategy::new(default_config());
        let mut ctx = base_context();
        ctx.current_volume = 100.0; // 1x, below 2.5x threshold
        let signal = strategy.evaluate(&ctx);
        assert!(signal.is_none());
    }

    #[test]
    fn no_signal_when_overbought() {
        let strategy = MomentumStrategy::new(default_config());
        let mut ctx = base_context();
        ctx.rsi_14 = 85.0;
        let signal = strategy.evaluate(&ctx);
        assert!(signal.is_none());
    }

    #[test]
    fn no_long_in_downtrend() {
        let strategy = MomentumStrategy::new(default_config());
        let mut ctx = base_context();
        ctx.tf_15m_trend = Trend::Down;
        let signal = strategy.evaluate(&ctx);
        assert!(signal.is_none());
    }

    #[test]
    fn disabled_returns_none() {
        let mut cfg = default_config();
        cfg.enabled = false;
        let strategy = MomentumStrategy::new(cfg);
        let signal = strategy.evaluate(&base_context());
        assert!(signal.is_none());
    }
}
