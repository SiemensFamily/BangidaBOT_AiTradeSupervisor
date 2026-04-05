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
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex, RwLock};
use tracing::info;

use scalper_core::config::ScalperConfig;
use scalper_data::orderbook::OrderBook;
use scalper_data::regime::RegimeDetector;
use scalper_execution::order_tracker::OrderTracker;
use scalper_risk::risk_manager::RiskManager;

use scalper_data::indicators::Indicator;

use crate::IndicatorState;

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
    #[serde(default)]
    pub entry_price: String,
    #[serde(default)]
    pub exit_price: String,
    #[serde(default)]
    pub duration_secs: f64,
    #[serde(default)]
    pub status: String,
}

// ── Signal / Analyst log ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct SignalRecord {
    pub timestamp_ms: u64,
    pub symbol: String,
    pub side: String,
    pub action: String,         // TAKE, REDUCE, SKIP, SKIP_PAPER
    pub score: f64,             // ensemble confidence 0-100
    pub rsi: f64,
    pub ema_trend: String,      // "UP" / "DOWN" / "FLAT"
    pub atr: f64,
    pub regime: String,
    pub imbalance: f64,
    pub cvd: f64,
    pub reason: String,         // why taken/skipped
}

// ── Console log buffer ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ConsoleEntry {
    pub timestamp_ms: u64,
    pub level: String,      // INFO, WARN, ERROR, SUCCESS
    pub message: String,
}

/// Ring buffer for console log entries (keeps last N entries).
pub struct ConsoleLog {
    entries: Vec<ConsoleEntry>,
    max_entries: usize,
}

impl ConsoleLog {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Vec::with_capacity(max_entries),
            max_entries,
        }
    }

    pub fn push(&mut self, level: &str, message: String) {
        let entry = ConsoleEntry {
            timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
            level: level.to_string(),
            message,
        };
        if self.entries.len() >= self.max_entries {
            self.entries.remove(0);
        }
        self.entries.push(entry);
    }

    pub fn entries(&self) -> &[ConsoleEntry] {
        &self.entries
    }
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
    pub signal_log: Arc<Mutex<Vec<SignalRecord>>>,
    pub console_log: Arc<Mutex<ConsoleLog>>,
    pub connected_exchanges: Arc<Mutex<std::collections::HashSet<String>>>,
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
    // Exchange connection status
    exchange_status: Vec<ExchangeStatus>,
    // Warmup
    warmup_ready: bool,
    indicators_ready: u32,
    indicators_total: u32,
    regime_ready: bool,
    regime_atr_count: usize,
    regime_atr_needed: usize,
    // Indicators
    rsi: f64,
    ema_9: f64,
    ema_21: f64,
    atr: f64,
    // History
    trades: Vec<TradeRecord>,
    signals: Vec<SignalRecord>,
    console: Vec<ConsoleEntry>,
    // Extended stats
    total_wins: u64,
    total_losses: u64,
    avg_win: f64,
    avg_loss: f64,
    best_trade: f64,
    worst_trade: f64,
}

#[derive(Serialize)]
struct ExchangeStatus {
    name: String,
    connected: bool,
    symbols: usize,
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

// ── HTML ───────────────────────────────────────────────────────────────────

const HTML: &str = include_str!("dashboard.html");
const MANIFEST: &str = include_str!("manifest.json");

const ICON_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><rect width="512" height="512" rx="64" fill="#0d1117"/><path d="M128 384V192l128-64 128 64v192l-128 64z" fill="none" stroke="#58a6ff" stroke-width="24" stroke-linejoin="round"/><path d="M256 128v320M128 192l128 64 128-64" fill="none" stroke="#58a6ff" stroke-width="16" stroke-linejoin="round"/><circle cx="256" cy="256" r="20" fill="#2ea043"/></svg>"##;

const SW_JS: &str = "self.addEventListener('fetch', e => e.respondWith(fetch(e.request)));";

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
        .route("/manifest.json", get(serve_manifest))
        .route("/icon.svg", get(serve_icon))
        .route("/sw.js", get(serve_sw))
        .route("/api/config", get(get_config).put(put_config))
        .route("/api/trades.csv", get(get_trades_csv))
        .route("/api/debug", get(get_debug_snapshot))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    info!("Dashboard listening on http://localhost:3000");
    axum::serve(listener, app).await.unwrap();
}

async fn serve_html() -> Html<&'static str> {
    Html(HTML)
}

async fn serve_manifest() -> ([(header::HeaderName, &'static str); 1], &'static str) {
    ([(header::CONTENT_TYPE, "application/manifest+json")], MANIFEST)
}

async fn serve_icon() -> ([(header::HeaderName, &'static str); 1], &'static str) {
    ([(header::CONTENT_TYPE, "image/svg+xml")], ICON_SVG)
}

async fn serve_sw() -> ([(header::HeaderName, &'static str); 1], &'static str) {
    ([(header::CONTENT_TYPE, "application/javascript")], SW_JS)
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

// ── REST: Debug snapshot ──────────────────────────────────────────────────

async fn get_debug_snapshot(State(state): State<DashboardState>) -> impl IntoResponse {
    let snapshot = build_snapshot(&state).await;
    let json = serde_json::to_string_pretty(&snapshot).unwrap_or_default();
    ([(header::CONTENT_TYPE, "application/json")], json)
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

// ── Snapshot builder ───────────────────────────────────────────────────────

async fn build_snapshot(state: &DashboardState) -> Snapshot {
    let now_ms = chrono::Utc::now().timestamp_millis() as u64;
    let uptime = (now_ms.saturating_sub(state.start_time_ms)) / 1000;

    // Risk manager
    let rm = state.risk_manager.lock().await;
    let tracker = rm.pnl_tracker();
    let cb = rm.circuit_breaker();

    let equity = tracker.equity();
    let starting_equity = tracker.starting_equity();
    let daily_pnl = tracker.daily_pnl();
    let total_pnl = tracker.total_pnl();
    let total_fees = tracker.total_fees();
    let drawdown_pct = tracker.drawdown_pct();
    let win_rate = tracker.win_rate();
    let pf = tracker.profit_factor();
    let profit_factor = if pf.is_finite() { pf } else { 0.0 };
    let total_trades = tracker.total_trades();
    let exp = tracker.expectancy();
    let expectancy = if exp.is_finite() { exp } else { 0.0 };

    let can_trade = cb.can_trade(now_ms);
    let consecutive_losses = cb.consecutive_losses();
    let trades_this_hour = cb.trades_this_hour();
    let daily_loss = cb.daily_loss();
    drop(rm);

    // Regime
    let rd = state.regime_detector.lock().await;
    let regime = format!("{:?}", rd.regime());
    let regime_ready = rd.is_ready();
    let regime_atr_count = rd.atr_count();
    drop(rd);

    // Indicators warmup + values
    let ind = state.indicators.lock().await;
    let (indicators_ready, indicators_total) = ind.readiness();
    let rsi = ind.rsi.as_ref().map(|i| i.value()).unwrap_or(0.0);
    let ema_9 = ind.ema_9.as_ref().map(|i| i.value()).unwrap_or(0.0);
    let ema_21 = ind.ema_21.as_ref().map(|i| i.value()).unwrap_or(0.0);
    let atr = ind.atr.as_ref().map(|i| i.value()).unwrap_or(0.0);
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

    // Exchange connection status
    drop(obs);
    let connected = state.connected_exchanges.lock().await;
    let exchange_names = ["Binance", "Bybit", "OKX", "Kraken"];
    let exchange_status: Vec<ExchangeStatus> = exchange_names.iter().map(|name| {
        ExchangeStatus {
            name: name.to_string(),
            connected: connected.contains(*name),
            symbols: 0,
        }
    }).collect();
    drop(connected);

    // Trade history + extended stats
    let trades_vec = state.trade_history.lock().await;
    let trades: Vec<TradeRecord> = trades_vec.clone();
    let (total_wins, total_losses, avg_win, avg_loss, best_trade, worst_trade) = {
        let wins: Vec<f64> = trades.iter().filter(|t| t.pnl > 0.0).map(|t| t.pnl).collect();
        let losses: Vec<f64> = trades.iter().filter(|t| t.pnl <= 0.0).map(|t| t.pnl).collect();
        let tw = wins.len() as u64;
        let tl = losses.len() as u64;
        let aw = if tw > 0 { wins.iter().sum::<f64>() / tw as f64 } else { 0.0 };
        let al = if tl > 0 { losses.iter().sum::<f64>() / tl as f64 } else { 0.0 };
        let best = wins.iter().cloned().fold(0.0f64, f64::max);
        let worst = losses.iter().cloned().fold(0.0f64, f64::min);
        (tw, tl, aw, al, best, worst)
    };
    drop(trades_vec);

    // Signal log
    let signals = state.signal_log.lock().await.clone();

    // Console log
    let console = state.console_log.lock().await.entries().to_vec();

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
        exchange_status,
        warmup_ready,
        indicators_ready,
        indicators_total,
        regime_ready,
        regime_atr_count,
        regime_atr_needed,
        rsi,
        ema_9,
        ema_21,
        atr,
        trades,
        signals,
        console,
        total_wins,
        total_losses,
        avg_win,
        avg_loss,
        best_trade,
        worst_trade,
    }
}
