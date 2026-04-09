pub mod circuit_breaker;
pub mod position_sizer;
pub mod pnl_tracker;
pub mod risk_manager;

pub use circuit_breaker::CircuitBreaker;
pub use position_sizer::PositionSizer;
pub use pnl_tracker::PnlTracker;
pub use risk_manager::RiskManager;

// Near the top or where you added it
#[derive(Default, Debug)]
pub struct PerformanceTracker {   // make sure it's `pub`
    pub recent_outcomes: Vec<bool>,
}

impl PerformanceTracker {
    pub fn record_trade(&mut self, was_win: bool) {
        self.recent_outcomes.push(was_win);
        if self.recent_outcomes.len() > 50 {
            self.recent_outcomes.remove(0);
        }
    }

    pub fn record_trade_outcome(&mut self, was_profitable: bool) {
        self.record_trade(was_profitable);   // This is the public method
    }

    pub fn get_win_rate(&self) -> f64 {
        if self.recent_outcomes.is_empty() {
            return 0.50;
        }
        let wins = self.recent_outcomes.iter().filter(|&&w| w).count() as f64;
        wins / self.recent_outcomes.len() as f64
    }

    pub fn get_weight_multiplier(&self) -> f64 {
        let wr = self.get_win_rate();
        if wr < 0.40 {
            0.60
        } else if wr < 0.50 {
            0.80
        } else {
            1.0
        }
    }
}