use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use rust_decimal::Decimal;
use std::sync::Arc;
use tracing::{error, info};

use bangida_core::{
    AccountBalance, OrderId, OrderRequest, OrderResponse, OrderStatus, OrderType, Position, Side,
    Symbol, TimeInForce,
};

use super::auth::BybitAuth;
use super::models::{
    BybitApiResponse, BybitCancelAllResult, BybitOrderResult, BybitPositionInfo,
    BybitPositionList, BybitWalletBalanceResult, parse_decimal,
};
use super::rate_limit::BybitRateLimiter;
use crate::traits::OrderManager;

/// Bybit V5 REST client.
#[derive(Clone)]
pub struct BybitClient {
    http: Client,
    auth: Arc<BybitAuth>,
    base_url: String,
    ws_base_url: String,
    rate_limiter: BybitRateLimiter,
    recv_window: u64,
}

impl BybitClient {
    pub fn new(
        api_key: impl Into<String>,
        api_secret: impl Into<String>,
        base_url: impl Into<String>,
        ws_base_url: impl Into<String>,
    ) -> Self {
        Self {
            http: Client::new(),
            auth: Arc::new(BybitAuth::new(api_key, api_secret)),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            ws_base_url: ws_base_url.into().trim_end_matches('/').to_string(),
            rate_limiter: BybitRateLimiter::new(),
            recv_window: 5000,
        }
    }

    /// Return a reference to the auth module (needed by the WS client).
    pub fn auth(&self) -> &BybitAuth {
        &self.auth
    }

    /// Return the WebSocket base URL.
    pub fn ws_base_url(&self) -> &str {
        &self.ws_base_url
    }

    /// Return a clone of the rate limiter.
    pub fn rate_limiter(&self) -> &BybitRateLimiter {
        &self.rate_limiter
    }

    // ----- HTTP helpers -----

    /// Signed GET request to a Bybit V5 endpoint.
    async fn signed_get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &str,
    ) -> Result<T> {
        self.rate_limiter.acquire(1).await;
        let (timestamp, sign, recv_window) = self.auth.sign_request(query, self.recv_window);
        let url = if query.is_empty() {
            format!("{}{path}", self.base_url)
        } else {
            format!("{}{path}?{query}", self.base_url)
        };

        let resp = self
            .http
            .get(&url)
            .header("X-BAPI-API-KEY", self.auth.api_key())
            .header("X-BAPI-TIMESTAMP", &timestamp)
            .header("X-BAPI-SIGN", &sign)
            .header("X-BAPI-RECV-WINDOW", &recv_window)
            .send()
            .await
            .context("Bybit signed GET failed")?;

        self.handle_response::<T>(resp).await
    }

    /// Signed POST request with JSON body.
    async fn signed_post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<T> {
        self.rate_limiter.acquire(1).await;
        let body_str = serde_json::to_string(body)?;
        let (timestamp, sign, recv_window) = self.auth.sign_request(&body_str, self.recv_window);
        let url = format!("{}{path}", self.base_url);

        let resp = self
            .http
            .post(&url)
            .header("X-BAPI-API-KEY", self.auth.api_key())
            .header("X-BAPI-TIMESTAMP", &timestamp)
            .header("X-BAPI-SIGN", &sign)
            .header("X-BAPI-RECV-WINDOW", &recv_window)
            .header("Content-Type", "application/json")
            .body(body_str)
            .send()
            .await
            .context("Bybit signed POST failed")?;

        self.handle_response::<T>(resp).await
    }

    /// Read and validate a Bybit API response.
    async fn handle_response<T: serde::de::DeserializeOwned>(
        &self,
        resp: reqwest::Response,
    ) -> Result<T> {
        let status = resp.status();
        let body = resp.text().await?;

        if !status.is_success() {
            error!(status = %status, body = %body, "Bybit HTTP error");
            bail!("Bybit HTTP {status}: {body}");
        }

        // Parse the envelope first to check retCode
        let envelope: BybitApiResponse<serde_json::Value> = serde_json::from_str(&body)
            .context("Failed to parse Bybit response envelope")?;

        if !envelope.is_ok() {
            error!(
                ret_code = envelope.ret_code,
                ret_msg = %envelope.ret_msg,
                "Bybit API error"
            );
            bail!(
                "Bybit API error {}: {}",
                envelope.ret_code,
                envelope.ret_msg
            );
        }

        // Now parse the full typed response
        let typed: BybitApiResponse<T> = serde_json::from_str(&body)
            .context("Failed to parse Bybit typed response")?;

        typed
            .result
            .context("Bybit API returned null result for a successful response")
    }

    // ----- Mapping helpers -----

    #[allow(dead_code)]
    pub(crate) fn parse_side(s: &str) -> Side {
        match s {
            "Buy" => Side::Buy,
            _ => Side::Sell,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn parse_order_status(s: &str) -> OrderStatus {
        match s {
            "New" | "Created" | "Untriggered" => OrderStatus::New,
            "PartiallyFilled" | "Active" => OrderStatus::PartiallyFilled,
            "Filled" => OrderStatus::Filled,
            "Cancelled" | "Deactivated" => OrderStatus::Canceled,
            "Rejected" => OrderStatus::Rejected,
            "Expired" => OrderStatus::Expired,
            _ => OrderStatus::New,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn parse_order_type(s: &str) -> OrderType {
        match s {
            "Market" => OrderType::Market,
            "Limit" => OrderType::Limit,
            _ => OrderType::Limit,
        }
    }
}

#[async_trait]
impl OrderManager for BybitClient {
    async fn place_order(&self, request: &OrderRequest) -> Result<OrderResponse> {
        let side = match request.side {
            Side::Buy => "Buy",
            Side::Sell => "Sell",
        };
        let order_type = match request.order_type {
            OrderType::Market => "Market",
            OrderType::Limit => "Limit",
            OrderType::StopMarket => "Market",
            OrderType::TakeProfitMarket => "Market",
        };
        let tif = match request.time_in_force {
            TimeInForce::Gtc => "GTC",
            TimeInForce::Ioc => "IOC",
            TimeInForce::Fok => "FOK",
            TimeInForce::PostOnly => "PostOnly",
        };

        let mut body = serde_json::json!({
            "category": "linear",
            "symbol": request.symbol.0,
            "side": side,
            "orderType": order_type,
            "qty": request.quantity.to_string(),
            "timeInForce": tif,
        });

        if let Some(price) = request.price {
            body["price"] = serde_json::Value::String(price.to_string());
        }

        if let Some(sp) = request.stop_price {
            body["triggerPrice"] = serde_json::Value::String(sp.to_string());
        }

        if request.reduce_only {
            body["reduceOnly"] = serde_json::Value::Bool(true);
        }

        info!(
            symbol = %request.symbol,
            side,
            order_type,
            qty = %request.quantity,
            "Placing Bybit order"
        );

        let result: BybitOrderResult = self
            .signed_post("/v5/order/create", &body)
            .await?;

        let order = OrderResponse {
            order_id: result.order_id,
            client_order_id: result.order_link_id.unwrap_or_default(),
            symbol: request.symbol.clone(),
            side: request.side,
            order_type: request.order_type,
            quantity: request.quantity,
            price: request.price,
            status: OrderStatus::New,
            timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
        };
        info!(order_id = %order.order_id, "Bybit order placed");
        Ok(order)
    }

    async fn cancel_order(
        &self,
        symbol: &Symbol,
        order_id: &OrderId,
    ) -> Result<OrderResponse> {
        let body = serde_json::json!({
            "category": "linear",
            "symbol": symbol.0,
            "orderId": order_id,
        });

        info!(symbol = %symbol, order_id = %order_id, "Cancelling Bybit order");

        let result: BybitOrderResult = self
            .signed_post("/v5/order/cancel", &body)
            .await?;

        Ok(OrderResponse {
            order_id: result.order_id,
            client_order_id: result.order_link_id.unwrap_or_default(),
            symbol: symbol.clone(),
            side: Side::Buy, // Not returned by cancel, placeholder
            order_type: OrderType::Limit,
            quantity: Decimal::ZERO,
            price: None,
            status: OrderStatus::Canceled,
            timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
        })
    }

    async fn cancel_all_orders(&self, symbol: &Symbol) -> Result<Vec<OrderResponse>> {
        let body = serde_json::json!({
            "category": "linear",
            "symbol": symbol.0,
        });

        info!(symbol = %symbol, "Cancelling all Bybit orders");

        let _result: BybitCancelAllResult = self
            .signed_post("/v5/order/cancel-all", &body)
            .await?;

        info!(symbol = %symbol, "All Bybit orders cancelled");
        Ok(vec![])
    }

    async fn get_position(&self, symbol: &Symbol) -> Result<Position> {
        let query = format!("category=linear&symbol={}", symbol.0);
        let result: BybitPositionList = self
            .signed_get("/v5/position/list", &query)
            .await?;

        let pos = result
            .list
            .into_iter()
            .find(|p| p.symbol == symbol.0)
            .unwrap_or(BybitPositionInfo {
                symbol: symbol.0.clone(),
                side: "None".to_string(),
                size: "0".to_string(),
                avg_price: "0".to_string(),
                unrealised_pnl: "0".to_string(),
                leverage: "1".to_string(),
                position_im: None,
            });

        let qty = parse_decimal(&pos.size)?;
        let side = Self::parse_side(&pos.side);
        let leverage: u32 = pos.leverage.parse().unwrap_or(1);
        let margin = pos
            .position_im
            .as_deref()
            .and_then(|s| parse_decimal(s).ok())
            .unwrap_or(Decimal::ZERO);

        Ok(Position {
            symbol: Symbol::new(&pos.symbol),
            side,
            quantity: qty,
            entry_price: parse_decimal(&pos.avg_price)?,
            unrealized_pnl: parse_decimal(&pos.unrealised_pnl)?,
            leverage,
            margin,
        })
    }

    async fn get_account_balance(&self) -> Result<AccountBalance> {
        let query = "accountType=UNIFIED";
        let result: BybitWalletBalanceResult = self
            .signed_get("/v5/account/wallet-balance", query)
            .await?;

        let acct = result
            .list
            .into_iter()
            .next()
            .context("No account info returned from Bybit")?;

        let total = parse_decimal(&acct.total_wallet_balance)?;
        let available = parse_decimal(&acct.total_available_balance)?;

        // Sum unrealised PnL from USDT coin entry if available
        let unrealized = acct
            .coin
            .as_ref()
            .and_then(|coins| {
                coins
                    .iter()
                    .find(|c| c.coin == "USDT")
                    .and_then(|c| parse_decimal(&c.unrealised_pnl).ok())
            })
            .unwrap_or(Decimal::ZERO);

        Ok(AccountBalance {
            total_balance: total,
            available_balance: available,
            unrealized_pnl: unrealized,
            margin_used: total - available,
        })
    }

    async fn set_leverage(&self, symbol: &Symbol, leverage: u32) -> Result<()> {
        let body = serde_json::json!({
            "category": "linear",
            "symbol": symbol.0,
            "buyLeverage": leverage.to_string(),
            "sellLeverage": leverage.to_string(),
        });

        info!(symbol = %symbol, leverage, "Setting Bybit leverage");

        // Bybit returns retCode 110043 if leverage is already set to this value.
        // We treat that as success.
        self.rate_limiter.acquire(1).await;
        let body_str = serde_json::to_string(&body)?;
        let (timestamp, sign, recv_window) =
            self.auth.sign_request(&body_str, self.recv_window);
        let url = format!("{}/v5/position/set-leverage", self.base_url);

        let resp = self
            .http
            .post(&url)
            .header("X-BAPI-API-KEY", self.auth.api_key())
            .header("X-BAPI-TIMESTAMP", &timestamp)
            .header("X-BAPI-SIGN", &sign)
            .header("X-BAPI-RECV-WINDOW", &recv_window)
            .header("Content-Type", "application/json")
            .body(body_str)
            .send()
            .await
            .context("Bybit set leverage failed")?;

        let status = resp.status();
        let resp_body = resp.text().await?;
        if !status.is_success() {
            bail!("Bybit HTTP {status}: {resp_body}");
        }

        let envelope: BybitApiResponse<serde_json::Value> =
            serde_json::from_str(&resp_body)?;

        // 110043 = "leverage not modified" - treat as success
        if envelope.ret_code != 0 && envelope.ret_code != 110043 {
            bail!(
                "Bybit set leverage error {}: {}",
                envelope.ret_code,
                envelope.ret_msg
            );
        }

        info!(symbol = %symbol, leverage, "Bybit leverage set");
        Ok(())
    }
}
