use bangida_core::config::FundingBiasConfig;
use bangida_core::Signal;
use tracing::debug;

use crate::signal::SignalExt;
use crate::traits::{MarketContext, Strategy};

/// Funding-rate bias (default weight 0.05).
///
/// Not a standalone signal generator.  It only fires when funding is extreme,
/// adding a directional nudge to the ensemble:
///
/// - Funding > threshold  → bias toward shorts (+0.1 strength).
/// - Funding < -threshold → bias toward longs  (+0.1 strength).
pub struct FundingArbStrategy {
    pub high_threshold: f64,
    pub bias_strength: f64,
    pub weight: f64,
}

impl FundingArbStrategy {
    pub fn new(cfg: &FundingBiasConfig) -> Self {
        Self {
            high_threshold: cfg.high_funding_threshold,
            bias_strength: 0.1,
            weight: cfg.weight,
        }
    }
}

impl Strategy for FundingArbStrategy {
    fn name(&self) -> &str {
        "funding_arb"
    }

    fn evaluate(&self, ctx: &MarketContext) -> Option<Signal> {
        if ctx.funding_rate > self.high_threshold {
            debug!(
                funding = ctx.funding_rate,
                threshold = self.high_threshold,
                "funding_arb: extreme positive funding → SHORT bias"
            );
            return Some(Signal::sell(
                ctx.symbol.clone(),
                self.bias_strength,
                "funding_arb",
            ));
        }

        if ctx.funding_rate < -self.high_threshold {
            debug!(
                funding = ctx.funding_rate,
                threshold = self.high_threshold,
                "funding_arb: extreme negative funding → LONG bias"
            );
            return Some(Signal::buy(
                ctx.symbol.clone(),
                self.bias_strength,
                "funding_arb",
            ));
        }

        None
    }

    fn weight(&self) -> f64 {
        self.weight
    }
}
