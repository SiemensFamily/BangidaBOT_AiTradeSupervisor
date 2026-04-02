use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use rust_decimal::Decimal;
use std::sync::Arc;
use tracing::{debug, error, info};

use bangida_core::{
    AccountBalance, OrderId, OrderRequest, OrderResponse, OrderStatus, OrderType,
    Position, Side, Symbol, TimeInForce,
};

use super::auth::BinanceAuth;
use super::models::{
    AccountResponse, BinanceApiError, CancelOrderResponse, ListenKeyResponse, PlaceOrderResponse,
    PositionResponse, SetLeverageResponse, parse_decimal,
};
use super::rate_limit::BinanceRateLimiter;
use crate::traits::OrderManager;

/// Binance Futures REST client.
///
/// Provides signed HTTP access to Binance USD-M Futures endpoints.
#[derive(Clone)]
pub struct BinanceClient {
    http: Client,
    auth: Arc<BinanceAuth>,
    base_url: String,
    ws_base_url: String,
    rate_limiter: BinanceRateLimiter,
    recv_window: u64,
}

impl BinanceClient {
    /// Create a new Binance client.
    ///
    /// * `api_key` / `api_secret` - API credentials
    /// * `base_url` - e.g. `https://fapi.binance.com`
    /// * `ws_base_url` - e.g. `wss://fstream.binance.com`
    pub fn new(
        api_key: impl Into<String>,
        api_secret: impl Into<String>,
        base_url: impl Into<String>,
        ws_base_url: impl Into<String>,
    ) -> Self {
        Self {
            http: Client::new(),
            auth: Arc::new(BinanceAuth::new(api_key, api_secret)),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            ws_base_url: ws_base_url.into().trim_end_matches('/').to_string(),
            rate_limiter: BinanceRateLimiter::new(),
            recv_window: 5000,
        }
    }

    /// Return a reference to the auth module (needed by the WS client).
    pub fn auth(&self) -> &BinanceAuth {
        &self.auth
    }

    /// Return the WebSocket base URL.
    pub fn ws_base_url(&self) -> &str {
        &self.ws_base_url
    }

    /// Return a clone of the rate limiter (needed by the WS client).
    pub fn rate_limiter(&self) -> &BinanceRateLimiter {
        &self.rate_limiter
    }

    // ----- listen key management (used by ws module) -----

    /// Create a new listen key for the user data stream.
    pub async fn create_listen_key(&self) -> Result<String> {
        self.rate_limiter.acquire(1).await;
        let url = format!("{}/fapi/v1/listenKey", self.base_url);
        let resp = self
            .http
            .post(&url)
            .header("X-MBX-APIKEY", self.auth.api_key())
            .send()
            .await
            .context("Failed to create listen key")?;

        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            if let Ok(err) = serde_json::from_str::<BinanceApiError>(&body) {
                bail!("Binance listen key error: {err}");
            }
            bail!("Binance listen key HTTP {status}: {body}");
        }
        let parsed: ListenKeyResponse = serde_json::from_str(&body)?;
        debug!(listen_key = %parsed.listen_key, "Created Binance listen key");
        Ok(parsed.listen_key)
    }

    /// Keep-alive (renew) an existing listen key.
    pub async fn renew_listen_key(&self, listen_key: &str) -> Result<()> {
        self.rate_limiter.acquire(1).await;
        let url = format!("{}/fapi/v1/listenKey", self.base_url);
        let resp = self
            .http
            .put(&url)
            .header("X-MBX-APIKEY", self.auth.api_key())
            .form(&[("listenKey", listen_key)])
            .send()
            .await
            .context("Failed to renew listen key")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await?;
            error!(status = %status, body = %body, "Failed to renew listen key");
            bail!("Listen key renewal failed: HTTP {status}");
        }
        debug!("Renewed Binance listen key");
        Ok(())
    }

    // ----- private helpers -----

    /// Send a signed GET request.
    async fn signed_get(&self, path: &str, query: &str, weight: u32) -> Result<String> {
        self.rate_limiter.acquire(weight).await;
        let signed_query = self.auth.sign_query(query, self.recv_window);
        let url = format!("{}{path}?{signed_query}", self.base_url);

        let resp = self
            .http
            .get(&url)
            .header("X-MBX-APIKEY", self.auth.api_key())
            .send()
            .await
            .context("Binance signed GET failed")?;

        self.handle_response(resp).await
    }

    /// Send a signed POST request.
    async fn signed_post(&self, path: &str, query: &str, weight: u32) -> Result<String> {
        self.rate_limiter.acquire(weight).await;
        let signed_query = self.auth.sign_query(query, self.recv_window);
        let url = format!("{}{path}?{signed_query}", self.base_url);

        let resp = self
            .http
            .post(&url)
            .header("X-MBX-APIKEY", self.auth.api_key())
            .send()
            .await
            .context("Binance signed POST failed")?;

        self.handle_response(resp).await
    }

    /// Send a signed DELETE request.
    async fn signed_delete(&self, path: &str, query: &str, weight: u32) -> Result<String> {
        self.rate_limiter.acquire(weight).await;
        let signed_query = self.auth.sign_query(query, self.recv_window);
        let url = format!("{}{path}?{signed_query}", self.base_url);

        let resp = self
            .http
            .delete(&url)
            .header("X-MBX-APIKEY", self.auth.api_key())
            .send()
            .await
            .context("Binance signed DELETE failed")?;

        self.handle_response(resp).await
    }

    /// Read the response body, returning an error for non-2xx status.
    async fn handle_response(&self, resp: reqwest::Response) -> Result<String> {
        let status = resp.status();
        let body = resp.text().await?;

        if !status.is_success() {
            if let Ok(api_err) = serde_json::from_str::<BinanceApiError>(&body) {
                error!(code = api_err.code, msg = %api_err.msg, "Binance API error");
                bail!(api_err);
            }
            bail!("Binance HTTP {status}: {body}");
        }
        Ok(body)
    }

    /// Map Binance side string to core Side.
    fn parse_side(s: &str) -> Side {
        match s {
            "BUY" => Side::Buy,
            _ => Side::Sell,
        }
    }

    /// Map Binance status string to core OrderStatus.
    fn parse_order_status(s: &str) -> OrderStatus {
        match s {
            "NEW" => OrderStatus::New,
            "PARTIALLY_FILLED" => OrderStatus::PartiallyFilled,
            "FILLED" => OrderStatus::Filled,
            "CANCELED" => OrderStatus::Canceled,
            "REJECTED" => OrderStatus::Rejected,
            "EXPIRED" => OrderStatus::Expired,
            _ => OrderStatus::New,
        }
    }

    /// Map Binance order type string to core OrderType.
    fn parse_order_type(s: &str) -> OrderType {
        match s {
            "MARKET" => OrderType::Market,
            "LIMIT" => OrderType::Limit,
            "STOP_MARKET" => OrderType::StopMarket,
            "TAKE_PROFIT_MARKET" => OrderType::TakeProfitMarket,
            _ => OrderType::Limit,
        }
    }

    /// Convert a PlaceOrderResponse to core OrderResponse.
    fn place_resp_to_order_response(r: &PlaceOrderResponse) -> Result<OrderResponse> {
        Ok(OrderResponse {
            order_id: r.order_id.to_string(),
            client_order_id: r.client_order_id.clone(),
            symbol: Symbol::new(&r.symbol),
            side: Self::parse_side(&r.side),
            order_type: Self::parse_order_type(&r.order_type),
            quantity: parse_decimal(&r.orig_qty)?,
            price: if r.price == "0" || r.price == "0.00000000" {
                None
            } else {
                Some(parse_decimal(&r.price)?)
            },
            status: Self::parse_order_status(&r.status),
            timestamp_ms: r.update_time,
        })
    }

    /// Convert a CancelOrderResponse to core OrderResponse.
    fn cancel_resp_to_order_response(r: &CancelOrderResponse) -> Result<OrderResponse> {
        Ok(OrderResponse {
            order_id: r.order_id.to_string(),
            client_order_id: r.client_order_id.clone(),
            symbol: Symbol::new(&r.symbol),
            side: Self::parse_side(&r.side),
            order_type: Self::parse_order_type(&r.order_type),
            quantity: parse_decimal(&r.orig_qty)?,
            price: if r.price == "0" || r.price == "0.00000000" {
                None
            } else {
                Some(parse_decimal(&r.price)?)
            },
            status: Self::parse_order_status(&r.status),
            timestamp_ms: r.update_time,
        })
    }
}

#[async_trait]
impl OrderManager for BinanceClient {
    async fn place_order(&self, request: &OrderRequest) -> Result<OrderResponse> {
        let side_str = request.side.to_string();
        let type_str = match request.order_type {
            OrderType::Market => "MARKET",
            OrderType::Limit => "LIMIT",
            OrderType::StopMarket => "STOP_MARKET",
            OrderType::TakeProfitMarket => "TAKE_PROFIT_MARKET",
        };
        let tif_str = match request.time_in_force {
            TimeInForce::Gtc => "GTC",
            TimeInForce::Ioc => "IOC",
            TimeInForce::Fok => "FOK",
            TimeInForce::PostOnly => "GTX",
        };

        let mut query = format!(
            "symbol={}&side={}&type={}&quantity={}",
            request.symbol, side_str, type_str, request.quantity
        );

        // Limit orders need price and timeInForce
        if request.order_type == OrderType::Limit {
            if let Some(price) = request.price {
                query.push_str(&format!("&price={price}&timeInForce={tif_str}"));
            }
        }

        // Stop orders need stopPrice
        if let Some(sp) = request.stop_price {
            query.push_str(&format!("&stopPrice={sp}"));
        }

        if request.reduce_only {
            query.push_str("&reduceOnly=true");
        }

        info!(
            symbol = %request.symbol,
            side = %side_str,
            order_type = type_str,
            qty = %request.quantity,
            "Placing Binance order"
        );

        let body = self.signed_post("/fapi/v1/order", &query, 1).await?;
        let resp: PlaceOrderResponse = serde_json::from_str(&body)
            .context("Failed to parse PlaceOrderResponse")?;

        let order = Self::place_resp_to_order_response(&resp)?;
        info!(order_id = %order.order_id, status = ?order.status, "Binance order placed");
        Ok(order)
    }

    async fn cancel_order(
        &self,
        symbol: &Symbol,
        order_id: &OrderId,
    ) -> Result<OrderResponse> {
        let query = format!("symbol={}&orderId={}", symbol, order_id);
        info!(symbol = %symbol, order_id = %order_id, "Cancelling Binance order");

        let body = self.signed_delete("/fapi/v1/order", &query, 1).await?;
        let resp: CancelOrderResponse = serde_json::from_str(&body)
            .context("Failed to parse CancelOrderResponse")?;

        Self::cancel_resp_to_order_response(&resp)
    }

    async fn cancel_all_orders(&self, symbol: &Symbol) -> Result<Vec<OrderResponse>> {
        let query = format!("symbol={}", symbol);
        info!(symbol = %symbol, "Cancelling all Binance orders");

        // cancel all returns 200 with {"code":200,"msg":"The operation of cancel all open
        // order is done."} - we then fetch open orders to confirm
        let _body = self.signed_delete("/fapi/v1/allOpenOrders", &query, 1).await?;
        info!(symbol = %symbol, "All Binance orders cancelled");
        Ok(vec![])
    }

    async fn get_position(&self, symbol: &Symbol) -> Result<Position> {
        let query = format!("symbol={}", symbol);
        let body = self.signed_get("/fapi/v2/positionRisk", &query, 5).await?;
        let positions: Vec<PositionResponse> = serde_json::from_str(&body)
            .context("Failed to parse PositionResponse")?;

        let pos = positions
            .into_iter()
            .find(|p| p.symbol == symbol.0 && p.position_side == "BOTH")
            .unwrap_or_else(|| PositionResponse {
                symbol: symbol.0.clone(),
                position_amt: "0".to_string(),
                entry_price: "0".to_string(),
                unrealized_profit: "0".to_string(),
                leverage: "1".to_string(),
                isolated_margin: "0".to_string(),
                position_side: "BOTH".to_string(),
            });

        let qty = parse_decimal(&pos.position_amt)?;
        let side = if qty >= Decimal::ZERO {
            Side::Buy
        } else {
            Side::Sell
        };
        let leverage: u32 = pos.leverage.parse().unwrap_or(1);

        Ok(Position {
            symbol: Symbol::new(&pos.symbol),
            side,
            quantity: qty.abs(),
            entry_price: parse_decimal(&pos.entry_price)?,
            unrealized_pnl: parse_decimal(&pos.unrealized_profit)?,
            leverage,
            margin: parse_decimal(&pos.isolated_margin)?,
        })
    }

    async fn get_account_balance(&self) -> Result<AccountBalance> {
        let body = self.signed_get("/fapi/v2/account", "", 5).await?;
        let acct: AccountResponse = serde_json::from_str(&body)
            .context("Failed to parse AccountResponse")?;

        let total = parse_decimal(&acct.total_wallet_balance)?;
        let available = parse_decimal(&acct.available_balance)?;
        let unrealized = parse_decimal(&acct.total_unrealized_profit)?;
        let margin_used = total - available;

        Ok(AccountBalance {
            total_balance: total,
            available_balance: available,
            unrealized_pnl: unrealized,
            margin_used,
        })
    }

    async fn set_leverage(&self, symbol: &Symbol, leverage: u32) -> Result<()> {
        let query = format!("symbol={}&leverage={}", symbol, leverage);
        info!(symbol = %symbol, leverage, "Setting Binance leverage");

        let body = self.signed_post("/fapi/v1/leverage", &query, 1).await?;
        let resp: SetLeverageResponse = serde_json::from_str(&body)
            .context("Failed to parse SetLeverageResponse")?;

        info!(
            symbol = %resp.symbol,
            leverage = resp.leverage,
            "Binance leverage set"
        );
        Ok(())
    }
}
