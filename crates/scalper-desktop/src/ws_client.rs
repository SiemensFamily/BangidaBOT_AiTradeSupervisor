use ewebsock::{WsEvent, WsMessage, WsReceiver, WsSender};
use serde::Deserialize;

const WS_URL: &str = "ws://localhost:3000/ws";

#[derive(Default, Clone, Deserialize)]
#[allow(dead_code)]
pub struct Snapshot {
    pub timestamp_ms: u64,
    pub mode: String,
    pub uptime_secs: u64,
    pub equity: f64,
    pub starting_equity: f64,
    pub daily_pnl: f64,
    pub total_pnl: f64,
    pub total_fees: f64,
    pub drawdown_pct: f64,
    pub win_rate: f64,
    pub profit_factor: f64,
    pub total_trades: u64,
    pub expectancy: f64,
    pub can_trade: bool,
    pub consecutive_losses: u32,
    pub trades_this_hour: u32,
    pub daily_loss: f64,
    pub regime: String,
    #[serde(default)]
    pub open_orders: Vec<OrderSnap>,
    #[serde(default)]
    pub markets: Vec<MarketSnap>,
    pub warmup_ready: bool,
    #[serde(default)]
    pub indicators_ready: u32,
    #[serde(default)]
    pub indicators_total: u32,
    #[serde(default)]
    pub regime_ready: bool,
    #[serde(default)]
    pub regime_atr_count: usize,
    #[serde(default)]
    pub regime_atr_needed: usize,
}

#[derive(Default, Clone, Deserialize)]
pub struct OrderSnap {
    pub order_id: String,
    pub symbol: String,
    pub side: String,
    pub price: String,
    pub quantity: String,
    pub filled_qty: String,
    pub status: String,
}

#[derive(Default, Clone, Deserialize)]
pub struct MarketSnap {
    pub symbol: String,
    pub best_bid: String,
    pub best_ask: String,
    pub spread: String,
}

pub struct WsClient {
    sender: Option<WsSender>,
    receiver: Option<WsReceiver>,
    connected: bool,
}

impl WsClient {
    pub fn new() -> Self {
        Self {
            sender: None,
            receiver: None,
            connected: false,
        }
    }

    /// Connect to the bot's WebSocket endpoint.
    /// The egui `Context` is forwarded so ewebsock can request repaints on
    /// incoming messages.
    pub fn connect(&mut self, ctx: &egui::Context) {
        let wake = ctx.clone();
        let options = ewebsock::Options::default();
        match ewebsock::connect_with_wakeup(
            WS_URL,
            options,
            move || {
                wake.request_repaint();
            },
        ) {
            Ok((sender, receiver)) => {
                self.sender = Some(sender);
                self.receiver = Some(receiver);
                self.connected = true;
            }
            Err(err) => {
                eprintln!("WebSocket connection error: {err}");
                self.connected = false;
            }
        }
    }

    /// Try to receive the latest snapshot from the WebSocket.
    /// Drains all queued messages and returns the most recent valid snapshot.
    pub fn poll(&mut self) -> Option<Snapshot> {
        let receiver = self.receiver.as_ref()?;
        let mut latest: Option<Snapshot> = None;
        while let Some(event) = receiver.try_recv() {
            match event {
                WsEvent::Message(WsMessage::Text(text)) => {
                    if let Ok(snap) = serde_json::from_str::<Snapshot>(&text) {
                        latest = Some(snap);
                    }
                }
                WsEvent::Closed | WsEvent::Error(_) => {
                    self.connected = false;
                }
                _ => {}
            }
        }
        latest
    }

    pub fn is_connected(&self) -> bool {
        self.connected
    }
}
