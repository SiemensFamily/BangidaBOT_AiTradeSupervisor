//! Offline backtest harness — validates strategies against historical data.
//!
//! Usage:
//!   cargo run --release --bin backtest -- --symbol PI_XBTUSD --days 30 --resolution 1m
//!
//! Loads the live config/default.toml to get the exact same strategy
//! parameters the live bot uses, then replays them against historical
//! OHLCV candles fetched from Kraken Futures.
//!
//! Output:
//!   • Console report (trades, win rate, profit factor, drawdown, Sharpe)
//!   • JSON report at data/backtest_reports/{symbol}_{resolution}_{days}d.json
//!
//! Historical data is cached at data/history/{symbol}_{resolution}.json so
//! subsequent runs on the same symbol/resolution are instant.

use anyhow::{Context, Result};
use scalper_backtest::historical::{load_candles_ex, Venue};
use scalper_backtest::replay::{replay_with_costs, CostModel};
use scalper_core::config::ScalperConfig;
use scalper_strategy::ensemble::EnsembleStrategy;
use scalper_strategy::funding_arb::FundingBiasStrategy;
use scalper_strategy::liquidation_wick::LiquidationWickStrategy;
use scalper_strategy::mean_reversion::MeanReversionStrategy;
use scalper_strategy::momentum::MomentumStrategy;
use scalper_strategy::ob_imbalance::ObImbalanceStrategy;
use scalper_strategy::traits::Strategy;

#[derive(Debug)]
struct Args {
    symbol: String,
    resolution: String,
    days: u32,
    notional: f64,
    max_hold_bars: usize,
    mode: String,
    from_file: Option<String>,
    venue: String,
}

impl Args {
    fn parse() -> Self {
        let mut symbol = "PI_XBTUSD".to_string();
        let mut resolution = "1m".to_string();
        let mut days: u32 = 30;
        let mut notional: f64 = 5000.0;
        let mut max_hold_bars: usize = 10;
        let mut mode = "paper".to_string();
        let mut from_file: Option<String> = None;
        let mut venue = "kraken".to_string();

        let args: Vec<String> = std::env::args().skip(1).collect();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--symbol" | "-s" => {
                    if let Some(v) = args.get(i + 1) {
                        symbol = v.clone();
                        i += 2;
                        continue;
                    }
                }
                "--resolution" | "-r" => {
                    if let Some(v) = args.get(i + 1) {
                        resolution = v.clone();
                        i += 2;
                        continue;
                    }
                }
                "--days" | "-d" => {
                    if let Some(v) = args.get(i + 1) {
                        days = v.parse().unwrap_or(days);
                        i += 2;
                        continue;
                    }
                }
                "--notional" | "-n" => {
                    if let Some(v) = args.get(i + 1) {
                        notional = v.parse().unwrap_or(notional);
                        i += 2;
                        continue;
                    }
                }
                "--max-hold" => {
                    if let Some(v) = args.get(i + 1) {
                        max_hold_bars = v.parse().unwrap_or(max_hold_bars);
                        i += 2;
                        continue;
                    }
                }
                "--mode" | "-m" => {
                    if let Some(v) = args.get(i + 1) {
                        mode = v.clone();
                        i += 2;
                        continue;
                    }
                }
                "--from-file" => {
                    if let Some(v) = args.get(i + 1) {
                        from_file = Some(v.clone());
                        i += 2;
                        continue;
                    }
                }
                "--venue" | "-v" => {
                    if let Some(v) = args.get(i + 1) {
                        venue = v.clone();
                        i += 2;
                        continue;
                    }
                }
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                _ => {
                    i += 1;
                }
            }
        }

        Self {
            symbol,
            resolution,
            days,
            notional,
            max_hold_bars,
            mode,
            from_file,
            venue,
        }
    }
}

fn print_help() {
    println!(
        r#"Offline backtest harness

USAGE:
  cargo run --release --bin backtest -- [OPTIONS]

OPTIONS:
  -s, --symbol <SYM>       Symbol to backtest.
                             Kraken: PI_XBTUSD, PI_ETHUSD (default)
                             Binance: BTCUSDT, ETHUSDT
  -r, --resolution <RES>   1m, 5m, 15m, 30m, 1h, 4h, 1d (default: 1m)
  -d, --days <N>           How many days of history to load (default: 30)
  -n, --notional <USD>     Dollar size per simulated trade (default: 5000)
      --max-hold <BARS>    Force-exit after N bars if still open (default: 10)
  -m, --mode <MODE>        Config mode to load (default: paper)
  -v, --venue <V>          Data source: kraken | binance (default: kraken)
      --from-file <PATH>   Load candles from a local JSON file instead of
                           fetching from the venue (same format as the
                           cache files under data/history/)
  -h, --help               Show this help

EXAMPLES:
  # Default: 30 days of 1m PI_XBTUSD on Kraken
  cargo run --release --bin backtest

  # 30 days of 5m on Binance (better fee profile — 2 bps vs 5 bps)
  cargo run --release --bin backtest -- -v binance -s BTCUSDT -r 5m -d 30

  # 7 days of 15m PI_ETHUSD with larger positions
  cargo run --release --bin backtest -- -s PI_ETHUSD -r 15m -d 7 -n 10000
"#
    );
}

fn build_strategies(config: &ScalperConfig) -> Vec<Box<dyn Strategy>> {
    let mut strategies: Vec<Box<dyn Strategy>> = Vec::new();
    if config.strategy.momentum.enabled {
        strategies.push(Box::new(MomentumStrategy::new(config.strategy.momentum.clone())));
    }
    if config.strategy.ob_imbalance.enabled {
        strategies.push(Box::new(ObImbalanceStrategy::new(config.strategy.ob_imbalance.clone())));
    }
    if config.strategy.liquidation_wick.enabled {
        strategies.push(Box::new(LiquidationWickStrategy::new(config.strategy.liquidation_wick.clone())));
    }
    if config.strategy.funding_bias.enabled {
        strategies.push(Box::new(FundingBiasStrategy::new(config.strategy.funding_bias.clone())));
    }
    if config.strategy.mean_reversion.enabled {
        strategies.push(Box::new(MeanReversionStrategy::new(config.strategy.mean_reversion.clone())));
    }
    strategies
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let venue = Venue::parse(&args.venue).context("parse --venue")?;
    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║             CRYPTO SCALPER — OFFLINE BACKTEST HARNESS           ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");
    println!();
    println!("Venue:       {}", venue.as_str());
    println!("Symbol:      {}", args.symbol);
    println!("Resolution:  {}", args.resolution);
    println!("Window:      {} days", args.days);
    println!("Notional:    ${:.0} per trade", args.notional);
    println!("Max hold:    {} bars", args.max_hold_bars);
    println!("Config mode: {}", args.mode);
    println!();

    // 1. Load live config (same file the live bot uses)
    let config = ScalperConfig::load(&args.mode).context("Failed to load configuration")?;

    // 2. Build the exact same strategies the live bot would run
    let strategies = build_strategies(&config);
    let strategy_names: Vec<String> = strategies.iter().map(|s| s.name().to_string()).collect();
    println!(
        "Strategies:  {} enabled — {}",
        strategies.len(),
        strategy_names.join(", ")
    );
    println!(
        "Ensemble threshold: {:.3}",
        config.strategy.ensemble_threshold
    );
    println!();

    if strategies.is_empty() {
        anyhow::bail!("No strategies are enabled in {}.toml — nothing to backtest", args.mode);
    }
    let ensemble = EnsembleStrategy::new(strategies, config.strategy.ensemble_threshold);

    // 3. Load candles (from file, cache, or the venue)
    let candles = if let Some(ref path) = args.from_file {
        println!("Loading candles from {}", path);
        let bytes = std::fs::read(path)
            .with_context(|| format!("read {}", path))?;
        let candles: Vec<scalper_backtest::historical::Candle> = serde_json::from_slice(&bytes)
            .with_context(|| format!("parse {}", path))?;
        candles
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

    // 4. Run the replay with venue-specific cost model
    let costs = match venue {
        Venue::Kraken => CostModel::KRAKEN,
        Venue::Binance => CostModel::BINANCE,
    };
    println!(
        "Cost model:  {:.1} bps fee + {:.1} bps slippage per leg",
        costs.fee_bps, costs.slippage_bps
    );
    println!();
    println!("Replaying...");
    let started = std::time::Instant::now();
    let report = replay_with_costs(
        &args.symbol,
        &candles,
        &ensemble,
        args.notional,
        args.max_hold_bars,
        costs,
    );
    let elapsed = started.elapsed();
    println!("Replay complete in {:.2}s", elapsed.as_secs_f64());
    println!();

    // 5. Print report
    println!("──────────────────────── RESULTS ────────────────────────");
    print!("{}", report);
    println!("─────────────────────────────────────────────────────────");
    println!();

    // 6. Verdict — honest assessment
    let verdict = if report.total_trades < 10 {
        "⚠  NOT ENOUGH TRADES — increase days or lower thresholds. Can't assess edge."
    } else if report.profit_factor >= 1.5 && report.win_rate >= 0.40 {
        "✓  STRONG EDGE — profit factor ≥ 1.5 on held-out data. Consider live (small size)."
    } else if report.profit_factor >= 1.2 {
        "○  MARGINAL EDGE — profit factor 1.2-1.5. Iterate on strategies before going live."
    } else if report.profit_factor >= 0.9 {
        "✗  NEAR BREAK-EVEN — profit factor < 1.2. Edge is noise. DO NOT go live."
    } else {
        "✗  NEGATIVE EDGE — strategies lose money on this data. DO NOT go live."
    };
    println!("Verdict: {}", verdict);
    println!();

    // 7. Save JSON report
    let report_dir = "data/backtest_reports";
    std::fs::create_dir_all(report_dir).ok();
    let filename = format!(
        "{}/{}_{}_{}_{}d.json",
        report_dir,
        venue.as_str(),
        args.symbol,
        args.resolution,
        args.days
    );
    #[derive(serde::Serialize)]
    struct JsonReport {
        venue: String,
        symbol: String,
        resolution: String,
        days: u32,
        notional: f64,
        max_hold_bars: usize,
        ensemble_threshold: f64,
        strategies: Vec<String>,
        total_trades: u64,
        winning_trades: u64,
        losing_trades: u64,
        win_rate: f64,
        profit_factor: f64,
        total_pnl: f64,
        total_fees: f64,
        net_pnl: f64,
        max_drawdown_pct: f64,
        sharpe_ratio: f64,
        avg_win: f64,
        avg_loss: f64,
        expectancy: f64,
        return_pct: f64,
        verdict: String,
        timestamp: String,
    }
    let j = JsonReport {
        venue: venue.as_str().to_string(),
        symbol: args.symbol.clone(),
        resolution: args.resolution.clone(),
        days: args.days,
        notional: args.notional,
        max_hold_bars: args.max_hold_bars,
        ensemble_threshold: config.strategy.ensemble_threshold,
        strategies: strategy_names.clone(),
        total_trades: report.total_trades,
        winning_trades: report.winning_trades,
        losing_trades: report.losing_trades,
        win_rate: report.win_rate,
        profit_factor: if report.profit_factor.is_finite() {
            report.profit_factor
        } else {
            999.0
        },
        total_pnl: report.total_pnl,
        total_fees: report.total_fees,
        net_pnl: report.net_pnl,
        max_drawdown_pct: report.max_drawdown_pct,
        sharpe_ratio: if report.sharpe_ratio.is_finite() {
            report.sharpe_ratio
        } else {
            0.0
        },
        avg_win: report.avg_win,
        avg_loss: report.avg_loss,
        expectancy: report.expectancy,
        return_pct: report.return_pct,
        verdict: verdict.to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    };
    std::fs::write(&filename, serde_json::to_string_pretty(&j)?)?;
    println!("Saved: {}", filename);

    Ok(())
}
