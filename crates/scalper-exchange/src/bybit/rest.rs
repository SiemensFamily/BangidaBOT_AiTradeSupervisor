use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use rust_decimal::Decimal;
use scalper_core::config::ExchangeConfig;
use scalper_core::types::{Exchange, OrderType, Side, TimeInForce};
use std::str::FromStr;
use tracing::{debug, warn};

use super::auth;
use super::models::*;
use crate::traits::{OrderManager, OrderResponse};

const RECV_WINDOW: u64 = 5000;

/// Bybit V5 Unified Trading REST client.
pub struct BybitClient {
    config: ExchangeConfig,
    http: Client,
}

impl BybitClient {
    pub fn new(config: ExchangeConfig) -> Self {
        Self {
            config,
            http: Client::new(),
        }
    }

    fn base_url(&self) -> &str {
        &self.config.base_url_rest
    }

    fn sign_headers(&self, payload: &str) -> Vec<(String, String)> {
        let ts = auth::timestamp_ms();
        let sig = auth::sign(&self.config.api_secret, ts, &self.config.api_key, RECV_WINDOW, payload);
        vec![
            ("X-BAPI-API-KEY".into(), self.config.api_key.clone()),
            ("X-BAPI-TIMESTAMP".into(), ts.to_string()),
            ("X-BAPI-SIGN".into(), sig),
            ("X-BAPI-RECV-WINDOW".into(), RECV_WINDOW.to_string()),
        ]
    }
}

#[async_trait]
impl OrderManager for BybitClient {
    async fn place_order(
        &self,
        symbol: &str,
        side: Side,
        order_type: OrderType,
        time_in_force: TimeInForce,
        quantity: Decimal,
        price: Option<Decimal>,
        reduce_only: bool,
    ) -> Result<OrderResponse> {
        let side_str = match side {
            Side::Buy => "Buy",
            Side::Sell => "Sell",
        };
        let type_str = match order_type {
            OrderType::Market => "Market",
            OrderType::Limit => "Limit",
            OrderType::StopMarket => "Market",
            OrderType::TakeProfitMarket => "Market",
        };
        let tif_str = match time_in_force {
            TimeInForce::GTC => "GTC",
            TimeInForce::IOC => "IOC",
            TimeInForce::FOK => "FOK",
            TimeInForce::PostOnly => "PostOnly",
        };

        let mut body = serde_json::json!({
            "category": "linear",
            "symbol": symbol,
            "side": side_str,
            "orderType": type_str,
            "qty": quantity.to_string(),
            "timeInForce": tif_str,
        });

        if let Some(p) = price {
            body["price"] = serde_json::Value::String(p.to_string());
        }
        if reduce_only {
            body["reduceOnly"] = serde_json::Value::Bool(true);
        }
        if order_type == OrderType::StopMarket {
            if let Some(p) = price {
                body["triggerPrice"] = serde_json::Value::String(p.to_string());
            }
            body["triggerDirection"] = serde_json::json!(if side == Side::Buy { 1 } else { 2 });
        }
        if order_type == OrderType::TakeProfitMarket {
            if let Some(p) = price {
                body["triggerPrice"] = serde_json::Value::String(p.to_string());
            }
            body["triggerDirection"] = serde_json::json!(if side == Side::Sell { 1 } else { 2 });
        }

        let payload = body.to_string();
        let headers = self.sign_headers(&payload);
        let url = format!("{}/v5/order/create", self.base_url());

        debug!("Bybit place_order: {symbol} {side_str} {type_str} qty={quantity}");

        let mut req = self.http.post(&url).header("Content-Type", "application/json");
        for (k, v) in &headers {
            req = req.header(k, v);
        }

        let resp = req.body(payload).send().await.context("Bybit place_order failed")?;
        let _status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();

        let response: BybitResponse<BybitOrderResult> =
            serde_json::from_str(&body_text).context("Failed to parse Bybit response")?;

        if response.ret_code != 0 {
            warn!("Bybit order error {}: {}", response.ret_code, response.ret_msg);
            anyhow::bail!("Bybit order failed: {}", response.ret_msg);
        }

        let result = response.result.context("Missing order result")?;
        Ok(OrderResponse {
            order_id: result.order_id,
            exchange: Exchange::Bybit,
            symbol: symbol.to_string(),
            status: "New".to_string(),
        })
    }

    async fn cancel_order(&self, symbol: &str, order_id: &str) -> Result<()> {
        let body = serde_json::json!({
            "category": "linear",
            "symbol": symbol,
            "orderId": order_id,
        });
        let payload = body.to_string();
        let headers = self.sign_headers(&payload);
        let url = format!("{}/v5/order/cancel", self.base_url());

        let mut req = self.http.post(&url).header("Content-Type", "application/json");
        for (k, v) in &headers {
            req = req.header(k, v);
        }

        let resp = req.body(payload).send().await.context("Bybit cancel failed")?;
        let body_text = resp.text().await.unwrap_or_default();
        let response: BybitResponse<serde_json::Value> = serde_json::from_str(&body_text)?;

        if response.ret_code != 0 {
            anyhow::bail!("Bybit cancel failed: {}", response.ret_msg);
        }
        Ok(())
    }

    async fn set_leverage(&self, symbol: &str, leverage: u32) -> Result<()> {
        let body = serde_json::json!({
            "category": "linear",
            "symbol": symbol,
            "buyLeverage": leverage.to_string(),
            "sellLeverage": leverage.to_string(),
        });
        let payload = body.to_string();
        let headers = self.sign_headers(&payload);
        let url = format!("{}/v5/position/set-leverage", self.base_url());

        let mut req = self.http.post(&url).header("Content-Type", "application/json");
        for (k, v) in &headers {
            req = req.header(k, v);
        }

        let resp = req.body(payload).send().await.context("Bybit leverage failed")?;
        let body_text = resp.text().await.unwrap_or_default();
        let response: BybitResponse<serde_json::Value> = serde_json::from_str(&body_text)?;

        // Code 110043 means leverage unchanged — not an error
        if response.ret_code != 0 && response.ret_code != 110043 {
            anyhow::bail!("Bybit set_leverage failed: {}", response.ret_msg);
        }
        Ok(())
    }

    async fn get_balance(&self) -> Result<Decimal> {
        let query = "accountType=UNIFIED";
        let headers = self.sign_headers(query);
        let url = format!("{}/v5/account/wallet-balance?{query}", self.base_url());

        let mut req = self.http.get(&url);
        for (k, v) in &headers {
            req = req.header(k, v);
        }

        let resp = req.send().await.context("Bybit get_balance failed")?;
        let body_text = resp.text().await.unwrap_or_default();
        let response: BybitResponse<BybitWalletBalance> = serde_json::from_str(&body_text)?;

        let result = response.result.context("Missing balance result")?;
        let account = result.list.first().context("No account data")?;
        Decimal::from_str(&account.total_available_balance).context("Failed to parse balance")
    }

    fn exchange(&self) -> Exchange {
        Exchange::Bybit
    }
}
