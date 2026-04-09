use dashmap::DashMap;
use rust_decimal::Decimal;
use scalper_core::types::{Exchange, OrderType, Side, TimeInForce};
use tracing::warn;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderStatus {
    New,
    PartiallyFilled,
    Filled,
    Cancelled,
    Rejected,
    Expired,
}

impl OrderStatus {
    /// Parse a status string (case-insensitive) into an OrderStatus.
    pub fn from_str_status(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "NEW" => OrderStatus::New,
            "PARTIALLY_FILLED" | "PARTIALLYFILLED" | "PARTIAL" => OrderStatus::PartiallyFilled,
            "FILLED" => OrderStatus::Filled,
            "CANCELLED" | "CANCELED" => OrderStatus::Cancelled,
            "REJECTED" => OrderStatus::Rejected,
            "EXPIRED" => OrderStatus::Expired,
            other => {
                warn!(status = other, "Unknown order status, defaulting to Rejected");
                OrderStatus::Rejected
            }
        }
    }

    /// Returns true if this status represents a terminal (final) state.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            OrderStatus::Filled | OrderStatus::Cancelled | OrderStatus::Rejected | OrderStatus::Expired
        )
    }
}

#[derive(Debug, Clone)]
pub struct ManagedOrder {
    pub order_id: String,
    pub symbol: String,
    pub exchange: Exchange,
    pub side: Side,
    pub order_type: OrderType,
    pub time_in_force: TimeInForce,
    pub price: Decimal,
    pub quantity: Decimal,
    pub filled_qty: Decimal,
    pub avg_fill_price: Decimal,
    pub status: OrderStatus,
    pub created_ms: u64,
    pub updated_ms: u64,
    /// Optional take-profit price (paper sim uses this to auto-close positions)
    pub take_profit: Option<Decimal>,
    /// Optional stop-loss price (paper sim uses this to auto-close positions)
    pub stop_loss: Option<Decimal>,
}

/// Concurrent order tracker backed by DashMap for lock-free reads.
pub struct OrderTracker {
    orders: DashMap<String, ManagedOrder>,
    auto_cancel_timeout_ms: u64,
}

impl OrderTracker {
    pub fn new(auto_cancel_timeout_ms: u64) -> Self {
        Self {
            orders: DashMap::new(),
            auto_cancel_timeout_ms,
        }
    }

    /// Begin tracking a new order.
    pub fn track(&self, order: ManagedOrder) {
        self.orders.insert(order.order_id.clone(), order);
    }

    /// Update an existing order's fill state and status.
    /// Computes VWAP: new_avg = (old_avg * old_filled + price * new_qty) / new_total_filled
    /// where new_qty = filled_qty - old_filled_qty (the incremental fill).
    pub fn update(
        &self,
        order_id: &str,
        filled_qty: Decimal,
        avg_price: Decimal,
        status: &str,
        now_ms: u64,
    ) {
        if let Some(mut entry) = self.orders.get_mut(order_id) {
            let order = entry.value_mut();
            let old_filled = order.filled_qty;
            let old_avg = order.avg_fill_price;
            let new_filled = filled_qty;

            // Compute VWAP if there is incremental fill
            if new_filled > old_filled && new_filled > Decimal::ZERO {
                let incremental = new_filled - old_filled;
                let total_cost = old_avg * old_filled + avg_price * incremental;
                order.avg_fill_price = total_cost / new_filled;
            } else if new_filled > Decimal::ZERO {
                // No incremental fill but filled_qty reported; keep existing avg or use reported
                order.avg_fill_price = avg_price;
            }

            order.filled_qty = new_filled;
            order.status = OrderStatus::from_str_status(status);
            order.updated_ms = now_ms;
        } else {
            warn!(order_id, "Attempted to update unknown order");
        }
    }

    /// Retrieve a snapshot of a single order.
    pub fn get(&self, order_id: &str) -> Option<ManagedOrder> {
        self.orders.get(order_id).map(|entry| entry.clone())
    }

    /// Return all orders that are still open (New or PartiallyFilled).
    pub fn open_orders(&self) -> Vec<ManagedOrder> {
        self.orders
            .iter()
            .filter(|entry| {
                matches!(
                    entry.value().status,
                    OrderStatus::New | OrderStatus::PartiallyFilled
                )
            })
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Return order IDs for stale orders: those that are New or PartiallyFilled,
    /// older than `auto_cancel_timeout_ms`, and less than 80% filled.
    pub fn stale_orders(&self, now_ms: u64) -> Vec<String> {
        let threshold = Decimal::from(80) / Decimal::from(100); // 0.80
        self.orders
            .iter()
            .filter(|entry| {
                let order = entry.value();
                let is_open = matches!(
                    order.status,
                    OrderStatus::New | OrderStatus::PartiallyFilled
                );
                let age = now_ms.saturating_sub(order.created_ms);
                let is_stale = age > self.auto_cancel_timeout_ms;
                let fill_ratio = if order.quantity > Decimal::ZERO {
                    order.filled_qty / order.quantity
                } else {
                    Decimal::ZERO
                };
                let is_underfilled = fill_ratio < threshold;
                is_open && is_stale && is_underfilled
            })
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Remove terminal orders (Filled, Cancelled, Rejected, Expired) that are
    /// older than `max_age_ms` relative to `now_ms`.
    pub fn remove_terminal(&self, max_age_ms: u64, now_ms: u64) {
        self.orders.retain(|_key, order| {
            if order.status.is_terminal() {
                let age = now_ms.saturating_sub(order.updated_ms);
                age <= max_age_ms
            } else {
                true
            }
        });
    }
}

impl Default for OrderTracker {
    fn default() -> Self {
        Self::new(5000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn make_order(id: &str, qty: Decimal, created_ms: u64) -> ManagedOrder {
        ManagedOrder {
            order_id: id.to_string(),
            symbol: "BTCUSDT".to_string(),
            exchange: Exchange::Binance,
            side: Side::Buy,
            order_type: OrderType::Limit,
            time_in_force: TimeInForce::GTC,
            price: dec!(50000),
            quantity: qty,
            filled_qty: Decimal::ZERO,
            avg_fill_price: Decimal::ZERO,
            status: OrderStatus::New,
            created_ms,
            updated_ms: created_ms,
            take_profit: None,
            stop_loss: None,
        }
    }

    #[test]
    fn test_track_and_get() {
        let tracker = OrderTracker::new(5000);
        let order = make_order("order-1", dec!(1.0), 1000);
        tracker.track(order);
        let retrieved = tracker.get("order-1").unwrap();
        assert_eq!(retrieved.order_id, "order-1");
        assert_eq!(retrieved.status, OrderStatus::New);
    }

    #[test]
    fn test_get_missing() {
        let tracker = OrderTracker::new(5000);
        assert!(tracker.get("nonexistent").is_none());
    }

    #[test]
    fn test_update_fill() {
        let tracker = OrderTracker::new(5000);
        let order = make_order("order-1", dec!(10), 1000);
        tracker.track(order);

        // Partial fill: 3 units at price 50100
        tracker.update("order-1", dec!(3), dec!(50100), "PARTIALLY_FILLED", 2000);
        let o = tracker.get("order-1").unwrap();
        assert_eq!(o.filled_qty, dec!(3));
        assert_eq!(o.avg_fill_price, dec!(50100));
        assert_eq!(o.status, OrderStatus::PartiallyFilled);

        // Another fill: total 7 units, incremental 4 at price 50200
        // VWAP = (50100*3 + 50200*4) / 7 = (150300 + 200800) / 7 = 351100 / 7 = 50157.142857...
        tracker.update("order-1", dec!(7), dec!(50200), "PARTIALLY_FILLED", 3000);
        let o = tracker.get("order-1").unwrap();
        assert_eq!(o.filled_qty, dec!(7));
        // Check VWAP is approximately correct
        let expected_vwap = (dec!(50100) * dec!(3) + dec!(50200) * dec!(4)) / dec!(7);
        assert_eq!(o.avg_fill_price, expected_vwap);
    }

    #[test]
    fn test_open_orders() {
        let tracker = OrderTracker::new(5000);
        tracker.track(make_order("o1", dec!(1), 1000));
        tracker.track(make_order("o2", dec!(1), 1000));
        tracker.track(make_order("o3", dec!(1), 1000));

        tracker.update("o2", dec!(1), dec!(50000), "FILLED", 2000);
        tracker.update("o3", dec!(0.5), dec!(50000), "PARTIALLY_FILLED", 2000);

        let open = tracker.open_orders();
        assert_eq!(open.len(), 2); // o1 (New) and o3 (PartiallyFilled)
        let ids: Vec<String> = open.iter().map(|o| o.order_id.clone()).collect();
        assert!(ids.contains(&"o1".to_string()));
        assert!(ids.contains(&"o3".to_string()));
    }

    #[test]
    fn test_stale_orders() {
        let tracker = OrderTracker::new(5000);
        // Order created at t=1000, check at t=7000 (age=6000 > 5000)
        tracker.track(make_order("stale", dec!(10), 1000));
        // Order created at t=6000, check at t=7000 (age=1000 < 5000)
        tracker.track(make_order("fresh", dec!(10), 6000));
        // Order that is 90% filled - not stale even though old
        let mut mostly_filled = make_order("filled_mostly", dec!(10), 1000);
        mostly_filled.filled_qty = dec!(9);
        mostly_filled.status = OrderStatus::PartiallyFilled;
        tracker.track(mostly_filled);

        let stale = tracker.stale_orders(7000);
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0], "stale");
    }

    #[test]
    fn test_remove_terminal() {
        let tracker = OrderTracker::new(5000);
        tracker.track(make_order("o1", dec!(1), 1000));
        tracker.track(make_order("o2", dec!(1), 1000));
        tracker.track(make_order("o3", dec!(1), 1000));

        tracker.update("o1", dec!(1), dec!(50000), "FILLED", 2000);
        tracker.update("o2", dec!(1), dec!(50000), "CANCELLED", 5000);

        // Remove terminal orders older than 2000ms at now=8000
        // o1 updated at 2000, age=6000 > 2000 -> removed
        // o2 updated at 5000, age=3000 > 2000 -> removed
        // o3 is still New -> kept
        tracker.remove_terminal(2000, 8000);

        assert!(tracker.get("o1").is_none());
        assert!(tracker.get("o2").is_none());
        assert!(tracker.get("o3").is_some());
    }

    #[test]
    fn test_order_status_parsing() {
        assert_eq!(OrderStatus::from_str_status("NEW"), OrderStatus::New);
        assert_eq!(
            OrderStatus::from_str_status("PARTIALLY_FILLED"),
            OrderStatus::PartiallyFilled
        );
        assert_eq!(
            OrderStatus::from_str_status("ParTiallyFilled"),
            OrderStatus::PartiallyFilled
        );
        assert_eq!(OrderStatus::from_str_status("filled"), OrderStatus::Filled);
        assert_eq!(
            OrderStatus::from_str_status("CANCELLED"),
            OrderStatus::Cancelled
        );
        assert_eq!(
            OrderStatus::from_str_status("CANCELED"),
            OrderStatus::Cancelled
        );
        assert_eq!(
            OrderStatus::from_str_status("REJECTED"),
            OrderStatus::Rejected
        );
        assert_eq!(
            OrderStatus::from_str_status("EXPIRED"),
            OrderStatus::Expired
        );
        assert_eq!(
            OrderStatus::from_str_status("UNKNOWN_GARBAGE"),
            OrderStatus::Rejected
        );
    }

    #[test]
    fn test_default_tracker() {
        let tracker = OrderTracker::default();
        assert_eq!(tracker.auto_cancel_timeout_ms, 5000);
    }
}
