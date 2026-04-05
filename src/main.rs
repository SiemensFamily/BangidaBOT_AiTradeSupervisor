use anyhow::{Context, Result};
use scalper_core::config::ScalperConfig;
use scalper_core::types::*;
use scalper_data::{
    candles::CandleManager,
    indicators::*,
    order_flow::OrderFlowTracker,
    orderbook::OrderBook,
    regime::RegimeDetector,
};
use scalper_execution::{executor::Executor, order_tracker::OrderTracker};
use scalper_risk::risk_manager::RiskManager;
use scalper_strategy::{
    ensemble::EnsembleStrategy,
    funding_arb::FundingBiasStrategy,
    liquidation_wick::LiquidationWickStrategy,
    momentum::MomentumStrategy,
    ob_imbalance::ObImbalanceStrategy,
    traits::{MarketContext, Strategy},
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Mutex};
use tracing::{error, info, warn};

mod dashboard;
mod paper_sim;

#[tokio::main]
async fn main() -> Result<()> {
    // Load environment variables from .env file
    match dotenvy::dotenv() {
        Ok(path) => {
            // tracing not initialised yet — use eprintln so it's visible in the terminal
            eprintln!("[init] .env loaded from {}", path.display());
        }
        Err(_) => {
            eprintln!("[init] No .env file found, using environment variables only");
        }
    }

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .json()
        .init();

    info!("Crypto Scalper starting...");

    // Load configuration
    let mode = std::env::var("SCALPER__GENERAL__MODE").unwrap_or_else(|_| "paper".to_string());
    let config = ScalperConfig::load(&mode).context("Failed to load configuration")?;

    info!(
        mode = config.general.mode,
        symbols = ?config.trading.symbols,
        leverage = config.trading.default_leverage,
        "Configuration loaded"
    );

    // Log resolved exchange key status (without exposing secrets)
    if let Some(ref k) = config.exchanges.kraken {
        info!(
            kraken_api_key_len = k.api_key.len(),
            kraken_ws_url = %k.base_url_ws,
            kraken_symbol_map_len = k.symbol_map.len(),
            "Kraken config resolved"
        );
    }

    // Initialize risk manager
    let initial_equity = config.risk.min_equity * 4.0; // Start at 4x min to have room
    let risk_manager = Arc::new(Mutex::new(RiskManager::new(
        config.risk.clone(),
        initial_equity,
    )));

    // Build strategies
    let strategies: Vec<Box<dyn Strategy>> = build_strategies(&config);
    let ensemble = EnsembleStrategy::new(strategies, config.strategy.ensemble_threshold);

    // Initialize execution
    let executor = Arc::new(Mutex::new(Executor::new()));
    let order_tracker = Arc::new(OrderTracker::new(5000)); // 5s auto-cancel timeout

    // Create channels
    let (market_tx, _) = broadcast::channel::<MarketEvent>(8192);
    let (signal_tx, mut signal_rx) = mpsc::channel::<Signal>(256);
    let (order_tx, mut order_rx) = mpsc::channel::<ValidatedSignal>(256);

    // Data aggregation state (shared across tasks)
    let orderbooks: Arc<Mutex<HashMap<String, OrderBook>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let indicators: Arc<Mutex<IndicatorState>> =
        Arc::new(Mutex::new(IndicatorState::new()));
    let candle_mgr: Arc<Mutex<CandleManager>> =
        Arc::new(Mutex::new(CandleManager::new()));
    let order_flow: Arc<Mutex<OrderFlowTracker>> =
        Arc::new(Mutex::new(OrderFlowTracker::new()));
    let regime_detector: Arc<Mutex<RegimeDetector>> =
        Arc::new(Mutex::new(RegimeDetector::new()));

    // Shared dashboard state (created early so tasks can log to it)
    let console_log = Arc::new(Mutex::new(dashboard::ConsoleLog::new(200)));
    let signal_log: Arc<Mutex<VecDeque<dashboard::SignalRecord>>> =
        Arc::new(Mutex::new(VecDeque::new()));
    let connected_exchanges: Arc<Mutex<HashSet<String>>> =
        Arc::new(Mutex::new(HashSet::new()));
    let trade_history: Arc<Mutex<Vec<dashboard::TradeRecord>>> =
        Arc::new(Mutex::new(Vec::new()));

    // Log startup
    {
        let mut cl = console_log.lock().await;
        cl.push(format!("Crypto Scalper starting in {} mode", mode));
        cl.push(format!("Symbols: {:?}", config.trading.symbols));
    }

    // Task a: Spawn exchange WebSocket feeds → market_tx
    spawn_exchange_feeds(&config, market_tx.clone(), connected_exchanges.clone(), console_log.clone());

    // Task b: Data aggregator — market_rx → update orderbook, indicators, candles
    {
        let mut market_rx = market_tx.subscribe();
        let orderbooks = orderbooks.clone();
        let indicators = indicators.clone();
        let candle_mgr = candle_mgr.clone();
        let order_flow = order_flow.clone();
        let regime_detector = regime_detector.clone();

        tokio::spawn(async move {
            info!("Data aggregator task started");
            loop {
                match market_rx.recv().await {
                    Ok(event) => {
                        process_market_event(
                            event,
                            &orderbooks,
                            &indicators,
                            &candle_mgr,
                            &order_flow,
                            &regime_detector,
                        )
                        .await;
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("Data aggregator lagged by {n} events");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        info!("Market channel closed, data aggregator exiting");
                        break;
                    }
                }
            }
        });
    }

    // Task c: Strategy engine — periodically evaluate ensemble → signal_tx
    {
        let symbols = config.trading.symbols.clone();
        let orderbooks = orderbooks.clone();
        let indicators = indicators.clone();
        let candle_mgr = candle_mgr.clone();
        let order_flow = order_flow.clone();
        let regime_detector = regime_detector.clone();

        tokio::spawn(async move {
            info!("Strategy engine task started");
            let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(100));
            loop {
                interval.tick().await;

                for symbol in &symbols {
                    let ctx = build_market_context(
                        symbol,
                        &orderbooks,
                        &indicators,
                        &candle_mgr,
                        &order_flow,
                        &regime_detector,
                    )
                    .await;

                    if let Some(ctx) = ctx {
                        if let Some(signal) = ensemble.evaluate(&ctx) {
                            if signal_tx.send(signal).await.is_err() {
                                return;
                            }
                        }
                    }
                }
            }
        });
    }

    // Task d: Risk pipeline — signal_rx → validate → order_tx
    {
        let risk_manager = risk_manager.clone();
        let regime_detector = regime_detector.clone();
        let indicators = indicators.clone();
        let signal_log = signal_log.clone();
        let console_log = console_log.clone();

        tokio::spawn(async move {
            info!("Risk pipeline task started");
            while let Some(signal) = signal_rx.recv().await {
                let rm = risk_manager.lock().await;
                let regime = regime_detector.lock().await.regime();
                let ind = indicators.lock().await;
                let atr = ind.atr.as_ref().map(|a| a.value());
                let price = decimal_to_f64(signal.take_profit.unwrap_or_default()); // approximate
                let now_ms = signal.timestamp_ms;

                let accepted = rm.validate_signal(&signal, regime, atr, price, now_ms);

                // Log signal to analyst log
                {
                    let record = dashboard::SignalRecord {
                        timestamp_ms: now_ms,
                        symbol: signal.symbol.clone(),
                        strategy: signal.strategy_name.clone(),
                        side: format!("{:?}", signal.side),
                        strength: signal.strength,
                        accepted: accepted.is_some(),
                    };
                    let mut sl = signal_log.lock().await;
                    if sl.len() >= 200 {
                        sl.pop_front();
                    }
                    sl.push_back(record);
                }

                if let Some(validated) = accepted {
                    console_log.lock().await.push(format!(
                        "Signal accepted: {} {:?} {} (strength: {:.2})",
                        signal.strategy_name,
                        signal.side,
                        signal.symbol,
                        signal.strength
                    ));
                    if order_tx.send(validated).await.is_err() {
                        break;
                    }
                }
            }
        });
    }

    // Task e: Executor — order_rx → place orders via exchange REST API
    {
        let executor = executor.clone();
        let order_tracker = order_tracker.clone();

        tokio::spawn(async move {
            info!("Executor task started");
            while let Some(validated) = order_rx.recv().await {
                let exec = executor.lock().await;

                // Prepare the order
                let best_bid = validated.signal.stop_loss.unwrap_or_default();
                let best_ask = validated.signal.take_profit.unwrap_or_default();
                let tick_size = rust_decimal_macros::dec!(0.1);

                let prepared = exec.prepare_order(&validated, best_bid, best_ask, tick_size);

                info!(
                    symbol = prepared.symbol,
                    side = ?prepared.side,
                    qty = %prepared.quantity,
                    price = ?prepared.price,
                    "Order prepared (paper mode — not sending to exchange)"
                );

                // In live mode, this would call exchange.place_order(...)
                // For now, track the order in the order tracker
                let managed = scalper_execution::order_tracker::ManagedOrder {
                    order_id: format!("sim-{}", validated.signal.timestamp_ms),
                    symbol: prepared.symbol.clone(),
                    exchange: prepared.exchange,
                    side: prepared.side,
                    order_type: prepared.order_type,
                    time_in_force: prepared.time_in_force,
                    price: prepared.price.unwrap_or_default(),
                    quantity: prepared.quantity,
                    filled_qty: rust_decimal_macros::dec!(0),
                    avg_fill_price: rust_decimal_macros::dec!(0),
                    status: scalper_execution::order_tracker::OrderStatus::New,
                    created_ms: validated.signal.timestamp_ms,
                    updated_ms: validated.signal.timestamp_ms,
                };
                order_tracker.track(managed);
            }
        });
    }

    // Task f: Stale order cleanup
    {
        let order_tracker = order_tracker.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
            loop {
                interval.tick().await;
                let now_ms = chrono::Utc::now().timestamp_millis() as u64;
                let stale = order_tracker.stale_orders(now_ms);
                for id in &stale {
                    info!(order_id = id, "Auto-cancelling stale order");
                }
                order_tracker.remove_terminal(60_000, now_ms);
            }
        });
    }

    // Task g: PnL reporter
    {
        let risk_manager = risk_manager.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                let rm = risk_manager.lock().await;
                let tracker = rm.pnl_tracker();
                info!(
                    equity = tracker.equity(),
                    drawdown = format!("{:.1}%", tracker.drawdown_pct()),
                    win_rate = format!("{:.1}%", tracker.win_rate() * 100.0),
                    trades = tracker.total_trades(),
                    "PnL Report"
                );
            }
        });
    }

    // Shared config for dashboard
    let shared_config = Arc::new(tokio::sync::RwLock::new(config.clone()));

    // Task h: Heartbeat — log scanning status every 10s
    {
        let console_log = console_log.clone();
        let orderbooks = orderbooks.clone();
        let config_symbols = config.trading.symbols.clone();
        // Build a map of original symbol → mapped aliases so we can check both
        let mut symbol_aliases: HashMap<String, Vec<String>> = HashMap::new();
        for cfg in [&config.exchanges.binance, &config.exchanges.bybit, &config.exchanges.kraken] {
            if let Some(c) = cfg {
                for (orig, mapped) in &c.symbol_map {
                    symbol_aliases.entry(orig.clone()).or_default().push(mapped.clone());
                }
            }
        }
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(10));
            loop {
                interval.tick().await;
                let obs = orderbooks.lock().await;
                let active = config_symbols
                    .iter()
                    .filter(|s| {
                        obs.contains_key(*s)
                            || symbol_aliases
                                .get(*s)
                                .map_or(false, |aliases| aliases.iter().any(|a| obs.contains_key(a)))
                    })
                    .count();
                drop(obs);
                console_log.lock().await.push(format!(
                    "Scanning markets... {}/{} symbols active",
                    active,
                    config_symbols.len()
                ));
            }
        });
    }

    // Task i: Paper fill simulator
    {
        let order_tracker = order_tracker.clone();
        let orderbooks = orderbooks.clone();
        let risk_manager = risk_manager.clone();
        let trade_history = trade_history.clone();
        let console_log = console_log.clone();
        tokio::spawn(paper_sim::run_paper_sim(
            order_tracker,
            orderbooks,
            risk_manager,
            trade_history,
            console_log,
        ));
    }

    // Task j: Web dashboard
    {
        let (ws_tx, _) = broadcast::channel::<String>(64);
        let dash_state = dashboard::DashboardState {
            config_mode: config.general.mode.clone(),
            config_symbols: config.trading.symbols.clone(),
            start_time_ms: chrono::Utc::now().timestamp_millis() as u64,
            risk_manager: risk_manager.clone(),
            order_tracker: order_tracker.clone(),
            orderbooks: orderbooks.clone(),
            regime_detector: regime_detector.clone(),
            indicators: indicators.clone(),
            config: shared_config,
            trade_history,
            console_log: console_log.clone(),
            signal_log,
            connected_exchanges,
            ws_tx,
        };
        tokio::spawn(dashboard::start_dashboard(dash_state));
    }

    info!("All tasks spawned. Crypto Scalper is running in {} mode.", mode);
    info!("Dashboard available at http://localhost:3000");
    info!("Press Ctrl+C to stop.");

    // Graceful shutdown on SIGINT/SIGTERM
    tokio::signal::ctrl_c()
        .await
        .context("Failed to listen for Ctrl+C")?;

    info!("Shutdown signal received. Stopping...");

    // Final report
    let rm = risk_manager.lock().await;
    let tracker = rm.pnl_tracker();
    info!(
        equity = tracker.equity(),
        total_trades = tracker.total_trades(),
        win_rate = format!("{:.1}%", tracker.win_rate() * 100.0),
        profit_factor = format!("{:.2}", tracker.profit_factor()),
        "Final Report"
    );

    Ok(())
}

/// Build all configured strategies.
fn build_strategies(config: &ScalperConfig) -> Vec<Box<dyn Strategy>> {
    let mut strategies: Vec<Box<dyn Strategy>> = Vec::new();

    if config.strategy.momentum.enabled {
        strategies.push(Box::new(MomentumStrategy::new(
            config.strategy.momentum.clone(),
        )));
    }
    if config.strategy.ob_imbalance.enabled {
        strategies.push(Box::new(ObImbalanceStrategy::new(
            config.strategy.ob_imbalance.clone(),
        )));
    }
    if config.strategy.liquidation_wick.enabled {
        strategies.push(Box::new(LiquidationWickStrategy::new(
            config.strategy.liquidation_wick.clone(),
        )));
    }
    if config.strategy.funding_bias.enabled {
        strategies.push(Box::new(FundingBiasStrategy::new(
            config.strategy.funding_bias.clone(),
        )));
    }

    info!("Loaded {} strategies", strategies.len());
    strategies
}

/// Spawn WebSocket feed tasks for each configured exchange.
fn map_symbols(symbols: &[String], cfg: &scalper_core::config::ExchangeConfig) -> Vec<String> {
    symbols
        .iter()
        .map(|s| cfg.symbol_map.get(s).cloned().unwrap_or_else(|| s.clone()))
        .collect()
}

fn spawn_exchange_feeds(
    config: &ScalperConfig,
    market_tx: broadcast::Sender<MarketEvent>,
    connected_exchanges: Arc<Mutex<HashSet<String>>>,
    console_log: Arc<Mutex<dashboard::ConsoleLog>>,
) {
    let symbols = config.trading.symbols.clone();

    if let Some(ref binance_cfg) = config.exchanges.binance {
        if !binance_cfg.api_key.is_empty() && !binance_cfg.base_url_ws.is_empty() {
            let feed = scalper_exchange::binance::BinanceWsFeed::new(binance_cfg.clone());
            let tx = market_tx.clone();
            let syms = map_symbols(&symbols, binance_cfg);
            let ce = connected_exchanges.clone();
            let cl = console_log.clone();
            tokio::spawn(async move {
                ce.lock().await.insert("binance".to_string());
                cl.lock().await.push("Binance WebSocket connected".to_string());
                if let Err(e) = scalper_exchange::MarketDataFeed::subscribe(&feed, &syms, tx).await {
                    error!("Binance feed error: {e}");
                    ce.lock().await.remove("binance");
                    cl.lock().await.push(format!("Binance disconnected: {e}"));
                }
            });
            info!("Binance WebSocket feed spawned");
        } else {
            info!(
                api_key_set = !binance_cfg.api_key.is_empty(),
                ws_url_set = !binance_cfg.base_url_ws.is_empty(),
                "Binance feed skipped (missing api_key or base_url_ws)"
            );
        }
    } else {
        info!("Binance exchange not configured");
    }

    if let Some(ref bybit_cfg) = config.exchanges.bybit {
        if !bybit_cfg.api_key.is_empty() && !bybit_cfg.base_url_ws.is_empty() {
            let feed = scalper_exchange::bybit::BybitWsFeed::new(bybit_cfg.clone());
            let tx = market_tx.clone();
            let syms = map_symbols(&symbols, bybit_cfg);
            let ce = connected_exchanges.clone();
            let cl = console_log.clone();
            tokio::spawn(async move {
                ce.lock().await.insert("bybit".to_string());
                cl.lock().await.push("Bybit WebSocket connected".to_string());
                if let Err(e) = scalper_exchange::MarketDataFeed::subscribe(&feed, &syms, tx).await {
                    error!("Bybit feed error: {e}");
                    ce.lock().await.remove("bybit");
                    cl.lock().await.push(format!("Bybit disconnected: {e}"));
                }
            });
            info!("Bybit WebSocket feed spawned");
        } else {
            info!(
                api_key_set = !bybit_cfg.api_key.is_empty(),
                ws_url_set = !bybit_cfg.base_url_ws.is_empty(),
                "Bybit feed skipped (missing api_key or base_url_ws)"
            );
        }
    } else {
        info!("Bybit exchange not configured");
    }

    if let Some(ref okx_cfg) = config.exchanges.okx {
        if !okx_cfg.api_key.is_empty() && !okx_cfg.base_url_ws.is_empty() {
            let feed = scalper_exchange::okx::OkxWsFeed::new(okx_cfg.clone());
            let tx = market_tx.clone();
            let syms = symbols.clone(); // OKX uses standard symbols
            let ce = connected_exchanges.clone();
            let cl = console_log.clone();
            tokio::spawn(async move {
                ce.lock().await.insert("okx".to_string());
                cl.lock().await.push("OKX WebSocket connected".to_string());
                if let Err(e) = scalper_exchange::MarketDataFeed::subscribe(&feed, &syms, tx).await {
                    error!("OKX feed error: {e}");
                    ce.lock().await.remove("okx");
                    cl.lock().await.push(format!("OKX disconnected: {e}"));
                }
            });
            info!("OKX WebSocket feed spawned");
        } else {
            info!(
                api_key_set = !okx_cfg.api_key.is_empty(),
                ws_url_set = !okx_cfg.base_url_ws.is_empty(),
                "OKX feed skipped (missing api_key or base_url_ws)"
            );
        }
    } else {
        info!("OKX exchange not configured");
    }

    if let Some(ref kraken_cfg) = config.exchanges.kraken {
        if !kraken_cfg.api_key.is_empty() && !kraken_cfg.base_url_ws.is_empty() {
            let feed = scalper_exchange::kraken::KrakenWsFeed::new(kraken_cfg.clone());
            let tx = market_tx.clone();
            let syms = map_symbols(&symbols, kraken_cfg);
            let ce = connected_exchanges.clone();
            let cl = console_log.clone();
            tokio::spawn(async move {
                ce.lock().await.insert("kraken".to_string());
                cl.lock().await.push("Kraken WebSocket connected".to_string());
                if let Err(e) = scalper_exchange::MarketDataFeed::subscribe(&feed, &syms, tx).await {
                    error!("Kraken feed error: {e}");
                    ce.lock().await.remove("kraken");
                    cl.lock().await.push(format!("Kraken disconnected: {e}"));
                }
            });
            info!("Kraken WebSocket feed spawned");
        } else {
            info!(
                api_key_set = !kraken_cfg.api_key.is_empty(),
                ws_url_set = !kraken_cfg.base_url_ws.is_empty(),
                "Kraken feed skipped (missing api_key or base_url_ws)"
            );
        }
    } else {
        info!("Kraken exchange not configured");
    }
}

/// Per-symbol indicator state.
pub(crate) struct IndicatorState {
    rsi: Option<RSI>,
    ema_9: Option<EMA>,
    ema_21: Option<EMA>,
    macd: Option<MACD>,
    bb: Option<BollingerBands>,
    vwap: Option<VWAP>,
    atr: Option<ATR>,
    obv: Option<OBV>,
    last_close: Option<f64>,
}

impl IndicatorState {
    fn new() -> Self {
        Self {
            rsi: Some(RSI::new(14)),
            ema_9: Some(EMA::new(9)),
            ema_21: Some(EMA::new(21)),
            macd: Some(MACD::new(12, 26, 9)),
            bb: Some(BollingerBands::new(20, 2.0)),
            vwap: Some(VWAP::new()),
            atr: Some(ATR::new(14)),
            obv: Some(OBV::new()),
            last_close: None,
        }
    }

    /// Returns (ready_count, total_count) for warmup progress.
    pub fn readiness(&self) -> (u32, u32) {
        let mut ready = 0u32;
        let total = 8u32;
        if self.rsi.as_ref().map_or(false, |i| i.is_ready()) { ready += 1; }
        if self.ema_9.as_ref().map_or(false, |i| i.is_ready()) { ready += 1; }
        if self.ema_21.as_ref().map_or(false, |i| i.is_ready()) { ready += 1; }
        if self.macd.as_ref().map_or(false, |i| i.is_ready()) { ready += 1; }
        if self.bb.as_ref().map_or(false, |i| i.is_ready()) { ready += 1; }
        if self.vwap.as_ref().map_or(false, |i| i.is_ready()) { ready += 1; }
        if self.atr.as_ref().map_or(false, |i| i.is_ready()) { ready += 1; }
        if self.obv.as_ref().map_or(false, |i| i.is_ready()) { ready += 1; }
        (ready, total)
    }

    fn update_price(&mut self, price: f64) {
        if let Some(ref mut rsi) = self.rsi {
            rsi.update(price);
        }
        if let Some(ref mut ema) = self.ema_9 {
            ema.update(price);
        }
        if let Some(ref mut ema) = self.ema_21 {
            ema.update(price);
        }
        if let Some(ref mut macd) = self.macd {
            macd.update(price);
        }
        if let Some(ref mut bb) = self.bb {
            bb.update(price);
        }
        self.last_close = Some(price);
    }

    fn update_ohlcv(&mut self, high: f64, low: f64, close: f64, volume: f64) {
        let prev_close = self.last_close.unwrap_or(close);
        if let Some(ref mut atr) = self.atr {
            atr.update_ohlc(high, low, prev_close);
        }
        if let Some(ref mut obv) = self.obv {
            obv.update_with_price(close, volume);
        }
        if let Some(ref mut vwap) = self.vwap {
            vwap.update_with_volume(close, volume);
        }
        self.last_close = Some(close);
    }
}

/// Process a single market event through the data aggregation layer.
async fn process_market_event(
    event: MarketEvent,
    orderbooks: &Mutex<HashMap<String, OrderBook>>,
    indicators: &Mutex<IndicatorState>,
    candle_mgr: &Mutex<CandleManager>,
    order_flow: &Mutex<OrderFlowTracker>,
    regime_detector: &Mutex<RegimeDetector>,
) {
    match event {
        MarketEvent::OrderBookUpdate {
            exchange,
            symbol,
            bids,
            asks,
            timestamp_ms,
        } => {
            let mut obs = orderbooks.lock().await;
            let ob = obs
                .entry(symbol.clone())
                .or_insert_with(|| OrderBook::new(symbol, exchange));
            ob.update(&bids, &asks, timestamp_ms);
        }
        MarketEvent::Trade {
            symbol,
            price,
            quantity,
            is_buyer_maker,
            timestamp_ms,
            ..
        } => {
            let price_f64 = decimal_to_f64(price);
            let qty_f64 = decimal_to_f64(quantity);

            let mut ind = indicators.lock().await;
            ind.update_price(price_f64);

            let mut of = order_flow.lock().await;
            of.on_trade(price_f64, qty_f64, is_buyer_maker);
            drop(of);

            let mut cm = candle_mgr.lock().await;
            let completed = cm.on_trade(&symbol, price_f64, qty_f64, timestamp_ms);
            drop(cm);

            // Feed completed 1m candles into indicators & regime detector.
            // This is essential for exchanges like Kraken that don't provide
            // native kline/candle WebSocket streams.
            for candle in completed {
                ind.update_ohlcv(candle.high, candle.low, candle.close, candle.volume);
                let prev_close = ind.last_close.unwrap_or(candle.close);
                let mut rd = regime_detector.lock().await;
                rd.update(candle.high, candle.low, prev_close);
            }
        }
        MarketEvent::KlineClose {
            high,
            low,
            close,
            volume,
            ..
        } => {
            let h = decimal_to_f64(high);
            let l = decimal_to_f64(low);
            let c = decimal_to_f64(close);
            let v = decimal_to_f64(volume);

            let mut ind = indicators.lock().await;
            ind.update_ohlcv(h, l, c, v);

            let mut rd = regime_detector.lock().await;
            let prev_close = ind.last_close.unwrap_or(c);
            rd.update(h, l, prev_close);
        }
        MarketEvent::LiquidationEvent {
            quantity,
            timestamp_ms,
            ..
        } => {
            let qty_f64 = decimal_to_f64(quantity);
            let mut of = order_flow.lock().await;
            of.on_liquidation(qty_f64, timestamp_ms);
        }
        MarketEvent::MarkPrice {
            symbol,
            mark_price,
            ..
        } => {
            // Use mark price updates to feed indicators. Kraken Futures has
            // infrequent trades but sends ticker/mark-price every second,
            // which is enough to warm up price-based indicators.
            let price_f64 = decimal_to_f64(mark_price);
            let mut ind = indicators.lock().await;
            ind.update_price(price_f64);

            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let mut cm = candle_mgr.lock().await;
            let completed = cm.on_trade(&symbol, price_f64, 0.0, ts);
            drop(cm);

            for candle in completed {
                ind.update_ohlcv(candle.high, candle.low, candle.close, candle.volume);
                let prev_close = ind.last_close.unwrap_or(candle.close);
                let mut rd = regime_detector.lock().await;
                rd.update(candle.high, candle.low, prev_close);
            }
        }
        _ => {} // OrderUpdate, PositionUpdate, BalanceUpdate handled elsewhere
    }
}

/// Build a MarketContext snapshot for strategy evaluation.
async fn build_market_context(
    symbol: &str,
    orderbooks: &Mutex<HashMap<String, OrderBook>>,
    indicators: &Mutex<IndicatorState>,
    candle_mgr: &Mutex<CandleManager>,
    order_flow: &Mutex<OrderFlowTracker>,
    regime_detector: &Mutex<RegimeDetector>,
) -> Option<MarketContext> {
    let obs = orderbooks.lock().await;
    let ob = obs.get(symbol)?;
    let (best_bid, _) = ob.best_bid()?;
    let (best_ask, _) = ob.best_ask()?;
    let mid = ob.mid_price()?;
    let spread = ob.spread()?;
    let imbalance = ob.imbalance_ratio(10);

    let ind = indicators.lock().await;
    let of = order_flow.lock().await;
    let cm = candle_mgr.lock().await;
    let rd = regime_detector.lock().await;

    let rsi_val = ind.rsi.as_ref().map(|i| i.value()).unwrap_or(50.0);
    let ema9_val = ind.ema_9.as_ref().map(|i| i.value()).unwrap_or(0.0);
    let ema21_val = ind.ema_21.as_ref().map(|i| i.value()).unwrap_or(0.0);
    let atr_val = ind.atr.as_ref().map(|i| i.value()).unwrap_or(0.0);
    let obv_val = ind.obv.as_ref().map(|i| i.value()).unwrap_or(0.0);
    let vwap_val = ind.vwap.as_ref().map(|i| i.value()).unwrap_or(0.0);

    let (_macd_line, _signal_line, histogram) = ind
        .macd
        .as_ref()
        .map(|m| m.lines())
        .unwrap_or((0.0, 0.0, 0.0));

    let (bb_upper, bb_mid, bb_lower) = ind
        .bb
        .as_ref()
        .map(|b| b.bands())
        .unwrap_or((0.0, 0.0, 0.0));

    let highest = cm.highest_high(symbol, "1m", 60).unwrap_or(0.0);
    let lowest = cm.lowest_low(symbol, "1m", 60).unwrap_or(0.0);

    // Determine multi-timeframe trends from EMA slope
    let tf_5m_trend = if ema9_val > ema21_val {
        Trend::Up
    } else if ema9_val < ema21_val {
        Trend::Down
    } else {
        Trend::Neutral
    };

    Some(MarketContext {
        symbol: symbol.to_string(),
        exchange: ob.exchange,
        last_price: mid,
        best_bid,
        best_ask,
        spread,
        tick_size: rust_decimal_macros::dec!(0.1),
        imbalance_ratio: imbalance,
        bid_depth_10: ob.bid_depth(10),
        ask_depth_10: ob.ask_depth(10),
        rsi_14: rsi_val,
        ema_9: ema9_val,
        ema_21: ema21_val,
        macd_histogram: histogram,
        bollinger_upper: bb_upper,
        bollinger_lower: bb_lower,
        bollinger_middle: bb_mid,
        vwap: vwap_val,
        atr_14: atr_val,
        obv: obv_val,
        cvd: of.cvd(),
        volume_ratio: of.volume_ratio(),
        liquidation_volume_1m: of.liquidation_volume_1m(),
        tf_5m_trend,
        tf_15m_trend: tf_5m_trend, // simplified: use same for now
        volatility_regime: rd.regime(),
        highest_high_60s: highest,
        lowest_low_60s: lowest,
        avg_volume_60s: 100.0,   // simplified
        current_volume: 100.0,   // simplified
        funding_rate: 0.001,     // filled from MarkPrice events
        funding_rate_secondary: 0.001,
        open_interest: None,
        price_velocity_30s: 0.0, // simplified
        timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
    })
}

fn decimal_to_f64(d: rust_decimal::Decimal) -> f64 {
    use std::str::FromStr;
    f64::from_str(&d.to_string()).unwrap_or(0.0)
}
