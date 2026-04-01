use bangida_core::{Side, Signal};
use bangida_core::time::now_ms;
use tracing::{debug, info};

use crate::traits::{MarketContext, Strategy};

/// Weighted ensemble that aggregates signals from multiple sub-strategies.
///
/// # Decision logic
///
/// 1. Evaluate every sub-strategy and collect emitted signals.
/// 2. Partition into long vs short buckets.
/// 3. Pick the majority direction (must have >1 agreeing strategy).
/// 4. Compute the weighted-average strength for that direction.
/// 5. If combined weighted strength exceeds `min_strength_threshold`, emit a
///    composite signal whose TP/SL come from the highest-weight contributor.
pub struct EnsembleStrategy {
    strategies: Vec<Box<dyn Strategy>>,
    min_strength_threshold: f64,
}

impl EnsembleStrategy {
    pub fn new(strategies: Vec<Box<dyn Strategy>>, min_strength_threshold: f64) -> Self {
        Self {
            strategies,
            min_strength_threshold,
        }
    }

    /// Convenience builder: uses the default 0.15 threshold.
    pub fn with_defaults(strategies: Vec<Box<dyn Strategy>>) -> Self {
        Self::new(strategies, 0.15)
    }

    pub fn evaluate(&self, ctx: &MarketContext) -> Option<Signal> {
        let mut long_signals: Vec<(&str, f64, f64, Option<Signal>)> = Vec::new(); // (name, weight, strength, signal)
        let mut short_signals: Vec<(&str, f64, f64, Option<Signal>)> = Vec::new();

        for strat in &self.strategies {
            if let Some(sig) = strat.evaluate(ctx) {
                let entry = (strat.name(), strat.weight(), sig.strength, Some(sig));
                match entry.3.as_ref().unwrap().side {
                    Side::Buy => long_signals.push(entry),
                    Side::Sell => short_signals.push(entry),
                }
            }
        }

        // Choose direction with more agreeing strategies.
        let (chosen, direction) = if long_signals.len() > short_signals.len() {
            (long_signals, Side::Buy)
        } else if short_signals.len() > long_signals.len() {
            (short_signals, Side::Sell)
        } else if !long_signals.is_empty() {
            // Tie-break: pick the side with greater total weighted strength.
            let long_w: f64 = long_signals.iter().map(|(_, w, s, _)| w * s).sum();
            let short_w: f64 = short_signals.iter().map(|(_, w, s, _)| w * s).sum();
            if long_w >= short_w {
                (long_signals, Side::Buy)
            } else {
                (short_signals, Side::Sell)
            }
        } else {
            debug!("ensemble: no signals from any strategy");
            return None;
        };

        // Need at least 2 strategies agreeing (unless only 1 strategy is loaded).
        if chosen.len() < 2 && self.strategies.len() > 1 {
            debug!(
                direction = %direction,
                count = chosen.len(),
                "ensemble: insufficient agreement"
            );
            return None;
        }

        // Weighted average strength.
        let total_weight: f64 = chosen.iter().map(|(_, w, _, _)| w).sum();
        if total_weight <= 0.0 {
            return None;
        }
        let weighted_strength: f64 =
            chosen.iter().map(|(_, w, s, _)| w * s).sum::<f64>() / total_weight;

        if weighted_strength < self.min_strength_threshold {
            debug!(
                weighted_strength,
                threshold = self.min_strength_threshold,
                "ensemble: strength below threshold"
            );
            return None;
        }

        // Find primary signal (highest-weight contributor) for TP/SL.
        let primary = chosen
            .iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap();

        let primary_sig = primary.3.as_ref().unwrap();

        let contributors: Vec<&str> = chosen.iter().map(|(name, _, _, _)| *name).collect();
        let source = format!("ensemble[{}]", contributors.join("+"));

        info!(
            direction = %direction,
            weighted_strength,
            contributors = ?contributors,
            "ensemble: emitting signal"
        );

        Some(Signal {
            symbol: ctx.symbol.clone(),
            side: direction,
            strength: weighted_strength,
            confidence: weighted_strength,
            source,
            take_profit: primary_sig.take_profit,
            stop_loss: primary_sig.stop_loss,
            timestamp_ms: now_ms(),
        })
    }
}
