use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct KrakenResponse<T> {
    pub result: Option<String>,
    #[serde(rename = "sendStatus")]
    pub send_status: Option<T>,
    pub error: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct KrakenOrderResult {
    pub order_id: Option<String>,
    #[serde(rename = "receivedTime")]
    pub received_time: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct KrakenAccountInfo {
    pub balances: Option<std::collections::HashMap<String, String>>,
}

/// Kraken Futures WebSocket message.
///
/// Kraken uses camelCase for some fields (e.g. `markPrice`, `fundingRate`).
/// We use `#[serde(alias)]` so both snake_case and camelCase are accepted.
#[derive(Debug, Deserialize)]
pub struct WsMessage {
    pub feed: Option<String>,
    pub product_id: Option<String>,
    // Order book
    pub bids: Option<Vec<WsPriceLevel>>,
    pub asks: Option<Vec<WsPriceLevel>>,
    // Trade
    pub side: Option<String>,
    pub price: Option<f64>,
    pub qty: Option<f64>,
    pub time: Option<u64>,
    // Ticker — Kraken sends camelCase: markPrice, fundingRate, nextFundingRateTime
    #[serde(alias = "markPrice")]
    pub mark_price: Option<f64>,
    #[serde(alias = "fundingRate")]
    pub funding_rate: Option<f64>,
    #[serde(alias = "nextFundingRateTime")]
    pub next_funding_rate_time: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct WsPriceLevel {
    pub price: f64,
    pub qty: f64,
}
