use bangida_core::config::MomentumConfig;
use bangida_core::Signal;
use rust_decimal::prelude::*;
use rust_decimal::Decimal;
use tracing::debug;

use crate::signal::SignalExt;
use crate::traits::{MarketContext, Strategy};

/// Momentum breakout scalping (default weight 0.20).
///
/// Enters when price breaks above the 60-second high (long) or below the
/// 60-second low (short), confirmed by a volume spike and an RSI filter to
/// avoid chasing overbought / oversold conditions.
pub struct MomentumStrategy {
    pub volume_spike_multiplier: f64,
    pub take_profit_pct: f64,
    pub trailing_stop_pct: f64,
    pub rsi_overbought: f64,
    pub rsi_oversold: f64,
    pub weight: f64,
}

impl MomentumStrategy {
    pub fn new(cfg: &MomentumConfig) -> Self {
        Self {
            volume_spike_multiplier: cfg.volume_spike_multiplier,
            take_profit_pct: cfg.take_profit_pct,
            trailing_stop_pct: cfg.trailing_stop_pct,
            rsi_overbought: 80.0,
            rsi_oversold: 20.0,
            weight: cfg.weight,
        }
    }

    fn pct_offset(price: Decimal, pct: f64) -> Decimal {
        let factor = Decimal::from_f64(pct / 100.0).unwrap_or(Decimal::ZERO);
        price * factor
    }
}

impl Strategy for MomentumStrategy {
    fn name(&self) -> &str {
        "momentum"
    }

    fn evaluate(&self, ctx: &MarketContext) -> Option<Signal> {
        // Need meaningful average volume to compare against.
        if ctx.avg_volume_60s <= 0.0 {
            return None;
        }

        let volume_ratio = ctx.volume_1s / ctx.avg_volume_60s;
        let volume_spike = volume_ratio >= self.volume_spike_multiplier;

        // --- Upward breakout ---
        if ctx.last_price > ctx.highest_high_60s && volume_spike && ctx.rsi < self.rsi_overbought {
            let strength = (volume_ratio / self.volume_spike_multiplier).min(2.0) / 2.0; // 0..1

            let tp = ctx.last_price + Self::pct_offset(ctx.last_price, self.take_profit_pct);
            let sl = ctx.last_price - Self::pct_offset(ctx.last_price, self.trailing_stop_pct);

            debug!(
                price = %ctx.last_price,
                high_60 = %ctx.highest_high_60s,
                volume_ratio,
                rsi = ctx.rsi,
                strength,
                "momentum: LONG breakout"
            );
            return Some(
                Signal::buy(ctx.symbol.clone(), strength, "momentum").with_targets(tp, sl),
            );
        }

        // --- Downward breakout ---
        if ctx.last_price < ctx.lowest_low_60s && volume_spike && ctx.rsi > self.rsi_oversold {
            let strength = (volume_ratio / self.volume_spike_multiplier).min(2.0) / 2.0;

            let tp = ctx.last_price - Self::pct_offset(ctx.last_price, self.take_profit_pct);
            let sl = ctx.last_price + Self::pct_offset(ctx.last_price, self.trailing_stop_pct);

            debug!(
                price = %ctx.last_price,
                low_60 = %ctx.lowest_low_60s,
                volume_ratio,
                rsi = ctx.rsi,
                strength,
                "momentum: SHORT breakout"
            );
            return Some(
                Signal::sell(ctx.symbol.clone(), strength, "momentum").with_targets(tp, sl),
            );
        }

        None
    }

    fn weight(&self) -> f64 {
        self.weight
    }
}
