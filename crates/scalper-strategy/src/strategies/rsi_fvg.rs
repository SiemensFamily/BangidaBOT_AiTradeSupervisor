use scalper_core::config::RsiFvgConfig;
use scalper_core::types::{Side, Signal, VolatilityRegime};
use tracing::debug;
use crate::traits::{MarketContext, Strategy};

pub struct RsiFvgStrategy {
    config: RsiFvgConfig,
}

impl RsiFvgStrategy {
    pub fn new(config: RsiFvgConfig) -> Self {
        Self { config }
    }
}

impl Strategy for RsiFvgStrategy {
    fn name(&self) -> &str {
        "rsi_fvg"
    }

    fn weight(&self) -> f64 {
        self.config.weight
    }

    fn evaluate(&self, ctx: &MarketContext) -> Option<Signal> {
        if !self.config.enabled {
            return None;
        }
        if ctx.volatility_regime == VolatilityRegime::Extreme {
            debug!("rsi_fvg: skip — Extreme regime");
            return None;
        }

        let rsi = ctx.rsi_14;

        let (strength, side) = if rsi < self.config.rsi_oversold {
            (0.78, Side::Buy)
        } else if rsi > self.config.rsi_overbought {
            (0.75, Side::Sell)
        } else {
            debug!("rsi_fvg: skip — RSI {:.1} in neutral zone ({:.0}..{:.0})",
                   rsi, self.config.rsi_oversold, self.config.rsi_overbought);
            return None;
        };

        debug!("rsi_fvg: FIRE {:?} strength={:.3} rsi={:.1}",
               side, strength, rsi);

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
