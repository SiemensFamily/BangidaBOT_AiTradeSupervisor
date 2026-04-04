use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct BinanceOrderResponse {
    #[serde(rename = "orderId")]
    pub order_id: u64,
    pub symbol: String,
    pub status: String,
    #[serde(rename = "clientOrderId")]
    pub client_order_id: String,
}

#[derive(Debug, Deserialize)]
pub struct BinanceBalance {
    pub asset: String,
    #[serde(rename = "availableBalance")]
    pub available_balance: String,
    #[serde(rename = "balance")]
    pub total_balance: String,
}

#[derive(Debug, Deserialize)]
pub struct BinanceAccountInfo {
    #[serde(rename = "totalWalletBalance")]
    pub total_wallet_balance: String,
    #[serde(rename = "availableBalance")]
    pub available_balance: String,
    pub assets: Vec<BinanceBalance>,
}

#[derive(Debug, Serialize)]
pub struct BinanceOrderRequest {
    pub symbol: String,
    pub side: String,
    #[serde(rename = "type")]
    pub order_type: String,
    #[serde(rename = "timeInForce", skip_serializing_if = "Option::is_none")]
    pub time_in_force: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<String>,
    #[serde(rename = "stopPrice", skip_serializing_if = "Option::is_none")]
    pub stop_price: Option<String>,
    #[serde(rename = "reduceOnly", skip_serializing_if = "Option::is_none")]
    pub reduce_only: Option<String>,
    #[serde(rename = "newOrderRespType")]
    pub resp_type: String,
    pub timestamp: String,
}

/// WebSocket stream message wrapper.
#[derive(Debug, Deserialize)]
pub struct WsStreamMessage {
    pub stream: String,
    pub data: serde_json::Value,
}

/// Depth update from WebSocket.
#[derive(Debug, Deserialize)]
pub struct WsDepthUpdate {
    pub s: String,
    pub b: Vec<[String; 2]>, // bids: [price, qty]
    pub a: Vec<[String; 2]>, // asks: [price, qty]
    #[serde(rename = "E")]
    pub event_time: u64,
}

/// Aggregated trade from WebSocket.
#[derive(Debug, Deserialize)]
pub struct WsAggTrade {
    pub s: String,
    pub p: String, // price
    pub q: String, // quantity
    pub m: bool,   // is buyer maker
    #[serde(rename = "E")]
    pub event_time: u64,
}

/// Kline/candlestick from WebSocket.
#[derive(Debug, Deserialize)]
pub struct WsKline {
    pub s: String,
    pub k: WsKlineInner,
}

#[derive(Debug, Deserialize)]
pub struct WsKlineInner {
    pub o: String,
    pub h: String,
    pub l: String,
    pub c: String,
    pub v: String,
    #[serde(rename = "T")]
    pub close_time: u64,
    pub x: bool, // is closed
}

/// Mark price update from WebSocket.
#[derive(Debug, Deserialize)]
pub struct WsMarkPrice {
    pub s: String,
    pub p: String,  // mark price
    pub r: String,  // funding rate
    #[serde(rename = "T")]
    pub next_funding_time: u64,
    #[serde(rename = "E")]
    pub event_time: u64,
}

/// Force order (liquidation) from WebSocket.
#[derive(Debug, Deserialize)]
pub struct WsForceOrder {
    pub o: WsForceOrderInner,
}

#[derive(Debug, Deserialize)]
pub struct WsForceOrderInner {
    pub s: String,  // symbol
    #[serde(rename = "S")]
    pub side: String,
    pub q: String,  // quantity
    pub p: String,  // price
    #[serde(rename = "T")]
    pub trade_time: u64,
}

/// Listen key response.
#[derive(Debug, Deserialize)]
pub struct ListenKeyResponse {
    #[serde(rename = "listenKey")]
    pub listen_key: String,
}
