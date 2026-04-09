//! Mean-Reversion Strategy — an anti-trend counterpart to `momentum_breakout`.
//!
//! Thesis: in ranging/normal-volatility regimes, price that has poked
//! meaningfully outside the Bollinger Bands while RSI is at an extreme
//! tends to snap back toward the mid-band. We fade the extreme.
//!
//! Entry (LONG):
//!   - Close is below BB lower band by at least `bb_penetration` * band-width
//!   - RSI(14) is below `rsi_oversold`
//!   - ADX(14) is BELOW `max_adx` (we want ranging, not trending — MR fails
//!     in strong trends because the range keeps expanding against you)
//!   - Volatility regime is Ranging or Normal (skip Volatile/Extreme)
//!
//! Entry (SHORT): mirrored.
//!
//! Exits are sized from ATR rather than percent, so targets adapt to the
//! volatility of the symbol + timeframe. `atr_tp_multiplier` is usually
//! set larger than `atr_sl_multiplier` to give trades room — mean reversion
//! typically has a *high* win rate with *small* wins and occasional
//! *larger* losses, so the RR ratio needs to respect that.
//!
//! This strategy is intentionally picky: it should fire far less often
//! than momentum, and when it does, the prior should be strong.

use rust_decimal::Decimal;
use scalper_core::config::MeanReversionConfig;
use scalper_core::types::{Side, Signal, VolatilityRegime};
use tracing::debug;

use crate::traits::{MarketContext, Strategy};

pub struct MeanReversionStrategy {
    config: MeanReversionConfig,
}

impl MeanReversionStrategy {
    pub fn new(config: MeanReversionConfig) -> Self {
        Self { config }
    }

    fn check_long(&self, ctx: &MarketContext) -> Option<(f64, Decimal, Decimal)> {
        let price = decimal_to_f64(ctx.last_price);

        // Bollinger bands must be populated (non-zero and not collapsed)
        let band_width = ctx.bollinger_upper - ctx.bollinger_lower;
        if band_width <= 1e-9 {
            return None;
        }

        // Penetration: how far below BB lower, measured as a fraction of
        // the band width. 0.0 means "just touching", 0.1 means "10% of
        // band-width below the lower band".
        let penetration = (ctx.bollinger_lower - price) / band_width;
        if penetration < self.config.bb_penetration {
            return None;
        }

        // RSI oversold
        if ctx.rsi_14 >= self.config.rsi_oversold {
            return None;
        }

        // ADX filter: only trade in ranging markets
        if ctx.adx_14 >= self.config.max_adx {
            return None;
        }

        // Volatility regime gate: skip volatile/extreme
        if matches!(
            ctx.volatility_regime,
            VolatilityRegime::Volatile | VolatilityRegime::Extreme
        ) {
            return None;
        }

        // ATR must be sane
        if ctx.atr_14 <= 0.0 {
            return None;
        }

        // Strength: how deep below the band + how oversold RSI is
        let rsi_component = ((self.config.rsi_oversold - ctx.rsi_14) / self.config.rsi_oversold)
            .clamp(0.0, 1.0);
        let bb_component = (penetration / (self.config.bb_penetration.max(0.01) * 4.0))
            .clamp(0.0, 1.0);
        let strength = (rsi_component * 0.5 + bb_component * 0.5).clamp(0.0, 1.0);

        // ATR-sized exits
        let tp_price = price + ctx.atr_14 * self.config.atr_tp_multiplier;
        let sl_price = price - ctx.atr_14 * self.config.atr_sl_multiplier;

        let tp = Decimal::from_f64_retain(tp_price)?;
        let sl = Decimal::from_f64_retain(sl_price)?;

        Some((strength, tp, sl))
    }

    fn check_short(&self, ctx: &MarketContext) -> Option<(f64, Decimal, Decimal)> {
        let price = decimal_to_f64(ctx.last_price);

        let band_width = ctx.bollinger_upper - ctx.bollinger_lower;
        if band_width <= 1e-9 {
            return None;
        }

        // Penetration: how far above BB upper, as a fraction of band width
        let penetration = (price - ctx.bollinger_upper) / band_width;
        if penetration < self.config.bb_penetration {
            return None;
        }

        if ctx.rsi_14 <= self.config.rsi_overbought {
            return None;
        }

        if ctx.adx_14 >= self.config.max_adx {
            return None;
        }

        if matches!(
            ctx.volatility_regime,
            VolatilityRegime::Volatile | VolatilityRegime::Extreme
        ) {
            return None;
        }

        if ctx.atr_14 <= 0.0 {
            return None;
        }

        let rsi_component = ((ctx.rsi_14 - self.config.rsi_overbought)
            / (100.0 - self.config.rsi_overbought).max(1.0))
        .clamp(0.0, 1.0);
        let bb_component = (penetration / (self.config.bb_penetration.max(0.01) * 4.0))
            .clamp(0.0, 1.0);
        let strength = (rsi_component * 0.5 + bb_component * 0.5).clamp(0.0, 1.0);

        let tp_price = price - ctx.atr_14 * self.config.atr_tp_multiplier;
        let sl_price = price + ctx.atr_14 * self.config.atr_sl_multiplier;

        let tp = Decimal::from_f64_retain(tp_price)?;
        let sl = Decimal::from_f64_retain(sl_price)?;

        Some((strength, tp, sl))
    }
}

impl Strategy for MeanReversionStrategy {
    fn name(&self) -> &str {
        "mean_reversion"
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
            "MeanReversion signal: {:?} {} strength={:.2} tp={} sl={}",
            side, ctx.symbol, strength, tp, sl
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

    fn default_config() -> MeanReversionConfig {
        MeanReversionConfig {
            enabled: true,
            weight: 0.20,
            rsi_oversold: 30.0,
            rsi_overbought: 70.0,
            bb_penetration: 0.05,
            atr_tp_multiplier: 1.5,
            atr_sl_multiplier: 1.0,
            max_adx: 25.0,
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
            rsi_14: 50.0,
            ema_9: 50000.0,
            ema_21: 50000.0,
            ema_50: 50000.0,
            ema_200: 50000.0,
            macd_histogram: 0.0,
            bollinger_upper: 50200.0,
            bollinger_lower: 49800.0,
            bollinger_middle: 50000.0,
            vwap: 50000.0,
            atr_14: 50.0,
            obv: 0.0,
            stoch_k: 50.0,
            stoch_d: 50.0,
            stoch_rsi: 50.0,
            cci_20: 0.0,
            adx_14: 15.0, // ranging
            psar: 0.0,
            psar_long: true,
            supertrend: 0.0,
            supertrend_up: true,
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
            funding_rate: 0.0,
            funding_rate_secondary: 0.0,
            open_interest: None,
            price_velocity_30s: 0.0,
            donchian: Default::default(),
            timestamp_ms: 1000000,
        }
    }

    #[test]
    fn fires_long_on_oversold_bb_break() {
        let strategy = MeanReversionStrategy::new(default_config());
        let mut ctx = base_context();
        // Price poke below lower band by 20% of band-width (400 wide → 80 below)
        ctx.last_price = dec!(49700);
        ctx.rsi_14 = 20.0;
        let signal = strategy.evaluate(&ctx).expect("should fire long");
        assert_eq!(signal.side, Side::Buy);
        assert!(signal.strength > 0.0);
        assert!(signal.take_profit.is_some());
        assert!(signal.stop_loss.is_some());
    }

    #[test]
    fn fires_short_on_overbought_bb_break() {
        let strategy = MeanReversionStrategy::new(default_config());
        let mut ctx = base_context();
        ctx.last_price = dec!(50300);
        ctx.rsi_14 = 82.0;
        let signal = strategy.evaluate(&ctx).expect("should fire short");
        assert_eq!(signal.side, Side::Sell);
    }

    #[test]
    fn no_signal_when_rsi_not_extreme() {
        let strategy = MeanReversionStrategy::new(default_config());
        let mut ctx = base_context();
        ctx.last_price = dec!(49700);
        ctx.rsi_14 = 45.0; // not oversold
        assert!(strategy.evaluate(&ctx).is_none());
    }

    #[test]
    fn no_signal_in_strong_trend() {
        let strategy = MeanReversionStrategy::new(default_config());
        let mut ctx = base_context();
        ctx.last_price = dec!(49700);
        ctx.rsi_14 = 20.0;
        ctx.adx_14 = 40.0; // strong trend
        assert!(strategy.evaluate(&ctx).is_none());
    }

    #[test]
    fn no_signal_in_extreme_volatility() {
        let strategy = MeanReversionStrategy::new(default_config());
        let mut ctx = base_context();
        ctx.last_price = dec!(49700);
        ctx.rsi_14 = 20.0;
        ctx.volatility_regime = VolatilityRegime::Extreme;
        assert!(strategy.evaluate(&ctx).is_none());
    }

    #[test]
    fn disabled_returns_none() {
        let mut cfg = default_config();
        cfg.enabled = false;
        let strategy = MeanReversionStrategy::new(cfg);
        assert!(strategy.evaluate(&base_context()).is_none());
    }
}
