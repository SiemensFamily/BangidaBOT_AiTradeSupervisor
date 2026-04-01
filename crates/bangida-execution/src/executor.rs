use std::sync::Arc;

use bangida_core::types::{
    OrderId, OrderRequest, OrderType, Price, Side, TimeInForce, ValidatedSignal,
};
use bangida_core::error::BangidaError;
use bangida_exchange::OrderManager;
use dashmap::DashMap;
use rust_decimal::Decimal;
use tracing::{debug, info};

use crate::order_manager::ManagedOrder;

/// Smart order executor that translates validated signals into exchange orders.
///
/// Prefers limit orders for maker rebates, escalating to aggressive pricing
/// or market orders when signal strength demands urgency.
pub struct Executor {
    order_manager: Arc<dyn OrderManager>,
    /// Tracks orders sent by this executor, keyed by order ID.
    sent_orders: DashMap<OrderId, ManagedOrder>,
    /// Minimum tick size for aggressive pricing (symbol-specific in production;
    /// a sensible default here).
    tick_size: Decimal,
}

impl Executor {
    pub fn new(order_manager: Arc<dyn OrderManager>) -> Self {
        Self {
            order_manager,
            sent_orders: DashMap::new(),
            tick_size: Decimal::new(1, 1), // 0.1 default tick
        }
    }

    /// Create an executor with a custom tick size.
    pub fn with_tick_size(order_manager: Arc<dyn OrderManager>, tick_size: Decimal) -> Self {
        Self {
            order_manager,
            sent_orders: DashMap::new(),
            tick_size,
        }
    }

    /// Execute a validated signal by placing the appropriate order.
    ///
    /// - For regular entries: limit order at best bid (buys) or best ask (sells)
    ///   to capture maker rebates.
    /// - For high-strength signals (> 0.8): aggressive pricing (cross the spread by 1 tick).
    /// - For stop losses: always market orders.
    pub async fn execute(
        &self,
        signal: ValidatedSignal,
        best_bid: Price,
        best_ask: Price,
    ) -> Result<OrderId, BangidaError> {
        let is_stop_loss = signal.signal.stop_loss.is_some()
            && signal.signal.strength <= 0.0;

        let (order_type, price) = if is_stop_loss {
            // Stop losses always use market orders for guaranteed fills
            debug!(symbol = %signal.signal.symbol, "using market order for stop loss");
            (OrderType::Market, None)
        } else if signal.signal.strength > 0.8 {
            // Aggressive: cross the spread by one tick
            let aggressive_price = match signal.signal.side {
                Side::Buy => best_bid + self.tick_size,
                Side::Sell => best_ask - self.tick_size,
            };
            debug!(
                symbol = %signal.signal.symbol,
                strength = signal.signal.strength,
                %aggressive_price,
                "aggressive limit order"
            );
            (OrderType::Limit, Some(aggressive_price))
        } else {
            // Passive: sit on our side of the book for maker rebates
            let passive_price = match signal.signal.side {
                Side::Buy => best_bid,
                Side::Sell => best_ask,
            };
            debug!(
                symbol = %signal.signal.symbol,
                strength = signal.signal.strength,
                %passive_price,
                "passive limit order"
            );
            (OrderType::Limit, Some(passive_price))
        };

        let time_in_force = match order_type {
            OrderType::Limit => TimeInForce::PostOnly,
            _ => TimeInForce::Ioc,
        };

        let request = OrderRequest {
            symbol: signal.signal.symbol.clone(),
            side: signal.signal.side,
            order_type,
            quantity: signal.quantity,
            price,
            stop_price: signal.signal.stop_loss,
            time_in_force,
            reduce_only: false,
        };

        let response = self
            .order_manager
            .place_order(&request)
            .await
            .map_err(|e| BangidaError::Exchange(e.to_string()))?;

        let order_id = response.order_id.clone();

        let managed = ManagedOrder {
            order_id: order_id.clone(),
            symbol: signal.signal.symbol,
            side: signal.signal.side,
            price,
            quantity: signal.quantity,
            status: response.status,
            created_at: response.timestamp_ms,
            fill_quantity: Decimal::ZERO,
            avg_fill_price: Decimal::ZERO,
        };

        self.sent_orders.insert(order_id.clone(), managed);

        info!(
            %order_id,
            "order placed successfully"
        );

        Ok(order_id)
    }

    /// Number of orders tracked by this executor.
    pub fn tracked_order_count(&self) -> usize {
        self.sent_orders.len()
    }

    /// Get a snapshot of a tracked order.
    pub fn get_order(&self, order_id: &str) -> Option<ManagedOrder> {
        self.sent_orders.get(order_id).map(|r| r.clone())
    }
}
