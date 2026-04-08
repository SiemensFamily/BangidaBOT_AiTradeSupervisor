//! Parameter sweep harness — runs the offline backtest hundreds of times
//! across a grid of strategy parameters and reports the top-N combinations
//! by profit factor (with a minimum-trades filter to reject overfit noise).
//!
//! This is how we iterate without looking at live data: brute-force a grid,
//! pick candidates that survive a minimum-trades threshold, and validate
//! them on a held-out timeframe before considering them live-worthy.
//!
//! USAGE:
//!   cargo run --release --bin backtest_sweep -- --symbol PI_XBTUSD --days 30
//!   cargo run --release --bin backtest_sweep -- -v binance -s BTCUSDT -r 5m
//!
//! OUTPUT:
//!   1. Console: top 10 combos sorted by profit factor (trades>=10 only)
//!   2. Full CSV at data/backtest_reports/sweep_{venue}_{symbol}_{res}_{days}d.csv

use anyhow::{Context, Result};
use scalper_backtest::historical::{load_candles_ex, Candle, Venue};
use scalper_backtest::replay::{replay_with_costs, CostModel};
use scalper_backtest::report::BacktestReport;
use scalper_core::config::{ScalperConfig, StrategyConfig};
use scalper_strategy::ensemble::EnsembleStrategy;
use scalper_strategy::mean_reversion::MeanReversionStrategy;
use scalper_strategy::momentum::MomentumStrategy;
use scalper_strategy::ob_imbalance::ObImbalanceStrategy;
use scalper_strategy::traits::Strategy;
use std::io::Write;

#[derive(Debug)]
struct Args {
    symbol: String,
    resolution: String,
    days: u32,
    notional: f64,
    max_hold_bars: usize,
    mode: String,
    venue: String,
    min_trades: u64,
    top_n: usize,
    strategy_set: String, // "momentum_ob" | "mean_reversion" | "all"
    from_file: Option<String>,
}

impl Args {
    fn parse() -> Self {
        let mut symbol = "PI_XBTUSD".to_string();
        let mut resolution = "1m".to_string();
        let mut days: u32 = 30;
        let mut notional: f64 = 5000.0;
        let mut max_hold_bars: usize = 10;
        let mut mode = "paper".to_string();
        let mut venue = "kraken".to_string();
        let mut min_trades: u64 = 5;
        let mut top_n: usize = 10;
        let mut strategy_set = "momentum_ob".to_string();
        let mut from_file: Option<String> = None;

        let args: Vec<String> = std::env::args().skip(1).collect();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--symbol" | "-s" => {
                    if let Some(v) = args.get(i + 1) { symbol = v.clone(); i += 2; continue; }
                }
                "--resolution" | "-r" => {
                    if let Some(v) = args.get(i + 1) { resolution = v.clone(); i += 2; continue; }
                }
                "--days" | "-d" => {
                    if let Some(v) = args.get(i + 1) { days = v.parse().unwrap_or(days); i += 2; continue; }
                }
                "--notional" | "-n" => {
                    if let Some(v) = args.get(i + 1) { notional = v.parse().unwrap_or(notional); i += 2; continue; }
                }
                "--max-hold" => {
                    if let Some(v) = args.get(i + 1) { max_hold_bars = v.parse().unwrap_or(max_hold_bars); i += 2; continue; }
                }
                "--mode" | "-m" => {
                    if let Some(v) = args.get(i + 1) { mode = v.clone(); i += 2; continue; }
                }
                "--venue" | "-v" => {
                    if let Some(v) = args.get(i + 1) { venue = v.clone(); i += 2; continue; }
                }
                "--min-trades" => {
                    if let Some(v) = args.get(i + 1) { min_trades = v.parse().unwrap_or(min_trades); i += 2; continue; }
                }
                "--top" => {
                    if let Some(v) = args.get(i + 1) { top_n = v.parse().unwrap_or(top_n); i += 2; continue; }
                }
                "--strategies" => {
                    if let Some(v) = args.get(i + 1) { strategy_set = v.clone(); i += 2; continue; }
                }
                "--from-file" => {
                    if let Some(v) = args.get(i + 1) { from_file = Some(v.clone()); i += 2; continue; }
                }
                "--help" | "-h" => { print_help(); std::process::exit(0); }
                _ => { i += 1; }
            }
        }

        Self {
            symbol, resolution, days, notional, max_hold_bars,
            mode, venue, min_trades, top_n, strategy_set, from_file,
        }
    }
}

fn print_help() {
    println!(
        r#"Parameter sweep harness

USAGE:
  cargo run --release --bin backtest_sweep -- [OPTIONS]

OPTIONS:
  -s, --symbol <SYM>       Symbol to sweep (default: PI_XBTUSD)
  -r, --resolution <RES>   1m, 5m, 15m, 30m, 1h, 4h, 1d (default: 1m)
  -d, --days <N>           Days of history (default: 30)
  -n, --notional <USD>     Dollar per trade (default: 5000)
      --max-hold <BARS>    Max bars to hold (default: 10)
  -m, --mode <MODE>        Config mode (default: paper)
  -v, --venue <V>          kraken | binance (default: kraken)
      --min-trades <N>     Skip combos with fewer trades than this (default: 5)
      --top <N>            Show top-N combos (default: 10)
      --strategies <SET>   momentum_ob | mean_reversion | all (default: momentum_ob)
      --from-file <PATH>   Load candles from a local JSON file instead of
                           fetching from the venue
  -h, --help               Show this help

EXAMPLES:
  # Sweep the current live strategies on 30 days of 5m Kraken BTC
  cargo run --release --bin backtest_sweep -- -r 5m -d 30

  # Sweep on Binance (2 bps fee, not 5 bps)
  cargo run --release --bin backtest_sweep -- -v binance -s BTCUSDT -r 5m -d 30

  # Sweep the mean-reversion strategy on 15m bars
  cargo run --release --bin backtest_sweep -- --strategies mean_reversion -r 15m -d 30
"#
    );
}

/// One row in the sweep results.
#[derive(Debug, Clone)]
struct SweepRow {
    // Params we swept
    ensemble_threshold: f64,
    momentum_tp_pct: f64,
    momentum_sl_pct: f64,
    momentum_vol_mult: f64,
    ob_imbalance_threshold: f64,
    ob_tp_ticks: u32,
    ob_sl_ticks: u32,
    mr_rsi_oversold: f64,
    mr_bb_penetration: f64,
    mr_atr_tp: f64,
    mr_atr_sl: f64,
    mr_max_adx: f64,
    // Results
    total_trades: u64,
    win_rate: f64,
    profit_factor: f64,
    net_pnl: f64,
    max_drawdown_pct: f64,
    sharpe: f64,
    expectancy: f64,
}

impl SweepRow {
    fn from_report(params: SweepParams, r: &BacktestReport) -> Self {
        Self {
            ensemble_threshold: params.ensemble_threshold,
            momentum_tp_pct: params.momentum_tp_pct,
            momentum_sl_pct: params.momentum_sl_pct,
            momentum_vol_mult: params.momentum_vol_mult,
            ob_imbalance_threshold: params.ob_imbalance_threshold,
            ob_tp_ticks: params.ob_tp_ticks,
            ob_sl_ticks: params.ob_sl_ticks,
            mr_rsi_oversold: params.mr_rsi_oversold,
            mr_bb_penetration: params.mr_bb_penetration,
            mr_atr_tp: params.mr_atr_tp,
            mr_atr_sl: params.mr_atr_sl,
            mr_max_adx: params.mr_max_adx,
            total_trades: r.total_trades,
            win_rate: r.win_rate,
            profit_factor: if r.profit_factor.is_finite() { r.profit_factor } else { 999.0 },
            net_pnl: r.net_pnl,
            max_drawdown_pct: r.max_drawdown_pct,
            sharpe: if r.sharpe_ratio.is_finite() { r.sharpe_ratio } else { 0.0 },
            expectancy: r.expectancy,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct SweepParams {
    ensemble_threshold: f64,
    momentum_tp_pct: f64,
    momentum_sl_pct: f64,
    momentum_vol_mult: f64,
    ob_imbalance_threshold: f64,
    ob_tp_ticks: u32,
    ob_sl_ticks: u32,
    mr_rsi_oversold: f64,
    mr_bb_penetration: f64,
    mr_atr_tp: f64,
    mr_atr_sl: f64,
    mr_max_adx: f64,
}

impl SweepParams {
    fn apply_to(self, cfg: &mut StrategyConfig) {
        cfg.ensemble_threshold = self.ensemble_threshold;
        cfg.momentum.take_profit_pct = self.momentum_tp_pct;
        cfg.momentum.stop_loss_pct = self.momentum_sl_pct;
        cfg.momentum.volume_spike_multiplier = self.momentum_vol_mult;
        cfg.ob_imbalance.imbalance_threshold = self.ob_imbalance_threshold;
        cfg.ob_imbalance.take_profit_ticks = self.ob_tp_ticks;
        cfg.ob_imbalance.stop_loss_ticks = self.ob_sl_ticks;
        cfg.mean_reversion.rsi_oversold = self.mr_rsi_oversold;
        cfg.mean_reversion.rsi_overbought = 100.0 - self.mr_rsi_oversold;
        cfg.mean_reversion.bb_penetration = self.mr_bb_penetration;
        cfg.mean_reversion.atr_tp_multiplier = self.mr_atr_tp;
        cfg.mean_reversion.atr_sl_multiplier = self.mr_atr_sl;
        cfg.mean_reversion.max_adx = self.mr_max_adx;
    }
}

/// Build a sweep grid for the requested strategy set. Small-enough grids to
/// finish in seconds; you can widen these if you want deeper searches.
fn build_grid(strategy_set: &str) -> Vec<SweepParams> {
    // Anchor defaults (used when a dimension isn't being swept)
    let default = SweepParams {
        ensemble_threshold: 0.25,
        momentum_tp_pct: 0.50,
        momentum_sl_pct: 0.25,
        momentum_vol_mult: 2.0,
        ob_imbalance_threshold: 0.70,
        ob_tp_ticks: 17,
        ob_sl_ticks: 5,
        mr_rsi_oversold: 30.0,
        mr_bb_penetration: 0.05,
        mr_atr_tp: 1.5,
        mr_atr_sl: 1.0,
        mr_max_adx: 25.0,
    };

    let mut grid = Vec::new();

    match strategy_set {
        "mean_reversion" => {
            // Sweep MR params only
            for &threshold in &[0.15, 0.20, 0.25, 0.30] {
                for &rsi_os in &[20.0_f64, 25.0, 30.0, 35.0] {
                    for &bb_pen in &[0.0_f64, 0.05, 0.10, 0.15] {
                        for &atr_tp in &[1.0_f64, 1.5, 2.0, 2.5] {
                            for &atr_sl in &[0.75_f64, 1.0, 1.5] {
                                for &max_adx in &[20.0_f64, 25.0, 35.0] {
                                    grid.push(SweepParams {
                                        ensemble_threshold: threshold,
                                        mr_rsi_oversold: rsi_os,
                                        mr_bb_penetration: bb_pen,
                                        mr_atr_tp: atr_tp,
                                        mr_atr_sl: atr_sl,
                                        mr_max_adx: max_adx,
                                        ..default
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        "all" => {
            // Coarser grid across all strategies (large combinatorial space)
            for &threshold in &[0.20, 0.30] {
                for &m_tp in &[0.40_f64, 0.60, 0.80] {
                    for &m_sl in &[0.20_f64, 0.30] {
                        for &m_vol in &[1.5_f64, 2.5] {
                            for &ob_thr in &[0.55_f64, 0.70] {
                                for &ob_tp in &[10_u32, 20, 30] {
                                    for &rsi_os in &[25.0_f64, 30.0] {
                                        for &atr_tp in &[1.5_f64, 2.0] {
                                            grid.push(SweepParams {
                                                ensemble_threshold: threshold,
                                                momentum_tp_pct: m_tp,
                                                momentum_sl_pct: m_sl,
                                                momentum_vol_mult: m_vol,
                                                ob_imbalance_threshold: ob_thr,
                                                ob_tp_ticks: ob_tp,
                                                mr_rsi_oversold: rsi_os,
                                                mr_atr_tp: atr_tp,
                                                ..default
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        _ => {
            // momentum_ob: focus on the two currently-enabled strategies
            for &threshold in &[0.15, 0.20, 0.25, 0.30, 0.35] {
                for &m_tp in &[0.30_f64, 0.50, 0.70, 1.00] {
                    for &m_sl in &[0.15_f64, 0.25, 0.35] {
                        for &m_vol in &[1.5_f64, 2.0, 2.5] {
                            for &ob_thr in &[0.55_f64, 0.65, 0.75] {
                                for &ob_tp in &[10_u32, 15, 20, 30] {
                                    for &ob_sl in &[5_u32, 10] {
                                        grid.push(SweepParams {
                                            ensemble_threshold: threshold,
                                            momentum_tp_pct: m_tp,
                                            momentum_sl_pct: m_sl,
                                            momentum_vol_mult: m_vol,
                                            ob_imbalance_threshold: ob_thr,
                                            ob_tp_ticks: ob_tp,
                                            ob_sl_ticks: ob_sl,
                                            ..default
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    grid
}

fn build_ensemble(cfg: &StrategyConfig, set: &str) -> EnsembleStrategy {
    let mut strategies: Vec<Box<dyn Strategy>> = Vec::new();
    match set {
        "mean_reversion" => {
            let mut mr_cfg = cfg.mean_reversion.clone();
            mr_cfg.enabled = true;
            strategies.push(Box::new(MeanReversionStrategy::new(mr_cfg)));
        }
        "all" => {
            let mut m = cfg.momentum.clone();
            m.enabled = true;
            strategies.push(Box::new(MomentumStrategy::new(m)));

            let mut o = cfg.ob_imbalance.clone();
            o.enabled = true;
            strategies.push(Box::new(ObImbalanceStrategy::new(o)));

            let mut mr = cfg.mean_reversion.clone();
            mr.enabled = true;
            strategies.push(Box::new(MeanReversionStrategy::new(mr)));
        }
        _ => {
            // momentum_ob
            let mut m = cfg.momentum.clone();
            m.enabled = true;
            strategies.push(Box::new(MomentumStrategy::new(m)));

            let mut o = cfg.ob_imbalance.clone();
            o.enabled = true;
            strategies.push(Box::new(ObImbalanceStrategy::new(o)));
        }
    }
    EnsembleStrategy::new(strategies, cfg.ensemble_threshold)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let args = Args::parse();
    let venue = Venue::parse(&args.venue).context("parse --venue")?;

    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║           CRYPTO SCALPER — PARAMETER SWEEP HARNESS              ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");
    println!();
    println!("Venue:       {}", venue.as_str());
    println!("Symbol:      {}", args.symbol);
    println!("Resolution:  {}", args.resolution);
    println!("Window:      {} days", args.days);
    println!("Strategies:  {}", args.strategy_set);
    println!("Min trades:  {}", args.min_trades);
    println!();

    // Load base config (defines ScalperConfig defaults we'll mutate)
    let base_config = ScalperConfig::load(&args.mode).context("load config")?;
    let base_strategy_cfg = base_config.strategy.clone();

    // Build grid
    let grid = build_grid(&args.strategy_set);
    println!("Grid size:   {} combinations", grid.len());
    println!();

    // Load candles once and reuse
    let candles: Vec<Candle> = if let Some(ref path) = args.from_file {
        println!("Loading candles from {}", path);
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path))?;
        serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path))?
    } else {
        load_candles_ex(venue, &args.symbol, &args.resolution, args.days).await?
    };
    if candles.len() < 60 {
        anyhow::bail!(
            "Only {} candles — not enough history to warm up indicators (need 60+)",
            candles.len()
        );
    }
    let start = candles.first().map(|c| c.time_ms).unwrap_or(0);
    let end = candles.last().map(|c| c.time_ms).unwrap_or(0);
    let start_s = chrono::DateTime::from_timestamp_millis(start as i64)
        .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "?".to_string());
    let end_s = chrono::DateTime::from_timestamp_millis(end as i64)
        .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "?".to_string());
    println!("Loaded {} candles — {} to {}", candles.len(), start_s, end_s);
    println!();

    let costs = match venue {
        Venue::Kraken => CostModel::KRAKEN,
        Venue::Binance => CostModel::BINANCE,
    };
    println!(
        "Cost model:  {:.1} bps fee + {:.1} bps slippage per leg",
        costs.fee_bps, costs.slippage_bps
    );
    println!();

    // Run the sweep
    println!("Running sweep...");
    let started = std::time::Instant::now();

    let mut rows: Vec<SweepRow> = Vec::with_capacity(grid.len());
    for (i, params) in grid.iter().enumerate() {
        let mut strategy_cfg = base_strategy_cfg.clone();
        params.apply_to(&mut strategy_cfg);
        let ensemble = build_ensemble(&strategy_cfg, &args.strategy_set);
        let report = replay_with_costs(
            &args.symbol,
            &candles,
            &ensemble,
            args.notional,
            args.max_hold_bars,
            costs,
        );
        rows.push(SweepRow::from_report(*params, &report));

        // Progress heartbeat every 10%
        let total = grid.len().max(1);
        let step = (total / 10).max(1);
        if (i + 1) % step == 0 || i + 1 == total {
            let pct = ((i + 1) * 100) / total;
            println!("  {}% ({}/{})", pct, i + 1, total);
        }
    }
    let elapsed = started.elapsed();
    println!("Sweep complete in {:.2}s", elapsed.as_secs_f64());
    println!();

    // Filter + rank
    let mut qualified: Vec<SweepRow> = rows
        .iter()
        .filter(|r| r.total_trades >= args.min_trades)
        .cloned()
        .collect();
    qualified.sort_by(|a, b| b.profit_factor.partial_cmp(&a.profit_factor).unwrap_or(std::cmp::Ordering::Equal));

    // Trade-count distribution across all combos (not just qualified) —
    // helps identify whether the strategy is firing at all and how its
    // rate changes with the param grid.
    println!("Trade count distribution across all {} combos:", rows.len());
    let buckets = [
        (0, 0, "0 trades (dead)"),
        (1, 4, "1-4 trades"),
        (5, 9, "5-9 trades"),
        (10, 19, "10-19 trades"),
        (20, 49, "20-49 trades"),
        (50, 99, "50-99 trades"),
        (100, u64::MAX, "100+ trades"),
    ];
    for (lo, hi, label) in buckets {
        let count = rows.iter().filter(|r| r.total_trades >= lo && r.total_trades <= hi).count();
        if count > 0 {
            // Best PF within this bucket (only for combos that had trades)
            let best_in_bucket = rows
                .iter()
                .filter(|r| r.total_trades >= lo && r.total_trades <= hi && lo > 0)
                .map(|r| r.profit_factor)
                .fold(f64::NEG_INFINITY, f64::max);
            if lo > 0 && best_in_bucket.is_finite() {
                println!(
                    "  {:<20} {:>6} combos    best PF: {:.2}",
                    label, count, best_in_bucket
                );
            } else {
                println!("  {:<20} {:>6} combos", label, count);
            }
        }
    }
    println!();

    // Choose which param columns to show based on the strategy set
    let (params_header, show_mr_params) = match args.strategy_set.as_str() {
        "mean_reversion" => ("Params (thr|rsi_os|bb_pen|atr_tp|atr_sl|max_adx)", true),
        "all" => ("Params (thr|m_tp|m_sl|ob_thr|ob_tp|rsi_os|atr_tp)", false), // will still show momentum_ob for "all"
        _ => ("Params (thr|m_tp|m_sl|ob_thr|ob_tp_ticks|ob_sl_ticks)", false),
    };

    println!(
        "──────────────────────── TOP {} (of {} qualified, min {} trades) ────────────────────────",
        args.top_n.min(qualified.len()),
        qualified.len(),
        args.min_trades
    );
    if qualified.is_empty() {
        println!("⚠  No combinations produced at least {} trades.", args.min_trades);
        println!("   Lower --min-trades, widen the grid, or check whether the");
        println!("   indicator stack has enough history to warm up.");
    } else {
        println!(
            "{:>4}  {:>7}  {:>6}  {:>6}  {:>9}  {:>7}  {:>7}  {}",
            "#", "Trades", "Win%", "PF", "Net$", "DD%", "Sharpe", params_header
        );
        for (i, row) in qualified.iter().take(args.top_n).enumerate() {
            let params_str = if show_mr_params {
                format!(
                    "{:.2}|{:>5.1}|{:>5.2}|{:>4.2}|{:>4.2}|{:>4.1}",
                    row.ensemble_threshold,
                    row.mr_rsi_oversold,
                    row.mr_bb_penetration,
                    row.mr_atr_tp,
                    row.mr_atr_sl,
                    row.mr_max_adx,
                )
            } else if args.strategy_set == "all" {
                format!(
                    "{:.2}|{:>4.2}|{:>4.2}|{:>4.2}|{:>3}|{:>4.1}|{:>4.2}",
                    row.ensemble_threshold,
                    row.momentum_tp_pct,
                    row.momentum_sl_pct,
                    row.ob_imbalance_threshold,
                    row.ob_tp_ticks,
                    row.mr_rsi_oversold,
                    row.mr_atr_tp,
                )
            } else {
                format!(
                    "{:.2}|{:>4.2}|{:>4.2}|{:>4.2}|{:>3}|{:>3}",
                    row.ensemble_threshold,
                    row.momentum_tp_pct,
                    row.momentum_sl_pct,
                    row.ob_imbalance_threshold,
                    row.ob_tp_ticks,
                    row.ob_sl_ticks,
                )
            };
            println!(
                "{:>4}  {:>7}  {:>5.1}%  {:>6.2}  {:>9.2}  {:>6.1}%  {:>7.2}  {}",
                i + 1,
                row.total_trades,
                row.win_rate * 100.0,
                row.profit_factor,
                row.net_pnl,
                row.max_drawdown_pct,
                row.sharpe,
                params_str
            );
        }
    }
    println!();

    // Best PF verdict
    if let Some(best) = qualified.first() {
        let verdict = if best.profit_factor >= 1.5 && best.win_rate >= 0.40 {
            "✓  STRONG EDGE — top combo PF ≥ 1.5 on held-out data."
        } else if best.profit_factor >= 1.2 {
            "○  MARGINAL EDGE — best PF 1.2-1.5. Iterate on wider grids."
        } else if best.profit_factor >= 0.9 {
            "✗  NEAR BREAK-EVEN — best PF < 1.2. Strategy family is noise."
        } else {
            "✗  NEGATIVE EDGE — even the best combo loses money. Pivot strategy."
        };
        println!("Best verdict: {}", verdict);
    } else {
        println!("Best verdict: insufficient data.");
    }
    println!();

    // Save full CSV of all rows (including rejected ones)
    let report_dir = "data/backtest_reports";
    std::fs::create_dir_all(report_dir).ok();
    let csv_path = format!(
        "{}/sweep_{}_{}_{}_{}d.csv",
        report_dir,
        venue.as_str(),
        args.symbol,
        args.resolution,
        args.days
    );
    let mut f = std::fs::File::create(&csv_path)?;
    writeln!(
        f,
        "ensemble_threshold,momentum_tp_pct,momentum_sl_pct,momentum_vol_mult,\
ob_imbalance_threshold,ob_tp_ticks,ob_sl_ticks,mr_rsi_oversold,mr_bb_penetration,\
mr_atr_tp,mr_atr_sl,mr_max_adx,\
total_trades,win_rate,profit_factor,net_pnl,max_drawdown_pct,sharpe,expectancy"
    )?;
    for row in &rows {
        writeln!(
            f,
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{:.4},{:.4},{:.2},{:.2},{:.4},{:.4}",
            row.ensemble_threshold,
            row.momentum_tp_pct,
            row.momentum_sl_pct,
            row.momentum_vol_mult,
            row.ob_imbalance_threshold,
            row.ob_tp_ticks,
            row.ob_sl_ticks,
            row.mr_rsi_oversold,
            row.mr_bb_penetration,
            row.mr_atr_tp,
            row.mr_atr_sl,
            row.mr_max_adx,
            row.total_trades,
            row.win_rate,
            row.profit_factor,
            row.net_pnl,
            row.max_drawdown_pct,
            row.sharpe,
            row.expectancy,
        )?;
    }
    println!("Saved: {}", csv_path);

    Ok(())
}
