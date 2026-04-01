use std::collections::BTreeMap;
use std::cmp::Reverse;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use bangida_core::{Price, Quantity};

/// L2 order book with sorted bid/ask levels.
///
/// Bids are stored in a `BTreeMap<Reverse<Decimal>, Decimal>` so the highest
/// bid is always first. Asks use a normal `BTreeMap` so the lowest ask is first.
#[derive(Debug, Clone)]
pub struct OrderBook {
    pub bids: BTreeMap<Reverse<Decimal>, Decimal>,
    pub asks: BTreeMap<Decimal, Decimal>,
    pub last_update_id: u64,
    pub timestamp_ms: u64,
}

impl OrderBook {
    pub fn new() -> Self {
        Self {
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            last_update_id: 0,
            timestamp_ms: 0,
        }
    }

    /// Apply incremental diff updates. A quantity of zero means remove that level.
    pub fn apply_update(&mut self, bids: &[(Price, Quantity)], asks: &[(Price, Quantity)]) {
        for &(price, qty) in bids {
            if qty.is_zero() {
                self.bids.remove(&Reverse(price));
            } else {
                self.bids.insert(Reverse(price), qty);
            }
        }
        for &(price, qty) in asks {
            if qty.is_zero() {
                self.asks.remove(&price);
            } else {
                self.asks.insert(price, qty);
            }
        }
    }

    /// Replace the entire book with a snapshot.
    pub fn apply_snapshot(&mut self, bids: &[(Price, Quantity)], asks: &[(Price, Quantity)]) {
        self.bids.clear();
        self.asks.clear();
        for &(price, qty) in bids {
            if !qty.is_zero() {
                self.bids.insert(Reverse(price), qty);
            }
        }
        for &(price, qty) in asks {
            if !qty.is_zero() {
                self.asks.insert(price, qty);
            }
        }
    }

    /// Best bid price and quantity.
    #[inline]
    pub fn best_bid(&self) -> Option<(Price, Quantity)> {
        self.bids.iter().next().map(|(Reverse(p), q)| (*p, *q))
    }

    /// Best ask price and quantity.
    #[inline]
    pub fn best_ask(&self) -> Option<(Price, Quantity)> {
        self.asks.iter().next().map(|(p, q)| (*p, *q))
    }

    /// Spread: best_ask - best_bid.
    pub fn spread(&self) -> Option<Decimal> {
        match (self.best_bid(), self.best_ask()) {
            (Some((bid, _)), Some((ask, _))) => Some(ask - bid),
            _ => None,
        }
    }

    /// Mid price: (best_bid + best_ask) / 2.
    pub fn mid_price(&self) -> Option<Decimal> {
        match (self.best_bid(), self.best_ask()) {
            (Some((bid, _)), Some((ask, _))) => Some((bid + ask) / Decimal::TWO),
            _ => None,
        }
    }

    /// Microprice: volume-weighted midpoint.
    /// (bid_price * ask_qty + ask_price * bid_qty) / (bid_qty + ask_qty)
    pub fn microprice(&self) -> Option<Decimal> {
        match (self.best_bid(), self.best_ask()) {
            (Some((bid_p, bid_q)), Some((ask_p, ask_q))) => {
                let denom = bid_q + ask_q;
                if denom.is_zero() {
                    return None;
                }
                Some((bid_p * ask_q + ask_p * bid_q) / denom)
            }
            _ => None,
        }
    }

    /// Total bid volume for the top N price levels.
    pub fn bid_depth(&self, levels: usize) -> Decimal {
        self.bids.values().take(levels).copied().sum()
    }

    /// Total ask volume for the top N price levels.
    pub fn ask_depth(&self, levels: usize) -> Decimal {
        self.asks.values().take(levels).copied().sum()
    }

    /// Order book imbalance for the top `depth` levels.
    /// Returns (bid_vol - ask_vol) / (bid_vol + ask_vol), range [-1.0, 1.0].
    /// Returns 0.0 if both sides are empty.
    pub fn imbalance(&self, depth: usize) -> f64 {
        let bid_vol = self.bid_depth(depth);
        let ask_vol = self.ask_depth(depth);
        let total = bid_vol + ask_vol;
        if total.is_zero() {
            return 0.0;
        }
        let diff = bid_vol - ask_vol;
        diff.to_f64().unwrap_or(0.0) / total.to_f64().unwrap_or(1.0)
    }
}

impl Default for OrderBook {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn snapshot_and_best() {
        let mut ob = OrderBook::new();
        ob.apply_snapshot(
            &[(dec!(100), dec!(1)), (dec!(99), dec!(2))],
            &[(dec!(101), dec!(1.5)), (dec!(102), dec!(3))],
        );
        assert_eq!(ob.best_bid(), Some((dec!(100), dec!(1))));
        assert_eq!(ob.best_ask(), Some((dec!(101), dec!(1.5))));
        assert_eq!(ob.spread(), Some(dec!(1)));
    }

    #[test]
    fn update_removes_zero_qty() {
        let mut ob = OrderBook::new();
        ob.apply_snapshot(
            &[(dec!(100), dec!(1))],
            &[(dec!(101), dec!(1))],
        );
        ob.apply_update(&[(dec!(100), dec!(0))], &[]);
        assert_eq!(ob.best_bid(), None);
    }

    #[test]
    fn microprice_calculation() {
        let mut ob = OrderBook::new();
        ob.apply_snapshot(
            &[(dec!(100), dec!(2))],
            &[(dec!(102), dec!(3))],
        );
        // microprice = (100*3 + 102*2) / (2+3) = (300+204)/5 = 504/5 = 100.8
        assert_eq!(ob.microprice(), Some(dec!(100.8)));
    }
}
