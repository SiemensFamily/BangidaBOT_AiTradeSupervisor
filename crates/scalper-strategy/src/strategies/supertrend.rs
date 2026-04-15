use scalper_core::config::SupertrendConfig;
use scalper_core::types::{Side, Signal, VolatilityRegime};
use tracing::debug;
use crate::traits::{MarketContext, Strategy};

pub struct SupertrendTrailingStrategy {
    _config: SupertrendConfig,
}

impl SupertrendTrailingStrategy {
    pub fn new(config: SupertrendConfig) -> Self {
        Self { _config: config }
    }
}

impl Strategy for SupertrendTrailingStrategy {
    fn name(&self) -> &str {
        "supertrend_trailing"
    }

    fn weight(&self) -> f64 {
        1.0
    }

    fn evaluate(&self, ctx: &MarketContext) -> Option<Signal> {
        if ctx.volatility_regime == VolatilityRegime::Extreme {
            debug!("supertrend: skip — Extreme regime");
            return None;
        }

        let trend_up = ctx.supertrend_up;
        let strength_base = if trend_up { 0.82 } else { 0.78 };

        let regime_mult = match ctx.volatility_regime {
            VolatilityRegime::Volatile => 1.15,
            VolatilityRegime::Ranging => 0.85,
            _ => 1.0,
        };

        let mut final_strength = strength_base * regime_mult;
        if final_strength > 1.0 {
            final_strength = 1.0;
        }

        if final_strength < 0.55 {
            debug!("supertrend: skip — strength {:.3} < 0.55 (trend_up={}, regime={:?})",
                   final_strength, trend_up, ctx.volatility_regime);
            return None;
        }

        let side = if trend_up { Side::Buy } else { Side::Sell };

        debug!("supertrend: FIRE {:?} strength={:.3} trend_up={} regime={:?}",
               side, final_strength, trend_up, ctx.volatility_regime);

        Some(Signal {
            strategy_name: self.name().to_string(),
            symbol: ctx.symbol.clone(),
            exchange: ctx.exchange,
            side,
            strength: final_strength,
            confidence: final_strength * 0.88,
            take_profit: None,
            stop_loss: None,
            timestamp_ms: ctx.timestamp_ms,
        })
    }
}
