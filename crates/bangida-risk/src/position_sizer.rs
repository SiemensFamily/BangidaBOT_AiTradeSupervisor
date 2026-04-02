use rust_decimal::Decimal;
use bangida_core::Quantity;
use tracing::debug;

/// Position sizing utilities for computing trade quantities
/// given account equity, risk parameters, and volatility.
pub struct PositionSizer;

impl PositionSizer {
    /// Fixed-fractional position sizing.
    ///
    /// Risks `risk_pct` of equity per trade, dividing by the stop distance
    /// to determine the maximum position size.
    ///
    /// `stop_distance_pct` is expressed as a fraction (e.g., 0.005 = 0.5%).
    pub fn fixed_fractional(
        equity: Decimal,
        risk_pct: f64,
        stop_distance_pct: f64,
    ) -> Quantity {
        if stop_distance_pct <= 0.0 || risk_pct <= 0.0 {
            debug!(
                risk_pct,
                stop_distance_pct,
                "invalid risk params, returning zero quantity"
            );
            return Decimal::ZERO;
        }

        let risk_amount = equity * Decimal::try_from(risk_pct).unwrap_or(Decimal::ZERO);
        let stop_dec = Decimal::try_from(stop_distance_pct).unwrap_or(Decimal::ONE);
        let qty = risk_amount / stop_dec;

        debug!(
            %equity,
            risk_pct,
            stop_distance_pct,
            %qty,
            "fixed_fractional position size"
        );

        qty
    }

    /// Kelly criterion position sizing.
    ///
    /// Returns the optimal fraction of capital to risk:
    ///   f* = W - (1 - W) / R
    /// where W = win_rate and R = avg_win / avg_loss.
    ///
    /// Returns 0.0 if inputs are invalid or the Kelly fraction is negative.
    pub fn kelly(win_rate: f64, avg_win: f64, avg_loss: f64) -> f64 {
        if avg_loss <= 0.0 || avg_win <= 0.0 || win_rate <= 0.0 || win_rate >= 1.0 {
            return 0.0;
        }

        let r = avg_win / avg_loss;
        let kelly_f = win_rate - (1.0 - win_rate) / r;

        debug!(win_rate, avg_win, avg_loss, r, kelly_f, "kelly fraction");

        kelly_f.max(0.0)
    }

    /// Volatility-adjusted position sizing using ATR (Average True Range).
    ///
    /// Computes position size as (equity * risk_pct) / ATR so that more
    /// volatile instruments get smaller positions.
    pub fn volatility_adjusted(
        equity: Decimal,
        atr: Decimal,
        risk_pct: f64,
    ) -> Quantity {
        if atr.is_zero() || risk_pct <= 0.0 {
            debug!(%atr, risk_pct, "invalid volatility params, returning zero quantity");
            return Decimal::ZERO;
        }

        let risk_amount = equity * Decimal::try_from(risk_pct).unwrap_or(Decimal::ZERO);
        let qty = risk_amount / atr;

        debug!(%equity, %atr, risk_pct, %qty, "volatility_adjusted position size");

        qty
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_fixed_fractional() {
        let qty = PositionSizer::fixed_fractional(dec!(10000), 0.01, 0.005);
        // risk = 100, stop = 0.005, qty = 20000
        assert_eq!(qty, dec!(20000));
    }

    #[test]
    fn test_fixed_fractional_zero_stop() {
        let qty = PositionSizer::fixed_fractional(dec!(10000), 0.01, 0.0);
        assert_eq!(qty, Decimal::ZERO);
    }

    #[test]
    fn test_kelly_basic() {
        // win_rate=0.6, avg_win=1.5, avg_loss=1.0 => K = 0.6 - 0.4/1.5 = 0.3333..
        let k = PositionSizer::kelly(0.6, 1.5, 1.0);
        assert!((k - 0.3333).abs() < 0.01);
    }

    #[test]
    fn test_kelly_negative_returns_zero() {
        // win_rate=0.3, avg_win=0.5, avg_loss=1.0 => K = 0.3 - 0.7/0.5 = -1.1
        let k = PositionSizer::kelly(0.3, 0.5, 1.0);
        assert_eq!(k, 0.0);
    }

    #[test]
    fn test_volatility_adjusted() {
        let qty = PositionSizer::volatility_adjusted(dec!(10000), dec!(50), 0.01);
        // risk = 100, atr = 50, qty = 2
        assert_eq!(qty, dec!(2));
    }
}
