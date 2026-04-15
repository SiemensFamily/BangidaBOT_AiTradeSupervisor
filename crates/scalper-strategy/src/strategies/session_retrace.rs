use scalper_core::config::SessionRetraceConfig;
use scalper_core::types::{Side, Signal, VolatilityRegime};
use crate::traits::{MarketContext, Strategy};

pub struct SessionBasedRetraceStrategy {
    config: SessionRetraceConfig,
}

impl SessionBasedRetraceStrategy {
    pub fn new(config: SessionRetraceConfig) -> Self {
        Self { config }
    }
}

impl Strategy for SessionBasedRetraceStrategy {
    fn name(&self) -> &str {
        "session_retrace"
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

        // Placeholder — in full version use timestamp to detect NY/London session retraces
        let strength = 0.65;

        if strength < 0.5 {
            return None;
        }

        let side = Side::Buy; // typical retrace direction after session high/low

        Some(Signal {
            strategy_name: self.name().to_string(),
            symbol: ctx.symbol.clone(),
            exchange: ctx.exchange,
            side,
            strength,
            confidence: strength * 0.8,
            take_profit: None,
            stop_loss: None,
            timestamp_ms: ctx.timestamp_ms,
        })
    }
}
