use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// WebSocket stream messages
// ---------------------------------------------------------------------------

/// Top-level Bybit WebSocket message envelope.
#[derive(Debug, Deserialize)]
pub struct BybitWsMessage {
    /// Topic, e.g. "orderbook.50.BTCUSDT"
    pub topic: Option<String>,
    /// "snapshot" or "delta"
    #[serde(rename = "type")]
    pub msg_type: Option<String>,
    /// Timestamp in ms
    pub ts: Option<u64>,
    /// Data payload (varies by topic)
    pub data: Option<serde_json::Value>,
    /// Operation response (subscribe, auth, pong)
    pub op: Option<String>,
    pub success: Option<bool>,
    pub ret_msg: Option<String>,
}

/// Order book snapshot from `orderbook.50` topic.
#[derive(Debug, Clone, Deserialize)]
pub struct OrderBookSnapshot {
    /// Symbol
    pub s: String,
    /// Bids: [[price, qty], ...]
    pub b: Vec<[String; 2]>,
    /// Asks: [[price, qty], ...]
    pub a: Vec<[String; 2]>,
    /// Update ID
    pub u: u64,
    /// Sequence
    pub seq: Option<u64>,
}

/// Order book delta update from `orderbook.50` topic.
#[derive(Debug, Clone, Deserialize)]
pub struct OrderBookDelta {
    pub s: String,
    pub b: Vec<[String; 2]>,
    pub a: Vec<[String; 2]>,
    pub u: u64,
    pub seq: Option<u64>,
}

/// Public trade from `publicTrade` topic.
#[derive(Debug, Clone, Deserialize)]
pub struct PublicTrade {
    /// Trade ID
    pub i: String,
    /// Timestamp ms
    #[serde(rename = "T")]
    pub timestamp: u64,
    /// Price
    pub p: String,
    /// Quantity
    pub v: String,
    /// Side: "Buy" or "Sell"
    #[serde(rename = "S")]
    pub side: String,
    /// Symbol
    pub s: String,
    /// Is block trade
    #[serde(rename = "BT")]
    pub is_block_trade: Option<bool>,
}

/// Kline data from `kline.1` topic.
#[derive(Debug, Clone, Deserialize)]
pub struct KlineData {
    /// Start timestamp ms
    pub start: u64,
    /// End timestamp ms
    pub end: u64,
    /// Interval
    pub interval: String,
    /// Open
    pub open: String,
    /// Close
    pub close: String,
    /// High
    pub high: String,
    /// Low
    pub low: String,
    /// Volume
    pub volume: String,
    /// Turnover
    pub turnover: String,
    /// Whether this kline is confirmed (closed)
    pub confirm: bool,
    /// Timestamp
    pub timestamp: u64,
}

/// Ticker data from `tickers` topic.
#[derive(Debug, Clone, Deserialize)]
pub struct TickerData {
    pub symbol: String,
    #[serde(rename = "markPrice")]
    pub mark_price: Option<String>,
    #[serde(rename = "fundingRate")]
    pub funding_rate: Option<String>,
    #[serde(rename = "nextFundingTime")]
    pub next_funding_time: Option<String>,
}

// ---------------------------------------------------------------------------
// REST API response structs
// ---------------------------------------------------------------------------

/// Bybit V5 API envelope.
#[derive(Debug, Clone, Deserialize)]
pub struct BybitApiResponse<T> {
    #[serde(rename = "retCode")]
    pub ret_code: i64,
    #[serde(rename = "retMsg")]
    pub ret_msg: String,
    pub result: Option<T>,
    pub time: Option<u64>,
}

impl<T> BybitApiResponse<T> {
    /// Check if the API call was successful.
    pub fn is_ok(&self) -> bool {
        self.ret_code == 0
    }
}

/// Order creation / cancellation result.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BybitOrderResult {
    #[serde(rename = "orderId")]
    pub order_id: String,
    #[serde(rename = "orderLinkId")]
    pub order_link_id: Option<String>,
}

/// Order info within a list response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BybitOrderInfo {
    #[serde(rename = "orderId")]
    pub order_id: String,
    #[serde(rename = "orderLinkId")]
    pub order_link_id: String,
    pub symbol: String,
    pub side: String,
    #[serde(rename = "orderType")]
    pub order_type: String,
    pub qty: String,
    pub price: String,
    #[serde(rename = "orderStatus")]
    pub order_status: String,
    #[serde(rename = "updatedTime")]
    pub updated_time: Option<String>,
}

/// Position info from /v5/position/list.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BybitPositionInfo {
    pub symbol: String,
    pub side: String,
    pub size: String,
    #[serde(rename = "avgPrice")]
    pub avg_price: String,
    #[serde(rename = "unrealisedPnl")]
    pub unrealised_pnl: String,
    pub leverage: String,
    #[serde(rename = "positionIM")]
    pub position_im: Option<String>,
}

/// Position list result.
#[derive(Debug, Clone, Deserialize)]
pub struct BybitPositionList {
    pub list: Vec<BybitPositionInfo>,
}

/// Wallet balance coin info.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BybitCoinBalance {
    pub coin: String,
    #[serde(rename = "walletBalance")]
    pub wallet_balance: String,
    #[serde(rename = "availableToWithdraw")]
    pub available_to_withdraw: String,
    #[serde(rename = "unrealisedPnl")]
    pub unrealised_pnl: String,
}

/// Account info from /v5/account/wallet-balance.
#[derive(Debug, Clone, Deserialize)]
pub struct BybitAccountInfo {
    #[serde(rename = "totalWalletBalance")]
    pub total_wallet_balance: String,
    #[serde(rename = "totalAvailableBalance")]
    pub total_available_balance: String,
    #[serde(rename = "totalMarginBalance")]
    pub total_margin_balance: String,
    pub coin: Option<Vec<BybitCoinBalance>>,
}

/// Wallet balance list result.
#[derive(Debug, Clone, Deserialize)]
pub struct BybitWalletBalanceResult {
    pub list: Vec<BybitAccountInfo>,
}

/// Set leverage response.
#[derive(Debug, Clone, Deserialize)]
pub struct BybitSetLeverageResult {}

/// Helper to parse a decimal string.
pub fn parse_decimal(s: &str) -> Result<Decimal, rust_decimal::Error> {
    s.parse::<Decimal>()
}

/// Bybit cancel-all response.
#[derive(Debug, Clone, Deserialize)]
pub struct BybitCancelAllResult {
    pub list: Option<Vec<BybitOrderResult>>,
    pub success: Option<String>,
}

impl std::fmt::Display for BybitApiResponse<serde_json::Value> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Bybit API retCode={}: {}", self.ret_code, self.ret_msg)
    }
}
