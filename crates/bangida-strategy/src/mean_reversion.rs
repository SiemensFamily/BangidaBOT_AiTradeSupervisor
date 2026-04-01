use bangida_core::Signal;
use rust_decimal::prelude::*;
use rust_decimal::Decimal;
use tracing::debug;

use crate::signal::SignalExt;
use crate::traits::{MarketContext, Strategy};

/// Mean-reversion strategy.
///
/// Enters when RSI reaches an extreme **and** price touches the corresponding
/// Bollinger Band.  Take-profit targets the middle band; stop-loss is placed
/// 1.5x the band-width beyond the entry side.
pub struct MeanReversionStrategy {
    pub rsi_oversold: f64,
    pub rsi_overbought: f64,
    pub weight: f64,
}

impl MeanReversionStrategy {
    pub fn new(rsi_oversold: f64, rsi_overbought: f64, weight: f64) -> Self {
        Self {
            rsi_oversold,
            rsi_overbought,
            weight,
        }
    }

    /// Convenience constructor with sensible defaults.
    pub fn default_config() -> Self {
        Self::new(25.0, 75.0, 0.0) // weight 0 – not in default ensemble
    }
}

impl Strategy for MeanReversionStrategy {
    fn name(&self) -> &str {
        "mean_reversion"
    }

    fn evaluate(&self, ctx: &MarketContext) -> Option<Signal> {
        let bb_upper = Decimal::from_f64(ctx.bb_upper)?;
        let bb_lower = Decimal::from_f64(ctx.bb_lower)?;
        let bb_middle = Decimal::from_f64(ctx.bb_middle)?;

        let band_width = bb_upper - bb_lower;
        if band_width <= Decimal::ZERO {
            return None;
        }

        // --- Long: RSI oversold AND price at or below lower band ---
        if ctx.rsi < self.rsi_oversold && ctx.last_price <= bb_lower {
            let distance = bb_middle - ctx.last_price;
            let strength = (distance / band_width)
                .to_f64()
                .unwrap_or(0.0)
                .clamp(0.0, 1.0);

            let tp = bb_middle;
            let sl_offset = band_width * Decimal::new(15, 1); // 1.5x
            let sl = ctx.last_price - sl_offset;

            debug!(
                rsi = ctx.rsi,
                price = %ctx.last_price,
                bb_lower = %bb_lower,
                strength,
                "mean_reversion: LONG signal"
            );
            return Some(
                Signal::buy(ctx.symbol.clone(), strength, "mean_reversion").with_targets(tp, sl),
            );
        }

        // --- Short: RSI overbought AND price at or above upper band ---
        if ctx.rsi > self.rsi_overbought && ctx.last_price >= bb_upper {
            let distance = ctx.last_price - bb_middle;
            let strength = (distance / band_width)
                .to_f64()
                .unwrap_or(0.0)
                .clamp(0.0, 1.0);

            let tp = bb_middle;
            let sl_offset = band_width * Decimal::new(15, 1); // 1.5x
            let sl = ctx.last_price + sl_offset;

            debug!(
                rsi = ctx.rsi,
                price = %ctx.last_price,
                bb_upper = %bb_upper,
                strength,
                "mean_reversion: SHORT signal"
            );
            return Some(
                Signal::sell(ctx.symbol.clone(), strength, "mean_reversion").with_targets(tp, sl),
            );
        }

        None
    }

    fn weight(&self) -> f64 {
        self.weight
    }
}
