use bangida_core::config::ObImbalanceConfig;
use bangida_core::Signal;
use rust_decimal::prelude::*;
use rust_decimal::Decimal;
use tracing::debug;

use crate::signal::SignalExt;
use crate::traits::{MarketContext, Strategy};

/// Primary strategy (default weight 0.50). Trades order-book imbalance confirmed
/// by cumulative volume delta (CVD).
pub struct ObImbalanceStrategy {
    pub imbalance_threshold: f64,
    pub take_profit_ticks: u32,
    pub stop_loss_ticks: u32,
    pub max_spread_multiple: f64,
    pub weight: f64,
}

impl ObImbalanceStrategy {
    pub fn new(cfg: &ObImbalanceConfig) -> Self {
        Self {
            imbalance_threshold: cfg.imbalance_threshold,
            take_profit_ticks: cfg.take_profit_ticks,
            stop_loss_ticks: cfg.stop_loss_ticks,
            max_spread_multiple: 2.0,
            weight: cfg.weight,
        }
    }

    /// Estimate a single tick size from the spread – fallback heuristic.
    /// In production this should come from exchange symbol info.
    fn tick_size(spread: Decimal) -> Decimal {
        // Assume tick ≈ spread / 2 as a rough lower bound.
        let two = Decimal::from(2);
        if spread > Decimal::ZERO {
            spread / two
        } else {
            Decimal::new(1, 2) // 0.01 fallback
        }
    }
}

impl Strategy for ObImbalanceStrategy {
    fn name(&self) -> &str {
        "ob_imbalance"
    }

    fn evaluate(&self, ctx: &MarketContext) -> Option<Signal> {
        // --- Guard: spread too wide (> max_spread_multiple * tick) ---
        // Use avg spread heuristic: if spread > max_spread_multiple * tick_size → skip.
        let tick = Self::tick_size(ctx.spread);
        let max_spread = tick * Decimal::from_f64(self.max_spread_multiple).unwrap_or(Decimal::TWO);
        if ctx.spread > max_spread {
            debug!(
                spread = %ctx.spread,
                max = %max_spread,
                "ob_imbalance: spread too wide, skipping"
            );
            return None;
        }

        let imb = ctx.orderbook_imbalance;
        let threshold = self.imbalance_threshold;

        // --- Long signal ---
        if imb > threshold && ctx.cvd > 0.0 {
            let excess = (imb - threshold) / (1.0 - threshold); // normalise to 0..1
            let strength = excess.clamp(0.0, 1.0);

            let tp_offset = tick * Decimal::from(self.take_profit_ticks);
            let sl_offset = tick * Decimal::from(self.stop_loss_ticks);
            let tp = ctx.mid_price + tp_offset;
            let sl = ctx.mid_price - sl_offset;

            debug!(
                imbalance = imb,
                cvd = ctx.cvd,
                strength,
                "ob_imbalance: LONG signal"
            );
            return Some(Signal::buy(ctx.symbol.clone(), strength, "ob_imbalance").with_targets(tp, sl));
        }

        // --- Short signal ---
        if imb < -threshold && ctx.cvd < 0.0 {
            let excess = (-imb - threshold) / (1.0 - threshold);
            let strength = excess.clamp(0.0, 1.0);

            let tp_offset = tick * Decimal::from(self.take_profit_ticks);
            let sl_offset = tick * Decimal::from(self.stop_loss_ticks);
            let tp = ctx.mid_price - tp_offset;
            let sl = ctx.mid_price + sl_offset;

            debug!(
                imbalance = imb,
                cvd = ctx.cvd,
                strength,
                "ob_imbalance: SHORT signal"
            );
            return Some(Signal::sell(ctx.symbol.clone(), strength, "ob_imbalance").with_targets(tp, sl));
        }

        None
    }

    fn weight(&self) -> f64 {
        self.weight
    }
}
