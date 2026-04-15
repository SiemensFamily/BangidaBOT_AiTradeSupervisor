use scalper_core::config::CvdDivergenceConfig;
use scalper_core::types::{Side, Signal, VolatilityRegime};
use tracing::debug;
use crate::traits::{MarketContext, Strategy};

pub struct CvdDivergenceStrategy {
    config: CvdDivergenceConfig,
}

impl CvdDivergenceStrategy {
    pub fn new(config: CvdDivergenceConfig) -> Self {
        Self { config }
    }
}

impl Strategy for CvdDivergenceStrategy {
    fn name(&self) -> &str {
        "cvd_divergence"
    }

    fn weight(&self) -> f64 {
        self.config.weight
    }

    fn evaluate(&self, ctx: &MarketContext) -> Option<Signal> {
        if !self.config.enabled {
            return None;
        }
        if ctx.volatility_regime == VolatilityRegime::Extreme {
            debug!("cvd_divergence: skip — Extreme regime");
            return None;
        }

        let cvd = ctx.cvd;
        let strength = if cvd > 0.0 { 0.78 } else { 0.65 };

        if strength < self.config.min_divergence_strength {
            debug!("cvd_divergence: skip — strength {:.3} < min {:.3} (cvd={:.2})",
                   strength, self.config.min_divergence_strength, cvd);
            return None;
        }

        let side = if cvd > 0.0 { Side::Buy } else { Side::Sell };

        debug!("cvd_divergence: FIRE {:?} strength={:.3} cvd={:.2}",
               side, strength, cvd);

        Some(Signal {
            strategy_name: self.name().to_string(),
            symbol: ctx.symbol.clone(),
            exchange: ctx.exchange,
            side,
            strength,
            confidence: strength * 0.85,
            take_profit: None,
            stop_loss: None,
            timestamp_ms: ctx.timestamp_ms,
        })
    }
}
