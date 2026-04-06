/// Position sizing utilities for scalping strategies.
pub struct PositionSizer;

impl PositionSizer {
    /// Fixed-fractional position sizing.
    ///
    /// Calculates notional position size based on equity, risk percentage,
    /// and stop distance percentage.
    ///
    /// Returns the notional position size (not units).
    pub fn fixed_fractional(equity: f64, risk_pct: f64, stop_distance_pct: f64) -> f64 {
        if stop_distance_pct <= 0.0 {
            return 0.0;
        }
        let risk_amount = equity * risk_pct / 100.0;
        risk_amount / (stop_distance_pct / 100.0)
    }

    /// Kelly Criterion optimal fraction of equity to risk.
    ///
    /// Uses quarter-Kelly for safety, clamped to [0.0, 0.25].
    pub fn kelly(win_rate: f64, avg_win: f64, avg_loss: f64) -> f64 {
        if avg_loss <= 0.0 || avg_win <= 0.0 {
            return 0.0;
        }
        let win_loss_ratio = avg_win / avg_loss;
        let f = win_rate - (1.0 - win_rate) / win_loss_ratio;
        f.clamp(0.0, 0.25)
    }

    /// Volatility-adjusted position sizing.
    ///
    /// Scales position size inversely with ATR. Returns quantity in units.
    ///
    /// ATR is in price units (e.g. $30 for BTC). The risk_amount divided by
    /// ATR gives quantity directly: risking $3 at $30/unit volatility = 0.1 unit.
    /// The `price` parameter is kept for API compatibility / future use.
    pub fn volatility_adjusted(equity: f64, risk_pct: f64, atr: f64, price: f64) -> f64 {
        if atr <= 0.0 || price <= 0.0 {
            return 0.0;
        }
        let risk_amount = equity * risk_pct / 100.0;
        // quantity = risk_amount / (stop_distance_in_price)
        // Using ATR as stop distance proxy.
        risk_amount / atr
    }

    /// Apply exchange minimum notional check.
    ///
    /// Returns `None` if the position is below the exchange minimum notional,
    /// otherwise returns `Some(quantity)`.
    pub fn apply_minimum(quantity: f64, min_notional: f64, price: f64) -> Option<f64> {
        if quantity * price < min_notional {
            None
        } else {
            Some(quantity)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fixed_fractional_basic() {
        // equity=10000, risk 1%, stop distance 0.5%
        // risk_amount = 100, position = 100 / 0.005 = 20000
        let size = PositionSizer::fixed_fractional(10000.0, 1.0, 0.5);
        assert!((size - 20000.0).abs() < 1e-6);
    }

    #[test]
    fn test_fixed_fractional_zero_stop() {
        let size = PositionSizer::fixed_fractional(10000.0, 1.0, 0.0);
        assert_eq!(size, 0.0);
    }

    #[test]
    fn test_fixed_fractional_small_account() {
        // equity=100, risk 2%, stop 1%
        // risk_amount = 2, position = 2 / 0.01 = 200
        let size = PositionSizer::fixed_fractional(100.0, 2.0, 1.0);
        assert!((size - 200.0).abs() < 1e-6);
    }

    #[test]
    fn test_kelly_positive() {
        // 60% win rate, avg_win=2, avg_loss=1
        // f = 0.6 - 0.4/2 = 0.6 - 0.2 = 0.4 -> clamped to 0.25
        let f = PositionSizer::kelly(0.6, 2.0, 1.0);
        assert!((f - 0.25).abs() < 1e-6);
    }

    #[test]
    fn test_kelly_negative_edge() {
        // 30% win rate, avg_win=1, avg_loss=1
        // f = 0.3 - 0.7/1 = -0.4 -> clamped to 0.0
        let f = PositionSizer::kelly(0.3, 1.0, 1.0);
        assert_eq!(f, 0.0);
    }

    #[test]
    fn test_kelly_moderate() {
        // 55% win rate, avg_win=1.2, avg_loss=1.0
        // f = 0.55 - 0.45/1.2 = 0.55 - 0.375 = 0.175
        let f = PositionSizer::kelly(0.55, 1.2, 1.0);
        assert!((f - 0.175).abs() < 1e-6);
    }

    #[test]
    fn test_kelly_zero_loss() {
        let f = PositionSizer::kelly(0.5, 1.0, 0.0);
        assert_eq!(f, 0.0);
    }

    #[test]
    fn test_volatility_adjusted_basic() {
        // equity=10000, risk 1%, atr=50 (price units)
        // risk_amount = $100, quantity = $100 / $50 = 2 units
        // (Risking $100 when each unit moves $50 = 2 units exposure)
        let qty = PositionSizer::volatility_adjusted(10000.0, 1.0, 50.0, 1000.0);
        assert!((qty - 2.0).abs() < 1e-9);
    }

    #[test]
    fn test_volatility_adjusted_zero_atr() {
        let qty = PositionSizer::volatility_adjusted(10000.0, 1.0, 0.0, 1000.0);
        assert_eq!(qty, 0.0);
    }

    #[test]
    fn test_volatility_adjusted_zero_price() {
        let qty = PositionSizer::volatility_adjusted(10000.0, 1.0, 50.0, 0.0);
        assert_eq!(qty, 0.0);
    }

    #[test]
    fn test_apply_minimum_above() {
        // quantity=0.01, price=50000, notional=500, min=10
        let result = PositionSizer::apply_minimum(0.01, 10.0, 50000.0);
        assert_eq!(result, Some(0.01));
    }

    #[test]
    fn test_apply_minimum_below() {
        // quantity=0.0001, price=100, notional=0.01, min=10
        let result = PositionSizer::apply_minimum(0.0001, 10.0, 100.0);
        assert_eq!(result, None);
    }

    #[test]
    fn test_apply_minimum_exact() {
        // quantity=0.1, price=100, notional=10.0, min=10.0
        // 10.0 < 10.0 is false, so Some
        let result = PositionSizer::apply_minimum(0.1, 10.0, 100.0);
        assert_eq!(result, Some(0.1));
    }
}
