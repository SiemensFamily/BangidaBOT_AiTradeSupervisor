use bangida_core::config::RiskConfig;
use bangida_core::types::{MarketEvent, Price, Quantity, Side, Symbol};
use bangida_risk::{CircuitBreaker, PnlTracker, RiskManager};
use bangida_strategy::{MarketContext, Strategy};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::report::generate_report;
use crate::sim_exchange::SimulatedExchange;

/// Summary report produced by a backtest run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestReport {
    pub total_trades: u64,
    pub winning_trades: u64,
    pub total_pnl: f64,
    pub max_drawdown: f64,
    pub sharpe: f64,
    pub sortino: f64,
    pub profit_factor: f64,
    pub win_rate: f64,
    pub expectancy: f64,
    pub equity_curve: Vec<(u64, f64)>,
}

/// A completed trade record used internally for report generation.
#[derive(Debug, Clone)]
pub struct TradeRecord {
    pub symbol: Symbol,
    pub side: Side,
    pub entry_price: Price,
    pub exit_price: Price,
    pub quantity: Quantity,
    pub pnl: Decimal,
    pub fees: Decimal,
    pub entry_time_ms: u64,
    pub exit_time_ms: u64,
}

/// Event-driven backtest engine.
///
/// Replays historical `MarketEvent`s through a strategy and simulated exchange,
/// tracking PnL and producing a comprehensive performance report.
pub struct BacktestEngine {
    events: Vec<MarketEvent>,
    strategy: Box<dyn Strategy>,
    risk_manager: RiskManager,
    circuit_breaker: CircuitBreaker,
    sim_exchange: SimulatedExchange,
    initial_equity: Decimal,
}

impl BacktestEngine {
    pub fn new(
        events: Vec<MarketEvent>,
        strategy: Box<dyn Strategy>,
        risk_config: RiskConfig,
        initial_equity: Decimal,
        fee_rate: Decimal,
    ) -> Self {
        Self {
            events,
            strategy,
            risk_manager: RiskManager::new(risk_config.clone()),
            circuit_breaker: CircuitBreaker::new(risk_config, initial_equity),
            sim_exchange: SimulatedExchange::new(initial_equity, fee_rate),
            initial_equity,
        }
    }

    /// Run the backtest and return a performance report.
    pub fn run(&mut self) -> BacktestReport {
        let mut pnl_tracker = PnlTracker::new(self.initial_equity);
        let mut trades: Vec<TradeRecord> = Vec::new();
        let mut equity_curve: Vec<(u64, f64)> = Vec::new();
        let mut last_ctx: Option<MarketContext> = None;
        let mut current_timestamp_ms: u64 = 0;

        info!(
            num_events = self.events.len(),
            %self.initial_equity,
            "starting backtest"
        );

        // Clone events to avoid borrow conflict with &mut self
        let events = self.events.clone();

        for event in &events {
            // Extract timestamp and update the sim exchange with market data
            match event {
                MarketEvent::OrderBookUpdate {
                    symbol,
                    bids,
                    asks,
                    timestamp_ms,
                    ..
                } => {
                    current_timestamp_ms = *timestamp_ms;

                    // Build a simplified market context from the order book
                    if let (Some(best_bid), Some(best_ask)) = (bids.first(), asks.first()) {
                        let mid = (best_bid.0 + best_ask.0) / Decimal::TWO;
                        let spread = best_ask.0 - best_bid.0;
                        let bid_depth: Decimal = bids.iter().map(|(_, q)| q).sum();
                        let ask_depth: Decimal = asks.iter().map(|(_, q)| q).sum();
                        let total_depth = bid_depth + ask_depth;
                        let imbalance = if !total_depth.is_zero() {
                            ((bid_depth - ask_depth) / total_depth)
                                .to_f64()
                                .unwrap_or(0.0)
                        } else {
                            0.0
                        };

                        self.sim_exchange.update_price(mid);

                        let ctx = MarketContext {
                            symbol: symbol.clone(),
                            orderbook_imbalance: imbalance,
                            spread,
                            mid_price: mid,
                            microprice: mid,
                            bid_depth,
                            ask_depth,
                            last_price: mid,
                            rsi: 50.0,
                            ema_fast: mid.to_f64().unwrap_or(0.0),
                            ema_slow: mid.to_f64().unwrap_or(0.0),
                            bb_upper: 0.0,
                            bb_lower: 0.0,
                            bb_middle: 0.0,
                            macd_line: 0.0,
                            macd_signal: 0.0,
                            macd_histogram: 0.0,
                            vwap: mid.to_f64().unwrap_or(0.0),
                            cvd: 0.0,
                            volume_1s: 0.0,
                            avg_volume_60s: 0.0,
                            funding_rate: 0.0,
                            highest_high_60s: mid,
                            lowest_low_60s: mid,
                            timestamp_ms: *timestamp_ms,
                        };
                        last_ctx = Some(ctx);
                    }
                }
                MarketEvent::Trade(trade) => {
                    current_timestamp_ms = trade.timestamp_ms;
                    self.sim_exchange.update_price(trade.price);

                    // Check if any open orders should fill at this trade price
                    let fills = self.sim_exchange.check_fills(trade.price);
                    for (fill_pnl, fill_fees) in fills {
                        pnl_tracker.record_trade(fill_pnl, fill_fees);
                        self.circuit_breaker.on_trade_result(fill_pnl - fill_fees);
                        trades.push(TradeRecord {
                            symbol: trade.symbol.clone(),
                            side: Side::Sell, // simplified
                            entry_price: Decimal::ZERO,
                            exit_price: trade.price,
                            quantity: Decimal::ZERO,
                            pnl: fill_pnl,
                            fees: fill_fees,
                            entry_time_ms: 0,
                            exit_time_ms: trade.timestamp_ms,
                        });
                    }
                }
                MarketEvent::KlineClose(kline) => {
                    current_timestamp_ms = kline.close_time_ms;
                    self.sim_exchange.update_price(kline.close);
                }
                _ => {}
            }

            // Evaluate strategy if we have context
            if let Some(ctx) = &last_ctx {
                if let Some(signal) = self.strategy.evaluate(ctx) {
                    // Check circuit breaker
                    if self.circuit_breaker.can_trade().is_err() {
                        debug!("circuit breaker active, skipping signal");
                        continue;
                    }

                    // Validate through risk manager
                    let equity = pnl_tracker.current_equity();
                    let open_positions = self.sim_exchange.open_position_count();
                    match self.risk_manager.validate_signal(&signal, equity, open_positions, 1) {
                        Ok(validated) => {
                            let current_price = self.sim_exchange.current_price();
                            let req = bangida_core::types::OrderRequest {
                                symbol: signal.symbol.clone(),
                                side: signal.side,
                                order_type: bangida_core::types::OrderType::Limit,
                                quantity: validated.quantity,
                                price: Some(current_price),
                                stop_price: signal.stop_loss,
                                time_in_force: bangida_core::types::TimeInForce::Gtc,
                                reduce_only: false,
                            };
                            let _resp = self.sim_exchange.place_order(req, current_price);
                        }
                        Err(e) => {
                            debug!(%e, "signal rejected by risk manager");
                        }
                    }
                }
            }

            // Record equity curve periodically
            equity_curve.push((
                current_timestamp_ms,
                pnl_tracker.current_equity().to_f64().unwrap_or(0.0),
            ));
        }

        info!(
            total_trades = pnl_tracker.total_trades(),
            total_pnl = %pnl_tracker.total_pnl(),
            "backtest complete"
        );

        generate_report(&pnl_tracker, &equity_curve)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bangida_core::config::RiskConfig;
    use rust_decimal_macros::dec;

    fn test_config() -> RiskConfig {
        RiskConfig {
            max_risk_per_trade_pct: 0.01,
            max_daily_loss_pct: 0.05,
            max_drawdown_pct: 0.10,
            max_consecutive_losses: 5,
            cooldown_minutes: 15,
            min_equity: 100.0,
            max_open_positions: 3,
            max_trades_per_hour: 60,
            max_leverage: 20,
        }
    }

    struct DummyStrategy;
    impl Strategy for DummyStrategy {
        fn name(&self) -> &str {
            "dummy"
        }
        fn evaluate(&self, _ctx: &MarketContext) -> Option<bangida_core::Signal> {
            None
        }
        fn weight(&self) -> f64 {
            1.0
        }
    }

    #[test]
    fn test_empty_backtest() {
        let mut engine = BacktestEngine::new(
            vec![],
            Box::new(DummyStrategy),
            test_config(),
            dec!(10000),
            dec!(0.0004),
        );
        let report = engine.run();
        assert_eq!(report.total_trades, 0);
        assert_eq!(report.total_pnl, 0.0);
    }
}
