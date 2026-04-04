/// Tracks profit/loss, equity curve, and performance metrics.
pub struct PnlTracker {
    #[allow(dead_code)]
    starting_equity: f64,
    current_equity: f64,
    peak_equity: f64,
    daily_pnl: f64,
    total_pnl: f64,
    total_fees: f64,
    total_trades: u64,
    winning_trades: u64,
    gross_profit: f64,
    gross_loss: f64,
}

impl PnlTracker {
    pub fn new(starting_equity: f64) -> Self {
        Self {
            starting_equity,
            current_equity: starting_equity,
            peak_equity: starting_equity,
            daily_pnl: 0.0,
            total_pnl: 0.0,
            total_fees: 0.0,
            total_trades: 0,
            winning_trades: 0,
            gross_profit: 0.0,
            gross_loss: 0.0,
        }
    }

    /// Record a completed trade with its PnL and fees.
    pub fn record_trade(&mut self, pnl: f64, fees: f64) {
        let net = pnl - fees;

        self.current_equity += net;
        if self.current_equity > self.peak_equity {
            self.peak_equity = self.current_equity;
        }

        self.daily_pnl += net;
        self.total_pnl += net;
        self.total_fees += fees;
        self.total_trades += 1;

        if net >= 0.0 {
            self.winning_trades += 1;
            self.gross_profit += net;
        } else {
            self.gross_loss += net.abs();
        }
    }

    /// Current equity.
    pub fn equity(&self) -> f64 {
        self.current_equity
    }

    /// Current drawdown from peak as a percentage.
    pub fn drawdown_pct(&self) -> f64 {
        if self.peak_equity > 0.0 {
            (self.peak_equity - self.current_equity) / self.peak_equity * 100.0
        } else {
            0.0
        }
    }

    /// Win rate as a fraction (0.0 to 1.0). Returns 0 if no trades.
    pub fn win_rate(&self) -> f64 {
        if self.total_trades == 0 {
            return 0.0;
        }
        self.winning_trades as f64 / self.total_trades as f64
    }

    /// Profit factor: gross_profit / gross_loss.
    /// Returns f64::INFINITY if no losses.
    pub fn profit_factor(&self) -> f64 {
        if self.gross_loss == 0.0 {
            return f64::INFINITY;
        }
        self.gross_profit / self.gross_loss
    }

    /// Average winning trade amount. Returns 0 if no winning trades.
    pub fn avg_win(&self) -> f64 {
        if self.winning_trades == 0 {
            return 0.0;
        }
        self.gross_profit / self.winning_trades as f64
    }

    /// Average losing trade amount. Returns 0 if no losing trades.
    pub fn avg_loss(&self) -> f64 {
        let losing_trades = self.total_trades - self.winning_trades;
        if losing_trades == 0 {
            return 0.0;
        }
        self.gross_loss / losing_trades as f64
    }

    /// Expectancy: (win_rate * avg_win) - ((1 - win_rate) * avg_loss).
    pub fn expectancy(&self) -> f64 {
        let wr = self.win_rate();
        (wr * self.avg_win()) - ((1.0 - wr) * self.avg_loss())
    }

    /// Total number of trades recorded.
    pub fn total_trades(&self) -> u64 {
        self.total_trades
    }

    /// Daily PnL.
    pub fn daily_pnl(&self) -> f64 {
        self.daily_pnl
    }

    /// Total cumulative PnL.
    pub fn total_pnl(&self) -> f64 {
        self.total_pnl
    }

    /// Total fees paid.
    pub fn total_fees(&self) -> f64 {
        self.total_fees
    }

    /// Starting equity.
    pub fn starting_equity(&self) -> f64 {
        self.starting_equity
    }

    /// Peak equity reached.
    pub fn peak_equity(&self) -> f64 {
        self.peak_equity
    }

    /// Reset daily PnL counter only.
    pub fn reset_daily(&mut self) {
        self.daily_pnl = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_tracker() {
        let t = PnlTracker::new(1000.0);
        assert_eq!(t.equity(), 1000.0);
        assert_eq!(t.drawdown_pct(), 0.0);
        assert_eq!(t.win_rate(), 0.0);
        assert_eq!(t.total_trades(), 0);
        assert_eq!(t.profit_factor(), f64::INFINITY);
    }

    #[test]
    fn test_record_winning_trade() {
        let mut t = PnlTracker::new(1000.0);
        t.record_trade(50.0, 2.0); // net = 48
        assert_eq!(t.equity(), 1048.0);
        assert_eq!(t.total_trades(), 1);
        assert_eq!(t.win_rate(), 1.0);
        assert!((t.avg_win() - 48.0).abs() < 1e-6);
        assert_eq!(t.avg_loss(), 0.0);
        assert_eq!(t.drawdown_pct(), 0.0);
    }

    #[test]
    fn test_record_losing_trade() {
        let mut t = PnlTracker::new(1000.0);
        t.record_trade(-30.0, 2.0); // net = -32
        assert_eq!(t.equity(), 968.0);
        assert_eq!(t.total_trades(), 1);
        assert_eq!(t.win_rate(), 0.0);
        assert_eq!(t.avg_win(), 0.0);
        assert!((t.avg_loss() - 32.0).abs() < 1e-6);
        // drawdown = (1000 - 968)/1000 * 100 = 3.2%
        assert!((t.drawdown_pct() - 3.2).abs() < 1e-6);
    }

    #[test]
    fn test_mixed_trades_and_metrics() {
        let mut t = PnlTracker::new(1000.0);
        // Win: net = 100 - 5 = 95
        t.record_trade(100.0, 5.0);
        // Loss: net = -50 - 5 = -55
        t.record_trade(-50.0, 5.0);
        // Win: net = 30 - 2 = 28
        t.record_trade(30.0, 2.0);

        assert_eq!(t.total_trades(), 3);
        assert!((t.win_rate() - 2.0 / 3.0).abs() < 1e-6);

        // gross_profit = 95 + 28 = 123
        // gross_loss = 55
        assert!((t.profit_factor() - 123.0 / 55.0).abs() < 1e-6);

        // avg_win = 123/2 = 61.5
        assert!((t.avg_win() - 61.5).abs() < 1e-6);
        // avg_loss = 55/1 = 55
        assert!((t.avg_loss() - 55.0).abs() < 1e-6);

        // expectancy = (2/3 * 61.5) - (1/3 * 55) = 41 - 18.333 = 22.666...
        let expected_expectancy = (2.0 / 3.0) * 61.5 - (1.0 / 3.0) * 55.0;
        assert!((t.expectancy() - expected_expectancy).abs() < 1e-6);

        // equity = 1000 + 95 - 55 + 28 = 1068
        assert!((t.equity() - 1068.0).abs() < 1e-6);
        // peak was 1095 (after first trade), current 1068
        // drawdown = (1095-1068)/1095 * 100
        let expected_dd = (1095.0 - 1068.0) / 1095.0 * 100.0;
        assert!((t.drawdown_pct() - expected_dd).abs() < 1e-6);
    }

    #[test]
    fn test_reset_daily() {
        let mut t = PnlTracker::new(1000.0);
        t.record_trade(50.0, 0.0);
        t.record_trade(-20.0, 0.0);
        t.reset_daily();
        // total_pnl should still reflect everything, but daily resets
        // We don't expose daily_pnl directly, but reset_daily should work without panic
        assert_eq!(t.total_trades(), 2);
        assert!((t.equity() - 1030.0).abs() < 1e-6);
    }

    #[test]
    fn test_zero_fees_trade() {
        let mut t = PnlTracker::new(500.0);
        t.record_trade(10.0, 0.0);
        assert_eq!(t.equity(), 510.0);
        assert_eq!(t.win_rate(), 1.0);
    }

    #[test]
    fn test_breakeven_trade_counts_as_win() {
        let mut t = PnlTracker::new(500.0);
        t.record_trade(5.0, 5.0); // net = 0, counts as win (>= 0)
        assert_eq!(t.win_rate(), 1.0);
        assert_eq!(t.equity(), 500.0);
    }
}
