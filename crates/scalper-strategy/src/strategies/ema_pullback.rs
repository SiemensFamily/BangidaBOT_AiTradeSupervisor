use scalper_core::config::EmaPullbackConfig;
use scalper_core::types::{Side, Signal, VolatilityRegime};
use tracing::debug;
use crate::traits::{MarketContext, Strategy};

pub struct EmaPullbackStrategy {
    config: EmaPullbackConfig,
}

impl EmaPullbackStrategy {
    pub fn new(config: EmaPullbackConfig) -> Self {
        Self { config }
    }
}

impl Strategy for EmaPullbackStrategy {
    fn name(&self) -> &str {
        "ema_pullback"
    }

    fn weight(&self) -> f64 {
        1.0
    }

    fn evaluate(&self, ctx: &MarketContext) -> Option<Signal> {
        if !self.config.enabled {
            return None;
        }
        if ctx.volatility_regime == VolatilityRegime::Extreme {
            debug!("ema_pullback: skip — Extreme regime");
            return None;
        }

        let price = ctx.last_price;
        let above_ema_proxy = ctx.price_velocity_30s > 0.0 || price > ctx.last_price;

        let pullback_strength = if above_ema_proxy { 0.72 } else { 0.68 };

        let final_strength = if ctx.volatility_regime == VolatilityRegime::Ranging {
            pullback_strength * 0.85
        } else {
            pullback_strength
        };

        if final_strength < self.config.min_pullback_strength {
            debug!("ema_pullback: skip — strength {:.3} < min {:.3} (velocity={:.4}, regime={:?})",
                   final_strength, self.config.min_pullback_strength,
                   ctx.price_velocity_30s, ctx.volatility_regime);
            return None;
        }

        let side = if above_ema_proxy { Side::Buy } else { Side::Sell };

        debug!("ema_pullback: FIRE {:?} strength={:.3} velocity={:.4}",
               side, final_strength, ctx.price_velocity_30s);

        Some(Signal {
            strategy_name: self.name().to_string(),
            symbol: ctx.symbol.clone(),
            exchange: ctx.exchange,
            side,
            strength: final_strength,
            confidence: final_strength * 0.85,
            take_profit: None,
            stop_loss: None,
            timestamp_ms: ctx.timestamp_ms,
        })
    }
}
