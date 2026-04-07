//! Paper trade fill simulator.
//!
//! Periodically checks open orders against live market data and simulates fills.
//! Market orders fill immediately at best bid/ask with slippage.
//! Limit orders fill when the market price crosses the limit price.
//!
//! Tracks open positions per symbol so that an opposing-side fill realizes PnL.

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

/// An open paper position for a symbol.
#[derive(Debug, Clone)]
struct OpenPosition {
    side: Side,
    avg_price: Decimal,
    quantity: Decimal,
    take_profit: Option<Decimal>,
    stop_loss: Option<Decimal>,
}

pub async fn run_paper_sim(
    order_tracker: Arc<OrderTracker>,
    orderbooks: Arc<Mutex<HashMap<String, OrderBook>>>,
    risk_manager: Arc<Mutex<RiskManager>>,
    trade_history: Arc<Mutex<Vec<TradeRecord>>>,
    console_log: Arc<Mutex<ConsoleLog>>,
) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(500));
    // Per-symbol open positions for realized PnL tracking
    let mut positions: HashMap<String, OpenPosition> = HashMap::new();
    // Per-symbol cooldown timestamps (skip new entries until this time)
    let mut cooldowns: HashMap<String, u64> = HashMap::new();
    const COOLDOWN_MS: u64 = 20_000; // 20s after a close — was 60s

    loop {
        interval.tick().await;

        // ── Auto-close open positions when price crosses TP or SL ───────
        if !positions.is_empty() {
            let obs = orderbooks.lock().await;
            let mut to_close: Vec<(String, Decimal, &'static str)> = Vec::new();
            for (sym, pos) in positions.iter() {
                let ob = match obs.get(sym) {
                    Some(ob) => ob,
                    None => continue,
                };
                let (best_bid, best_ask) = match (ob.best_bid(), ob.best_ask()) {
                    (Some((b, _)), Some((a, _))) => (b, a),
                    _ => continue,
                };
                // For a long, exit price is best_bid; for a short, best_ask
                let exit_px = match pos.side {
                    Side::Buy => best_bid,
                    Side::Sell => best_ask,
                };
                // TP/SL triggers
                if let Some(tp) = pos.take_profit {
                    let hit = match pos.side {
                        Side::Buy => exit_px >= tp,
                        Side::Sell => exit_px <= tp,
                    };
                    if hit {
                        to_close.push((sym.clone(), exit_px, "TP"));
                        continue;
                    }
                }
                if let Some(sl) = pos.stop_loss {
                    let hit = match pos.side {
                        Side::Buy => exit_px <= sl,
                        Side::Sell => exit_px >= sl,
                    };
                    if hit {
                        to_close.push((sym.clone(), exit_px, "SL"));
                    }
                }
            }
            drop(obs);

            // Realize closures
            for (sym, exit_px, reason) in to_close {
                if let Some(pos) = positions.remove(&sym) {
                    let pnl_dec = match pos.side {
                        Side::Buy => (exit_px - pos.avg_price) * pos.quantity,
                        Side::Sell => (pos.avg_price - exit_px) * pos.quantity,
                    };
                    let pnl_f64 = pnl_dec.to_f64().unwrap_or(0.0);
                    let fees_dec = exit_px * pos.quantity
                        * Decimal::from_f64(TAKER_FEE_BPS / 10_000.0).unwrap_or(Decimal::ZERO);
                    let fees_f64 = fees_dec.to_f64().unwrap_or(0.0);
                    let now_ms = chrono::Utc::now().timestamp_millis() as u64;

                    {
                        let mut rm = risk_manager.lock().await;
                        rm.on_trade_result(pnl_f64, fees_f64, now_ms);
                    }
                    cooldowns.insert(sym.clone(), now_ms + COOLDOWN_MS);

                    let exit_side = match pos.side {
                        Side::Buy => Side::Sell,
                        Side::Sell => Side::Buy,
                    };
                    trade_history.lock().await.push(TradeRecord {
                        timestamp_ms: now_ms,
                        symbol: sym.clone(),
                        side: format!("{:?}", exit_side),
                        price: exit_px.to_string(),
                        quantity: pos.quantity.to_string(),
                        pnl: pnl_f64,
                        fees: fees_f64,
                        order_id: format!("close-{}-{}", reason, now_ms),
                    });

                    console_log.lock().await.push(format!(
                        "Auto-close [{}]: {} {:.6} {} @ {} → PnL ${:.4}",
                        reason,
                        format!("{:?}", exit_side),
                        pos.quantity.to_f64().unwrap_or(0.0),
                        sym,
                        exit_px.round_dp(2),
                        pnl_f64,
                    ));
                }
            }
        }

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

            // ── Entry guards ───────────────────────────────────────────
            // Skip if a cooldown is active for this symbol
            let now_check = chrono::Utc::now().timestamp_millis() as u64;
            if let Some(&cd_until) = cooldowns.get(&order.symbol) {
                if now_check < cd_until {
                    order_tracker.update(&order.order_id, Decimal::ZERO, Decimal::ZERO, "Cancelled", now_check);
                    continue;
                }
            }
            // Skip if a same-side position is already open (no averaging)
            if let Some(pos) = positions.get(&order.symbol) {
                if pos.side == order.side {
                    order_tracker.update(&order.order_id, Decimal::ZERO, Decimal::ZERO, "Cancelled", now_check);
                    continue;
                }
            }

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
                let fees_f64 = fees.to_f64().unwrap_or(0.0);

                // Realized PnL via position tracking. If there's an open
                // position on the same symbol with the OPPOSITE side, this
                // fill closes (or partially closes) it. Otherwise, open or
                // add to a same-side position.
                let pnl = match positions.get_mut(&order.symbol) {
                    Some(pos) if pos.side != order.side => {
                        // Closing trade — compute realized PnL on the matched qty
                        let close_qty = qty.min(pos.quantity);
                        let pnl_dec = match pos.side {
                            Side::Buy => (fill_price - pos.avg_price) * close_qty,
                            Side::Sell => (pos.avg_price - fill_price) * close_qty,
                        };
                        pos.quantity -= close_qty;
                        let pnl_f64 = pnl_dec.to_f64().unwrap_or(0.0);
                        if pos.quantity <= Decimal::ZERO {
                            positions.remove(&order.symbol);
                            cooldowns.insert(order.symbol.clone(), now_ms + COOLDOWN_MS);
                        }
                        pnl_f64
                    }
                    Some(pos) => {
                        // Same side — average into the position
                        let total_qty = pos.quantity + qty;
                        if total_qty > Decimal::ZERO {
                            pos.avg_price = (pos.avg_price * pos.quantity + fill_price * qty) / total_qty;
                            pos.quantity = total_qty;
                        }
                        // Refresh TP/SL from the latest order
                        if order.take_profit.is_some() { pos.take_profit = order.take_profit; }
                        if order.stop_loss.is_some() { pos.stop_loss = order.stop_loss; }
                        0.0
                    }
                    None => {
                        // New position
                        positions.insert(order.symbol.clone(), OpenPosition {
                            side: order.side,
                            avg_price: fill_price,
                            quantity: qty,
                            take_profit: order.take_profit,
                            stop_loss: order.stop_loss,
                        });
                        0.0
                    }
                };

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
