use anyhow::Result;
use rust_decimal_macros::dec;
use tracing::{info, warn};

use bangida_core::config::AppConfig;
use bangida_core::TradingMode;

fn init_tracing(log_level: &str) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .init();
}

fn print_banner(config: &AppConfig) {
    info!("========================================");
    info!("  BangidaBOT AI Trade Supervisor");
    info!("  High-Frequency Scalping Engine");
    info!("========================================");
    info!("Mode: {}", config.general.mode);
    info!("Symbols: {:?}", config.trading.symbols);
    info!("Default leverage: {}x", config.trading.default_leverage);
    info!("Max risk/trade: {}%", config.risk.max_risk_per_trade_pct);
    info!("Max daily loss: {}%", config.risk.max_daily_loss_pct);
    info!("Max drawdown: {}%", config.risk.max_drawdown_pct);
    info!(
        "Exchanges: Binance (testnet={}), Bybit (testnet={})",
        config.exchanges.binance.testnet, config.exchanges.bybit.testnet
    );
}

fn parse_mode(mode: &str) -> TradingMode {
    match mode.to_lowercase().as_str() {
        "live" => TradingMode::Live,
        "paper" => TradingMode::Paper,
        "backtest" => TradingMode::Backtest,
        other => {
            warn!("Unknown mode '{}', defaulting to Paper", other);
            TradingMode::Paper
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Load configuration (optional mode override from CLI arg)
    let mode_arg = std::env::args().nth(1);
    let config = match &mode_arg {
        Some(mode) => AppConfig::load_with_mode(mode)?,
        None => AppConfig::load()?,
    };

    // 2. Initialize logging
    init_tracing(&config.general.log_level);
    print_banner(&config);

    let mode = parse_mode(&config.general.mode);

    // 3. Initialize database
    std::fs::create_dir_all("data").ok();
    let _db = bangida_data::storage::Database::new(&config.database.path)?;
    info!("Database initialized at {}", config.database.path);

    // 4. Build risk management components
    let initial_equity = dec!(100); // $100 starting capital
    let _circuit_breaker =
        bangida_risk::CircuitBreaker::new(config.risk.clone(), initial_equity);

    let _risk_manager = bangida_risk::RiskManager::new(config.risk.clone());

    let _pnl_tracker = bangida_risk::PnlTracker::new(initial_equity);

    info!("Risk management initialized");
    info!(
        "Circuit breaker: max {} consecutive losses, {}% daily loss, {}% max drawdown",
        config.risk.max_consecutive_losses,
        config.risk.max_daily_loss_pct,
        config.risk.max_drawdown_pct
    );

    // 5. Build strategies from config
    let mut strategies: Vec<Box<dyn bangida_strategy::Strategy>> = Vec::new();

    if config.strategy.ob_imbalance.enabled {
        strategies.push(Box::new(bangida_strategy::ObImbalanceStrategy::new(
            &config.strategy.ob_imbalance,
        )));
    }
    if config.strategy.stat_arb.enabled {
        strategies.push(Box::new(bangida_strategy::StatArbStrategy::new(
            &config.strategy.stat_arb,
        )));
    }
    if config.strategy.momentum.enabled {
        strategies.push(Box::new(bangida_strategy::MomentumStrategy::new(
            &config.strategy.momentum,
        )));
    }
    if config.strategy.funding_bias.enabled {
        strategies.push(Box::new(bangida_strategy::FundingArbStrategy::new(
            &config.strategy.funding_bias,
        )));
    }

    info!("Loaded {} strategies:", strategies.len());
    for s in &strategies {
        info!("  - {} (weight: {:.2})", s.name(), s.weight());
    }

    let _ensemble = bangida_strategy::EnsembleStrategy::new(strategies, 0.15);
    info!("Ensemble strategy ready (min strength threshold: 0.15)");

    // 6. Mode-specific startup
    match mode {
        TradingMode::Live | TradingMode::Paper => {
            info!("Starting {} trading mode...", config.general.mode);

            if mode == TradingMode::Live {
                if config.exchanges.binance.api_key.is_empty() {
                    anyhow::bail!(
                        "Binance API key not configured. Set BANGIDA__EXCHANGES__BINANCE__API_KEY"
                    );
                }
                warn!("LIVE TRADING MODE - Real money at risk!");
            }

            // Create event pipeline channels
            let (market_tx, _) =
                tokio::sync::broadcast::channel::<bangida_core::MarketEvent>(10_000);
            let (_signal_tx, _signal_rx) =
                tokio::sync::mpsc::channel::<bangida_core::Signal>(1_000);
            let (_order_tx, _order_rx) =
                tokio::sync::mpsc::channel::<bangida_core::ValidatedSignal>(1_000);

            info!("Event pipeline channels created (capacity: 10000 market events)");

            // TODO: Spawn exchange WebSocket readers -> market_tx
            // TODO: Spawn strategy engine: market_rx -> signal evaluation -> signal_tx
            // TODO: Spawn risk pipeline: signal_rx -> validation -> order_tx
            // TODO: Spawn executor: order_rx -> exchange REST order placement

            info!("System initialized. Waiting for market data...");
            info!("Press Ctrl+C to shut down gracefully");

            // Wait for shutdown
            tokio::signal::ctrl_c().await?;
            info!("Shutdown signal received");
            drop(market_tx);
            info!("Graceful shutdown complete");
        }
        TradingMode::Backtest => {
            info!("Starting backtest mode...");
            info!("Backtest engine ready - supply historical data to run");
        }
    }

    Ok(())
}
