use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tracing::info;

use scalper_data::orderbook::OrderBook;
use scalper_data::regime::RegimeDetector;
use scalper_execution::order_tracker::OrderTracker;
use scalper_risk::risk_manager::RiskManager;

/// Shared state the dashboard reads from (never mutates).
#[derive(Clone)]
pub struct DashboardState {
    pub config_mode: String,
    pub config_symbols: Vec<String>,
    pub start_time_ms: u64,
    pub risk_manager: Arc<Mutex<RiskManager>>,
    pub order_tracker: Arc<OrderTracker>,
    pub orderbooks: Arc<Mutex<HashMap<String, OrderBook>>>,
    pub regime_detector: Arc<Mutex<RegimeDetector>>,
    pub ws_tx: broadcast::Sender<String>,
}

#[derive(Serialize)]
struct Snapshot {
    timestamp_ms: u64,
    mode: String,
    uptime_secs: u64,
    equity: f64,
    starting_equity: f64,
    daily_pnl: f64,
    total_pnl: f64,
    total_fees: f64,
    drawdown_pct: f64,
    win_rate: f64,
    profit_factor: f64,
    total_trades: u64,
    expectancy: f64,
    can_trade: bool,
    consecutive_losses: u32,
    trades_this_hour: u32,
    daily_loss: f64,
    regime: String,
    open_orders: Vec<OrderSnap>,
    markets: Vec<MarketSnap>,
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

const HTML: &str = include_str!("dashboard.html");

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
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    info!("Dashboard listening on http://localhost:3000");
    axum::serve(listener, app).await.unwrap();
}

async fn serve_html() -> Html<&'static str> {
    Html(HTML)
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<DashboardState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(mut socket: WebSocket, state: DashboardState) {
    // Send immediate snapshot on connect
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

async fn build_snapshot(state: &DashboardState) -> Snapshot {
    let now_ms = chrono::Utc::now().timestamp_millis() as u64;
    let uptime = (now_ms.saturating_sub(state.start_time_ms)) / 1000;

    // Lock risk manager to read PnL + circuit breaker
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
    let profit_factor = tracker.profit_factor();
    let total_trades = tracker.total_trades();
    let expectancy = tracker.expectancy();

    let can_trade = cb.can_trade(now_ms);
    let consecutive_losses = cb.consecutive_losses();
    let trades_this_hour = cb.trades_this_hour();
    let daily_loss = cb.daily_loss();

    drop(rm); // Release lock early

    // Regime
    let regime = format!("{:?}", state.regime_detector.lock().await.regime());

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
    }
}
