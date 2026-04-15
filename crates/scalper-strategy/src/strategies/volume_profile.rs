use scalper_core::config::VolumeProfileConfig;
use scalper_core::types::{Side, Signal, VolatilityRegime};
use crate::traits::{MarketContext, Strategy};

pub struct VolumeProfileStrategy {
    config: VolumeProfileConfig,
}

impl VolumeProfileStrategy {
    pub fn new(config: VolumeProfileConfig) -> Self {
        Self { config }
    }
}

impl Strategy for VolumeProfileStrategy {
    fn name(&self) -> &str {
        "volume_profile"
    }

    fn weight(&self) -> f64 {
        self.config.weight
    }

    fn evaluate(&self, ctx: &MarketContext) -> Option<Signal> {
        if !self.config.enabled {
            return None;
        }
        if ctx.volatility_regime == VolatilityRegime::Extreme {
            return None;
        }

        // Placeholder logic — in full version use POC / Value Area from order flow or candles
        let strength = 0.68;

        if strength < 0.55 {
            return None;
        }

        // Mean reversion at value area
        let side = Side::Buy; // adjust with real POC logic later

        Some(Signal {
            strategy_name: self.name().to_string(),
            symbol: ctx.symbol.clone(),
            exchange: ctx.exchange,
            side,
            strength,
            confidence: strength * 0.82,
            take_profit: None,
            stop_loss: None,
            timestamp_ms: ctx.timestamp_ms,
        })
    }
}
