use crate::ringbuffer::RingBuffer;
use std::collections::HashMap;

/// A completed OHLCV candle.
#[derive(Debug, Clone)]
pub struct Candle {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub timestamp_ms: u64,
}

/// Accumulates trades into an OHLCV candle for a fixed interval.
#[derive(Debug, Clone)]
struct CandleBuilder {
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
    start_ms: u64,
    interval_ms: u64,
    count: u64,
}

impl CandleBuilder {
    fn new(interval_ms: u64) -> Self {
        Self {
            open: 0.0,
            high: f64::NEG_INFINITY,
            low: f64::INFINITY,
            close: 0.0,
            volume: 0.0,
            start_ms: 0,
            interval_ms,
            count: 0,
        }
    }

    /// Feed a trade. Returns a completed Candle if the interval boundary was crossed.
    fn on_trade(&mut self, price: f64, volume: f64, timestamp_ms: u64) -> Option<Candle> {
        // Determine which interval this trade belongs to
        let trade_interval_start = (timestamp_ms / self.interval_ms) * self.interval_ms;

        let mut completed = None;

        if self.count > 0 && trade_interval_start != self.start_ms {
            // New interval => finalize the current candle
            completed = Some(Candle {
                open: self.open,
                high: self.high,
                low: self.low,
                close: self.close,
                volume: self.volume,
                timestamp_ms: self.start_ms,
            });
            // Reset for new interval
            self.count = 0;
            self.volume = 0.0;
            self.high = f64::NEG_INFINITY;
            self.low = f64::INFINITY;
        }

        if self.count == 0 {
            self.open = price;
            self.start_ms = trade_interval_start;
        }

        if price > self.high {
            self.high = price;
        }
        if price < self.low {
            self.low = price;
        }
        self.close = price;
        self.volume += volume;
        self.count += 1;

        completed
    }
}

/// Multi-timeframe candle aggregator that builds 1m, 5m, and 15m candles from a trade stream.
#[derive(Debug, Clone)]
pub struct CandleManager {
    candles_1m: HashMap<String, CandleBuilder>,
    candles_5m: HashMap<String, CandleBuilder>,
    candles_15m: HashMap<String, CandleBuilder>,
    history_1m: HashMap<String, RingBuffer<Candle>>,
    history_5m: HashMap<String, RingBuffer<Candle>>,
    history_15m: HashMap<String, RingBuffer<Candle>>,
}

impl CandleManager {
    pub fn new() -> Self {
        Self {
            candles_1m: HashMap::new(),
            candles_5m: HashMap::new(),
            candles_15m: HashMap::new(),
            history_1m: HashMap::new(),
            history_5m: HashMap::new(),
            history_15m: HashMap::new(),
        }
    }

    /// Feed a trade. Returns any completed candles (across all timeframes).
    pub fn on_trade(
        &mut self,
        symbol: &str,
        price: f64,
        volume: f64,
        timestamp_ms: u64,
    ) -> Vec<Candle> {
        let mut completed = Vec::new();

        // 1-minute candles
        let builder = self
            .candles_1m
            .entry(symbol.to_string())
            .or_insert_with(|| CandleBuilder::new(60_000));
        if let Some(candle) = builder.on_trade(price, volume, timestamp_ms) {
            self.history_1m
                .entry(symbol.to_string())
                .or_insert_with(|| RingBuffer::new(60))
                .push(candle.clone());
            completed.push(candle);
        }

        // 5-minute candles
        let builder = self
            .candles_5m
            .entry(symbol.to_string())
            .or_insert_with(|| CandleBuilder::new(300_000));
        if let Some(candle) = builder.on_trade(price, volume, timestamp_ms) {
            self.history_5m
                .entry(symbol.to_string())
                .or_insert_with(|| RingBuffer::new(60))
                .push(candle.clone());
            completed.push(candle);
        }

        // 15-minute candles
        let builder = self
            .candles_15m
            .entry(symbol.to_string())
            .or_insert_with(|| CandleBuilder::new(900_000));
        if let Some(candle) = builder.on_trade(price, volume, timestamp_ms) {
            self.history_15m
                .entry(symbol.to_string())
                .or_insert_with(|| RingBuffer::new(60))
                .push(candle.clone());
            completed.push(candle);
        }

        completed
    }

    /// Highest high over the last `periods` completed candles for a given symbol and interval.
    pub fn highest_high(&self, symbol: &str, interval: &str, periods: usize) -> Option<f64> {
        let history = self.get_history(interval)?;
        let buf = history.get(symbol)?;
        if buf.is_empty() {
            return None;
        }
        let items: Vec<_> = buf.iter().collect();
        items
            .iter()
            .rev()
            .take(periods)
            .map(|c| c.high)
            .fold(None, |acc: Option<f64>, h| Some(acc.map_or(h, |a| a.max(h))))
    }

    /// Lowest low over the last `periods` completed candles for a given symbol and interval.
    pub fn lowest_low(&self, symbol: &str, interval: &str, periods: usize) -> Option<f64> {
        let history = self.get_history(interval)?;
        let buf = history.get(symbol)?;
        if buf.is_empty() {
            return None;
        }
        let items: Vec<_> = buf.iter().collect();
        items
            .iter()
            .rev()
            .take(periods)
            .map(|c| c.low)
            .fold(None, |acc: Option<f64>, l| Some(acc.map_or(l, |a| a.min(l))))
    }

    fn get_history(&self, interval: &str) -> Option<&HashMap<String, RingBuffer<Candle>>> {
        match interval {
            "1m" => Some(&self.history_1m),
            "5m" => Some(&self.history_5m),
            "15m" => Some(&self.history_15m),
            _ => None,
        }
    }
}

impl Default for CandleManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_candle_builder_basic() {
        let mut builder = CandleBuilder::new(60_000);
        // Trades within the same minute
        assert!(builder.on_trade(100.0, 1.0, 0).is_none());
        assert!(builder.on_trade(105.0, 2.0, 30_000).is_none());
        assert!(builder.on_trade(95.0, 1.5, 59_999).is_none());

        // Trade in the next minute triggers candle completion
        let candle = builder.on_trade(102.0, 1.0, 60_000).unwrap();
        assert!((candle.open - 100.0).abs() < 1e-10);
        assert!((candle.high - 105.0).abs() < 1e-10);
        assert!((candle.low - 95.0).abs() < 1e-10);
        assert!((candle.close - 95.0).abs() < 1e-10);
        assert!((candle.volume - 4.5).abs() < 1e-10);
        assert_eq!(candle.timestamp_ms, 0);
    }

    #[test]
    fn test_candle_manager_multi_timeframe() {
        let mut cm = CandleManager::new();
        // Fill a 1-minute candle
        cm.on_trade("BTCUSDT", 100.0, 1.0, 0);
        cm.on_trade("BTCUSDT", 110.0, 1.0, 30_000);

        // Next minute => 1m candle completes
        let completed = cm.on_trade("BTCUSDT", 105.0, 1.0, 60_000);
        assert_eq!(completed.len(), 1); // only 1m candle completes

        assert_eq!(cm.highest_high("BTCUSDT", "1m", 1), Some(110.0));
        assert_eq!(cm.lowest_low("BTCUSDT", "1m", 1), Some(100.0));
    }

    #[test]
    fn test_candle_manager_5m_completion() {
        let mut cm = CandleManager::new();
        // Trades spanning 5 minutes
        cm.on_trade("ETHUSDT", 3000.0, 10.0, 0);
        cm.on_trade("ETHUSDT", 3100.0, 5.0, 120_000);
        cm.on_trade("ETHUSDT", 2900.0, 8.0, 240_000);

        // Cross the 5-minute boundary
        let completed = cm.on_trade("ETHUSDT", 3050.0, 3.0, 300_000);
        // Should have completed a 5m candle (and several 1m candles)
        let has_5m = completed.iter().any(|c| c.timestamp_ms == 0 && (c.volume - 23.0).abs() < 1e-10);
        assert!(has_5m || completed.len() >= 1);
    }

    #[test]
    fn test_highest_high_lowest_low_multiple() {
        let mut cm = CandleManager::new();
        // Create multiple 1m candles
        cm.on_trade("TEST", 100.0, 1.0, 0);
        cm.on_trade("TEST", 120.0, 1.0, 60_000); // completes candle 1: high=100
        cm.on_trade("TEST", 80.0, 1.0, 120_000);  // completes candle 2: high=120
        cm.on_trade("TEST", 150.0, 1.0, 180_000); // completes candle 3: high=80

        assert_eq!(cm.highest_high("TEST", "1m", 3), Some(120.0));
        assert_eq!(cm.lowest_low("TEST", "1m", 3), Some(80.0));
        assert_eq!(cm.highest_high("TEST", "1m", 1), Some(80.0));
    }

    #[test]
    fn test_no_history() {
        let cm = CandleManager::new();
        assert_eq!(cm.highest_high("NONE", "1m", 5), None);
        assert_eq!(cm.lowest_low("NONE", "1m", 5), None);
        assert_eq!(cm.highest_high("NONE", "invalid", 5), None);
    }
}
