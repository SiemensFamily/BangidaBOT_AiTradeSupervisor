use serde::{Deserialize, Serialize};

use bangida_core::{Kline, Price, Quantity, Symbol};

/// Supported candle intervals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CandleInterval {
    /// 1 second
    S1,
    /// 5 seconds
    S5,
    /// 15 seconds
    S15,
    /// 1 minute
    M1,
}

impl CandleInterval {
    /// Duration in milliseconds.
    #[inline]
    pub fn as_millis(&self) -> u64 {
        match self {
            CandleInterval::S1 => 1_000,
            CandleInterval::S5 => 5_000,
            CandleInterval::S15 => 15_000,
            CandleInterval::M1 => 60_000,
        }
    }
}

/// Builds real-time candles from incoming trade events.
#[derive(Debug, Clone)]
pub struct CandleAggregator {
    symbol: Symbol,
    interval: CandleInterval,
    interval_ms: u64,
    current: Option<CandleState>,
}

#[derive(Debug, Clone)]
struct CandleState {
    open: Price,
    high: Price,
    low: Price,
    close: Price,
    volume: Quantity,
    trade_count: u64,
    open_time_ms: u64,
    close_time_ms: u64,
}

impl CandleAggregator {
    /// Create a new aggregator for the given symbol and interval.
    pub fn new(symbol: Symbol, interval: CandleInterval) -> Self {
        Self {
            symbol,
            interval,
            interval_ms: interval.as_millis(),
            current: None,
        }
    }

    /// Returns the candle interval.
    pub fn interval(&self) -> CandleInterval {
        self.interval
    }

    /// Feed a trade into the aggregator. Returns a completed [`Kline`] when the
    /// candle's time window closes (i.e. the incoming trade belongs to the next
    /// interval).
    pub fn on_trade(
        &mut self,
        price: Price,
        quantity: Quantity,
        timestamp_ms: u64,
    ) -> Option<Kline> {
        let candle_open = self.align_timestamp(timestamp_ms);
        let candle_close = candle_open + self.interval_ms - 1;

        // Check if the trade belongs to a new interval.
        let completed = if let Some(ref state) = self.current {
            if candle_open != state.open_time_ms {
                // The current candle is finished; emit it.
                Some(self.state_to_kline(state))
            } else {
                None
            }
        } else {
            None
        };

        // If we emitted a candle (or there's no current state), start fresh or update.
        if completed.is_some() || self.current.is_none() {
            if candle_open != self.current.as_ref().map_or(0, |s| s.open_time_ms)
                || self.current.is_none()
            {
                self.current = Some(CandleState {
                    open: price,
                    high: price,
                    low: price,
                    close: price,
                    volume: quantity,
                    trade_count: 1,
                    open_time_ms: candle_open,
                    close_time_ms: candle_close,
                });
                return completed;
            }
        }

        // Update the current candle.
        if let Some(ref mut state) = self.current {
            if price > state.high {
                state.high = price;
            }
            if price < state.low {
                state.low = price;
            }
            state.close = price;
            state.volume += quantity;
            state.trade_count += 1;
        }

        completed
    }

    /// Force-close the current candle (e.g. on disconnect).
    pub fn flush(&mut self) -> Option<Kline> {
        self.current.take().map(|state| self.state_to_kline(&state))
    }

    /// Align a timestamp to the start of its interval window.
    #[inline]
    fn align_timestamp(&self, ts: u64) -> u64 {
        ts - (ts % self.interval_ms)
    }

    fn state_to_kline(&self, state: &CandleState) -> Kline {
        Kline {
            symbol: self.symbol.clone(),
            open: state.open,
            high: state.high,
            low: state.low,
            close: state.close,
            volume: state.volume,
            open_time_ms: state.open_time_ms,
            close_time_ms: state.close_time_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn candle_closes_on_new_interval() {
        let sym = Symbol::new("BTCUSDT");
        let mut agg = CandleAggregator::new(sym, CandleInterval::S1);

        // Trades in the 0-999ms window
        assert!(agg.on_trade(dec!(100), dec!(1), 0).is_none());
        assert!(agg.on_trade(dec!(105), dec!(2), 500).is_none());
        assert!(agg.on_trade(dec!(95), dec!(1), 900).is_none());

        // Trade in the 1000-1999ms window closes the first candle
        let kline = agg.on_trade(dec!(101), dec!(1), 1000);
        assert!(kline.is_some());
        let k = kline.unwrap();
        assert_eq!(k.open, dec!(100));
        assert_eq!(k.high, dec!(105));
        assert_eq!(k.low, dec!(95));
        assert_eq!(k.close, dec!(95));
        assert_eq!(k.volume, dec!(4));
    }
}
