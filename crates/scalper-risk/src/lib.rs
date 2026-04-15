pub mod circuit_breaker;
pub mod position_sizer;
pub mod pnl_tracker;
pub mod risk_manager;

pub use circuit_breaker::CircuitBreaker;
pub use position_sizer::PositionSizer;
pub use pnl_tracker::PnlTracker;
pub use risk_manager::RiskManager;

use std::collections::HashMap;
use tracing::{info, debug};

// ─────────────────────────────────────────────────────────────
// Enhanced PerformanceTracker + AiTradeSupervisor
// ─────────────────────────────────────────────────────────────
#[derive(Default, Debug)]
pub struct PerformanceTracker {
    pub recent_outcomes: Vec<bool>,           // overall wins
    recent_pnls: Vec<f64>,                    // net pnl per trade (last 50)
    strategy_scores: HashMap<String, f64>,    // EMA profit factor per strategy (higher = better)
    trade_count: usize,
}

impl PerformanceTracker {
    pub fn record_trade(&mut self, strategy_name: &str, pnl: f64, fees: f64) {
        let net = pnl - fees;
        let was_win = net > 0.0;
        self.recent_outcomes.push(was_win);
        if self.recent_outcomes.len() > 50 {
            self.recent_outcomes.remove(0);
        }
        self.recent_pnls.push(net);
        if self.recent_pnls.len() > 50 {
            self.recent_pnls.remove(0);
        }

        self.trade_count += 1;

        // Simple EMA update for strategy score (higher = better)
        let alpha = 0.2; // learning rate
        let entry = self.strategy_scores.entry(strategy_name.to_string()).or_insert(0.0);
        *entry = alpha * net + (1.0 - alpha) * *entry;

        debug!("PerformanceTracker: {} | PnL={:.4} net={:.4} | score={:.3}",
               strategy_name, pnl, net, *entry);
    }

    pub fn get_win_rate(&self) -> f64 {
        if self.recent_outcomes.is_empty() {
            return 0.50;
        }
        let wins = self.recent_outcomes.iter().filter(|&&w| w).count() as f64;
        wins / self.recent_outcomes.len() as f64
    }

    /// Profit factor = sum of wins / abs(sum of losses). Returns 1.0 if no losses yet.
    pub fn get_profit_factor(&self) -> f64 {
        let (mut wins, mut losses) = (0.0f64, 0.0f64);
        for &p in &self.recent_pnls {
            if p > 0.0 { wins += p; } else { losses += -p; }
        }
        if losses <= 0.0 {
            if wins > 0.0 { 99.0 } else { 1.0 }
        } else {
            wins / losses
        }
    }

    pub fn sample_size(&self) -> usize {
        self.recent_outcomes.len()
    }

    pub fn get_strategy_score(&self, strategy_name: &str) -> f64 {
        *self.strategy_scores.get(strategy_name).unwrap_or(&0.0)
    }

    // For supervisor to decide weight multiplier
    pub fn get_weight_multiplier(&self, strategy_name: &str) -> f64 {
        let score = self.get_strategy_score(strategy_name);
        if score < -0.5 {
            0.4
        } else if score < 0.0 {
            0.7
        } else if score > 1.0 {
            1.3
        } else {
            1.0
        }
    }
}

// Simple AiTradeSupervisor (lightweight, no heavy ML)
#[derive(Default, Debug)]
pub struct AiTradeSupervisor {
    tracker: PerformanceTracker,
    min_strength_threshold: f64,
}

impl AiTradeSupervisor {
    pub fn new(min_strength_threshold: f64) -> Self {
        let sup = Self {
            tracker: PerformanceTracker::default(),
            min_strength_threshold,
        };
        info!("AiTradeSupervisor initialized with min_strength_threshold = {:.3}", min_strength_threshold);
        sup
    }

    pub fn record_trade(&mut self, strategy_name: &str, pnl: f64, fees: f64) {
        self.tracker.record_trade(strategy_name, pnl, fees);

        // Log learning decision
        if self.tracker.trade_count % 5 == 0 {
            let wr = self.tracker.get_win_rate();
            info!("AiTradeSupervisor: Trade #{} | WinRate={:.1}% | OB_score={:.3} | MinStrength={:.3}",
                  self.tracker.trade_count, wr * 100.0,
                  self.tracker.get_strategy_score("ob_imbalance"),
                  self.min_strength_threshold);
        }
    }

    pub fn get_adjusted_min_strength(&self) -> f64 {
        // Need at least 5 trades before adapting — don't react to noise
        if self.tracker.sample_size() < 5 {
            return self.min_strength_threshold;
        }

        let wr = self.tracker.get_win_rate();
        let pf = self.tracker.get_profit_factor();

        // Tier 1: bleeding hard (WR < 15%) — clamp down aggressively
        if wr < 0.15 {
            return (self.min_strength_threshold * 1.35).min(0.75);
        }

        // Tier 2: decent PF overrides mediocre win rate — relax
        if pf > 1.5 {
            return (self.min_strength_threshold * 0.85).max(0.35);
        }

        // Tier 3: moderate losing zone (WR < 20%) — raise a bit
        if wr < 0.20 {
            return (self.min_strength_threshold * 1.25).min(0.70);
        }

        // Tier 4: strong win rate — relax
        if wr > 0.40 {
            return (self.min_strength_threshold * 0.85).max(0.35);
        }

        self.min_strength_threshold
    }

    pub fn get_strategy_weight_multiplier(&self, strategy_name: &str) -> f64 {
        self.tracker.get_weight_multiplier(strategy_name)
    }
}