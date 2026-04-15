use scalper_core::config::EnsembleConfig;
use scalper_core::types::{Side, Signal, VolatilityRegime};
use scalper_risk::AiTradeSupervisor;
use tracing::debug;

use crate::traits::{MarketContext, Strategy};

#[derive(Debug, Clone)]
pub struct StrategyVote {
    pub name: String,
    pub fired: bool,
    pub side: Option<Side>,
    pub strength: f64,
}

pub struct EvalResult {
    pub signal: Option<Signal>,
    pub votes: Vec<StrategyVote>,
}

pub struct EnsembleStrategy {
    strategies: Vec<Box<dyn Strategy>>,
    supervisor: AiTradeSupervisor,
    min_atr_ratio: f64,
    min_consensus: u32,
}

impl EnsembleStrategy {
    pub fn new(strategies: Vec<Box<dyn Strategy>>, min_strength_threshold: f64) -> Self {
        Self {
            strategies,
            supervisor: AiTradeSupervisor::new(min_strength_threshold),
            min_atr_ratio: 0.0,
            min_consensus: 2,
        }
    }

    pub fn with_config(strategies: Vec<Box<dyn Strategy>>, cfg: &EnsembleConfig) -> Self {
        let threshold = if cfg.min_strength_threshold > 0.0 { cfg.min_strength_threshold } else { 0.40 };
        let consensus = if cfg.min_consensus > 0 { cfg.min_consensus } else { 2 };
        Self {
            strategies,
            supervisor: AiTradeSupervisor::new(threshold),
            min_atr_ratio: cfg.min_atr_ratio.max(0.0),
            min_consensus: consensus,
        }
    }

    fn regime_weight(&self, strategy_name: &str, regime: VolatilityRegime, base_weight: f64) -> f64 {
        match regime {
            VolatilityRegime::Volatile => match strategy_name {
                "momentum_breakout" => 0.50,
                "liquidation_wick" => 0.30,
                "ob_imbalance" => 0.10,
                "funding_bias" => 0.10,
                "supertrend_trailing" => 0.45,
                "ema_pullback" => 0.35,
                _ => base_weight,
            },
            VolatilityRegime::Normal => base_weight,
            VolatilityRegime::Ranging => match strategy_name {
                "ob_imbalance" => 0.40,
                "liquidation_wick" => 0.25,
                "momentum_breakout" => 0.20,
                "funding_bias" => 0.15,
                "supertrend_trailing" => 0.25,
                "ema_pullback" => 0.40,
                _ => base_weight,
            },
            VolatilityRegime::Extreme => 0.0,
        }
    }

    pub fn evaluate(&self, ctx: &MarketContext) -> Option<Signal> {
        if ctx.volatility_regime == VolatilityRegime::Extreme {
            debug!("Ensemble: EXTREME volatility regime — all strategies paused");
            return None;
        }

        let adjusted_threshold = self.supervisor.get_adjusted_min_strength();

        let mut signals: Vec<(Signal, f64)> = Vec::new();

        for strategy in &self.strategies {
            let base_weight = strategy.weight(); // assuming your strategies have .weight()
            let multiplier = self.supervisor.get_strategy_weight_multiplier(strategy.name());
            let dynamic_weight = base_weight * multiplier;
            let weight = self.regime_weight(strategy.name(), ctx.volatility_regime, dynamic_weight);

            if weight <= 0.0 {
                continue;
            }

            if let Some(signal) = strategy.evaluate(ctx) {
                signals.push((signal, weight));
            }
        }

        if signals.is_empty() {
            return None;
        }

        // Separate buy/sell
        let mut buy_signals: Vec<&(Signal, f64)> = Vec::new();
        let mut sell_signals: Vec<&(Signal, f64)> = Vec::new();

        for entry in &signals {
            match entry.0.side {
                Side::Buy => buy_signals.push(entry),
                Side::Sell => sell_signals.push(entry),
            }
        }

        let (chosen_signals, side) = if buy_signals.len() >= sell_signals.len() {
            (buy_signals, Side::Buy)
        } else {
            (sell_signals, Side::Sell)
        };

        // Consensus check (2 strategies minimum for small capital, allow high-conviction solo)
        let total_enabled = self.strategies.len();
        let solo_high_conviction = chosen_signals.len() == 1 && chosen_signals[0].0.strength >= 0.5;

        if total_enabled > 1 && chosen_signals.len() < 2 && !solo_high_conviction {
            debug!("Ensemble: insufficient agreement ({} < 2)", chosen_signals.len());
            return None;
        }

        let total_weight: f64 = chosen_signals.iter().map(|(_, w)| *w).sum();
        if total_weight <= 0.0 {
            return None;
        }

        let weighted_strength: f64 = chosen_signals
            .iter()
            .map(|(s, w)| s.strength * *w)
            .sum::<f64>() / total_weight;

        if weighted_strength < adjusted_threshold {
            debug!("Ensemble: weighted_strength {:.3} below adjusted threshold {:.3}", 
                   weighted_strength, adjusted_threshold);
            return None;
        }

        // Pick best TP/SL from strongest signal
        let mut best_tp = None;
        let mut best_sl = None;
        let mut best_weight = 0.0_f64;

        for (signal, weight) in &chosen_signals {
            if *weight > best_weight && signal.take_profit.is_some() {
                best_tp = signal.take_profit;
                best_sl = signal.stop_loss;
                best_weight = *weight;
            }
        }

        let best_signal = &chosen_signals[0].0;

        debug!(
            "Ensemble APPROVED: {:?} {} strength={:.3} ({} strategies agree)",
            side, ctx.symbol, weighted_strength, chosen_signals.len()
        );

        Some(Signal {
            strategy_name: "ensemble".to_string(),
            symbol: ctx.symbol.clone(),
            exchange: ctx.exchange,
            side,
            strength: weighted_strength,
            confidence: weighted_strength * 0.85,
            take_profit: best_tp.or(best_signal.take_profit),
            stop_loss: best_sl.or(best_signal.stop_loss),
            timestamp_ms: ctx.timestamp_ms,
        })
    }

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

        // Minimum expected-move filter: require ATR to be at least min_atr_ratio of price
        if self.min_atr_ratio > 0.0 {
            let price = {
                use std::str::FromStr;
                f64::from_str(&ctx.last_price.to_string()).unwrap_or(0.0)
            };
            if price > 0.0 && ctx.atr_14 > 0.0 {
                let atr_ratio = ctx.atr_14 / price;
                if atr_ratio < self.min_atr_ratio {
                    debug!("[{}] Ensemble: SKIP — ATR ratio {:.5} < min {:.5} (low volatility chop)",
                           ctx.symbol, atr_ratio, self.min_atr_ratio);
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
            }
        }

        let adjusted_threshold = self.supervisor.get_adjusted_min_strength();
        let mut signals: Vec<(Signal, f64)> = Vec::new();

        for strategy in &self.strategies {
            let base_weight = strategy.weight();
            let multiplier = self.supervisor.get_strategy_weight_multiplier(strategy.name());
            let dynamic_weight = base_weight * multiplier;
            let weight = self.regime_weight(strategy.name(), ctx.volatility_regime, dynamic_weight);

            if weight <= 0.0 {
                debug!("[{}] {} weight=0 (regime={:?}), skipped",
                       ctx.symbol, strategy.name(), ctx.volatility_regime);
                votes.push(StrategyVote {
                    name: strategy.name().to_string(),
                    fired: false,
                    side: None,
                    strength: 0.0,
                });
                continue;
            }

            let result = strategy.evaluate(ctx);
            match &result {
                Some(signal) => {
                    debug!("[{}] {} FIRED {:?} strength={:.3} weight={:.3}",
                           ctx.symbol, strategy.name(), signal.side, signal.strength, weight);
                    votes.push(StrategyVote {
                        name: strategy.name().to_string(),
                        fired: true,
                        side: Some(signal.side),
                        strength: signal.strength,
                    });
                    signals.push((signal.clone(), weight));
                }
                None => {
                    debug!("[{}] {} no signal (weight={:.3})",
                           ctx.symbol, strategy.name(), weight);
                    votes.push(StrategyVote {
                        name: strategy.name().to_string(),
                        fired: false,
                        side: None,
                        strength: 0.0,
                    });
                }
            }
        }

        if signals.is_empty() {
            debug!("[{}] Ensemble: no strategies fired", ctx.symbol);
            return EvalResult { signal: None, votes };
        }

        let mut buy_signals: Vec<&(Signal, f64)> = Vec::new();
        let mut sell_signals: Vec<&(Signal, f64)> = Vec::new();

        for entry in &signals {
            match entry.0.side {
                Side::Buy => buy_signals.push(entry),
                Side::Sell => sell_signals.push(entry),
            }
        }

        let n_buy = buy_signals.len();
        let n_sell = sell_signals.len();

        let (chosen_signals, side) = if n_buy >= n_sell {
            (buy_signals, Side::Buy)
        } else {
            (sell_signals, Side::Sell)
        };

        debug!("[{}] Ensemble: {} fired, buy={} sell={}, choosing {:?}",
               ctx.symbol, signals.len(), n_buy, n_sell, side);

        let total_enabled = self.strategies.len();
        let need = self.min_consensus as usize;
        let solo_high_conviction = chosen_signals.len() == 1 && chosen_signals[0].0.strength >= 0.5;

        if total_enabled > 1 && chosen_signals.len() < need && !solo_high_conviction {
            debug!("[{}] Ensemble: REJECTED — only {} agree on {:?} (need {}, {} total enabled)",
                   ctx.symbol, chosen_signals.len(), side, need, total_enabled);
            return EvalResult { signal: None, votes };
        }

        let total_weight: f64 = chosen_signals.iter().map(|(_, w)| *w).sum();
        if total_weight <= 0.0 {
            return EvalResult { signal: None, votes };
        }

        let weighted_strength: f64 = chosen_signals
            .iter()
            .map(|(s, w)| s.strength * *w)
            .sum::<f64>() / total_weight;

        if weighted_strength < adjusted_threshold {
            debug!("[{}] Ensemble: REJECTED — strength {:.3} < threshold {:.3} ({} agreeing)",
                   ctx.symbol, weighted_strength, adjusted_threshold, chosen_signals.len());
            return EvalResult { signal: None, votes };
        }

        let mut best_tp = None;
        let mut best_sl = None;
        let mut best_weight = 0.0_f64;

        for (signal, weight) in &chosen_signals {
            if *weight > best_weight && signal.take_profit.is_some() {
                best_tp = signal.take_profit;
                best_sl = signal.stop_loss;
                best_weight = *weight;
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