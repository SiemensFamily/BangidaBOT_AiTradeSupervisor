use bangida_core::{Price, Side, Signal, Symbol};
use bangida_core::time::now_ms;

/// Extension trait that adds ergonomic constructors to [`Signal`].
pub trait SignalExt {
    /// Create a BUY signal.
    fn buy(symbol: Symbol, strength: f64, source: impl Into<String>) -> Signal;

    /// Create a SELL signal.
    fn sell(symbol: Symbol, strength: f64, source: impl Into<String>) -> Signal;

    /// Attach take-profit and stop-loss levels.
    fn with_targets(self, tp: Price, sl: Price) -> Self;
}

impl SignalExt for Signal {
    fn buy(symbol: Symbol, strength: f64, source: impl Into<String>) -> Signal {
        Signal {
            symbol,
            side: Side::Buy,
            strength: strength.clamp(0.0, 1.0),
            confidence: strength.clamp(0.0, 1.0),
            source: source.into(),
            take_profit: None,
            stop_loss: None,
            timestamp_ms: now_ms(),
        }
    }

    fn sell(symbol: Symbol, strength: f64, source: impl Into<String>) -> Signal {
        Signal {
            symbol,
            side: Side::Sell,
            strength: strength.clamp(0.0, 1.0),
            confidence: strength.clamp(0.0, 1.0),
            source: source.into(),
            take_profit: None,
            stop_loss: None,
            timestamp_ms: now_ms(),
        }
    }

    fn with_targets(mut self, tp: Price, sl: Price) -> Self {
        self.take_profit = Some(tp);
        self.stop_loss = Some(sl);
        self
    }
}
