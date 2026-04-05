//! Paper trade fill simulator.
//!
//! Periodically checks open orders against live market data and simulates fills.
//! Market orders fill immediately at best bid/ask with slippage.
//! Limit orders fill when the market price crosses the limit price.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use rust_decimal::prelude::*;
use scalper_core::types::{OrderType, Side};
use scalper_data::orderbook::OrderBook;
use scalper_execution::order_tracker::OrderTracker;
use scalper_risk::risk_manager::RiskManager;

use crate::dashboard::{ConsoleLog, TradeRecord};

const SLIPPAGE_BPS: f64 = 2.0;
const TAKER_FEE_BPS: f64 = 4.0;
const MAKER_FEE_BPS: f64 = 2.0;

pub async fn run_paper_sim(
    order_tracker: Arc<OrderTracker>,
    orderbooks: Arc<Mutex<HashMap<String, OrderBook>>>,
    risk_manager: Arc<Mutex<RiskManager>>,
    trade_history: Arc<Mutex<Vec<TradeRecord>>>,
    console_log: Arc<Mutex<ConsoleLog>>,
) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(500));

    loop {
        interval.tick().await;

        let open_orders = order_tracker.open_orders();
        if open_orders.is_empty() {
            continue;
        }

        let obs = orderbooks.lock().await;

        for order in &open_orders {
            let ob = match obs.get(&order.symbol) {
                Some(ob) => ob,
                None => continue,
            };

            let best_bid = ob.best_bid().map(|(p, _)| p);
            let best_ask = ob.best_ask().map(|(p, _)| p);

            if best_bid.is_none() || best_ask.is_none() {
                continue;
            }

            let best_bid = best_bid.unwrap();
            let best_ask = best_ask.unwrap();

            let fill_result = match order.order_type {
                OrderType::Market => {
                    // Fill at best bid/ask with slippage
                    let slippage_mult =
                        Decimal::from_f64(SLIPPAGE_BPS / 10_000.0).unwrap_or(Decimal::ZERO);
                    let fill_price = match order.side {
                        Side::Buy => best_ask * (Decimal::ONE + slippage_mult),
                        Side::Sell => best_bid * (Decimal::ONE - slippage_mult),
                    };
                    let fee_rate =
                        Decimal::from_f64(TAKER_FEE_BPS / 10_000.0).unwrap_or(Decimal::ZERO);
                    Some((fill_price, fee_rate))
                }
                OrderType::Limit => {
                    // Fill when market crosses the limit price
                    let should_fill = match order.side {
                        Side::Buy => best_ask <= order.price,
                        Side::Sell => best_bid >= order.price,
                    };
                    if should_fill {
                        let fee_rate =
                            Decimal::from_f64(MAKER_FEE_BPS / 10_000.0).unwrap_or(Decimal::ZERO);
                        Some((order.price, fee_rate))
                    } else {
                        None
                    }
                }
                // Stop/TP orders: treat like market when triggered
                OrderType::StopMarket | OrderType::TakeProfitMarket => {
                    let triggered = match order.side {
                        Side::Buy => best_ask >= order.price,
                        Side::Sell => best_bid <= order.price,
                    };
                    if triggered {
                        let slippage_mult =
                            Decimal::from_f64(SLIPPAGE_BPS / 10_000.0).unwrap_or(Decimal::ZERO);
                        let fill_price = match order.side {
                            Side::Buy => best_ask * (Decimal::ONE + slippage_mult),
                            Side::Sell => best_bid * (Decimal::ONE - slippage_mult),
                        };
                        let fee_rate =
                            Decimal::from_f64(TAKER_FEE_BPS / 10_000.0).unwrap_or(Decimal::ZERO);
                        Some((fill_price, fee_rate))
                    } else {
                        None
                    }
                }
            };

            if let Some((fill_price, fee_rate)) = fill_result {
                let qty = order.quantity;
                let notional = fill_price * qty;
                let fees = notional * fee_rate;
                let now_ms = chrono::Utc::now().timestamp_millis() as u64;

                // Compute PnL: for paper mode, entry is the order price, fill is market
                // For a buy: PnL is 0 at fill (position opened), realized when closed
                // Since we're simulating single fills, PnL = 0 for opening trades
                // The PnL tracking happens when the position is eventually closed
                // For simplicity: record the fill, PnL = 0 for now (position open)
                let pnl = 0.0;
                let fees_f64 = fees.to_f64().unwrap_or(0.0);

                // Update order tracker
                order_tracker.update(
                    &order.order_id,
                    qty,
                    fill_price,
                    "Filled",
                    now_ms,
                );

                // Record in risk manager
                {
                    let mut rm = risk_manager.lock().await;
                    rm.on_trade_result(pnl, fees_f64, now_ms);
                }

                // Record in trade history
                let record = TradeRecord {
                    timestamp_ms: now_ms,
                    symbol: order.symbol.clone(),
                    side: format!("{:?}", order.side),
                    price: fill_price.to_string(),
                    quantity: qty.to_string(),
                    pnl,
                    fees: fees_f64,
                    order_id: order.order_id.clone(),
                };
                trade_history.lock().await.push(record);

                // Log to console
                console_log.lock().await.push(format!(
                    "Paper fill: {} {} {} @ {} (fees: ${:.4})",
                    format!("{:?}", order.side),
                    qty,
                    order.symbol,
                    fill_price.round_dp(2),
                    fees_f64,
                ));
            }
        }
    }
}
