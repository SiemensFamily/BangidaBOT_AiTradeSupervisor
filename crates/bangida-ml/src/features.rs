use serde::{Deserialize, Serialize};
use tracing::debug;

/// Feature vector for ML model inference.
///
/// Each field represents a normalized or raw feature extracted from
/// current market state. This struct is the bridge between market data
/// and the ML prediction pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureVector {
    /// Order book imbalance ratio (-1.0 to 1.0).
    pub ob_imbalance: f64,
    /// Spread in basis points.
    pub spread_bps: f64,
    /// RSI (0-100).
    pub rsi: f64,
    /// MACD histogram value.
    pub macd_hist: f64,
    /// 1-minute cumulative volume delta.
    pub cvd_1m: f64,
    /// Current volume / average volume ratio.
    pub volume_ratio: f64,
    /// Current funding rate (annualized or per-period).
    pub funding_rate: f64,
    /// VWAP deviation: (price - vwap) / vwap.
    pub vwap_deviation: f64,
    /// Fast EMA / slow EMA ratio.
    pub ema_ratio: f64,
    /// Bollinger band width as a fraction of mid price.
    pub bb_width: f64,
    /// Position of price within Bollinger bands (0.0 = lower, 1.0 = upper).
    pub bb_position: f64,
    /// Micro-price deviation from mid-price.
    pub microprice_deviation: f64,
}

impl FeatureVector {
    /// Extract features from individual market data parameters.
    ///
    /// This takes individual parameters rather than a MarketContext to avoid
    /// circular dependencies between bangida-ml and bangida-strategy.
    #[allow(clippy::too_many_arguments)]
    pub fn extract(
        ob_imbalance: f64,
        spread: f64,
        mid_price: f64,
        rsi: f64,
        macd_hist: f64,
        cvd_1m: f64,
        volume_1s: f64,
        avg_volume_60s: f64,
        funding_rate: f64,
        vwap: f64,
        ema_fast: f64,
        ema_slow: f64,
        bb_upper: f64,
        bb_lower: f64,
        bb_middle: f64,
        microprice: f64,
    ) -> Self {
        let spread_bps = if mid_price > 0.0 {
            (spread / mid_price) * 10_000.0
        } else {
            0.0
        };

        let volume_ratio = if avg_volume_60s > 0.0 {
            volume_1s / avg_volume_60s
        } else {
            0.0
        };

        let vwap_deviation = if vwap > 0.0 {
            (mid_price - vwap) / vwap
        } else {
            0.0
        };

        let ema_ratio = if ema_slow > 0.0 {
            ema_fast / ema_slow
        } else {
            1.0
        };

        let bb_width = if bb_middle > 0.0 {
            (bb_upper - bb_lower) / bb_middle
        } else {
            0.0
        };

        let bb_position = if (bb_upper - bb_lower).abs() > f64::EPSILON {
            (mid_price - bb_lower) / (bb_upper - bb_lower)
        } else {
            0.5
        };

        let microprice_deviation = if mid_price > 0.0 {
            (microprice - mid_price) / mid_price
        } else {
            0.0
        };

        let fv = Self {
            ob_imbalance,
            spread_bps,
            rsi,
            macd_hist,
            cvd_1m,
            volume_ratio,
            funding_rate,
            vwap_deviation,
            ema_ratio,
            bb_width,
            bb_position,
            microprice_deviation,
        };

        debug!(?fv, "features extracted");
        fv
    }

    /// Convert to a flat array suitable for model input.
    pub fn to_array(&self) -> [f64; 12] {
        [
            self.ob_imbalance,
            self.spread_bps,
            self.rsi,
            self.macd_hist,
            self.cvd_1m,
            self.volume_ratio,
            self.funding_rate,
            self.vwap_deviation,
            self.ema_ratio,
            self.bb_width,
            self.bb_position,
            self.microprice_deviation,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_basic() {
        let fv = FeatureVector::extract(
            0.5,      // ob_imbalance
            10.0,     // spread
            50000.0,  // mid_price
            55.0,     // rsi
            0.1,      // macd_hist
            100.0,    // cvd_1m
            50.0,     // volume_1s
            30.0,     // avg_volume_60s
            0.0001,   // funding_rate
            49990.0,  // vwap
            50010.0,  // ema_fast
            50000.0,  // ema_slow
            51000.0,  // bb_upper
            49000.0,  // bb_lower
            50000.0,  // bb_middle
            50005.0,  // microprice
        );

        assert!((fv.spread_bps - 2.0).abs() < 0.01); // 10/50000 * 10000 = 2
        assert!((fv.volume_ratio - 1.6667).abs() < 0.01);
        assert!((fv.ema_ratio - 1.0002).abs() < 0.001);
        assert_eq!(fv.ob_imbalance, 0.5);
    }

    #[test]
    fn test_to_array() {
        let fv = FeatureVector::extract(
            0.0, 0.0, 100.0, 50.0, 0.0, 0.0, 0.0, 0.0, 0.0, 100.0,
            100.0, 100.0, 110.0, 90.0, 100.0, 100.0,
        );
        let arr = fv.to_array();
        assert_eq!(arr.len(), 12);
    }

    #[test]
    fn test_zero_mid_price() {
        let fv = FeatureVector::extract(
            0.0, 0.0, 0.0, 50.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        );
        assert_eq!(fv.spread_bps, 0.0);
        assert_eq!(fv.vwap_deviation, 0.0);
    }
}
