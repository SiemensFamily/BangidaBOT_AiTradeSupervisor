use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::header,
    response::{Html, IntoResponse},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex, RwLock};
use tracing::info;

use scalper_core::config::ScalperConfig;
use scalper_data::orderbook::OrderBook;
use scalper_data::regime::RegimeDetector;
use scalper_execution::order_tracker::OrderTracker;
use scalper_risk::risk_manager::RiskManager;

use crate::IndicatorState;
use scalper_strategy::StrategyVote;

// ── Trade history ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct TradeRecord {
    pub timestamp_ms: u64,
    pub symbol: String,
    pub side: String,
    pub price: String,
    pub quantity: String,
    pub pnl: f64,
    pub fees: f64,
    pub order_id: String,
}

// ── Console log ───────────────────────────────────────────────────────────

#[derive(Clone, Serialize)]
pub struct ConsoleEntry {
    pub timestamp_ms: u64,
    pub message: String,
}

pub struct ConsoleLog {
    entries: VecDeque<ConsoleEntry>,
    capacity: usize,
}

impl ConsoleLog {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, message: String) {
        let entry = ConsoleEntry {
            timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
            message,
        };
        if self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    pub fn entries(&self) -> Vec<ConsoleEntry> {
        self.entries.iter().cloned().collect()
    }
}

// ── Signal log ────────────────────────────────────────────────────────────

#[derive(Clone, Serialize)]
pub struct SignalRecord {
    pub timestamp_ms: u64,
    pub symbol: String,
    pub strategy: String,
    pub side: String,
    pub strength: f64,
    pub accepted: bool,
}

// ── Shared state ───────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct DashboardState {
    pub config_mode: String,
    pub config_symbols: Vec<String>,
    pub start_time_ms: u64,
    pub risk_manager: Arc<Mutex<RiskManager>>,
    pub order_tracker: Arc<OrderTracker>,
    pub orderbooks: Arc<Mutex<HashMap<String, OrderBook>>>,
    pub regime_detector: Arc<Mutex<RegimeDetector>>,
    pub indicators: Arc<Mutex<IndicatorState>>,
    pub config: Arc<RwLock<ScalperConfig>>,
    pub trade_history: Arc<Mutex<Vec<TradeRecord>>>,
    pub console_log: Arc<Mutex<ConsoleLog>>,
    pub signal_log: Arc<Mutex<VecDeque<SignalRecord>>>,
    pub connected_exchanges: Arc<Mutex<HashSet<String>>>,
    pub strategy_votes: Arc<Mutex<Vec<StrategyVote>>>,
    pub ws_tx: broadcast::Sender<String>,
}

// ── Snapshot types ─────────────────────────────────────────────────────────

#[derive(Serialize)]
struct Snapshot {
    timestamp_ms: u64,
    mode: String,
    uptime_secs: u64,
    // Equity
    equity: f64,
    starting_equity: f64,
    daily_pnl: f64,
    total_pnl: f64,
    total_fees: f64,
    drawdown_pct: f64,
    // Performance
    win_rate: f64,
    profit_factor: f64,
    total_trades: u64,
    expectancy: f64,
    // Risk
    can_trade: bool,
    consecutive_losses: u32,
    trades_this_hour: u32,
    daily_loss: f64,
    regime: String,
    // Data
    open_orders: Vec<OrderSnap>,
    markets: Vec<MarketSnap>,
    // Warmup
    warmup_ready: bool,
    indicators_ready: u32,
    indicators_total: u32,
    regime_ready: bool,
    regime_atr_count: usize,
    regime_atr_needed: usize,
    // Console & logs
    console_log: Vec<ConsoleEntry>,
    signal_log: Vec<SignalRecord>,
    trade_history: Vec<TradeRecord>,
    // Exchange status
    exchange_status: Vec<ExchangeStatus>,
    // Strategy status
    strategy_status: Vec<StrategyStatusSnap>,
}

#[derive(Serialize)]
struct OrderSnap {
    order_id: String,
    symbol: String,
    side: String,
    price: String,
    quantity: String,
    filled_qty: String,
    status: String,
}

#[derive(Serialize)]
struct MarketSnap {
    symbol: String,
    best_bid: String,
    best_ask: String,
    spread: String,
}

#[derive(Serialize)]
struct ExchangeStatus {
    name: String,
    connected: bool,
}

#[derive(Serialize)]
struct StrategyStatusSnap {
    name: String,
    active: bool,
    side: String,
    strength: f64,
}

// ── Helpers ───────────────────────────────────────────────────────────────

/// Replace INFINITY, NEG_INFINITY, NaN with 0.0 so serde_json doesn't error.
fn sanitize_f64(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

// ── HTML ───────────────────────────────────────────────────────────────────

const HTML: &str = include_str!("dashboard.html");

// ── Server ─────────────────────────────────────────────────────────────────

pub async fn start_dashboard(state: DashboardState) {
    // Snapshot broadcaster (every 500ms)
    let snap_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(500));
        loop {
            interval.tick().await;
            if snap_state.ws_tx.receiver_count() == 0 {
                continue;
            }
            let snapshot = build_snapshot(&snap_state).await;
            if let Ok(json) = serde_json::to_string(&snapshot) {
                let _ = snap_state.ws_tx.send(json);
            }
        }
    });

    let app = Router::new()
        .route("/", get(serve_html))
        .route("/ws", get(ws_handler))
        .route("/api/config", get(get_config).put(put_config))
        .route("/api/trades.csv", get(get_trades_csv))
        .route("/api/signals.csv", get(get_signals_csv))
        .route("/api/console.csv", get(get_console_csv))
        .route("/api/debug", get(get_debug))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    info!("Dashboard listening on http://localhost:3000");
    axum::serve(listener, app).await.unwrap();
}

async fn serve_html() -> Html<&'static str> {
    Html(HTML)
}

// ── WebSocket ──────────────────────────────────────────────────────────────

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<DashboardState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(mut socket: WebSocket, state: DashboardState) {
    let snapshot = build_snapshot(&state).await;
    if let Ok(json) = serde_json::to_string(&snapshot) {
        let _ = socket.send(Message::Text(json.into())).await;
    }
    let mut rx = state.ws_tx.subscribe();
    while let Ok(msg) = rx.recv().await {
        if socket.send(Message::Text(msg.into())).await.is_err() {
            break;
        }
    }
}

// ── REST: Config ───────────────────────────────────────────────────────────

async fn get_config(State(state): State<DashboardState>) -> Json<ScalperConfig> {
    let cfg = state.config.read().await.clone();
    Json(cfg)
}

#[derive(Deserialize)]
struct ConfigUpdate {
    #[serde(flatten)]
    config: ScalperConfig,
}

async fn put_config(
    State(state): State<DashboardState>,
    Json(update): Json<ConfigUpdate>,
) -> impl IntoResponse {
    // Write to config/default.toml
    let toml_str = match toml::to_string_pretty(&update.config) {
        Ok(s) => s,
        Err(e) => return (axum::http::StatusCode::BAD_REQUEST, e.to_string()),
    };
    if let Err(e) = tokio::fs::write("config/default.toml", &toml_str).await {
        return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
    }
    // Update in-memory config
    *state.config.write().await = update.config;
    (axum::http::StatusCode::OK, "Config saved. Restart for full effect.".to_string())
}

// ── REST: CSV export ───────────────────────────────────────────────────────

async fn get_trades_csv(State(state): State<DashboardState>) -> impl IntoResponse {
    let trades = state.trade_history.lock().await;
    let mut csv = String::from("timestamp,symbol,side,price,quantity,pnl,fees,order_id\n");
    for t in trades.iter() {
        csv.push_str(&format!(
            "{},{},{},{},{},{:.4},{:.4},{}\n",
            t.timestamp_ms, t.symbol, t.side, t.price, t.quantity, t.pnl, t.fees, t.order_id
        ));
    }
    (
        [(header::CONTENT_TYPE, "text/csv"), (header::CONTENT_DISPOSITION, "attachment; filename=\"trades.csv\"")],
        csv,
    )
}

async fn get_signals_csv(State(state): State<DashboardState>) -> impl IntoResponse {
    let signals = state.signal_log.lock().await;
    let mut csv = String::from("timestamp,symbol,strategy,side,strength,accepted\n");
    for s in signals.iter() {
        csv.push_str(&format!(
            "{},{},{},\"{}\",{:.4},{}\n",
            s.timestamp_ms, s.symbol, s.strategy, s.side, s.strength, s.accepted
        ));
    }
    (
        [(header::CONTENT_TYPE, "text/csv"), (header::CONTENT_DISPOSITION, "attachment; filename=\"signals.csv\"")],
        csv,
    )
}

async fn get_console_csv(State(state): State<DashboardState>) -> impl IntoResponse {
    let log = state.console_log.lock().await;
    let mut csv = String::from("timestamp,message\n");
    for e in log.entries() {
        csv.push_str(&format!(
            "{},\"{}\"\n",
            e.timestamp_ms, e.message.replace('"', "\"\"")
        ));
    }
    (
        [(header::CONTENT_TYPE, "text/csv"), (header::CONTENT_DISPOSITION, "attachment; filename=\"console.csv\"")],
        csv,
    )
}

// ── REST: Debug ───────────────────────────────────────────────────────────

async fn get_debug(State(state): State<DashboardState>) -> Json<Snapshot> {
    Json(build_snapshot(&state).await)
}

// ── Snapshot builder ───────────────────────────────────────────────────────

async fn build_snapshot(state: &DashboardState) -> Snapshot {
    let now_ms = chrono::Utc::now().timestamp_millis() as u64;
    let uptime = (now_ms.saturating_sub(state.start_time_ms)) / 1000;

    // Risk manager
    let rm = state.risk_manager.lock().await;
    let tracker = rm.pnl_tracker();
    let cb = rm.circuit_breaker();

    let equity = sanitize_f64(tracker.equity());
    let starting_equity = sanitize_f64(tracker.starting_equity());
    let daily_pnl = sanitize_f64(tracker.daily_pnl());
    let total_pnl = sanitize_f64(tracker.total_pnl());
    let total_fees = sanitize_f64(tracker.total_fees());
    let drawdown_pct = sanitize_f64(tracker.drawdown_pct());
    let win_rate = sanitize_f64(tracker.win_rate());
    let profit_factor = sanitize_f64(tracker.profit_factor());
    let total_trades = tracker.total_trades();
    let expectancy = sanitize_f64(tracker.expectancy());

    let can_trade = cb.can_trade(now_ms);
    let consecutive_losses = cb.consecutive_losses();
    let trades_this_hour = cb.trades_this_hour();
    let daily_loss = sanitize_f64(cb.daily_loss());
    drop(rm);

    // Regime
    let rd = state.regime_detector.lock().await;
    let regime = format!("{:?}", rd.regime());
    let regime_ready = rd.is_ready();
    let regime_atr_count = rd.atr_count();
    drop(rd);

    // Indicators warmup
    let ind = state.indicators.lock().await;
    let (indicators_ready, indicators_total) = ind.readiness();
    drop(ind);

    let warmup_ready = regime_ready && indicators_ready == indicators_total;
    let regime_atr_needed = 50; // EMA-50 of ATR values

    // Open orders
    let open = state.order_tracker.open_orders();
    let open_orders: Vec<OrderSnap> = open
        .iter()
        .map(|o| OrderSnap {
            order_id: o.order_id.clone(),
            symbol: o.symbol.clone(),
            side: format!("{:?}", o.side),
            price: o.price.to_string(),
            quantity: o.quantity.to_string(),
            filled_qty: o.filled_qty.to_string(),
            status: format!("{:?}", o.status),
        })
        .collect();

    // Market data
    let obs = state.orderbooks.lock().await;
    let markets: Vec<MarketSnap> = state
        .config_symbols
        .iter()
        .filter_map(|sym| {
            let ob = obs.get(sym)?;
            let (bid, _) = ob.best_bid()?;
            let (ask, _) = ob.best_ask()?;
            let spread = ob.spread()?;
            Some(MarketSnap {
                symbol: sym.clone(),
                best_bid: bid.to_string(),
                best_ask: ask.to_string(),
                spread: spread.to_string(),
            })
        })
        .collect();
    drop(obs);

    // Console log
    let console_log = state.console_log.lock().await.entries();

    // Signal log (last 100)
    let sig = state.signal_log.lock().await;
    let signal_log: Vec<SignalRecord> = sig.iter().cloned().collect();
    drop(sig);

    // Trade history (last 100)
    let th = state.trade_history.lock().await;
    let trade_history: Vec<TradeRecord> = th.iter().rev().take(100).rev().cloned().collect();
    drop(th);

    // Exchange status
    let connected = state.connected_exchanges.lock().await;
    let all_exchanges = ["binance", "bybit", "okx", "kraken"];
    let exchange_status: Vec<ExchangeStatus> = all_exchanges
        .iter()
        .map(|name| ExchangeStatus {
            name: name.to_string(),
            connected: connected.contains(*name),
        })
        .collect();
    drop(connected);

    // Strategy status
    let votes = state.strategy_votes.lock().await;
    let strategy_status: Vec<StrategyStatusSnap> = votes
        .iter()
        .map(|v| StrategyStatusSnap {
            name: v.name.clone(),
            active: v.fired,
            side: v.side.map(|s| format!("{:?}", s)).unwrap_or_default(),
            strength: v.strength,
        })
        .collect();
    drop(votes);

    Snapshot {
        timestamp_ms: now_ms,
        mode: state.config_mode.clone(),
        uptime_secs: uptime,
        equity,
        starting_equity,
        daily_pnl,
        total_pnl,
        total_fees,
        drawdown_pct,
        win_rate,
        profit_factor,
        total_trades,
        expectancy,
        can_trade,
        consecutive_losses,
        trades_this_hour,
        daily_loss,
        regime,
        open_orders,
        markets,
        warmup_ready,
        indicators_ready,
        indicators_total,
        regime_ready,
        regime_atr_count,
        regime_atr_needed,
        console_log,
        signal_log,
        trade_history,
        exchange_status,
        strategy_status,
    }
}
