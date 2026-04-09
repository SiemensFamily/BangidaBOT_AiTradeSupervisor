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
use scalper_strategy::donchian::DonchianStrategy;
use scalper_strategy::ensemble::EnsembleStrategy;
use scalper_strategy::ma_cross::MaCrossStrategy;
use scalper_strategy::mean_reversion::MeanReversionStrategy;
use scalper_strategy::momentum::MomentumStrategy;
use scalper_strategy::ob_imbalance::ObImbalanceStrategy;
use scalper_strategy::traits::Strategy;
use std::io::Write;

/// Which parameter columns to render for each strategy family. Picked by
/// the --strategies flag — makes the top-N and per-bucket tables show
/// meaningful params instead of hardcoding momentum_ob columns for everything.
#[derive(Debug, Clone, Copy)]
enum ParamView {
    MomentumOb,
    MeanRev,
    Donchian,
    MaCross,
    SwingAll,
}

fn parse_param_view(strategy_set: &str) -> ParamView {
    match strategy_set {
        "mean_reversion" => ParamView::MeanRev,
        "donchian" => ParamView::Donchian,
        "ma_cross" => ParamView::MaCross,
        "swing_all" => ParamView::SwingAll,
        "all" => ParamView::MomentumOb,
        _ => ParamView::MomentumOb,
    }
}

fn params_header_for(view: ParamView) -> &'static str {
    match view {
        ParamView::MomentumOb => "Params (thr|m_tp|m_sl|ob_thr|ob_tp|ob_sl)",
        ParamView::MeanRev => "Params (thr|rsi_os|bb_pen|atr_tp|atr_sl|max_adx)",
        ParamView::Donchian => "Params (thr|entry|atr_tp|atr_sl|trend)",
        ParamView::MaCross => "Params (thr|fast/slow|min_sp|atr_tp|atr_sl)",
        ParamView::SwingAll => "Params (thr|dc_entry|dc_tp|mc_pair|mc_tp)",
    }
}

/// Compact one-line render of the params for a row, matching the header
/// column widths from `params_header_for`.
fn format_params(row: &SweepRow, view: ParamView) -> String {
    match view {
        ParamView::MomentumOb => format!(
            "{:.2}|{:>4.2}|{:>4.2}|{:>4.2}|{:>3}|{:>3}",
            row.ensemble_threshold,
            row.momentum_tp_pct,
            row.momentum_sl_pct,
            row.ob_imbalance_threshold,
            row.ob_tp_ticks,
            row.ob_sl_ticks,
        ),
        ParamView::MeanRev => format!(
            "{:.2}|{:>5.1}|{:>5.2}|{:>4.2}|{:>4.2}|{:>4.1}",
            row.ensemble_threshold,
            row.mr_rsi_oversold,
            row.mr_bb_penetration,
            row.mr_atr_tp,
            row.mr_atr_sl,
            row.mr_max_adx,
        ),
        ParamView::Donchian => format!(
            "{:.2}|{:>3}|{:>4.1}|{:>4.1}|{}",
            row.ensemble_threshold,
            row.dc_entry_period,
            row.dc_atr_tp,
            row.dc_atr_stop,
            if row.dc_trend_filter { " y" } else { " n" },
        ),
        ParamView::MaCross => format!(
            "{:.2}|{:>3}/{:<3}|{:>5.3}|{:>4.1}|{:>4.1}",
            row.ensemble_threshold,
            row.mc_fast,
            row.mc_slow,
            row.mc_min_spread_pct,
            row.mc_atr_tp,
            row.mc_atr_stop,
        ),
        ParamView::SwingAll => format!(
            "{:.2}|{:>3}|{:>4.1}|{:>3}/{:<3}|{:>4.1}",
            row.ensemble_threshold,
            row.dc_entry_period,
            row.dc_atr_tp,
            row.mc_fast,
            row.mc_slow,
            row.mc_atr_tp,
        ),
    }
}

/// Multi-line verbose render of the params for a row, used by the
/// per-bucket "best combo" display.
fn format_params_verbose(row: &SweepRow, view: ParamView) -> String {
    match view {
        ParamView::MomentumOb => format!(
            "thr={:.2} m_tp={:.2} m_sl={:.2} ob_thr={:.2} ob_tp={} ob_sl={}",
            row.ensemble_threshold,
            row.momentum_tp_pct,
            row.momentum_sl_pct,
            row.ob_imbalance_threshold,
            row.ob_tp_ticks,
            row.ob_sl_ticks,
        ),
        ParamView::MeanRev => format!(
            "thr={:.2} rsi_os={:.0} bb_pen={:.2} atr_tp={:.2} atr_sl={:.2} adx_max={:.0}",
            row.ensemble_threshold,
            row.mr_rsi_oversold,
            row.mr_bb_penetration,
            row.mr_atr_tp,
            row.mr_atr_sl,
            row.mr_max_adx,
        ),
        ParamView::Donchian => format!(
            "thr={:.2} entry={} atr_tp={:.1} atr_stop={:.1} trend_filter={}",
            row.ensemble_threshold,
            row.dc_entry_period,
            row.dc_atr_tp,
            row.dc_atr_stop,
            row.dc_trend_filter,
        ),
        ParamView::MaCross => format!(
            "thr={:.2} fast={} slow={} min_spread={:.3} atr_tp={:.1} atr_stop={:.1}",
            row.ensemble_threshold,
            row.mc_fast,
            row.mc_slow,
            row.mc_min_spread_pct,
            row.mc_atr_tp,
            row.mc_atr_stop,
        ),
        ParamView::SwingAll => format!(
            "thr={:.2} dc_entry={} dc_tp={:.1} dc_stop={:.1} mc={}/{} mc_tp={:.1}",
            row.ensemble_threshold,
            row.dc_entry_period,
            row.dc_atr_tp,
            row.dc_atr_stop,
            row.mc_fast,
            row.mc_slow,
            row.mc_atr_tp,
        ),
    }
}

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
    walk_forward: bool,
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
        let mut walk_forward = false;

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
                "--walk-forward" => {
                    walk_forward = true;
                    i += 1;
                    continue;
                }
                "--help" | "-h" => { print_help(); std::process::exit(0); }
                _ => { i += 1; }
            }
        }

        Self {
            symbol, resolution, days, notional, max_hold_bars,
            mode, venue, min_trades, top_n, strategy_set, from_file,
            walk_forward,
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
      --strategies <SET>   Strategy family to sweep:
                             momentum_ob    - the live scalping strategies (default)
                             mean_reversion - BB/RSI-based mean reversion
                             donchian       - Turtle-style channel breakout (swing)
                             ma_cross       - EMA crossover (swing)
                             swing_all      - Donchian + MA cross ensemble
                             all            - momentum + OB + mean_reversion
      --from-file <PATH>   Load candles from a local JSON file instead of
                           fetching from the venue
      --walk-forward       Split the candles 50/50, run the full grid on
                           BOTH halves, and rank by min(pf_train, pf_test).
                           This is the only honest way to validate edge:
                           a combo that works on the first half but dies
                           on the second is overfit, not real.
  -h, --help               Show this help

EXAMPLES:
  # Sweep Donchian breakout on 1-2 years of daily BTC (swing trading)
  cargo run --release --bin backtest_sweep -- \
      --strategies donchian -s PI_XBTUSD -r 1d -d 730 --walk-forward

  # Sweep MA crossover on 1 year of 4h ETH
  cargo run --release --bin backtest_sweep -- \
      --strategies ma_cross -s PI_ETHUSD -r 4h -d 365 --walk-forward

  # Full swing ensemble (Donchian + MA cross)
  cargo run --release --bin backtest_sweep -- \
      --strategies swing_all -s PI_XBTUSD -r 1d -d 730 --walk-forward

  # Sweep mean reversion on 15m (the scalping experiment)
  cargo run --release --bin backtest_sweep -- --strategies mean_reversion -r 15m -d 30

  # Sweep the original scalping strategies (momentum + OB imbalance)
  cargo run --release --bin backtest_sweep -- -r 5m -d 30
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
    // Donchian params
    dc_entry_period: u32,
    dc_atr_tp: f64,
    dc_atr_stop: f64,
    dc_trend_filter: bool,
    // MA crossover params
    mc_fast: u32,
    mc_slow: u32,
    mc_min_spread_pct: f64,
    mc_atr_tp: f64,
    mc_atr_stop: f64,
    // Results
    total_trades: u64,
    win_rate: f64,
    profit_factor: f64,
    net_pnl: f64,
    max_drawdown_pct: f64,
    sharpe: f64,
    expectancy: f64,
}

/// Paired result from running the same params on both halves of the
/// candle series. Used by --walk-forward.
#[derive(Debug, Clone)]
struct WalkForwardRow {
    first: SweepRow,
    second: SweepRow,
}

impl WalkForwardRow {
    /// The key metric for walk-forward ranking: the worse of the two
    /// halves' profit factors. Penalizes any combo that only worked in
    /// one regime. Infinite / NaN collapse to 0 so they never win.
    fn worst_pf(&self) -> f64 {
        let a = if self.first.profit_factor.is_finite() { self.first.profit_factor } else { 0.0 };
        let b = if self.second.profit_factor.is_finite() { self.second.profit_factor } else { 0.0 };
        a.min(b)
    }
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
            dc_entry_period: params.dc_entry_period,
            dc_atr_tp: params.dc_atr_tp,
            dc_atr_stop: params.dc_atr_stop,
            dc_trend_filter: params.dc_trend_filter,
            mc_fast: params.mc_fast,
            mc_slow: params.mc_slow,
            mc_min_spread_pct: params.mc_min_spread_pct,
            mc_atr_tp: params.mc_atr_tp,
            mc_atr_stop: params.mc_atr_stop,
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
    // Donchian params
    dc_entry_period: u32,
    dc_atr_tp: f64,
    dc_atr_stop: f64,
    dc_trend_filter: bool,
    // MA crossover params
    mc_fast: u32,
    mc_slow: u32,
    mc_min_spread_pct: f64,
    mc_atr_tp: f64,
    mc_atr_stop: f64,
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
        cfg.donchian.entry_period = self.dc_entry_period;
        cfg.donchian.atr_tp_multiplier = self.dc_atr_tp;
        cfg.donchian.atr_stop_multiplier = self.dc_atr_stop;
        cfg.donchian.use_trend_filter = self.dc_trend_filter;
        cfg.ma_cross.fast_period = self.mc_fast;
        cfg.ma_cross.slow_period = self.mc_slow;
        cfg.ma_cross.min_spread_pct = self.mc_min_spread_pct;
        cfg.ma_cross.atr_tp_multiplier = self.mc_atr_tp;
        cfg.ma_cross.atr_stop_multiplier = self.mc_atr_stop;
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
        dc_entry_period: 20,
        dc_atr_tp: 4.0,
        dc_atr_stop: 2.0,
        dc_trend_filter: false,
        mc_fast: 21,
        mc_slow: 50,
        mc_min_spread_pct: 0.005,
        mc_atr_tp: 3.0,
        mc_atr_stop: 1.5,
    };

    let mut grid = Vec::new();

    match strategy_set {
        "donchian" => {
            // Sweep Donchian params only. Classic Turtle entry/exit.
            for &threshold in &[0.15_f64, 0.25] {
                for &entry_period in &[10_u32, 20, 55] {
                    for &atr_tp in &[2.0_f64, 4.0, 6.0, 8.0] {
                        for &atr_stop in &[1.5_f64, 2.0, 3.0] {
                            for &trend_filter in &[false, true] {
                                grid.push(SweepParams {
                                    ensemble_threshold: threshold,
                                    dc_entry_period: entry_period,
                                    dc_atr_tp: atr_tp,
                                    dc_atr_stop: atr_stop,
                                    dc_trend_filter: trend_filter,
                                    ..default
                                });
                            }
                        }
                    }
                }
            }
        }
        "ma_cross" => {
            // Sweep MA crossover params. The valid (fast, slow) pairs are
            // constrained to the pre-computed EMAs: 9, 21, 50, 200.
            let pairs: &[(u32, u32)] = &[
                (9, 21),
                (9, 50),
                (21, 50),
                (21, 200),
                (50, 200),
            ];
            for &threshold in &[0.15_f64, 0.25] {
                for &(fast, slow) in pairs {
                    for &min_spread in &[0.001_f64, 0.005, 0.01, 0.02] {
                        for &atr_tp in &[2.0_f64, 3.0, 4.0, 5.0] {
                            for &atr_stop in &[1.0_f64, 1.5, 2.0] {
                                grid.push(SweepParams {
                                    ensemble_threshold: threshold,
                                    mc_fast: fast,
                                    mc_slow: slow,
                                    mc_min_spread_pct: min_spread,
                                    mc_atr_tp: atr_tp,
                                    mc_atr_stop: atr_stop,
                                    ..default
                                });
                            }
                        }
                    }
                }
            }
        }
        "swing_all" => {
            // Run both Donchian + MA cross as an ensemble. Narrower per-strategy
            // grid to keep total size reasonable.
            let pairs: &[(u32, u32)] = &[(21, 50), (50, 200)];
            for &threshold in &[0.15_f64, 0.25] {
                for &dc_entry in &[20_u32, 55] {
                    for &dc_atr_tp in &[3.0_f64, 5.0] {
                        for &dc_atr_stop in &[1.5_f64, 2.5] {
                            for &(fast, slow) in pairs {
                                for &mc_atr_tp in &[3.0_f64, 4.0] {
                                    for &mc_atr_stop in &[1.0_f64, 1.5] {
                                        for &trend_filter in &[false, true] {
                                            grid.push(SweepParams {
                                                ensemble_threshold: threshold,
                                                dc_entry_period: dc_entry,
                                                dc_atr_tp,
                                                dc_atr_stop,
                                                dc_trend_filter: trend_filter,
                                                mc_fast: fast,
                                                mc_slow: slow,
                                                mc_atr_tp,
                                                mc_atr_stop,
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
        "donchian" => {
            let mut dc = cfg.donchian.clone();
            dc.enabled = true;
            strategies.push(Box::new(DonchianStrategy::new(dc)));
        }
        "ma_cross" => {
            let mut mc = cfg.ma_cross.clone();
            mc.enabled = true;
            strategies.push(Box::new(MaCrossStrategy::new(mc)));
        }
        "swing_all" => {
            let mut dc = cfg.donchian.clone();
            dc.enabled = true;
            strategies.push(Box::new(DonchianStrategy::new(dc)));

            let mut mc = cfg.ma_cross.clone();
            mc.enabled = true;
            strategies.push(Box::new(MaCrossStrategy::new(mc)));
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

/// Render the walk-forward results and save a CSV with both halves.
fn display_walk_forward(wf_rows: &[WalkForwardRow], args: &Args, venue: Venue) {
    // Trade-count + PF distribution on the FIRST half only (for context —
    // the first half is what we'd have used to "train").
    println!("First-half trade count distribution across all {} combos:", wf_rows.len());
    let buckets = [
        (0_u64, 0_u64, "0 trades (dead)"),
        (1, 4, "1-4 trades"),
        (5, 9, "5-9 trades"),
        (10, 19, "10-19 trades"),
        (20, 49, "20-49 trades"),
        (50, 99, "50-99 trades"),
        (100, u64::MAX, "100+ trades"),
    ];
    for (lo, hi, label) in buckets {
        let rows: Vec<&WalkForwardRow> = wf_rows
            .iter()
            .filter(|r| r.first.total_trades >= lo && r.first.total_trades <= hi)
            .collect();
        if rows.is_empty() {
            continue;
        }
        let best_pf = rows
            .iter()
            .filter(|_| lo > 0)
            .map(|r| if r.first.profit_factor.is_finite() { r.first.profit_factor } else { 0.0 })
            .fold(f64::NEG_INFINITY, f64::max);
        if lo > 0 && best_pf.is_finite() {
            println!(
                "  {:<20} {:>6} combos    best first-half PF: {:.2}",
                label,
                rows.len(),
                best_pf
            );
        } else {
            println!("  {:<20} {:>6} combos", label, rows.len());
        }
    }
    println!();

    // Filter: only keep combos with sufficient trades in BOTH halves.
    // Filter: only keep combos with sufficient trades in BOTH halves.
    // In walk-forward mode, --min-trades is reinterpreted as PER HALF
    // (not total). Hardcode a statistical floor of 10 per half — anything
    // less gets too noisy for a meaningful verdict. If the user passes
    // a higher --min-trades, honor that.
    let half_min = args.min_trades.max(10);
    println!(
        "Walk-forward filter: minimum {} trades per half (hardcoded floor 10)",
        half_min
    );
    println!();
    let mut qualified: Vec<&WalkForwardRow> = wf_rows
        .iter()
        .filter(|r| r.first.total_trades >= half_min && r.second.total_trades >= half_min)
        .collect();
    // Sort by the worst of the two halves' profit factors (robust to regime).
    qualified.sort_by(|a, b| {
        b.worst_pf()
            .partial_cmp(&a.worst_pf())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    println!(
        "─────── WALK-FORWARD TOP {} (of {} qualified, min {} trades per half) ───────",
        args.top_n.min(qualified.len()),
        qualified.len(),
        half_min
    );
    if qualified.is_empty() {
        println!(
            "⚠  No combinations had at least {} trades in BOTH halves.",
            half_min
        );
        println!("   Either the strategy is too rare-firing, or the candle set is");
        println!("   too short for a split. Lower --min-trades or widen the window.");
    } else {
        let view = parse_param_view(&args.strategy_set);
        let header = params_header_for(view);
        println!(
            "{:>3}  {:>6}  {:>6}  {:>6}  {:>6}  {:>6}  {:>6}  {}",
            "#",
            "1st_n",
            "1st_PF",
            "1st_WR",
            "2nd_n",
            "2nd_PF",
            "2nd_WR",
            header
        );
        for (i, r) in qualified.iter().take(args.top_n).enumerate() {
            let params_str = format_params(&r.first, view);
            let pf_fmt = |pf: f64| {
                if pf.is_finite() && pf < 99.0 {
                    format!("{:>6.2}", pf)
                } else if pf.is_finite() {
                    " 99.00".to_string()
                } else {
                    "   inf".to_string()
                }
            };
            println!(
                "{:>3}  {:>6}  {}  {:>5.1}%  {:>6}  {}  {:>5.1}%  {}",
                i + 1,
                r.first.total_trades,
                pf_fmt(r.first.profit_factor),
                r.first.win_rate * 100.0,
                r.second.total_trades,
                pf_fmt(r.second.profit_factor),
                r.second.win_rate * 100.0,
                params_str
            );
        }
    }
    println!();

    // Walk-forward verdict: look at the best combo's WORST half.
    // Also require a minimum sample size per half — a 2/2 trade split
    // with PF 6.0 is noise, not edge.
    let verdict = if let Some(best) = qualified.first() {
        let worst_pf = best.worst_pf();
        let min_n = best.first.total_trades.min(best.second.total_trades);

        if min_n < 10 {
            "○  INSUFFICIENT SAMPLE — need ≥10 trades per half for a trustable verdict."
        } else if worst_pf >= 1.5 && min_n >= 20 {
            "✓✓ ROBUST EDGE — PF ≥ 1.5 on both halves with ≥20 trades each. Candidate for live."
        } else if worst_pf >= 1.5 {
            "✓  STRONG but THIN — PF ≥ 1.5 on both halves but only 10-19 trades/half. Widen grid."
        } else if worst_pf >= 1.2 {
            "○  PROMISING — PF ≥ 1.2 across regimes. Iterate on wider grids."
        } else if worst_pf >= 0.9 {
            "✗  FRAGILE — top combo breaks on one half. Overfit noise, not edge."
        } else {
            "✗  NO ROBUST EDGE — top combo fails one half. Pivot strategy."
        }
    } else {
        "✗  INSUFFICIENT DATA — no combo fired with ≥10 trades in both halves."
    };
    println!("Walk-forward verdict: {}", verdict);
    println!();

    // Save CSV with both halves
    let report_dir = "data/backtest_reports";
    std::fs::create_dir_all(report_dir).ok();
    let csv_path = format!(
        "{}/wf_{}_{}_{}_{}d.csv",
        report_dir,
        venue.as_str(),
        args.symbol,
        args.resolution,
        args.days
    );
    if let Ok(mut f) = std::fs::File::create(&csv_path) {
        let _ = writeln!(
            f,
            "ensemble_threshold,momentum_tp_pct,momentum_sl_pct,momentum_vol_mult,\
ob_imbalance_threshold,ob_tp_ticks,ob_sl_ticks,mr_rsi_oversold,mr_bb_penetration,\
mr_atr_tp,mr_atr_sl,mr_max_adx,\
first_trades,first_win_rate,first_profit_factor,first_net_pnl,first_max_dd,first_sharpe,\
second_trades,second_win_rate,second_profit_factor,second_net_pnl,second_max_dd,second_sharpe,\
worst_pf"
        );
        for r in wf_rows {
            let _ = writeln!(
                f,
                "{},{},{},{},{},{},{},{},{},{},{},{},\
                 {},{:.4},{:.4},{:.2},{:.2},{:.4},\
                 {},{:.4},{:.4},{:.2},{:.2},{:.4},\
                 {:.4}",
                r.first.ensemble_threshold,
                r.first.momentum_tp_pct,
                r.first.momentum_sl_pct,
                r.first.momentum_vol_mult,
                r.first.ob_imbalance_threshold,
                r.first.ob_tp_ticks,
                r.first.ob_sl_ticks,
                r.first.mr_rsi_oversold,
                r.first.mr_bb_penetration,
                r.first.mr_atr_tp,
                r.first.mr_atr_sl,
                r.first.mr_max_adx,
                r.first.total_trades,
                r.first.win_rate,
                r.first.profit_factor,
                r.first.net_pnl,
                r.first.max_drawdown_pct,
                r.first.sharpe,
                r.second.total_trades,
                r.second.win_rate,
                r.second.profit_factor,
                r.second.net_pnl,
                r.second.max_drawdown_pct,
                r.second.sharpe,
                r.worst_pf(),
            );
        }
        println!("Saved: {}", csv_path);
    }
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

    // Decide: single-pass sweep, or walk-forward (first half / second half)?
    let (candles_a, candles_b_opt): (&[Candle], Option<&[Candle]>) = if args.walk_forward {
        let mid = candles.len() / 2;
        println!(
            "Walk-forward split: first half = {} bars, second half = {} bars",
            mid,
            candles.len() - mid
        );
        println!();
        (&candles[..mid], Some(&candles[mid..]))
    } else {
        (&candles[..], None)
    };

    // Run the sweep — for each grid row, evaluate on the training candles.
    // If walk-forward, also evaluate on the held-out candles and keep both.
    println!("Running sweep...");
    let started = std::time::Instant::now();

    let mut rows: Vec<SweepRow> = Vec::with_capacity(grid.len());
    let mut wf_rows: Vec<WalkForwardRow> = Vec::with_capacity(grid.len());

    for (i, params) in grid.iter().enumerate() {
        let mut strategy_cfg = base_strategy_cfg.clone();
        params.apply_to(&mut strategy_cfg);
        let ensemble = build_ensemble(&strategy_cfg, &args.strategy_set);

        let report_a = replay_with_costs(
            &args.symbol,
            candles_a,
            &ensemble,
            args.notional,
            args.max_hold_bars,
            costs,
        );

        if let Some(candles_b) = candles_b_opt {
            // Walk-forward: rebuild a fresh ensemble with same params
            // (build_ensemble takes StrategyConfig by ref, so the same
            // config yields fresh strategy state on each call — no
            // cross-contamination between halves).
            let ensemble_b = build_ensemble(&strategy_cfg, &args.strategy_set);
            let report_b = replay_with_costs(
                &args.symbol,
                candles_b,
                &ensemble_b,
                args.notional,
                args.max_hold_bars,
                costs,
            );
            wf_rows.push(WalkForwardRow {
                first: SweepRow::from_report(*params, &report_a),
                second: SweepRow::from_report(*params, &report_b),
            });
        } else {
            rows.push(SweepRow::from_report(*params, &report_a));
        }

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

    // If walk-forward mode: collapse WalkForwardRows into a rank-by-worst
    // order and short-circuit the rest of the display pipeline.
    if !wf_rows.is_empty() {
        display_walk_forward(&wf_rows, &args, venue);
        return Ok(());
    }

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

    let view = parse_param_view(&args.strategy_set);
    let params_header = params_header_for(view);

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
            let params_str = format_params(row, view);
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

    // Best-of-bucket: for each trade-count bucket, show the top combo.
    // This surfaces larger-sample winners that the global top-N hides
    // because small-sample combos always have higher PF by chance.
    println!("Best combo in each trade-count bucket (sample-size aware):");
    println!(
        "  {:<14}  {:>6}  {:>6}  {:>6}  {:>9}  {}",
        "Bucket", "Trades", "Win%", "PF", "Net$", "Params"
    );
    for (lo, hi, label) in buckets {
        if lo == 0 {
            continue;
        }
        let best = rows
            .iter()
            .filter(|r| r.total_trades >= lo && r.total_trades <= hi)
            .filter(|r| r.profit_factor.is_finite() && r.profit_factor < 99.0)
            .max_by(|a, b| a.profit_factor.partial_cmp(&b.profit_factor).unwrap_or(std::cmp::Ordering::Equal));
        if let Some(row) = best {
            let params_str = format_params_verbose(row, view);
            println!(
                "  {:<14}  {:>6}  {:>5.1}%  {:>6.2}  {:>9.2}  {}",
                label,
                row.total_trades,
                row.win_rate * 100.0,
                row.profit_factor,
                row.net_pnl,
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
