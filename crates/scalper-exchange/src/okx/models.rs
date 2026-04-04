use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct OkxResponse<T> {
    pub code: String,
    pub msg: String,
    pub data: Vec<T>,
}

#[derive(Debug, Deserialize)]
pub struct OkxOrderResult {
    #[serde(rename = "ordId")]
    pub ord_id: String,
    #[serde(rename = "clOrdId")]
    pub cl_ord_id: String,
    #[serde(rename = "sCode")]
    pub s_code: String,
    #[serde(rename = "sMsg")]
    pub s_msg: String,
}

#[derive(Debug, Deserialize)]
pub struct OkxBalanceData {
    #[serde(rename = "totalEq")]
    pub total_eq: String,
    #[serde(rename = "availBal")]
    pub avail_bal: Option<String>,
    pub details: Vec<OkxBalanceDetail>,
}

#[derive(Debug, Deserialize)]
pub struct OkxBalanceDetail {
    pub ccy: String,
    #[serde(rename = "availBal")]
    pub avail_bal: String,
    #[serde(rename = "cashBal")]
    pub cash_bal: String,
}

/// WebSocket push message.
#[derive(Debug, Deserialize)]
pub struct WsPushMessage {
    pub arg: Option<WsArg>,
    pub data: Option<Vec<serde_json::Value>>,
    pub event: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WsArg {
    pub channel: String,
    #[serde(rename = "instId")]
    pub inst_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WsTradeData {
    #[serde(rename = "instId")]
    pub inst_id: String,
    pub px: String,
    pub sz: String,
    pub side: String,
    pub ts: String,
}

#[derive(Debug, Deserialize)]
pub struct WsTickerData {
    #[serde(rename = "instId")]
    pub inst_id: String,
    #[serde(rename = "markPx")]
    pub mark_px: Option<String>,
    #[serde(rename = "fundingRate")]
    pub funding_rate: Option<String>,
    #[serde(rename = "nextFundingTime")]
    pub next_funding_time: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WsBookData {
    pub asks: Vec<[String; 4]>, // [price, size, liquidated_orders, num_orders]
    pub bids: Vec<[String; 4]>,
    pub ts: String,
    #[serde(rename = "instId")]
    pub inst_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WsCandleData {
    pub ts: Option<String>,     // OKX sends candles as arrays, but we use object form
    pub o: Option<String>,
    pub h: Option<String>,
    pub l: Option<String>,
    pub c: Option<String>,
    pub vol: Option<String>,
    pub confirm: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WsLiquidationData {
    #[serde(rename = "instId")]
    pub inst_id: String,
    pub side: String,
    pub sz: String,
    #[serde(rename = "bkPx")]
    pub bk_px: String,
    pub ts: String,
}
