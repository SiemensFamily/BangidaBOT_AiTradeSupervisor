use bangida_core::types::{OrderId, OrderResponse, OrderStatus, Price, Quantity, Side, Symbol};
use dashmap::DashMap;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tracing::debug;

/// A locally tracked order with fill state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedOrder {
    pub order_id: OrderId,
    pub symbol: Symbol,
    pub side: Side,
    pub price: Option<Price>,
    pub quantity: Quantity,
    pub status: OrderStatus,
    pub created_at: u64,
    pub fill_quantity: Quantity,
    pub avg_fill_price: Price,
}

impl ManagedOrder {
    /// Whether this order is in a terminal state (filled, canceled, rejected, expired).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            OrderStatus::Filled | OrderStatus::Canceled | OrderStatus::Rejected | OrderStatus::Expired
        )
    }

    /// Whether this order still has unfilled quantity.
    pub fn is_open(&self) -> bool {
        matches!(self.status, OrderStatus::New | OrderStatus::PartiallyFilled)
    }
}

/// Tracks the lifecycle of all orders, updating state from exchange events.
pub struct OrderTracker {
    orders: DashMap<OrderId, ManagedOrder>,
}

impl OrderTracker {
    pub fn new() -> Self {
        Self {
            orders: DashMap::new(),
        }
    }

    /// Insert or update an order from an exchange order response.
    pub fn on_order_update(&self, update: OrderResponse) {
        let order_id = update.order_id.clone();

        if let Some(mut entry) = self.orders.get_mut(&order_id) {
            let prev_status = entry.status;
            entry.status = update.status;

            // If the response carries fill information via quantity and price
            if update.status == OrderStatus::Filled || update.status == OrderStatus::PartiallyFilled
            {
                if let Some(fill_price) = update.price {
                    // Update VWAP for fills
                    let old_value = entry.avg_fill_price * entry.fill_quantity;
                    let fill_qty = update.quantity;
                    let new_value = fill_price * fill_qty;
                    let total_qty = entry.fill_quantity + fill_qty;
                    if !total_qty.is_zero() {
                        entry.avg_fill_price = (old_value + new_value) / total_qty;
                    }
                    entry.fill_quantity = total_qty;
                }
            }

            debug!(
                %order_id,
                ?prev_status,
                new_status = ?update.status,
                "order updated"
            );
        } else {
            // New order we haven't seen — track it
            let managed = ManagedOrder {
                order_id: order_id.clone(),
                symbol: update.symbol,
                side: update.side,
                price: update.price,
                quantity: update.quantity,
                status: update.status,
                created_at: update.timestamp_ms,
                fill_quantity: Decimal::ZERO,
                avg_fill_price: Decimal::ZERO,
            };
            self.orders.insert(order_id.clone(), managed);
            debug!(%order_id, "new order tracked");
        }
    }

    /// Returns a snapshot of all currently open (non-terminal) orders.
    pub fn open_orders(&self) -> Vec<ManagedOrder> {
        self.orders
            .iter()
            .filter(|entry| entry.value().is_open())
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Find orders older than `max_age_ms` that are still open.
    /// Returns order IDs suitable for cancellation.
    pub fn cancel_stale_orders(&self, max_age_ms: u64) -> Vec<ManagedOrder> {
        let now = bangida_core::time::now_ms();
        self.orders
            .iter()
            .filter(|entry| {
                let order = entry.value();
                order.is_open() && (now - order.created_at) > max_age_ms
            })
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Get a snapshot of a specific order.
    pub fn get(&self, order_id: &str) -> Option<ManagedOrder> {
        self.orders.get(order_id).map(|r| r.clone())
    }

    /// Total number of tracked orders (all states).
    pub fn len(&self) -> usize {
        self.orders.len()
    }

    /// Whether the tracker has no orders.
    pub fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }

    /// Remove terminal orders older than `max_age_ms` to free memory.
    pub fn prune_terminal(&self, max_age_ms: u64) {
        let now = bangida_core::time::now_ms();
        self.orders.retain(|_, order| {
            !(order.is_terminal() && (now - order.created_at) > max_age_ms)
        });
    }
}

impl Default for OrderTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bangida_core::types::{OrderStatus, OrderType, Side, Symbol};
    use rust_decimal_macros::dec;

    fn make_response(id: &str, status: OrderStatus) -> OrderResponse {
        OrderResponse {
            order_id: id.to_string(),
            client_order_id: format!("client_{}", id),
            symbol: Symbol::new("BTCUSDT"),
            side: Side::Buy,
            order_type: OrderType::Limit,
            quantity: dec!(0.1),
            price: Some(dec!(50000)),
            status,
            timestamp_ms: 1000,
        }
    }

    #[test]
    fn test_track_new_order() {
        let tracker = OrderTracker::new();
        tracker.on_order_update(make_response("ord1", OrderStatus::New));
        assert_eq!(tracker.len(), 1);
        assert_eq!(tracker.open_orders().len(), 1);
    }

    #[test]
    fn test_order_filled() {
        let tracker = OrderTracker::new();
        tracker.on_order_update(make_response("ord1", OrderStatus::New));
        tracker.on_order_update(make_response("ord1", OrderStatus::Filled));
        assert_eq!(tracker.open_orders().len(), 0);
        let order = tracker.get("ord1").unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
    }

    #[test]
    fn test_stale_orders_empty_when_recent() {
        let tracker = OrderTracker::new();
        tracker.on_order_update(OrderResponse {
            order_id: "ord1".to_string(),
            client_order_id: "c1".to_string(),
            symbol: Symbol::new("BTCUSDT"),
            side: Side::Buy,
            order_type: OrderType::Limit,
            quantity: dec!(0.1),
            price: Some(dec!(50000)),
            status: OrderStatus::New,
            timestamp_ms: bangida_core::time::now_ms(),
        });
        let stale = tracker.cancel_stale_orders(60_000);
        assert!(stale.is_empty());
    }
}
