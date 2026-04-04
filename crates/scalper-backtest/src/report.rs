/// Performance metrics for a backtest run.
#[derive(Debug, Clone)]
pub struct BacktestReport {
    pub total_trades: u64,
    pub winning_trades: u64,
    pub losing_trades: u64,
    pub win_rate: f64,
    pub profit_factor: f64,
    pub total_pnl: f64,
    pub total_fees: f64,
    pub net_pnl: f64,
    pub max_drawdown_pct: f64,
    pub sharpe_ratio: f64,
    pub avg_trade_pnl: f64,
    pub avg_win: f64,
    pub avg_loss: f64,
    pub expectancy: f64,
    pub final_equity: f64,
    pub return_pct: f64,
}

/// Builder for accumulating backtest results.
pub struct ReportBuilder {
    starting_equity: f64,
    equity: f64,
    peak_equity: f64,
    max_drawdown_pct: f64,
    total_pnl: f64,
    total_fees: f64,
    wins: u64,
    losses: u64,
    gross_profit: f64,
    gross_loss: f64,
    returns: Vec<f64>, // per-trade returns for Sharpe calculation
}

impl ReportBuilder {
    pub fn new(starting_equity: f64) -> Self {
        Self {
            starting_equity,
            equity: starting_equity,
            peak_equity: starting_equity,
            max_drawdown_pct: 0.0,
            total_pnl: 0.0,
            total_fees: 0.0,
            wins: 0,
            losses: 0,
            gross_profit: 0.0,
            gross_loss: 0.0,
            returns: Vec::new(),
        }
    }

    pub fn record_trade(&mut self, pnl: f64, fee: f64) {
        let net = pnl - fee;
        self.total_pnl += pnl;
        self.total_fees += fee;
        self.equity += net;

        if net > 0.0 {
            self.wins += 1;
            self.gross_profit += net;
        } else {
            self.losses += 1;
            self.gross_loss += net.abs();
        }

        if self.equity > self.peak_equity {
            self.peak_equity = self.equity;
        }

        if self.peak_equity > 0.0 {
            let dd = (self.peak_equity - self.equity) / self.peak_equity * 100.0;
            if dd > self.max_drawdown_pct {
                self.max_drawdown_pct = dd;
            }
        }

        // Store return as percentage of equity at time of trade
        if self.equity > 0.0 {
            self.returns.push(net / self.equity);
        }
    }

    pub fn build(&self) -> BacktestReport {
        let total = self.wins + self.losses;
        let win_rate = if total > 0 {
            self.wins as f64 / total as f64
        } else {
            0.0
        };
        let profit_factor = if self.gross_loss > 0.0 {
            self.gross_profit / self.gross_loss
        } else if self.gross_profit > 0.0 {
            f64::INFINITY
        } else {
            0.0
        };
        let avg_win = if self.wins > 0 {
            self.gross_profit / self.wins as f64
        } else {
            0.0
        };
        let avg_loss = if self.losses > 0 {
            self.gross_loss / self.losses as f64
        } else {
            0.0
        };
        let expectancy = (win_rate * avg_win) - ((1.0 - win_rate) * avg_loss);
        let avg_trade_pnl = if total > 0 {
            (self.total_pnl - self.total_fees) / total as f64
        } else {
            0.0
        };

        // Annualized Sharpe ratio (crypto = 365 trading days)
        let sharpe = if self.returns.len() > 1 {
            let mean: f64 = self.returns.iter().sum::<f64>() / self.returns.len() as f64;
            let variance: f64 = self.returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>()
                / (self.returns.len() - 1) as f64;
            let std_dev = variance.sqrt();
            if std_dev > 0.0 {
                (mean / std_dev) * (365.0_f64).sqrt()
            } else {
                0.0
            }
        } else {
            0.0
        };

        let return_pct = if self.starting_equity > 0.0 {
            (self.equity - self.starting_equity) / self.starting_equity * 100.0
        } else {
            0.0
        };

        BacktestReport {
            total_trades: total,
            winning_trades: self.wins,
            losing_trades: self.losses,
            win_rate,
            profit_factor,
            total_pnl: self.total_pnl,
            total_fees: self.total_fees,
            net_pnl: self.total_pnl - self.total_fees,
            max_drawdown_pct: self.max_drawdown_pct,
            sharpe_ratio: sharpe,
            avg_trade_pnl,
            avg_win,
            avg_loss,
            expectancy,
            final_equity: self.equity,
            return_pct,
        }
    }
}

impl std::fmt::Display for BacktestReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "=== Backtest Report ===")?;
        writeln!(f, "Trades:       {} ({} wins / {} losses)", self.total_trades, self.winning_trades, self.losing_trades)?;
        writeln!(f, "Win Rate:     {:.1}%", self.win_rate * 100.0)?;
        writeln!(f, "Profit Factor: {:.2}", self.profit_factor)?;
        writeln!(f, "Net PnL:      ${:.2}", self.net_pnl)?;
        writeln!(f, "Total Fees:   ${:.2}", self.total_fees)?;
        writeln!(f, "Max Drawdown: {:.1}%", self.max_drawdown_pct)?;
        writeln!(f, "Sharpe Ratio: {:.2}", self.sharpe_ratio)?;
        writeln!(f, "Expectancy:   ${:.4}", self.expectancy)?;
        writeln!(f, "Final Equity: ${:.2} ({:+.1}%)", self.final_equity, self.return_pct)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_with_wins_and_losses() {
        let mut builder = ReportBuilder::new(100.0);
        builder.record_trade(5.0, 0.5);   // win: +4.5
        builder.record_trade(-3.0, 0.5);  // loss: -3.5
        builder.record_trade(4.0, 0.5);   // win: +3.5
        let report = builder.build();
        assert_eq!(report.total_trades, 3);
        assert_eq!(report.winning_trades, 2);
        assert!(report.win_rate > 0.6);
        assert!(report.final_equity > 100.0);
    }

    #[test]
    fn empty_report() {
        let builder = ReportBuilder::new(100.0);
        let report = builder.build();
        assert_eq!(report.total_trades, 0);
        assert_eq!(report.win_rate, 0.0);
        assert_eq!(report.final_equity, 100.0);
    }

    #[test]
    fn drawdown_tracking() {
        let mut builder = ReportBuilder::new(100.0);
        builder.record_trade(10.0, 0.0); // equity = 110
        builder.record_trade(-20.0, 0.0); // equity = 90, dd = 18.2%
        let report = builder.build();
        assert!(report.max_drawdown_pct > 18.0);
    }
}
