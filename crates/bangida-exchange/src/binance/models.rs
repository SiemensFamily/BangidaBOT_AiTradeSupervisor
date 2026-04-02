use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// WebSocket stream messages
// ---------------------------------------------------------------------------

/// Binance combined stream wrapper: `{"stream":"btcusdt@depth@100ms","data":{...}}`
#[derive(Debug, Deserialize)]
pub struct CombinedStreamMessage {
    pub stream: String,
    pub data: serde_json::Value,
}

/// Depth update from the `depth@100ms` stream.
#[derive(Debug, Clone, Deserialize)]
pub struct DepthUpdate {
    /// Event type, should be "depthUpdate".
    #[serde(rename = "e")]
    pub event_type: String,
    /// Event time in ms.
    #[serde(rename = "E")]
    pub event_time: u64,
    /// Transaction time in ms.
    #[serde(rename = "T")]
    pub transaction_time: u64,
    /// Symbol.
    #[serde(rename = "s")]
    pub symbol: String,
    /// First update ID in event.
    #[serde(rename = "U")]
    pub first_update_id: u64,
    /// Final update ID in event.
    #[serde(rename = "u")]
    pub final_update_id: u64,
    /// Previous final update ID.
    #[serde(rename = "pu")]
    pub prev_final_update_id: u64,
    /// Bids to update: [[price, quantity], ...]
    #[serde(rename = "b")]
    pub bids: Vec<[String; 2]>,
    /// Asks to update: [[price, quantity], ...]
    #[serde(rename = "a")]
    pub asks: Vec<[String; 2]>,
}

/// Aggregated trade from the `aggTrade` stream.
#[derive(Debug, Clone, Deserialize)]
pub struct BinanceAggTrade {
    #[serde(rename = "e")]
    pub event_type: String,
    #[serde(rename = "E")]
    pub event_time: u64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "a")]
    pub agg_trade_id: u64,
    #[serde(rename = "p")]
    pub price: String,
    #[serde(rename = "q")]
    pub quantity: String,
    #[serde(rename = "f")]
    pub first_trade_id: u64,
    #[serde(rename = "l")]
    pub last_trade_id: u64,
    #[serde(rename = "T")]
    pub trade_time: u64,
    #[serde(rename = "m")]
    pub is_buyer_maker: bool,
}

/// Kline / candlestick from the `kline_1m` stream.
#[derive(Debug, Clone, Deserialize)]
pub struct BinanceKlineEvent {
    #[serde(rename = "e")]
    pub event_type: String,
    #[serde(rename = "E")]
    pub event_time: u64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "k")]
    pub kline: BinanceKline,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BinanceKline {
    #[serde(rename = "t")]
    pub open_time: u64,
    #[serde(rename = "T")]
    pub close_time: u64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "i")]
    pub interval: String,
    #[serde(rename = "o")]
    pub open: String,
    #[serde(rename = "c")]
    pub close: String,
    #[serde(rename = "h")]
    pub high: String,
    #[serde(rename = "l")]
    pub low: String,
    #[serde(rename = "v")]
    pub volume: String,
    #[serde(rename = "x")]
    pub is_closed: bool,
}

/// Mark price from the `markPrice@1s` stream.
#[derive(Debug, Clone, Deserialize)]
pub struct BinanceMarkPrice {
    #[serde(rename = "e")]
    pub event_type: String,
    #[serde(rename = "E")]
    pub event_time: u64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "p")]
    pub mark_price: String,
    #[serde(rename = "r")]
    pub funding_rate: String,
    #[serde(rename = "T")]
    pub next_funding_time: u64,
}

// ---------------------------------------------------------------------------
// User data stream events
// ---------------------------------------------------------------------------

/// Order update from the user data stream.
#[derive(Debug, Clone, Deserialize)]
pub struct UserDataOrder {
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "c")]
    pub client_order_id: String,
    #[serde(rename = "S")]
    pub side: String,
    #[serde(rename = "o")]
    pub order_type: String,
    #[serde(rename = "q")]
    pub original_quantity: String,
    #[serde(rename = "p")]
    pub original_price: String,
    #[serde(rename = "X")]
    pub order_status: String,
    #[serde(rename = "i")]
    pub order_id: u64,
    #[serde(rename = "T")]
    pub order_trade_time: u64,
}

/// Account update from the user data stream.
#[derive(Debug, Clone, Deserialize)]
pub struct UserDataAccount {
    #[serde(rename = "B")]
    pub balances: Vec<UserDataBalance>,
    #[serde(rename = "P")]
    pub positions: Vec<UserDataPosition>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserDataBalance {
    #[serde(rename = "a")]
    pub asset: String,
    #[serde(rename = "wb")]
    pub wallet_balance: String,
    #[serde(rename = "cw")]
    pub cross_wallet_balance: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserDataPosition {
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "pa")]
    pub position_amount: String,
    #[serde(rename = "ep")]
    pub entry_price: String,
    #[serde(rename = "up")]
    pub unrealized_pnl: String,
    #[serde(rename = "ps")]
    pub position_side: String,
}

/// Wrapping envelope for user data stream events.
#[derive(Debug, Clone, Deserialize)]
pub struct UserDataEvent {
    #[serde(rename = "e")]
    pub event_type: String,
    #[serde(rename = "E")]
    pub event_time: u64,
    /// Present when event_type == "ORDER_TRADE_UPDATE"
    #[serde(rename = "o")]
    pub order: Option<UserDataOrder>,
    /// Present when event_type == "ACCOUNT_UPDATE"
    #[serde(rename = "a")]
    pub account: Option<UserDataAccount>,
}

// ---------------------------------------------------------------------------
// REST API response structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlaceOrderResponse {
    #[serde(rename = "orderId")]
    pub order_id: u64,
    #[serde(rename = "clientOrderId")]
    pub client_order_id: String,
    pub symbol: String,
    pub side: String,
    #[serde(rename = "type")]
    pub order_type: String,
    #[serde(rename = "origQty")]
    pub orig_qty: String,
    pub price: String,
    pub status: String,
    #[serde(rename = "updateTime")]
    pub update_time: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CancelOrderResponse {
    #[serde(rename = "orderId")]
    pub order_id: u64,
    #[serde(rename = "clientOrderId")]
    pub client_order_id: String,
    pub symbol: String,
    pub side: String,
    #[serde(rename = "type")]
    pub order_type: String,
    #[serde(rename = "origQty")]
    pub orig_qty: String,
    pub price: String,
    pub status: String,
    #[serde(rename = "updateTime")]
    pub update_time: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PositionResponse {
    pub symbol: String,
    #[serde(rename = "positionAmt")]
    pub position_amt: String,
    #[serde(rename = "entryPrice")]
    pub entry_price: String,
    #[serde(rename = "unRealizedProfit")]
    pub unrealized_profit: String,
    pub leverage: String,
    #[serde(rename = "isolatedMargin")]
    pub isolated_margin: String,
    #[serde(rename = "positionSide")]
    pub position_side: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccountResponse {
    #[serde(rename = "totalWalletBalance")]
    pub total_wallet_balance: String,
    #[serde(rename = "availableBalance")]
    pub available_balance: String,
    #[serde(rename = "totalUnrealizedProfit")]
    pub total_unrealized_profit: String,
    #[serde(rename = "totalMarginBalance")]
    pub total_margin_balance: String,
    #[serde(rename = "totalCrossUnPnl")]
    pub total_cross_un_pnl: String,
    pub positions: Vec<PositionResponse>,
}

/// Listen key for the user data stream.
#[derive(Debug, Clone, Deserialize)]
pub struct ListenKeyResponse {
    #[serde(rename = "listenKey")]
    pub listen_key: String,
}

/// Binance API error payload.
#[derive(Debug, Clone, Deserialize)]
pub struct BinanceApiError {
    pub code: i64,
    pub msg: String,
}

/// Set-leverage response.
#[derive(Debug, Clone, Deserialize)]
pub struct SetLeverageResponse {
    pub leverage: u32,
    pub symbol: String,
    #[serde(rename = "maxNotionalValue")]
    pub max_notional_value: Option<String>,
}

/// Bulk cancel response element.
#[derive(Debug, Clone, Deserialize)]
pub struct CancelAllResponse {
    pub code: i64,
    pub msg: String,
}

impl std::fmt::Display for BinanceApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Binance API error {}: {}", self.code, self.msg)
    }
}

impl std::error::Error for BinanceApiError {}

/// Helper to parse a decimal string.
pub fn parse_decimal(s: &str) -> Result<Decimal, rust_decimal::Error> {
    s.parse::<Decimal>()
}
