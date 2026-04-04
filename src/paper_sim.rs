use rust_decimal::Decimal;
use scalper_core::types::{MarketEvent, Side};
use scalper_execution::order_tracker::OrderTracker;
use scalper_risk::risk_manager::RiskManager;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

use crate::dashboard::{ConsoleLog, TradeRecord};

/// Order info sent from the executor to the fill simulator.
pub struct SimOrder {
    pub order_id: String,
    pub symbol: String,
    pub side: Side,
    pub entry_price: Decimal,
    pub quantity: Decimal,
    pub take_profit: Decimal,
    pub stop_loss: Decimal,
}

/// An open simulated position waiting for TP/SL exit.
struct PaperPosition {
    order_id: String,
    symbol: String,
    side: Side,
    entry_price: Decimal,
    quantity: Decimal,
    take_profit: Decimal,
    stop_loss: Decimal,
}

/// Paper trade fill simulator. Watches market data and simulates
/// order fills + position exits for paper mode.
pub struct PaperFillSim {
    /// Orders waiting to be filled at entry price.
    pending: HashMap<String, SimOrder>,
    /// Filled orders that are now open positions waiting for TP/SL.
    positions: HashMap<String, PaperPosition>,
}

impl PaperFillSim {
    pub fn new() -> Self {
        Self {
            pending: HashMap::new(),
            positions: HashMap::new(),
        }
    }

    /// Register a new order from the executor.
    pub fn add_order(&mut self, order: SimOrder) {
        self.pending.insert(order.order_id.clone(), order);
    }

    /// Process a market event. Check for entry fills and TP/SL exits.
    pub async fn on_market_event(
        &mut self,
        event: MarketEvent,
        order_tracker: &Arc<OrderTracker>,
        risk_manager: &Arc<Mutex<RiskManager>>,
        trade_history: &Arc<Mutex<Vec<TradeRecord>>>,
        console_log: &Arc<Mutex<ConsoleLog>>,
    ) {
        let (symbol, trade_price, timestamp_ms) = match &event {
            MarketEvent::Trade {
                symbol,
                price,
                timestamp_ms,
                ..
            } => (symbol.clone(), decimal_to_f64(*price), *timestamp_ms),
            _ => return,
        };

        let trade_price_dec = match &event {
            MarketEvent::Trade { price, .. } => *price,
            _ => return,
        };

        // Phase 1: Check pending orders for entry fills
        let mut filled_ids = Vec::new();
        for (id, order) in &self.pending {
            if order.symbol != symbol {
                continue;
            }

            let should_fill = match order.side {
                // Buy limit: fill when price drops to or below entry
                Side::Buy => trade_price_dec <= order.entry_price,
                // Sell limit: fill when price rises to or above entry
                Side::Sell => trade_price_dec >= order.entry_price,
            };

            // Also fill immediately if entry_price is 0 (market order)
            if should_fill || order.entry_price == Decimal::ZERO {
                let fill_price = if order.entry_price == Decimal::ZERO {
                    trade_price_dec
                } else {
                    order.entry_price
                };

                // Update order tracker to show filled
                order_tracker.update(
                    &id,
                    order.quantity,
                    fill_price,
                    "FILLED",
                    timestamp_ms,
                );

                info!(
                    order_id = %id,
                    symbol = %order.symbol,
                    side = ?order.side,
                    price = %fill_price,
                    qty = %order.quantity,
                    "Paper fill: order entered"
                );

                console_log.lock().await.push("SUCCESS", format!(
                    "FILLED: {} {:?} {} @ {} | TP: {} | SL: {}",
                    order.symbol, order.side, order.quantity, fill_price,
                    order.take_profit, order.stop_loss
                ));

                filled_ids.push(id.clone());
            }
        }

        // Move filled orders to open positions
        for id in filled_ids {
            if let Some(order) = self.pending.remove(&id) {
                let fill_price = if order.entry_price == Decimal::ZERO {
                    trade_price_dec
                } else {
                    order.entry_price
                };

                self.positions.insert(
                    id.clone(),
                    PaperPosition {
                        order_id: id,
                        symbol: order.symbol,
                        side: order.side,
                        entry_price: fill_price,
                        quantity: order.quantity,
                        take_profit: order.take_profit,
                        stop_loss: order.stop_loss,
                    },
                );
            }
        }

        // Phase 2: Check open positions for TP/SL exits
        let mut closed_ids = Vec::new();
        for (id, pos) in &self.positions {
            if pos.symbol != symbol {
                continue;
            }

            let exit = match pos.side {
                Side::Buy => {
                    // Long position
                    if pos.take_profit > Decimal::ZERO && trade_price_dec >= pos.take_profit {
                        Some((pos.take_profit, true)) // TP hit
                    } else if pos.stop_loss > Decimal::ZERO && trade_price_dec <= pos.stop_loss {
                        Some((pos.stop_loss, false)) // SL hit
                    } else {
                        None
                    }
                }
                Side::Sell => {
                    // Short position
                    if pos.take_profit > Decimal::ZERO && trade_price_dec <= pos.take_profit {
                        Some((pos.take_profit, true)) // TP hit
                    } else if pos.stop_loss > Decimal::ZERO && trade_price_dec >= pos.stop_loss {
                        Some((pos.stop_loss, false)) // SL hit
                    } else {
                        None
                    }
                }
            };

            if let Some((exit_price, is_tp)) = exit {
                let entry_f = decimal_to_f64(pos.entry_price);
                let exit_f = decimal_to_f64(exit_price);
                let qty_f = decimal_to_f64(pos.quantity);

                // PnL calculation
                let gross_pnl = match pos.side {
                    Side::Buy => (exit_f - entry_f) * qty_f,
                    Side::Sell => (entry_f - exit_f) * qty_f,
                };

                // Fees: 0.04% taker on entry + 0.04% on exit = 0.08% round trip
                let fees = 0.0008 * entry_f * qty_f;

                info!(
                    order_id = %id,
                    symbol = %pos.symbol,
                    side = ?pos.side,
                    entry = %pos.entry_price,
                    exit = %exit_price,
                    pnl = format!("{:.4}", gross_pnl - fees),
                    exit_type = if is_tp { "TP" } else { "SL" },
                    "Paper fill: position closed"
                );

                // Update risk manager
                risk_manager
                    .lock()
                    .await
                    .on_trade_result(gross_pnl, fees, timestamp_ms);

                let net_pnl = gross_pnl - fees;
                let pnl_color = if net_pnl >= 0.0 { "SUCCESS" } else { "ERROR" };
                console_log.lock().await.push(pnl_color, format!(
                    "CLOSED: {} {:?} | Entry: {} Exit: {} | P&L: ${:.2} ({}) | Fees: ${:.4}",
                    pos.symbol, pos.side, pos.entry_price, exit_price,
                    net_pnl, if is_tp { "TP" } else { "SL" }, fees
                ));

                // Record trade in history
                trade_history.lock().await.push(TradeRecord {
                    timestamp_ms,
                    symbol: pos.symbol.clone(),
                    side: format!("{:?}", pos.side),
                    price: exit_price.to_string(),
                    quantity: pos.quantity.to_string(),
                    pnl: net_pnl,
                    fees,
                    order_id: id.clone(),
                    entry_price: pos.entry_price.to_string(),
                    exit_price: exit_price.to_string(),
                    duration_secs: 0.0, // TODO: track entry time
                    status: "CLOSED".to_string(),
                });

                closed_ids.push(id.clone());
            }
        }

        // Remove closed positions
        for id in closed_ids {
            self.positions.remove(&id);
        }
    }
}

fn decimal_to_f64(d: Decimal) -> f64 {
    use std::str::FromStr;
    f64::from_str(&d.to_string()).unwrap_or(0.0)
}
