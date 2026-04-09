use scalper_core::types::{Side, Signal, VolatilityRegime};
use tracing::debug;

use crate::traits::{MarketContext, Strategy};

/// Per-strategy vote from the last ensemble evaluation.
#[derive(Debug, Clone)]
pub struct StrategyVote {
    pub name: String,
    pub fired: bool,
    pub side: Option<Side>,
    pub strength: f64,
}

/// Result of a detailed ensemble evaluation.
pub struct EvalResult {
    pub signal: Option<Signal>,
    pub votes: Vec<StrategyVote>,
}

/// Regime-adaptive ensemble strategy that combines signals from multiple
/// child strategies using weighted voting.
///
/// Regime-dependent weights:
/// - High volatility: Momentum 0.50, LiqWick 0.30, OB 0.10, Funding 0.10
/// - Normal: Momentum 0.40, OB 0.25, LiqWick 0.20, Funding 0.15
/// - Low/Ranging: OB 0.40, Momentum 0.20, LiqWick 0.25, Funding 0.15
/// - Extreme: All paused (circuit breaker handles this)
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

    /// Adjust strategy weight based on current volatility regime.
    fn regime_weight(&self, strategy_name: &str, regime: VolatilityRegime, base_weight: f64) -> f64 {
        match regime {
            VolatilityRegime::Volatile => match strategy_name {
                "momentum_breakout" => 0.50,
                "liquidation_wick" => 0.30,
                "ob_imbalance" => 0.10,
                "funding_bias" => 0.10,
                _ => base_weight,
            },
            VolatilityRegime::Normal => base_weight, // use configured weights
            VolatilityRegime::Ranging => match strategy_name {
                "ob_imbalance" => 0.40,
                "liquidation_wick" => 0.25,
                "momentum_breakout" => 0.20,
                "funding_bias" => 0.15,
                _ => base_weight,
            },
            VolatilityRegime::Extreme => 0.0, // pause everything
        }
    }

    pub fn evaluate(&self, ctx: &MarketContext) -> Option<Signal> {
        self.evaluate_detailed(ctx).signal
    }

    /// Evaluate all strategies and return both the ensemble signal and
    /// per-strategy vote details for dashboard display.
    pub fn evaluate_detailed(&self, ctx: &MarketContext) -> EvalResult {
        let mut votes: Vec<StrategyVote> = Vec::new();

        if ctx.volatility_regime == VolatilityRegime::Extreme {
            debug!("Ensemble: EXTREME volatility regime — all strategies paused");
            for strategy in &self.strategies {
                votes.push(StrategyVote {
                    name: strategy.name().to_string(),
                    fired: false,
                    side: None,
                    strength: 0.0,
                });
            }
            return EvalResult { signal: None, votes };
        }

        // Evaluate all strategies, capturing per-strategy votes
        let mut signals: Vec<(Signal, f64)> = Vec::new();
        for strategy in &self.strategies {
            let weight = self.regime_weight(
                strategy.name(),
                ctx.volatility_regime,
                strategy.weight(),
            );
            if weight <= 0.0 {
                votes.push(StrategyVote {
                    name: strategy.name().to_string(),
                    fired: false,
                    side: None,
                    strength: 0.0,
                });
                continue;
            }
            if let Some(signal) = strategy.evaluate(ctx) {
                votes.push(StrategyVote {
                    name: strategy.name().to_string(),
                    fired: true,
                    side: Some(signal.side),
                    strength: signal.strength,
                });
                signals.push((signal, weight));
            } else {
                votes.push(StrategyVote {
                    name: strategy.name().to_string(),
                    fired: false,
                    side: None,
                    strength: 0.0,
                });
            }
        }

        if signals.is_empty() {
            return EvalResult { signal: None, votes };
        }

        // Separate into buy and sell buckets
        let mut buy_signals: Vec<&(Signal, f64)> = Vec::new();
        let mut sell_signals: Vec<&(Signal, f64)> = Vec::new();

        for entry in &signals {
            match entry.0.side {
                Side::Buy => buy_signals.push(entry),
                Side::Sell => sell_signals.push(entry),
            }
        }

        // Choose majority direction
        let (chosen_signals, side) = if buy_signals.len() >= sell_signals.len() {
            (buy_signals, Side::Buy)
        } else {
            (sell_signals, Side::Sell)
        };

        // Require minimum 2 strategies agreeing — UNLESS a solo signal has
        // moderate-to-high conviction (>= 0.5 strength). With profit factor
        // running ~3, the marginal signals add to expected value even if
        // win rate dips slightly.
        let total_enabled = self.strategies.len();
        let solo_high_conviction = chosen_signals.len() == 1
            && chosen_signals[0].0.strength >= 0.5;
        if total_enabled > 1 && chosen_signals.len() < 2 && !solo_high_conviction {
            return EvalResult { signal: None, votes };
        }

        // Calculate weighted average strength
        let total_weight: f64 = chosen_signals.iter().map(|(_, w)| w).sum();
        if total_weight <= 0.0 {
            return EvalResult { signal: None, votes };
        }
        let weighted_strength: f64 = chosen_signals
            .iter()
            .map(|(s, w)| s.strength * w)
            .sum::<f64>()
            / total_weight;

        // Check threshold
        if weighted_strength < self.min_strength_threshold {
            debug!(
                "Ensemble: weighted_strength {:.3} below threshold {:.3}",
                weighted_strength, self.min_strength_threshold
            );
            return EvalResult { signal: None, votes };
        }

        // Use TP/SL from the highest-weight signal that has them
        let mut best_tp = None;
        let mut best_sl = None;
        let mut best_weight = 0.0_f64;

        for (signal, weight) in &chosen_signals {
            if *weight > best_weight {
                if signal.take_profit.is_some() {
                    best_tp = signal.take_profit;
                    best_sl = signal.stop_loss;
                    best_weight = *weight;
                }
            }
        }

        let best_signal = &chosen_signals[0].0;

        debug!(
            "Ensemble signal: {:?} {} strength={:.3} ({} strategies agree)",
            side, ctx.symbol, weighted_strength, chosen_signals.len()
        );

        let signal = Signal {
            strategy_name: "ensemble".to_string(),
            symbol: ctx.symbol.clone(),
            exchange: ctx.exchange,
            side,
            strength: weighted_strength,
            confidence: weighted_strength * 0.85,
            take_profit: best_tp.or(best_signal.take_profit),
            stop_loss: best_sl.or(best_signal.stop_loss),
            timestamp_ms: ctx.timestamp_ms,
        };

        EvalResult { signal: Some(signal), votes }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;
    use scalper_core::types::*;
    use crate::traits::MarketContext;

    struct MockStrategy {
        name: String,
        weight: f64,
        signal: Option<Signal>,
    }

    impl Strategy for MockStrategy {
        fn name(&self) -> &str { &self.name }
        fn evaluate(&self, _ctx: &MarketContext) -> Option<Signal> { self.signal.clone() }
        fn weight(&self) -> f64 { self.weight }
    }

    fn base_ctx() -> MarketContext {
        MarketContext {
            symbol: "BTCUSDT".into(),
            exchange: Exchange::Binance,
            last_price: dec!(50000),
            best_bid: dec!(49999),
            best_ask: dec!(50001),
            spread: dec!(2),
            tick_size: dec!(0.1),
            imbalance_ratio: 0.0,
            bid_depth_10: dec!(100),
            ask_depth_10: dec!(100),
            rsi_14: 50.0,
            ema_9: 50000.0,
            ema_21: 50000.0,
            ema_50: 50000.0,
            ema_200: 50000.0,
            macd_histogram: 0.0,
            bollinger_upper: 51000.0,
            bollinger_lower: 49000.0,
            bollinger_middle: 50000.0,
            vwap: 50000.0,
            atr_14: 200.0,
            obv: 0.0,
            cvd: 0.0,
            volume_ratio: 1.0,
            liquidation_volume_1m: 0.0,
            tf_5m_trend: Trend::Neutral,
            tf_15m_trend: Trend::Neutral,
            volatility_regime: VolatilityRegime::Normal,
            highest_high_60s: 50100.0,
            lowest_low_60s: 49900.0,
            avg_volume_60s: 100.0,
            current_volume: 100.0,
            funding_rate: 0.001,
            funding_rate_secondary: 0.001,
            open_interest: None,
            price_velocity_30s: 0.0,
            stoch_k: 50.0,
            stoch_d: 50.0,
            stoch_rsi: 50.0,
            cci_20: 0.0,
            adx_14: 20.0,
            psar: 49900.0,
            psar_long: true,
            supertrend: 49900.0,
            supertrend_up: true,
            donchian: Default::default(),
            timestamp_ms: 1000000,
        }
    }

    fn make_signal(side: Side, strength: f64) -> Signal {
        Signal {
            strategy_name: "test".into(),
            symbol: "BTCUSDT".into(),
            exchange: Exchange::Binance,
            side,
            strength,
            confidence: strength * 0.9,
            take_profit: Some(dec!(50250)),
            stop_loss: Some(dec!(49875)),
            timestamp_ms: 1000000,
        }
    }

    #[test]
    fn consensus_produces_signal() {
        let strategies: Vec<Box<dyn Strategy>> = vec![
            Box::new(MockStrategy { name: "momentum_breakout".into(), weight: 0.40, signal: Some(make_signal(Side::Buy, 0.7)) }),
            Box::new(MockStrategy { name: "ob_imbalance".into(), weight: 0.25, signal: Some(make_signal(Side::Buy, 0.5)) }),
        ];
        let ensemble = EnsembleStrategy::new(strategies, 0.20);
        let signal = ensemble.evaluate(&base_ctx());
        assert!(signal.is_some());
        assert_eq!(signal.unwrap().side, Side::Buy);
    }

    #[test]
    fn no_signal_without_consensus_low_strength() {
        // Low-strength solo signal: should be rejected (needs 2 strategies)
        let strategies: Vec<Box<dyn Strategy>> = vec![
            Box::new(MockStrategy { name: "a".into(), weight: 0.40, signal: Some(make_signal(Side::Buy, 0.3)) }),
            Box::new(MockStrategy { name: "b".into(), weight: 0.25, signal: None }),
        ];
        let ensemble = EnsembleStrategy::new(strategies, 0.20);
        let signal = ensemble.evaluate(&base_ctx());
        assert!(signal.is_none());
    }

    #[test]
    fn solo_high_conviction_passes() {
        // Solo signal with strength >= 0.5 should pass even without consensus
        let strategies: Vec<Box<dyn Strategy>> = vec![
            Box::new(MockStrategy { name: "a".into(), weight: 0.40, signal: Some(make_signal(Side::Buy, 0.8)) }),
            Box::new(MockStrategy { name: "b".into(), weight: 0.25, signal: None }),
        ];
        let ensemble = EnsembleStrategy::new(strategies, 0.20);
        let signal = ensemble.evaluate(&base_ctx());
        assert!(signal.is_some());
        assert_eq!(signal.unwrap().side, Side::Buy);
    }

    #[test]
    fn paused_in_extreme_regime() {
        let strategies: Vec<Box<dyn Strategy>> = vec![
            Box::new(MockStrategy { name: "a".into(), weight: 0.40, signal: Some(make_signal(Side::Buy, 0.9)) }),
            Box::new(MockStrategy { name: "b".into(), weight: 0.25, signal: Some(make_signal(Side::Buy, 0.8)) }),
        ];
        let ensemble = EnsembleStrategy::new(strategies, 0.20);
        let mut ctx = base_ctx();
        ctx.volatility_regime = VolatilityRegime::Extreme;
        let signal = ensemble.evaluate(&ctx);
        assert!(signal.is_none());
    }

    #[test]
    fn below_threshold_rejected() {
        let strategies: Vec<Box<dyn Strategy>> = vec![
            Box::new(MockStrategy { name: "a".into(), weight: 0.40, signal: Some(make_signal(Side::Buy, 0.1)) }),
            Box::new(MockStrategy { name: "b".into(), weight: 0.25, signal: Some(make_signal(Side::Buy, 0.05)) }),
        ];
        let ensemble = EnsembleStrategy::new(strategies, 0.20);
        let signal = ensemble.evaluate(&base_ctx());
        assert!(signal.is_none()); // weighted strength below 0.20
    }
}
