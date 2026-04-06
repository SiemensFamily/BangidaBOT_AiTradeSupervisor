use crate::ringbuffer::RingBuffer;

/// Tracks order flow metrics: CVD, volume ratio, and liquidations.
#[derive(Debug, Clone)]
pub struct OrderFlowTracker {
    cvd: f64,
    /// Short-window CVD that resets each minute for directional checks.
    cvd_short: f64,
    buy_volume_1m: RingBuffer<f64>,
    sell_volume_1m: RingBuffer<f64>,
    current_buy_vol: f64,
    current_sell_vol: f64,
    liquidation_volume_1m: f64,
    last_liquidation_reset_ms: u64,
}

impl OrderFlowTracker {
    pub fn new() -> Self {
        Self {
            cvd: 0.0,
            cvd_short: 0.0,
            buy_volume_1m: RingBuffer::new(60),  // keep 60 samples
            sell_volume_1m: RingBuffer::new(60),
            current_buy_vol: 0.0,
            current_sell_vol: 0.0,
            liquidation_volume_1m: 0.0,
            last_liquidation_reset_ms: 0,
        }
    }

    /// Process an incoming trade.
    /// `is_buyer_maker` means the buyer placed the resting order,
    /// so the trade was initiated by a seller (aggressive sell).
    pub fn on_trade(&mut self, _price: f64, qty: f64, is_buyer_maker: bool) {
        if is_buyer_maker {
            // Aggressive sell
            self.cvd -= qty;
            self.cvd_short -= qty;
            self.current_sell_vol += qty;
        } else {
            // Aggressive buy
            self.cvd += qty;
            self.cvd_short += qty;
            self.current_buy_vol += qty;
        }
    }

    /// Record a liquidation event.
    pub fn on_liquidation(&mut self, qty: f64, now_ms: u64) {
        // If more than 60s since last reset, clear first
        if now_ms.saturating_sub(self.last_liquidation_reset_ms) >= 60_000 {
            self.liquidation_volume_1m = 0.0;
            self.last_liquidation_reset_ms = now_ms;
        }
        self.liquidation_volume_1m += qty;
    }

    /// Cumulative Volume Delta (all-time).
    pub fn cvd(&self) -> f64 {
        self.cvd
    }

    /// Short-window CVD (resets each minute). Better for directional
    /// confirmation since all-time CVD can drift far from zero.
    pub fn cvd_short(&self) -> f64 {
        self.cvd_short
    }

    /// Total volume in the current accumulating minute (buy + sell).
    pub fn current_volume(&self) -> f64 {
        self.current_buy_vol + self.current_sell_vol
    }

    /// Average per-minute total volume over the historical window.
    /// Returns 0.0 if no history.
    pub fn avg_volume_60s(&self) -> f64 {
        let total: f64 = self.buy_volume_1m.iter().sum::<f64>()
            + self.sell_volume_1m.iter().sum::<f64>();
        let count = self.buy_volume_1m.len();
        if count == 0 { 0.0 } else { total / count as f64 }
    }

    /// Buy volume / sell volume ratio over the sampled window.
    /// Returns f64::INFINITY if no sell volume, 0.0 if no data.
    pub fn volume_ratio(&self) -> f64 {
        let total_buy: f64 = self.buy_volume_1m.iter().sum::<f64>() + self.current_buy_vol;
        let total_sell: f64 = self.sell_volume_1m.iter().sum::<f64>() + self.current_sell_vol;
        if total_sell == 0.0 {
            if total_buy == 0.0 {
                return 0.0;
            }
            return f64::INFINITY;
        }
        total_buy / total_sell
    }

    /// Liquidation volume in the last minute window.
    pub fn liquidation_volume_1m(&self) -> f64 {
        self.liquidation_volume_1m
    }

    /// Flush the current accumulator into the ring buffers and reset for a new minute.
    pub fn reset_minute(&mut self, now_ms: u64) {
        self.buy_volume_1m.push(self.current_buy_vol);
        self.sell_volume_1m.push(self.current_sell_vol);
        self.current_buy_vol = 0.0;
        self.current_sell_vol = 0.0;
        self.cvd_short = 0.0;

        // Reset liquidation if more than 60s
        if now_ms.saturating_sub(self.last_liquidation_reset_ms) >= 60_000 {
            self.liquidation_volume_1m = 0.0;
            self.last_liquidation_reset_ms = now_ms;
        }
    }
}

impl Default for OrderFlowTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cvd_tracking() {
        let mut oft = OrderFlowTracker::new();
        oft.on_trade(100.0, 5.0, false); // aggressive buy +5
        oft.on_trade(101.0, 3.0, true);  // aggressive sell -3
        assert!((oft.cvd() - 2.0).abs() < 1e-10);
    }

    #[test]
    fn test_volume_ratio() {
        let mut oft = OrderFlowTracker::new();
        oft.on_trade(100.0, 10.0, false); // buy 10
        oft.on_trade(101.0, 5.0, true);   // sell 5
        assert!((oft.volume_ratio() - 2.0).abs() < 1e-10);
    }

    #[test]
    fn test_volume_ratio_no_data() {
        let oft = OrderFlowTracker::new();
        assert_eq!(oft.volume_ratio(), 0.0);
    }

    #[test]
    fn test_liquidation() {
        let mut oft = OrderFlowTracker::new();
        oft.on_liquidation(100.0, 1000);
        oft.on_liquidation(50.0, 1500);
        assert!((oft.liquidation_volume_1m() - 150.0).abs() < 1e-10);
    }

    #[test]
    fn test_liquidation_reset_after_60s() {
        let mut oft = OrderFlowTracker::new();
        oft.on_liquidation(100.0, 1000);
        // 61 seconds later
        oft.on_liquidation(50.0, 62_000);
        // Should have reset first, then added 50
        assert!((oft.liquidation_volume_1m() - 50.0).abs() < 1e-10);
    }

    #[test]
    fn test_reset_minute() {
        let mut oft = OrderFlowTracker::new();
        oft.on_trade(100.0, 10.0, false);
        oft.on_trade(100.0, 5.0, true);
        oft.reset_minute(60_000);
        // Current accumulators should be zero
        assert!((oft.current_buy_vol).abs() < 1e-10);
        assert!((oft.current_sell_vol).abs() < 1e-10);
        // But ring buffers have the data
        assert_eq!(oft.buy_volume_1m.len(), 1);
        assert_eq!(oft.sell_volume_1m.len(), 1);
    }
}
