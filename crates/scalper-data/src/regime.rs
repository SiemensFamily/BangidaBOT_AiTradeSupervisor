use crate::indicators::{ATR, EMA, Indicator};
use scalper_core::types::VolatilityRegime;

/// Detects the current volatility regime by comparing short-term ATR to a longer-term baseline.
#[derive(Debug, Clone)]
pub struct RegimeDetector {
    atr_short: ATR,    // 14-period ATR
    atr_long: EMA,     // 50-period EMA of ATR values (baseline)
    regime: VolatilityRegime,
    atr_count: usize,
}

impl RegimeDetector {
    pub fn new() -> Self {
        Self {
            atr_short: ATR::new(14),
            atr_long: EMA::new(50),
            regime: VolatilityRegime::Normal,
            atr_count: 0,
        }
    }

    /// Feed OHLC data to update the regime.
    pub fn update(&mut self, high: f64, low: f64, prev_close: f64) {
        self.atr_short.update_ohlc(high, low, prev_close);

        if self.atr_short.is_ready() {
            let atr_val = self.atr_short.value();
            self.atr_long.update(atr_val);
            self.atr_count += 1;

            if self.atr_long.is_ready() {
                let baseline = self.atr_long.value();
                if baseline > 0.0 {
                    let ratio = atr_val / baseline;
                    self.regime = if ratio < 0.7 {
                        VolatilityRegime::Ranging
                    } else if ratio <= 1.3 {
                        VolatilityRegime::Normal
                    } else if ratio <= 2.5 {
                        VolatilityRegime::Volatile
                    } else {
                        VolatilityRegime::Extreme
                    };
                }
            }
        }
    }

    /// Current detected regime.
    pub fn regime(&self) -> VolatilityRegime {
        self.regime
    }

    /// Whether both ATR and the baseline EMA have enough data.
    pub fn is_ready(&self) -> bool {
        self.atr_short.is_ready() && self.atr_long.is_ready()
    }

    /// Number of ATR values computed so far (regime ready at ~50).
    pub fn atr_count(&self) -> usize {
        self.atr_count
    }
}

impl Default for RegimeDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_regime_not_ready_initially() {
        let rd = RegimeDetector::new();
        assert!(!rd.is_ready());
        assert!(matches!(rd.regime(), VolatilityRegime::Normal));
    }

    #[test]
    fn test_regime_becomes_ready() {
        let mut rd = RegimeDetector::new();
        // Need 14 bars for ATR + 50 updates for EMA baseline = 64 bars
        for i in 0..80 {
            let base = 100.0 + (i as f64 * 0.1);
            rd.update(base + 1.0, base - 1.0, base);
        }
        assert!(rd.is_ready());
    }

    #[test]
    fn test_stable_regime_is_normal_or_ranging() {
        let mut rd = RegimeDetector::new();
        // Feed steady data: constant range
        for i in 0..80 {
            let base = 100.0;
            let _ = i;
            rd.update(base + 1.0, base - 1.0, base);
        }
        assert!(rd.is_ready());
        let regime = rd.regime();
        // With constant ATR and a well-converged EMA baseline, ratio ~ 1.0
        assert!(
            matches!(regime, VolatilityRegime::Normal | VolatilityRegime::Ranging),
            "Expected Normal or Ranging, got {:?}",
            regime
        );
    }

    #[test]
    fn test_volatile_regime_on_spike() {
        let mut rd = RegimeDetector::new();
        // Build up baseline with small range
        for _ in 0..70 {
            rd.update(101.0, 99.0, 100.0); // TR = 2
        }
        // Now spike the range dramatically
        for _ in 0..20 {
            rd.update(120.0, 80.0, 100.0); // TR = 40
        }
        assert!(rd.is_ready());
        let regime = rd.regime();
        assert!(
            matches!(regime, VolatilityRegime::Volatile | VolatilityRegime::Extreme),
            "Expected Volatile or Extreme after spike, got {:?}",
            regime
        );
    }
}
