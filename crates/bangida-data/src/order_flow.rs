use std::collections::BTreeMap;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use bangida_core::Price;
use crate::ringbuffer::RingBuffer;

// ---------------------------------------------------------------------------
// Cumulative Volume Delta
// ---------------------------------------------------------------------------

/// Tracks the running difference between buy-aggressor and sell-aggressor
/// volume. Internally uses a `RingBuffer` of timestamped deltas so that
/// rolling windows can be computed without scanning the full history.
#[derive(Debug, Clone)]
pub struct CumulativeVolumeDelta {
    /// Time-stamped (timestamp_ms, signed_qty) entries.
    buffer: RingBuffer<(u64, f64)>,
    /// Running total across the entire buffer.
    total_delta: f64,
}

impl CumulativeVolumeDelta {
    /// Create a new tracker with the given buffer capacity (max number of
    /// trades to retain for rolling-window queries).
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: RingBuffer::new(capacity),
            total_delta: 0.0,
        }
    }

    /// Record a trade.
    ///
    /// * `quantity` – unsigned trade size.
    /// * `is_buyer_maker` – if `true` the trade was a sell aggressor (taker
    ///   sold into the bid), so we subtract from delta; otherwise we add.
    /// * `timestamp_ms` – exchange timestamp.
    pub fn on_trade(&mut self, quantity: Decimal, is_buyer_maker: bool, timestamp_ms: u64) {
        let qty_f = quantity.to_f64().unwrap_or(0.0);
        let signed = if is_buyer_maker { -qty_f } else { qty_f };

        // If the buffer is full, the oldest entry is about to be evicted.
        if self.buffer.is_full() {
            if let Some(&(_, old_signed)) = self.buffer.get(0) {
                self.total_delta -= old_signed;
            }
        }

        self.buffer.push((timestamp_ms, signed));
        self.total_delta += signed;
    }

    /// Cumulative delta over the entire buffer.
    #[inline]
    pub fn total(&self) -> f64 {
        self.total_delta
    }

    /// Cumulative delta for the last `window_ms` milliseconds relative to the
    /// most recent trade timestamp. This scans from newest to oldest so it
    /// short-circuits once outside the window.
    pub fn rolling_delta(&self, window_ms: u64) -> f64 {
        if self.buffer.len() == 0 {
            return 0.0;
        }

        let latest_ts = match self.buffer.last() {
            Some(&(ts, _)) => ts,
            None => return 0.0,
        };
        let cutoff = latest_ts.saturating_sub(window_ms);

        let mut sum = 0.0;
        // Walk backwards (newest first) by iterating in reverse logical order.
        for i in (0..self.buffer.len()).rev() {
            if let Some(&(ts, signed)) = self.buffer.get(i) {
                if ts < cutoff {
                    break;
                }
                sum += signed;
            }
        }
        sum
    }
}

// ---------------------------------------------------------------------------
// Volume Profile
// ---------------------------------------------------------------------------

/// Tracks cumulative volume at each price level. Useful for identifying the
/// Point of Control (POC) and the Value Area.
#[derive(Debug, Clone)]
pub struct VolumeProfile {
    /// price -> cumulative volume
    levels: BTreeMap<Decimal, Decimal>,
    total_volume: Decimal,
}

impl VolumeProfile {
    pub fn new() -> Self {
        Self {
            levels: BTreeMap::new(),
            total_volume: Decimal::ZERO,
        }
    }

    /// Record a trade at the given price and quantity.
    pub fn on_trade(&mut self, price: Price, quantity: Decimal) {
        *self.levels.entry(price).or_insert(Decimal::ZERO) += quantity;
        self.total_volume += quantity;
    }

    /// Point of Control: the price level with the highest traded volume.
    pub fn poc(&self) -> Option<Price> {
        self.levels
            .iter()
            .max_by_key(|(_, v)| *v)
            .map(|(p, _)| *p)
    }

    /// Value Area – the price range containing `pct` (e.g. 0.70 for 70 %) of
    /// total volume, expanding outward from the POC.
    ///
    /// Returns `(low_price, high_price)`.
    pub fn value_area(&self, pct: f64) -> Option<(Price, Price)> {
        if self.levels.is_empty() {
            return None;
        }

        let target_vol = self.total_volume.to_f64().unwrap_or(0.0) * pct;

        // Collect into a Vec sorted by price for indexed access.
        let sorted: Vec<(Decimal, Decimal)> =
            self.levels.iter().map(|(p, v)| (*p, *v)).collect();

        // Find the POC index.
        let poc_idx = sorted
            .iter()
            .enumerate()
            .max_by_key(|(_, (_, v))| *v)
            .map(|(i, _)| i)
            .unwrap_or(0);

        let mut lo = poc_idx;
        let mut hi = poc_idx;
        let mut accum = sorted[poc_idx].1.to_f64().unwrap_or(0.0);

        while accum < target_vol {
            let expand_lo = if lo > 0 {
                Some(sorted[lo - 1].1.to_f64().unwrap_or(0.0))
            } else {
                None
            };
            let expand_hi = if hi + 1 < sorted.len() {
                Some(sorted[hi + 1].1.to_f64().unwrap_or(0.0))
            } else {
                None
            };

            match (expand_lo, expand_hi) {
                (Some(lo_vol), Some(hi_vol)) => {
                    if lo_vol >= hi_vol {
                        lo -= 1;
                        accum += lo_vol;
                    } else {
                        hi += 1;
                        accum += hi_vol;
                    }
                }
                (Some(lo_vol), None) => {
                    lo -= 1;
                    accum += lo_vol;
                }
                (None, Some(hi_vol)) => {
                    hi += 1;
                    accum += hi_vol;
                }
                (None, None) => break,
            }
        }

        Some((sorted[lo].0, sorted[hi].0))
    }

    /// Reset the profile (e.g. at session boundary).
    pub fn reset(&mut self) {
        self.levels.clear();
        self.total_volume = Decimal::ZERO;
    }

    /// Total tracked volume.
    pub fn total_volume(&self) -> Decimal {
        self.total_volume
    }
}

impl Default for VolumeProfile {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn cvd_buy_sell() {
        let mut cvd = CumulativeVolumeDelta::new(100);
        // Buy aggressor (is_buyer_maker = false) -> positive delta
        cvd.on_trade(dec!(10), false, 1000);
        assert!((cvd.total() - 10.0).abs() < 1e-10);

        // Sell aggressor (is_buyer_maker = true) -> negative delta
        cvd.on_trade(dec!(5), true, 2000);
        assert!((cvd.total() - 5.0).abs() < 1e-10);
    }

    #[test]
    fn cvd_rolling_window() {
        let mut cvd = CumulativeVolumeDelta::new(100);
        cvd.on_trade(dec!(10), false, 1000);
        cvd.on_trade(dec!(20), false, 3000);
        cvd.on_trade(dec!(5), true, 5000);

        // Rolling 2000ms from latest (5000): includes trades at 5000 and 3000
        let delta = cvd.rolling_delta(2000);
        // 5000: -5, 3000: +20 -> 15
        assert!((delta - 15.0).abs() < 1e-10);
    }

    #[test]
    fn volume_profile_poc() {
        let mut vp = VolumeProfile::new();
        vp.on_trade(dec!(100), dec!(5));
        vp.on_trade(dec!(101), dec!(10));
        vp.on_trade(dec!(102), dec!(3));
        assert_eq!(vp.poc(), Some(dec!(101)));
    }

    #[test]
    fn volume_profile_value_area() {
        let mut vp = VolumeProfile::new();
        vp.on_trade(dec!(100), dec!(10));
        vp.on_trade(dec!(101), dec!(50));
        vp.on_trade(dec!(102), dec!(10));
        vp.on_trade(dec!(103), dec!(5));

        // 70% of total (75) = 52.5 -> POC at 101 (50), expand to include 100 (10) -> 60 > 52.5
        let (lo, hi) = vp.value_area(0.70).unwrap();
        assert_eq!(lo, dec!(100));
        assert_eq!(hi, dec!(101));
    }
}
