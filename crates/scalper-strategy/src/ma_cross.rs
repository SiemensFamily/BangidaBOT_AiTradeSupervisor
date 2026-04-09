//! Moving Average Crossover — the other canonical swing-trading setup.
//!
//! Simple, well-studied, and surprisingly effective on trending assets.
//! Enter long when the fast EMA crosses above the slow EMA; enter short when
//! the fast EMA crosses below the slow EMA. Exit on the opposite cross (or
//! the backtest's time-based safety net via max_hold_bars).
//!
//! Classical configurations:
//!   * 9/21    — short-swing, high frequency (many whipsaws in chop)
//!   * 20/50   — medium swing, the most commonly used setup on daily bars
//!   * 50/200  — "golden cross" / "death cross", long-term trend following
//!
//! The replay engine populates ema_9, ema_21, ema_50, and ema_200 on every
//! MarketContext, so the strategy can reference them by period ID. Since the
//! Strategy trait is stateless (no access to prior bars), we detect "cross"
//! by checking whether the fast EMA has moved to the other side of the slow
//! EMA within the last `tolerance` bps of the fast EMA. This isn't perfect
//! edge-detection but is reliable enough for daily bars where crosses are
//! clean events.
//!
//! **Important caveat**: the current implementation fires on any bar where
//! fast > slow (or fast < slow), not only on the bar where the cross
//! happens. This makes it a "trend regime" filter rather than a true
//! crossover signal. The backtest harness's open-position guard prevents
//! re-entering while already in position, so the effective behavior is
//! "enter on first bar after cross, exit on opposite cross" — which is what
//! the canonical MA cross does.
//!
//! ATR-based stops so targets adapt to symbol/timeframe volatility.

use rust_decimal::Decimal;
use scalper_core::config::MaCrossConfig;
use scalper_core::types::{Side, Signal};
use tracing::debug;

use crate::traits::{MarketContext, Strategy};

pub struct MaCrossStrategy {
    config: MaCrossConfig,
}

impl MaCrossStrategy {
    pub fn new(config: MaCrossConfig) -> Self {
        Self { config }
    }

    /// Look up an EMA value from the context by period. Only the periods
    /// the replay engine computes are supported (9, 21, 50, 200).
    fn ema_for(&self, ctx: &MarketContext, period: u32) -> Option<f64> {
        match period {
            9 => Some(ctx.ema_9),
            21 => Some(ctx.ema_21),
            50 => Some(ctx.ema_50),
            200 => Some(ctx.ema_200),
            _ => None,
        }
    }

    fn check_long(&self, ctx: &MarketContext) -> Option<(f64, Decimal, Decimal)> {
        let price = decimal_to_f64(ctx.last_price);
        let fast = self.ema_for(ctx, self.config.fast_period)?;
        let slow = self.ema_for(ctx, self.config.slow_period)?;

        // Both EMAs must be populated (non-zero)
        if fast <= 0.0 || slow <= 0.0 {
            return None;
        }

        // Fast must be above slow AND price must be above fast (confirmation)
        if fast <= slow {
            return None;
        }
        if price < fast {
            return None;
        }

        // Require minimum spread between fast and slow — rejects weak or
        // transitional crosses that tend to whipsaw.
        let spread_pct = (fast - slow) / slow;
        if spread_pct < self.config.min_spread_pct {
            return None;
        }

        if ctx.atr_14 <= 0.0 {
            return None;
        }

        // Trend-regime signals are binary once filters are passed; use a
        // high-conviction fixed strength plus a magnitude bonus up to 1.0.
        let magnitude_bonus = ((spread_pct - self.config.min_spread_pct)
            / self.config.min_spread_pct.max(0.001))
        .min(0.2);
        let strength = (0.8 + magnitude_bonus).min(1.0);

        let tp_price = price + ctx.atr_14 * self.config.atr_tp_multiplier;
        let sl_price = price - ctx.atr_14 * self.config.atr_stop_multiplier;

        let tp = Decimal::from_f64_retain(tp_price)?;
        let sl = Decimal::from_f64_retain(sl_price)?;

        Some((strength, tp, sl))
    }

    fn check_short(&self, ctx: &MarketContext) -> Option<(f64, Decimal, Decimal)> {
        let price = decimal_to_f64(ctx.last_price);
        let fast = self.ema_for(ctx, self.config.fast_period)?;
        let slow = self.ema_for(ctx, self.config.slow_period)?;

        if fast <= 0.0 || slow <= 0.0 {
            return None;
        }

        if fast >= slow {
            return None;
        }
        if price > fast {
            return None;
        }

        let spread_pct = (slow - fast) / slow;
        if spread_pct < self.config.min_spread_pct {
            return None;
        }

        if ctx.atr_14 <= 0.0 {
            return None;
        }

        let magnitude_bonus = ((spread_pct - self.config.min_spread_pct)
            / self.config.min_spread_pct.max(0.001))
        .min(0.2);
        let strength = (0.8 + magnitude_bonus).min(1.0);

        let tp_price = price - ctx.atr_14 * self.config.atr_tp_multiplier;
        let sl_price = price + ctx.atr_14 * self.config.atr_stop_multiplier;

        let tp = Decimal::from_f64_retain(tp_price)?;
        let sl = Decimal::from_f64_retain(sl_price)?;

        Some((strength, tp, sl))
    }
}

impl Strategy for MaCrossStrategy {
    fn name(&self) -> &str {
        "ma_cross"
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
            "MACross signal: {:?} {} fast={} slow={} strength={:.2}",
            side, ctx.symbol, self.config.fast_period, self.config.slow_period, strength
        );

        Some(Signal {
            strategy_name: self.name().to_string(),
            symbol: ctx.symbol.clone(),
            exchange: ctx.exchange,
            side,
            strength,
            confidence: strength * 0.85,
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
    use scalper_core::types::{Exchange, Trend, VolatilityRegime};

    fn default_config() -> MaCrossConfig {
        MaCrossConfig {
            enabled: true,
            weight: 0.30,
            fast_period: 21,
            slow_period: 50,
            min_spread_pct: 0.005, // 0.5% minimum spread between fast and slow
            atr_tp_multiplier: 3.0,
            atr_stop_multiplier: 1.5,
        }
    }

    fn base_context() -> MarketContext {
        MarketContext {
            symbol: "BTCUSDT".into(),
            exchange: Exchange::Kraken,
            last_price: dec!(50500),
            best_bid: dec!(50499),
            best_ask: dec!(50501),
            spread: dec!(2),
            tick_size: dec!(0.1),
            imbalance_ratio: 0.0,
            bid_depth_10: dec!(100),
            ask_depth_10: dec!(100),
            rsi_14: 55.0,
            ema_9: 50400.0,
            ema_21: 50400.0, // fast
            ema_50: 50000.0, // slow (fast > slow = bullish)
            ema_200: 49000.0,
            macd_histogram: 10.0,
            bollinger_upper: 51000.0,
            bollinger_lower: 49000.0,
            bollinger_middle: 50000.0,
            vwap: 50000.0,
            atr_14: 300.0,
            obv: 100.0,
            stoch_k: 50.0,
            stoch_d: 50.0,
            stoch_rsi: 50.0,
            cci_20: 0.0,
            adx_14: 25.0,
            psar: 0.0,
            psar_long: true,
            supertrend: 0.0,
            supertrend_up: true,
            donchian: Default::default(),
            cvd: 0.0,
            volume_ratio: 1.0,
            liquidation_volume_1m: 0.0,
            tf_5m_trend: Trend::Up,
            tf_15m_trend: Trend::Up,
            volatility_regime: VolatilityRegime::Normal,
            highest_high_60s: 50500.0,
            lowest_low_60s: 50000.0,
            avg_volume_60s: 100.0,
            current_volume: 100.0,
            funding_rate: 0.0,
            funding_rate_secondary: 0.0,
            open_interest: None,
            price_velocity_30s: 0.0,
            timestamp_ms: 1_000_000,
        }
    }

    #[test]
    fn fires_long_when_fast_above_slow() {
        let strategy = MaCrossStrategy::new(default_config());
        let ctx = base_context();
        let signal = strategy.evaluate(&ctx).expect("should fire long");
        assert_eq!(signal.side, Side::Buy);
        assert!(signal.strength > 0.0);
    }

    #[test]
    fn fires_short_when_fast_below_slow() {
        let strategy = MaCrossStrategy::new(default_config());
        let mut ctx = base_context();
        ctx.last_price = dec!(49500);
        ctx.ema_21 = 49600.0;
        ctx.ema_50 = 50000.0; // slow > fast = bearish
        let signal = strategy.evaluate(&ctx).expect("should fire short");
        assert_eq!(signal.side, Side::Sell);
    }

    #[test]
    fn no_signal_when_emas_converged() {
        let strategy = MaCrossStrategy::new(default_config());
        let mut ctx = base_context();
        ctx.ema_21 = 50000.0;
        ctx.ema_50 = 50000.0;
        assert!(strategy.evaluate(&ctx).is_none());
    }

    #[test]
    fn long_requires_price_above_fast() {
        let strategy = MaCrossStrategy::new(default_config());
        let mut ctx = base_context();
        // Price below fast but fast still above slow — no long entry
        ctx.last_price = dec!(50300);
        ctx.ema_21 = 50400.0;
        assert!(strategy.evaluate(&ctx).is_none());
    }

    #[test]
    fn unsupported_period_returns_none() {
        let mut cfg = default_config();
        cfg.fast_period = 33; // not in {9, 21, 50, 200}
        let strategy = MaCrossStrategy::new(cfg);
        assert!(strategy.evaluate(&base_context()).is_none());
    }

    #[test]
    fn disabled_returns_none() {
        let mut cfg = default_config();
        cfg.enabled = false;
        let strategy = MaCrossStrategy::new(cfg);
        assert!(strategy.evaluate(&base_context()).is_none());
    }
}
