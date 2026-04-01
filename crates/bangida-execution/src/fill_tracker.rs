use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tracing::debug;

/// Tracks partial fills for a single order, computing volume-weighted
/// average fill price and remaining quantity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FillTracker {
    /// Target quantity for this order.
    target_quantity: Decimal,
    /// Total filled quantity so far.
    filled_quantity: Decimal,
    /// Running cost basis: sum of (fill_price * fill_qty) for VWAP calculation.
    cost_basis: Decimal,
    /// Number of individual fills received.
    fill_count: u32,
}

impl FillTracker {
    /// Create a new fill tracker for an order with the given target quantity.
    pub fn new(target_quantity: Decimal) -> Self {
        Self {
            target_quantity,
            filled_quantity: Decimal::ZERO,
            cost_basis: Decimal::ZERO,
            fill_count: 0,
        }
    }

    /// Record a partial or complete fill.
    pub fn on_fill(&mut self, quantity: Decimal, price: Decimal) {
        self.filled_quantity += quantity;
        self.cost_basis += quantity * price;
        self.fill_count += 1;

        debug!(
            %quantity,
            %price,
            filled = %self.filled_quantity,
            target = %self.target_quantity,
            fill_count = self.fill_count,
            "fill recorded"
        );
    }

    /// Volume-weighted average fill price.
    /// Returns `Decimal::ZERO` if no fills have been recorded.
    pub fn avg_fill_price(&self) -> Decimal {
        if self.filled_quantity.is_zero() {
            return Decimal::ZERO;
        }
        self.cost_basis / self.filled_quantity
    }

    /// Quantity remaining to be filled.
    pub fn remaining_quantity(&self) -> Decimal {
        (self.target_quantity - self.filled_quantity).max(Decimal::ZERO)
    }

    /// Whether the order is completely filled.
    pub fn is_complete(&self) -> bool {
        self.filled_quantity >= self.target_quantity
    }

    /// Total filled quantity.
    pub fn filled_quantity(&self) -> Decimal {
        self.filled_quantity
    }

    /// Number of individual fills received.
    pub fn fill_count(&self) -> u32 {
        self.fill_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_single_fill() {
        let mut ft = FillTracker::new(dec!(1.0));
        ft.on_fill(dec!(1.0), dec!(50000));
        assert!(ft.is_complete());
        assert_eq!(ft.avg_fill_price(), dec!(50000));
        assert_eq!(ft.remaining_quantity(), dec!(0));
    }

    #[test]
    fn test_partial_fills() {
        let mut ft = FillTracker::new(dec!(1.0));
        ft.on_fill(dec!(0.3), dec!(50000));
        ft.on_fill(dec!(0.7), dec!(50100));
        assert!(ft.is_complete());
        // VWAP: (0.3*50000 + 0.7*50100) / 1.0 = (15000 + 35070) / 1.0 = 50070
        assert_eq!(ft.avg_fill_price(), dec!(50070));
        assert_eq!(ft.fill_count(), 2);
    }

    #[test]
    fn test_remaining_quantity() {
        let mut ft = FillTracker::new(dec!(2.0));
        ft.on_fill(dec!(0.5), dec!(100));
        assert_eq!(ft.remaining_quantity(), dec!(1.5));
        assert!(!ft.is_complete());
    }

    #[test]
    fn test_no_fills() {
        let ft = FillTracker::new(dec!(1.0));
        assert_eq!(ft.avg_fill_price(), dec!(0));
        assert_eq!(ft.remaining_quantity(), dec!(1.0));
        assert!(!ft.is_complete());
    }
}
