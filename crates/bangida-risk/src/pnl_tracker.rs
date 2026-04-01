use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use tracing::debug;

/// Real-time PnL tracking and performance metrics.
///
/// Maintains a running tally of equity, drawdowns, win/loss statistics,
/// and computes performance ratios used by the risk manager and reporting.
#[derive(Debug, Clone)]
pub struct PnlTracker {
    peak_equity: Decimal,
    current_equity: Decimal,
    daily_pnl: Decimal,
    total_trades: u64,
    winning_trades: u64,
    total_pnl: Decimal,
    gross_profit: Decimal,
    gross_loss: Decimal,
    total_fees: Decimal,
}

impl PnlTracker {
    pub fn new(initial_equity: Decimal) -> Self {
        Self {
            peak_equity: initial_equity,
            current_equity: initial_equity,
            daily_pnl: Decimal::ZERO,
            total_trades: 0,
            winning_trades: 0,
            total_pnl: Decimal::ZERO,
            gross_profit: Decimal::ZERO,
            gross_loss: Decimal::ZERO,
            total_fees: Decimal::ZERO,
        }
    }

    /// Record a completed trade with its PnL (before fees) and fee amount.
    pub fn record_trade(&mut self, pnl: Decimal, fees: Decimal) {
        self.total_trades += 1;
        let net_pnl = pnl - fees;
        self.total_pnl += net_pnl;
        self.daily_pnl += net_pnl;
        self.total_fees += fees;

        if pnl > Decimal::ZERO {
            self.winning_trades += 1;
            self.gross_profit += pnl;
        } else if pnl < Decimal::ZERO {
            self.gross_loss += pnl.abs();
        }

        self.current_equity += net_pnl;
        if self.current_equity > self.peak_equity {
            self.peak_equity = self.current_equity;
        }

        debug!(
            %pnl,
            %fees,
            %net_pnl,
            total_trades = self.total_trades,
            %self.current_equity,
            "trade recorded"
        );
    }

    /// Update equity from an external source (e.g., balance snapshot).
    pub fn update_equity(&mut self, equity: Decimal) {
        self.current_equity = equity;
        if equity > self.peak_equity {
            self.peak_equity = equity;
        }
    }

    /// Current drawdown from peak equity, as a percentage (0.0 to 100.0+).
    pub fn drawdown_pct(&self) -> f64 {
        if self.peak_equity.is_zero() {
            return 0.0;
        }
        let dd = (self.peak_equity - self.current_equity) / self.peak_equity;
        dd.to_f64().unwrap_or(0.0) * 100.0
    }

    /// Win rate as a fraction (0.0 to 1.0).
    pub fn win_rate(&self) -> f64 {
        if self.total_trades == 0 {
            return 0.0;
        }
        self.winning_trades as f64 / self.total_trades as f64
    }

    /// Profit factor: gross_profit / gross_loss.
    /// Returns f64::INFINITY if there are no losses, 0.0 if no profits.
    pub fn profit_factor(&self) -> f64 {
        if self.gross_loss.is_zero() {
            if self.gross_profit.is_zero() {
                return 0.0;
            }
            return f64::INFINITY;
        }
        let pf = self.gross_profit / self.gross_loss;
        pf.to_f64().unwrap_or(0.0)
    }

    /// Annualized Sharpe ratio from a slice of periodic returns.
    ///
    /// `returns` should be periodic (e.g., daily) returns as fractions.
    /// Annualization factor defaults to 365 (crypto markets trade every day).
    pub fn sharpe_ratio(returns: &[f64]) -> f64 {
        if returns.len() < 2 {
            return 0.0;
        }
        let n = returns.len() as f64;
        let mean = returns.iter().sum::<f64>() / n;
        let variance = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0);
        let std_dev = variance.sqrt();
        if std_dev == 0.0 {
            return 0.0;
        }
        // Annualize: multiply by sqrt(365) for daily returns in crypto
        (mean / std_dev) * (365.0_f64).sqrt()
    }

    /// Expectancy per trade: (win_rate * avg_win) - (loss_rate * avg_loss).
    ///
    /// Returns the expected dollar value per trade.
    pub fn expectancy(&self) -> f64 {
        if self.total_trades == 0 {
            return 0.0;
        }
        let wr = self.win_rate();
        let losing_trades = self.total_trades - self.winning_trades;

        let avg_win = if self.winning_trades > 0 {
            self.gross_profit.to_f64().unwrap_or(0.0) / self.winning_trades as f64
        } else {
            0.0
        };

        let avg_loss = if losing_trades > 0 {
            self.gross_loss.to_f64().unwrap_or(0.0) / losing_trades as f64
        } else {
            0.0
        };

        (wr * avg_win) - ((1.0 - wr) * avg_loss)
    }

    /// Reset daily PnL counter. Call at UTC midnight.
    pub fn reset_daily(&mut self) {
        self.daily_pnl = Decimal::ZERO;
    }

    // --- Accessors ---

    pub fn peak_equity(&self) -> Decimal {
        self.peak_equity
    }

    pub fn current_equity(&self) -> Decimal {
        self.current_equity
    }

    pub fn daily_pnl(&self) -> Decimal {
        self.daily_pnl
    }

    pub fn total_trades(&self) -> u64 {
        self.total_trades
    }

    pub fn winning_trades(&self) -> u64 {
        self.winning_trades
    }

    pub fn total_pnl(&self) -> Decimal {
        self.total_pnl
    }

    pub fn gross_profit(&self) -> Decimal {
        self.gross_profit
    }

    pub fn gross_loss(&self) -> Decimal {
        self.gross_loss
    }

    pub fn total_fees(&self) -> Decimal {
        self.total_fees
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_record_winning_trade() {
        let mut tracker = PnlTracker::new(dec!(10000));
        tracker.record_trade(dec!(100), dec!(1));
        assert_eq!(tracker.total_trades(), 1);
        assert_eq!(tracker.winning_trades(), 1);
        assert_eq!(tracker.total_pnl(), dec!(99));
        assert_eq!(tracker.current_equity(), dec!(10099));
    }

    #[test]
    fn test_record_losing_trade() {
        let mut tracker = PnlTracker::new(dec!(10000));
        tracker.record_trade(dec!(-50), dec!(1));
        assert_eq!(tracker.total_trades(), 1);
        assert_eq!(tracker.winning_trades(), 0);
        assert_eq!(tracker.gross_loss(), dec!(50));
        assert_eq!(tracker.current_equity(), dec!(9949));
    }

    #[test]
    fn test_drawdown_pct() {
        let mut tracker = PnlTracker::new(dec!(10000));
        tracker.record_trade(dec!(-500), dec!(0));
        // peak=10000, current=9500, dd = 5%
        assert!((tracker.drawdown_pct() - 5.0).abs() < 0.01);
    }

    #[test]
    fn test_win_rate() {
        let mut tracker = PnlTracker::new(dec!(10000));
        tracker.record_trade(dec!(10), dec!(0));
        tracker.record_trade(dec!(-5), dec!(0));
        tracker.record_trade(dec!(10), dec!(0));
        assert!((tracker.win_rate() - 0.6667).abs() < 0.01);
    }

    #[test]
    fn test_profit_factor() {
        let mut tracker = PnlTracker::new(dec!(10000));
        tracker.record_trade(dec!(200), dec!(0));
        tracker.record_trade(dec!(-100), dec!(0));
        // profit_factor = 200/100 = 2.0
        assert!((tracker.profit_factor() - 2.0).abs() < 0.01);
    }

    #[test]
    fn test_sharpe_ratio() {
        let returns = vec![0.01, 0.02, -0.005, 0.015, 0.01];
        let sharpe = PnlTracker::sharpe_ratio(&returns);
        assert!(sharpe > 0.0);
    }

    #[test]
    fn test_expectancy() {
        let mut tracker = PnlTracker::new(dec!(10000));
        tracker.record_trade(dec!(100), dec!(0));
        tracker.record_trade(dec!(120), dec!(0));
        tracker.record_trade(dec!(-50), dec!(0));
        // wr = 2/3, avg_win = 110, avg_loss = 50
        // expectancy = (2/3)*110 - (1/3)*50 = 73.33 - 16.67 = 56.67
        let e = tracker.expectancy();
        assert!((e - 56.67).abs() < 0.1);
    }

    #[test]
    fn test_empty_tracker() {
        let tracker = PnlTracker::new(dec!(10000));
        assert_eq!(tracker.win_rate(), 0.0);
        assert_eq!(tracker.profit_factor(), 0.0);
        assert_eq!(tracker.expectancy(), 0.0);
        assert_eq!(tracker.drawdown_pct(), 0.0);
    }
}
