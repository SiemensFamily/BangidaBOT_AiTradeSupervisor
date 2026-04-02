use std::sync::Mutex;

use bangida_core::config::StatArbConfig;
use bangida_core::Signal;
use tracing::debug;

use crate::signal::SignalExt;
use crate::traits::{MarketContext, Strategy};

// ---------------------------------------------------------------------------
// Rolling statistics helper
// ---------------------------------------------------------------------------

/// Fixed-capacity ring buffer that tracks running mean and variance.
struct RollingStats {
    buf: Vec<f64>,
    capacity: usize,
    head: usize,
    count: usize,
    sum: f64,
    sum_sq: f64,
}

impl RollingStats {
    fn new(capacity: usize) -> Self {
        Self {
            buf: vec![0.0; capacity],
            capacity,
            head: 0,
            count: 0,
            sum: 0.0,
            sum_sq: 0.0,
        }
    }

    fn push(&mut self, value: f64) {
        if self.count == self.capacity {
            // Evict oldest value.
            let old = self.buf[self.head];
            self.sum -= old;
            self.sum_sq -= old * old;
        } else {
            self.count += 1;
        }
        self.buf[self.head] = value;
        self.sum += value;
        self.sum_sq += value * value;
        self.head = (self.head + 1) % self.capacity;
    }

    fn mean(&self) -> f64 {
        if self.count == 0 {
            return 0.0;
        }
        self.sum / self.count as f64
    }

    fn std_dev(&self) -> f64 {
        if self.count < 2 {
            return 0.0;
        }
        let n = self.count as f64;
        let variance = (self.sum_sq / n) - (self.mean() * self.mean());
        if variance <= 0.0 {
            0.0
        } else {
            variance.sqrt()
        }
    }

    fn is_ready(&self) -> bool {
        self.count == self.capacity
    }
}

// ---------------------------------------------------------------------------
// Strategy
// ---------------------------------------------------------------------------

/// Cross-exchange statistical arbitrage (default weight 0.25).
///
/// Maintains a rolling window of Binance-vs-Bybit spread and fires when the
/// z-score exceeds a configurable threshold.  For now we emit a single-legged
/// directional signal on the primary exchange.
pub struct StatArbStrategy {
    entry_z_score: f64,
    min_spread_pct: f64,
    weight: f64,
    /// Interior mutability so `evaluate` can update the rolling window even
    /// though the `Strategy` trait takes `&self`.
    rolling: Mutex<RollingStats>,
}

impl StatArbStrategy {
    pub fn new(cfg: &StatArbConfig) -> Self {
        // Window capacity: one sample per second * configured seconds.
        let capacity = cfg.spread_window_seconds.max(10) as usize;
        Self {
            entry_z_score: cfg.entry_z_score,
            min_spread_pct: cfg.min_spread_pct,
            weight: cfg.weight,
            rolling: Mutex::new(RollingStats::new(capacity)),
        }
    }

    /// Feed the latest Binance-vs-Bybit spread into the rolling window and
    /// return the current z-score (if the window is full).
    fn update_and_zscore(&self, spread_pct: f64) -> Option<f64> {
        let mut rs = self.rolling.lock().unwrap();
        rs.push(spread_pct);
        if !rs.is_ready() {
            return None;
        }
        let std = rs.std_dev();
        if std < 1e-12 {
            return None;
        }
        Some((spread_pct - rs.mean()) / std)
    }
}

impl Strategy for StatArbStrategy {
    fn name(&self) -> &str {
        "stat_arb"
    }

    fn evaluate(&self, ctx: &MarketContext) -> Option<Signal> {
        // The MarketContext currently gives a single mid_price. In a multi-
        // exchange setup the caller would populate `microprice` with one
        // exchange price and `mid_price` with the other.  We treat
        //   spread_pct = (microprice - mid_price) / mid_price
        // as the cross-exchange spread proxy (Binance mid vs Bybit mid).
        let mid = ctx.mid_price.to_f64()?;
        let micro = ctx.microprice.to_f64()?;
        if mid.abs() < 1e-12 {
            return None;
        }
        let spread_pct = (micro - mid) / mid;

        let z = self.update_and_zscore(spread_pct)?;

        if z.abs() < self.entry_z_score {
            return None;
        }
        if spread_pct.abs() < self.min_spread_pct {
            return None;
        }

        let strength = ((z.abs() - self.entry_z_score) / self.entry_z_score).clamp(0.0, 1.0);

        if z > 0.0 {
            // microprice (Binance) > mid_price (Bybit) → short primary
            debug!(z, spread_pct, strength, "stat_arb: SHORT signal (Binance > Bybit)");
            Some(Signal::sell(ctx.symbol.clone(), strength, "stat_arb"))
        } else {
            // microprice (Binance) < mid_price (Bybit) → long primary
            debug!(z, spread_pct, strength, "stat_arb: LONG signal (Bybit > Binance)");
            Some(Signal::buy(ctx.symbol.clone(), strength, "stat_arb"))
        }
    }

    fn weight(&self) -> f64 {
        self.weight
    }
}

use rust_decimal::prelude::ToPrimitive;
