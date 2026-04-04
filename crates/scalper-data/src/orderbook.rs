use rust_decimal::Decimal;
use scalper_core::types::Exchange;
use std::collections::BTreeMap;
use std::cmp::Reverse;

/// Order book manager with bid/ask levels.
/// Bids are stored with `Reverse<Decimal>` keys so that iterating yields highest price first.
/// Asks are stored normally so that iterating yields lowest price first.
#[derive(Debug, Clone)]
pub struct OrderBook {
    pub symbol: String,
    pub exchange: Exchange,
    bids: BTreeMap<Reverse<Decimal>, Decimal>, // price (desc) -> quantity
    asks: BTreeMap<Decimal, Decimal>,           // price (asc) -> quantity
    last_update_ms: u64,
}

impl OrderBook {
    pub fn new(symbol: String, exchange: Exchange) -> Self {
        Self {
            symbol,
            exchange,
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            last_update_ms: 0,
        }
    }

    /// Apply a batch update. Levels with zero quantity are removed.
    pub fn update(&mut self, bids: &[(Decimal, Decimal)], asks: &[(Decimal, Decimal)], ts: u64) {
        for &(price, qty) in bids {
            if qty == Decimal::ZERO {
                self.bids.remove(&Reverse(price));
            } else {
                self.bids.insert(Reverse(price), qty);
            }
        }
        for &(price, qty) in asks {
            if qty == Decimal::ZERO {
                self.asks.remove(&price);
            } else {
                self.asks.insert(price, qty);
            }
        }
        self.last_update_ms = ts;
    }

    /// Best (highest) bid price and its quantity.
    pub fn best_bid(&self) -> Option<(Decimal, Decimal)> {
        self.bids.iter().next().map(|(Reverse(p), q)| (*p, *q))
    }

    /// Best (lowest) ask price and its quantity.
    pub fn best_ask(&self) -> Option<(Decimal, Decimal)> {
        self.asks.iter().next().map(|(p, q)| (*p, *q))
    }

    /// Mid price = (best_bid + best_ask) / 2.
    pub fn mid_price(&self) -> Option<Decimal> {
        let (bid, _) = self.best_bid()?;
        let (ask, _) = self.best_ask()?;
        Some((bid + ask) / Decimal::from(2))
    }

    /// Spread = best_ask - best_bid.
    pub fn spread(&self) -> Option<Decimal> {
        let (bid, _) = self.best_bid()?;
        let (ask, _) = self.best_ask()?;
        Some(ask - bid)
    }

    /// Order book imbalance ratio over top `depth` levels.
    /// Returns value in [-1.0, 1.0]. Positive means bid-heavy.
    /// Formula: (bid_qty - ask_qty) / (bid_qty + ask_qty).
    pub fn imbalance_ratio(&self, depth: usize) -> f64 {
        use rust_decimal::prelude::ToPrimitive;
        let bid_qty = self.bid_depth(depth);
        let ask_qty = self.ask_depth(depth);
        let total = bid_qty + ask_qty;
        if total == Decimal::ZERO {
            return 0.0;
        }
        let num = (bid_qty - ask_qty).to_f64().unwrap_or(0.0);
        let den = total.to_f64().unwrap_or(1.0);
        num / den
    }

    /// Total quantity in top N bid levels.
    pub fn bid_depth(&self, levels: usize) -> Decimal {
        self.bids.values().take(levels).copied().sum()
    }

    /// Total quantity in top N ask levels.
    pub fn ask_depth(&self, levels: usize) -> Decimal {
        self.asks.values().take(levels).copied().sum()
    }

    pub fn last_update_ms(&self) -> u64 {
        self.last_update_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn make_book() -> OrderBook {
        let mut ob = OrderBook::new("BTCUSDT".into(), Exchange::Binance);
        ob.update(
            &[
                (dec!(50000), dec!(1.0)),
                (dec!(49990), dec!(2.0)),
                (dec!(49980), dec!(3.0)),
            ],
            &[
                (dec!(50010), dec!(1.5)),
                (dec!(50020), dec!(2.5)),
                (dec!(50030), dec!(0.5)),
            ],
            1000,
        );
        ob
    }

    #[test]
    fn test_best_bid_ask() {
        let ob = make_book();
        let (bp, bq) = ob.best_bid().unwrap();
        assert_eq!(bp, dec!(50000));
        assert_eq!(bq, dec!(1.0));
        let (ap, aq) = ob.best_ask().unwrap();
        assert_eq!(ap, dec!(50010));
        assert_eq!(aq, dec!(1.5));
    }

    #[test]
    fn test_mid_price_and_spread() {
        let ob = make_book();
        assert_eq!(ob.mid_price().unwrap(), dec!(50005));
        assert_eq!(ob.spread().unwrap(), dec!(10));
    }

    #[test]
    fn test_depth() {
        let ob = make_book();
        assert_eq!(ob.bid_depth(2), dec!(3.0)); // 1.0 + 2.0
        assert_eq!(ob.ask_depth(2), dec!(4.0)); // 1.5 + 2.5
    }

    #[test]
    fn test_remove_zero_qty() {
        let mut ob = make_book();
        // Remove best bid
        ob.update(&[(dec!(50000), dec!(0))], &[], 2000);
        let (bp, _) = ob.best_bid().unwrap();
        assert_eq!(bp, dec!(49990));
    }

    #[test]
    fn test_imbalance_ratio() {
        let mut ob = OrderBook::new("TEST".into(), Exchange::Binance);
        ob.update(
            &[(dec!(100), dec!(10))],
            &[(dec!(101), dec!(10))],
            1,
        );
        // Equal depth => 0.0
        assert!((ob.imbalance_ratio(1) - 0.0).abs() < 1e-10);

        // Bid-heavy
        ob.update(&[(dec!(100), dec!(20))], &[], 2);
        assert!(ob.imbalance_ratio(1) > 0.0);
    }

    #[test]
    fn test_empty_book() {
        let ob = OrderBook::new("EMPTY".into(), Exchange::Binance);
        assert!(ob.best_bid().is_none());
        assert!(ob.best_ask().is_none());
        assert!(ob.mid_price().is_none());
        assert!(ob.spread().is_none());
        assert_eq!(ob.imbalance_ratio(5), 0.0);
    }
}
