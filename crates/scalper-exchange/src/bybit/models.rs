use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct BybitResponse<T> {
    #[serde(rename = "retCode")]
    pub ret_code: i32,
    #[serde(rename = "retMsg")]
    pub ret_msg: String,
    pub result: Option<T>,
}

#[derive(Debug, Deserialize)]
pub struct BybitOrderResult {
    #[serde(rename = "orderId")]
    pub order_id: String,
    #[serde(rename = "orderLinkId")]
    pub order_link_id: String,
}

#[derive(Debug, Deserialize)]
pub struct BybitWalletBalance {
    pub list: Vec<BybitAccountInfo>,
}

#[derive(Debug, Deserialize)]
pub struct BybitAccountInfo {
    #[serde(rename = "totalAvailableBalance")]
    pub total_available_balance: String,
    #[serde(rename = "totalWalletBalance")]
    pub total_wallet_balance: String,
}

/// WebSocket message envelope.
#[derive(Debug, Deserialize)]
pub struct WsMessage {
    pub topic: Option<String>,
    pub data: Option<serde_json::Value>,
    #[serde(rename = "type")]
    pub msg_type: Option<String>,
    pub ts: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct WsOrderBookData {
    pub s: String,
    pub b: Vec<[String; 2]>,
    pub a: Vec<[String; 2]>,
    pub u: u64,
}

#[derive(Debug, Deserialize)]
pub struct WsTradeItem {
    #[serde(rename = "S")]
    pub side: String,
    pub s: String,
    pub p: String,
    pub v: String,
    #[serde(rename = "T")]
    pub timestamp: u64,
}

#[derive(Debug, Deserialize)]
pub struct WsTickerData {
    pub symbol: String,
    #[serde(rename = "markPrice")]
    pub mark_price: Option<String>,
    #[serde(rename = "fundingRate")]
    pub funding_rate: Option<String>,
    #[serde(rename = "nextFundingTime")]
    pub next_funding_time: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WsKlineItem {
    pub start: u64,
    pub end: u64,
    pub open: String,
    pub high: String,
    pub low: String,
    pub close: String,
    pub volume: String,
    pub confirm: bool,
}
