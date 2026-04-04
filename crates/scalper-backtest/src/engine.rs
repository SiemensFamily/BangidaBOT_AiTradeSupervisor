use scalper_core::types::Side;
use scalper_risk::risk_manager::RiskManager;
use scalper_strategy::ensemble::EnsembleStrategy;
use scalper_strategy::traits::MarketContext;

use crate::report::ReportBuilder;
use crate::sim_exchange::SimExchange;

/// Backtest engine: runs strategies on historical market snapshots.
pub struct BacktestEngine {
    ensemble: EnsembleStrategy,
    risk_manager: RiskManager,
    sim_exchange: SimExchange,
    report_builder: ReportBuilder,
}

impl BacktestEngine {
    pub fn new(
        ensemble: EnsembleStrategy,
        risk_manager: RiskManager,
        initial_balance: f64,
    ) -> Self {
        Self {
            ensemble,
            risk_manager,
            sim_exchange: SimExchange::new(initial_balance, 2.0, -2.0, 4.0),
            report_builder: ReportBuilder::new(initial_balance),
        }
    }

    /// Process a single market data snapshot through the strategy + risk pipeline.
    pub fn process_snapshot(&mut self, ctx: &MarketContext) {
        let now_ms = ctx.timestamp_ms;

        // Evaluate ensemble strategy
        let signal = match self.ensemble.evaluate(ctx) {
            Some(s) => s,
            None => return,
        };

        let price_f64 = decimal_to_f64(ctx.last_price);

        // Validate through risk manager
        let validated = match self.risk_manager.validate_signal(
            &signal,
            ctx.volatility_regime,
            Some(ctx.atr_14),
            price_f64,
            now_ms,
        ) {
            Some(v) => v,
            None => return,
        };

        // Simulate entry fill
        let is_buy = validated.signal.side == Side::Buy;
        let entry_fill = self.sim_exchange.fill_market(price_f64, decimal_to_f64(validated.quantity), is_buy);

        // Calculate PnL at TP or SL
        let tp = validated.signal.take_profit.map(decimal_to_f64);
        let sl = validated.signal.stop_loss.map(decimal_to_f64);

        // Simple simulation: assume trade hits TP with probability based on signal strength
        // This is a simplified model — a real backtest would need tick-by-tick replay
        let (exit_price, _hit_tp) = if signal.strength > 0.5 {
            (tp.unwrap_or(price_f64), true)
        } else {
            (sl.unwrap_or(price_f64), false)
        };

        let pnl = if is_buy {
            (exit_price - entry_fill.fill_price) * entry_fill.quantity
        } else {
            (entry_fill.fill_price - exit_price) * entry_fill.quantity
        };

        let exit_fill = self.sim_exchange.fill_market(exit_price, entry_fill.quantity, !is_buy);
        let total_fees = entry_fill.fee + exit_fill.fee;

        // Record results
        self.sim_exchange.update_balance(pnl, total_fees);
        self.risk_manager.on_trade_result(pnl, total_fees, now_ms);
        self.report_builder.record_trade(pnl, total_fees);
    }

    /// Run backtest on a sequence of market snapshots.
    pub fn run(&mut self, snapshots: &[MarketContext]) {
        for ctx in snapshots {
            self.process_snapshot(ctx);
        }
    }

    /// Generate the final report.
    pub fn report(&self) -> crate::report::BacktestReport {
        self.report_builder.build()
    }

    pub fn final_balance(&self) -> f64 {
        self.sim_exchange.balance()
    }
}

fn decimal_to_f64(d: rust_decimal::Decimal) -> f64 {
    use std::str::FromStr;
    f64::from_str(&d.to_string()).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use scalper_core::config::*;
    use scalper_core::types::*;

    fn make_risk_config() -> RiskConfig {
        RiskConfig {
            max_risk_per_trade_pct: 3.0,
            max_daily_loss_pct: 10.0,
            max_drawdown_pct: 25.0,
            max_consecutive_losses: 3,
            cooldown_minutes: 15,
            min_equity: 25.0,
            max_open_positions: 1,
            max_trades_per_hour: 15,
            max_leverage: 20,
        }
    }

    #[test]
    fn engine_creates_report() {
        let strategies: Vec<Box<dyn scalper_strategy::traits::Strategy>> = vec![];
        let ensemble = EnsembleStrategy::new(strategies, 0.20);
        let risk = RiskManager::new(make_risk_config(), 100.0);
        let mut engine = BacktestEngine::new(ensemble, risk, 100.0);
        let report = engine.report();
        assert_eq!(report.total_trades, 0);
        assert_eq!(report.final_equity, 100.0);
    }
}
