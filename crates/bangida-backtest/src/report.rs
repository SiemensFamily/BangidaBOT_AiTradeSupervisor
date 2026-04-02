use bangida_risk::PnlTracker;
use rust_decimal::prelude::ToPrimitive;

use crate::engine::BacktestReport;

/// Generate a complete backtest report from a PnL tracker and equity curve.
pub fn generate_report(
    tracker: &PnlTracker,
    equity_curve: &[(u64, f64)],
) -> BacktestReport {
    let total_pnl = tracker.total_pnl().to_f64().unwrap_or(0.0);

    // Compute periodic returns from equity curve
    let returns = compute_returns(equity_curve);
    let sharpe = PnlTracker::sharpe_ratio(&returns);
    let sortino = compute_sortino(&returns);
    let max_drawdown = compute_max_drawdown(equity_curve);

    BacktestReport {
        total_trades: tracker.total_trades(),
        winning_trades: tracker.winning_trades(),
        total_pnl,
        max_drawdown,
        sharpe,
        sortino,
        profit_factor: tracker.profit_factor(),
        win_rate: tracker.win_rate(),
        expectancy: tracker.expectancy(),
        equity_curve: equity_curve.to_vec(),
    }
}

/// Print a formatted report to the console.
pub fn print_report(report: &BacktestReport) {
    println!("╔══════════════════════════════════════════╗");
    println!("║         BACKTEST PERFORMANCE REPORT      ║");
    println!("╠══════════════════════════════════════════╣");
    println!("║ Total Trades:      {:>20} ║", report.total_trades);
    println!("║ Winning Trades:    {:>20} ║", report.winning_trades);
    println!("║ Win Rate:          {:>19.2}% ║", report.win_rate * 100.0);
    println!("║ Total PnL:         {:>19.2}$ ║", report.total_pnl);
    println!("║ Profit Factor:     {:>20.3} ║", report.profit_factor);
    println!("║ Expectancy:        {:>19.2}$ ║", report.expectancy);
    println!("║ Max Drawdown:      {:>19.2}% ║", report.max_drawdown * 100.0);
    println!("║ Sharpe Ratio:      {:>20.3} ║", report.sharpe);
    println!("║ Sortino Ratio:     {:>20.3} ║", report.sortino);
    println!("╚══════════════════════════════════════════╝");
}

/// Compute simple returns from an equity curve.
fn compute_returns(equity_curve: &[(u64, f64)]) -> Vec<f64> {
    if equity_curve.len() < 2 {
        return Vec::new();
    }
    equity_curve
        .windows(2)
        .filter_map(|w| {
            let prev = w[0].1;
            let curr = w[1].1;
            if prev != 0.0 {
                Some((curr - prev) / prev)
            } else {
                None
            }
        })
        .collect()
}

/// Compute the Sortino ratio (like Sharpe but only penalizes downside volatility).
fn compute_sortino(returns: &[f64]) -> f64 {
    if returns.len() < 2 {
        return 0.0;
    }
    let n = returns.len() as f64;
    let mean = returns.iter().sum::<f64>() / n;

    // Downside deviation: only negative returns count
    let downside_variance = returns
        .iter()
        .map(|r| if *r < 0.0 { r.powi(2) } else { 0.0 })
        .sum::<f64>()
        / (n - 1.0);

    let downside_dev = downside_variance.sqrt();
    if downside_dev == 0.0 {
        return 0.0;
    }

    // Annualize
    (mean / downside_dev) * (365.0_f64).sqrt()
}

/// Compute the maximum drawdown percentage from an equity curve.
fn compute_max_drawdown(equity_curve: &[(u64, f64)]) -> f64 {
    if equity_curve.is_empty() {
        return 0.0;
    }
    let mut peak = equity_curve[0].1;
    let mut max_dd = 0.0_f64;

    for &(_, equity) in equity_curve {
        if equity > peak {
            peak = equity;
        }
        if peak > 0.0 {
            let dd = (peak - equity) / peak;
            if dd > max_dd {
                max_dd = dd;
            }
        }
    }
    max_dd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_returns() {
        let curve = vec![(0, 100.0), (1, 110.0), (2, 105.0)];
        let returns = compute_returns(&curve);
        assert_eq!(returns.len(), 2);
        assert!((returns[0] - 0.10).abs() < 0.001);
        assert!((returns[1] - (-0.04545)).abs() < 0.001);
    }

    #[test]
    fn test_max_drawdown() {
        let curve = vec![(0, 100.0), (1, 120.0), (2, 90.0), (3, 110.0)];
        let dd = compute_max_drawdown(&curve);
        // Peak 120, trough 90 => dd = 30/120 = 0.25
        assert!((dd - 0.25).abs() < 0.001);
    }

    #[test]
    fn test_sortino_positive_returns() {
        let returns = vec![0.01, 0.02, 0.015, 0.01, 0.005];
        let s = compute_sortino(&returns);
        // All positive returns => downside dev = 0 => sortino = 0
        assert_eq!(s, 0.0);
    }

    #[test]
    fn test_sortino_mixed_returns() {
        let returns = vec![0.01, -0.02, 0.015, -0.005, 0.01];
        let s = compute_sortino(&returns);
        assert!(s != 0.0);
    }
}
