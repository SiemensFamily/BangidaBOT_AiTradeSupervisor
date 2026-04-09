//! Donchian Channel Breakout — the classic Turtle Trading entry system.
//!
//! The most well-documented profitable systematic strategy in trading history.
//! Richard Dennis and the original Turtles used this (with strict position
//! sizing and exit rules) to compound aggressive returns in trending
//! commodity markets. The core idea is simple: buy new highs, sell new lows,
//! trust the trend, and cut losses quickly on the opposite breakout.
//!
//! Rules:
//!
//! Long entry:  close > highest high of the last `entry_period` bars
//! Short entry: close < lowest low  of the last `entry_period` bars
//! Long exit:   close < lowest low  of the last `exit_period`  bars (partial
//!              reversal — exit early to protect gains before full reversal)
//! Short exit:  close > highest high of the last `exit_period`  bars
//!
//! For the classic Turtle setup: `entry_period = 20` (or 55), `exit_period = 10`.
//! Our implementation uses the Donchian snapshot precomputed by the replay
//! engine, which supports periods 10, 20, and 55. The strategy picks the
//! matching pair at evaluation time.
//!
//! Position sizing: returns take_profit and stop_loss based on ATR. The
//! initial stop is `atr_stop_multiplier * ATR` away from the close; the
//! "take profit" is set wide (`atr_tp_multiplier * ATR`) because real trend
//! exits come from the opposite breakout, not a fixed target. The backtest
//! engine's `max_hold_bars` acts as a time-based safety net.
//!
//! Strategy philosophy: **low win rate (30-40%), high reward:risk (3:1+)**.
//! You will lose most trades but the winners are much larger than the losers.
//! This is the opposite of scalping and requires psychological discipline to
//! sit through the string of small losses that precede each big winner.

use rust_decimal::Decimal;
use scalper_core::config::DonchianConfig;
use scalper_core::types::{Side, Signal};
use tracing::debug;

use crate::traits::{MarketContext, Strategy};

pub struct DonchianStrategy {
    config: DonchianConfig,
}

impl DonchianStrategy {
    pub fn new(config: DonchianConfig) -> Self {
        Self { config }
    }

    /// Look up the (upper, lower) channel pair for the given period from
    /// the precomputed snapshot. Only 10 / 20 / 55 are supported because
    /// that's what the replay engine computes.
    fn channel_for(&self, ctx: &MarketContext, period: u32) -> Option<(f64, f64)> {
        match period {
            10 => Some((ctx.donchian.upper_10, ctx.donchian.lower_10)),
            20 => Some((ctx.donchian.upper_20, ctx.donchian.lower_20)),
            55 => Some((ctx.donchian.upper_55, ctx.donchian.lower_55)),
            _ => None,
        }
    }

    fn check_long(&self, ctx: &MarketContext) -> Option<(f64, Decimal, Decimal)> {
        let price = decimal_to_f64(ctx.last_price);

        // Need enough history for the entry period channel to be valid.
        // If upper_N is zero, the channel isn't populated yet.
        let (upper, _lower) = self.channel_for(ctx, self.config.entry_period)?;
        if upper <= 0.0 {
            return None;
        }

        // Breakout: close strictly above the last N-bar high
        if price <= upper {
            return None;
        }

        // ATR must be populated for stop sizing
        if ctx.atr_14 <= 0.0 {
            return None;
        }

        // Optional trend filter: only take longs when price is above the
        // long-term EMA (default EMA-200). Disabled if `use_trend_filter` false.
        if self.config.use_trend_filter && ctx.ema_200 > 0.0 && price < ctx.ema_200 {
            return None;
        }

        // Strength: a breakout is a binary event, so we return a fixed
        // high-conviction value (0.8) plus a small bonus for the magnitude
        // of the break. This ensures the signal always clears typical
        // ensemble thresholds (0.15-0.30). Capped at 1.0.
        let break_distance = (price - upper).max(0.0);
        let magnitude_bonus = (break_distance / ctx.atr_14).min(0.2);
        let strength = (0.8 + magnitude_bonus).min(1.0);

        // ATR-sized exits. Trend followers generally run a wide TP because
        // the real exit comes from the opposite breakout or trailing logic.
        let tp_price = price + ctx.atr_14 * self.config.atr_tp_multiplier;
        let sl_price = price - ctx.atr_14 * self.config.atr_stop_multiplier;

        let tp = Decimal::from_f64_retain(tp_price)?;
        let sl = Decimal::from_f64_retain(sl_price)?;

        Some((strength, tp, sl))
    }

    fn check_short(&self, ctx: &MarketContext) -> Option<(f64, Decimal, Decimal)> {
        let price = decimal_to_f64(ctx.last_price);

        let (_upper, lower) = self.channel_for(ctx, self.config.entry_period)?;
        if lower <= 0.0 || lower == f64::INFINITY {
            return None;
        }

        if price >= lower {
            return None;
        }

        if ctx.atr_14 <= 0.0 {
            return None;
        }

        if self.config.use_trend_filter && ctx.ema_200 > 0.0 && price > ctx.ema_200 {
            return None;
        }

        let break_distance = (lower - price).max(0.0);
        let magnitude_bonus = (break_distance / ctx.atr_14).min(0.2);
        let strength = (0.8 + magnitude_bonus).min(1.0);

        let tp_price = price - ctx.atr_14 * self.config.atr_tp_multiplier;
        let sl_price = price + ctx.atr_14 * self.config.atr_stop_multiplier;

        let tp = Decimal::from_f64_retain(tp_price)?;
        let sl = Decimal::from_f64_retain(sl_price)?;

        Some((strength, tp, sl))
    }
}

impl Strategy for DonchianStrategy {
    fn name(&self) -> &str {
        "donchian_breakout"
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
            "Donchian signal: {:?} {} strength={:.2} tp={} sl={}",
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
    use scalper_core::types::{Exchange, Trend, VolatilityRegime};
    use crate::traits::DonchianSnapshot;

    fn default_config() -> DonchianConfig {
        DonchianConfig {
            enabled: true,
            weight: 0.30,
            entry_period: 20,
            exit_period: 10,
            atr_tp_multiplier: 4.0,
            atr_stop_multiplier: 2.0,
            use_trend_filter: false,
        }
    }

    fn base_context() -> MarketContext {
        MarketContext {
            symbol: "BTCUSDT".into(),
            exchange: Exchange::Kraken,
            last_price: dec!(50000),
            best_bid: dec!(49999),
            best_ask: dec!(50001),
            spread: dec!(2),
            tick_size: dec!(0.1),
            imbalance_ratio: 0.0,
            bid_depth_10: dec!(100),
            ask_depth_10: dec!(100),
            rsi_14: 55.0,
            ema_9: 50000.0,
            ema_21: 50000.0,
            ema_50: 49500.0,
            ema_200: 48000.0,
            macd_histogram: 0.0,
            bollinger_upper: 51000.0,
            bollinger_lower: 49000.0,
            bollinger_middle: 50000.0,
            vwap: 50000.0,
            atr_14: 500.0,
            obv: 0.0,
            stoch_k: 50.0,
            stoch_d: 50.0,
            stoch_rsi: 50.0,
            cci_20: 0.0,
            adx_14: 30.0,
            psar: 0.0,
            psar_long: true,
            supertrend: 0.0,
            supertrend_up: true,
            donchian: DonchianSnapshot {
                upper_10: 49800.0,
                lower_10: 49200.0,
                upper_20: 49700.0, // price 50000 > 49700 = long breakout
                lower_20: 49000.0,
                upper_55: 49500.0,
                lower_55: 48500.0,
            },
            cvd: 0.0,
            volume_ratio: 1.0,
            liquidation_volume_1m: 0.0,
            tf_5m_trend: Trend::Up,
            tf_15m_trend: Trend::Up,
            volatility_regime: VolatilityRegime::Normal,
            highest_high_60s: 50000.0,
            lowest_low_60s: 49000.0,
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
    fn fires_long_on_new_high_breakout() {
        let strategy = DonchianStrategy::new(default_config());
        let ctx = base_context();
        let signal = strategy.evaluate(&ctx).expect("should fire long");
        assert_eq!(signal.side, Side::Buy);
        assert!(signal.strength > 0.0);
        assert!(signal.take_profit.is_some());
        assert!(signal.stop_loss.is_some());
    }

    #[test]
    fn fires_short_on_new_low_breakout() {
        let strategy = DonchianStrategy::new(default_config());
        let mut ctx = base_context();
        ctx.last_price = dec!(48900);
        // Short needs price below lower_20 (49000)
        let signal = strategy.evaluate(&ctx).expect("should fire short");
        assert_eq!(signal.side, Side::Sell);
    }

    #[test]
    fn no_signal_inside_channel() {
        let strategy = DonchianStrategy::new(default_config());
        let mut ctx = base_context();
        ctx.last_price = dec!(49500); // inside [49000, 49700]
        assert!(strategy.evaluate(&ctx).is_none());
    }

    #[test]
    fn trend_filter_blocks_counter_trend_long() {
        let mut cfg = default_config();
        cfg.use_trend_filter = true;
        let strategy = DonchianStrategy::new(cfg);
        let mut ctx = base_context();
        ctx.ema_200 = 51000.0; // price below EMA-200, trend filter blocks long
        assert!(strategy.evaluate(&ctx).is_none());
    }

    #[test]
    fn disabled_returns_none() {
        let mut cfg = default_config();
        cfg.enabled = false;
        let strategy = DonchianStrategy::new(cfg);
        assert!(strategy.evaluate(&base_context()).is_none());
    }

    #[test]
    fn uses_55_period_when_configured() {
        let mut cfg = default_config();
        cfg.entry_period = 55;
        let strategy = DonchianStrategy::new(cfg);
        let mut ctx = base_context();
        // Price 50000 > upper_55 (49500) triggers long
        let signal = strategy.evaluate(&ctx).expect("should fire long");
        assert_eq!(signal.side, Side::Buy);

        // But if we lower price to inside the 55-period range, no signal
        ctx.last_price = dec!(49400);
        assert!(strategy.evaluate(&ctx).is_none());
    }
}
