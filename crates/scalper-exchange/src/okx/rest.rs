use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use rust_decimal::Decimal;
use scalper_core::config::OkxExchangeConfig;
use scalper_core::types::{Exchange, OrderType, Side, TimeInForce};
use std::str::FromStr;
use tracing::{debug, warn};

use super::auth;
use super::models::*;
use crate::traits::{OrderManager, OrderResponse};

/// OKX V5 REST client.
pub struct OkxClient {
    config: OkxExchangeConfig,
    http: Client,
}

impl OkxClient {
    pub fn new(config: OkxExchangeConfig) -> Self {
        Self {
            config,
            http: Client::new(),
        }
    }

    fn base_url(&self) -> &str {
        &self.config.base_url_rest
    }

    fn auth_headers(&self, method: &str, path: &str, body: &str) -> Vec<(String, String)> {
        let timestamp = auth::timestamp_iso();
        let sig = auth::sign(&self.config.api_secret, &timestamp, method, path, body);
        vec![
            ("OK-ACCESS-KEY".into(), self.config.api_key.clone()),
            ("OK-ACCESS-SIGN".into(), sig),
            ("OK-ACCESS-TIMESTAMP".into(), timestamp),
            ("OK-ACCESS-PASSPHRASE".into(), self.config.passphrase.clone()),
            ("Content-Type".into(), "application/json".into()),
        ]
    }
}

#[async_trait]
impl OrderManager for OkxClient {
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
            Side::Buy => "buy",
            Side::Sell => "sell",
        };
        let type_str = match order_type {
            OrderType::Market => "market",
            OrderType::Limit => "limit",
            OrderType::StopMarket | OrderType::TakeProfitMarket => "market",
        };

        let mut body = serde_json::json!({
            "instId": symbol,
            "tdMode": "cross",
            "side": side_str,
            "ordType": type_str,
            "sz": quantity.to_string(),
        });

        if let Some(p) = price {
            if order_type == OrderType::Limit {
                body["px"] = serde_json::Value::String(p.to_string());
            }
        }
        if reduce_only {
            body["reduceOnly"] = serde_json::Value::Bool(true);
        }
        if time_in_force == TimeInForce::PostOnly && order_type == OrderType::Limit {
            body["ordType"] = serde_json::Value::String("post_only".into());
        }

        let payload = body.to_string();
        let path = "/api/v5/trade/order";
        let headers = self.auth_headers("POST", path, &payload);
        let url = format!("{}{path}", self.base_url());

        debug!("OKX place_order: {symbol} {side_str} {type_str} qty={quantity}");

        let mut req = self.http.post(&url);
        for (k, v) in &headers {
            req = req.header(k, v);
        }

        let resp = req.body(payload).send().await.context("OKX place_order failed")?;
        let body_text = resp.text().await.unwrap_or_default();
        let response: OkxResponse<OkxOrderResult> =
            serde_json::from_str(&body_text).context("Failed to parse OKX response")?;

        if response.code != "0" {
            warn!("OKX order error {}: {}", response.code, response.msg);
            anyhow::bail!("OKX order failed: {}", response.msg);
        }

        let result = response.data.first().context("Missing order result")?;
        if result.s_code != "0" {
            anyhow::bail!("OKX order rejected: {}", result.s_msg);
        }

        Ok(OrderResponse {
            order_id: result.ord_id.clone(),
            exchange: Exchange::OKX,
            symbol: symbol.to_string(),
            status: "live".to_string(),
        })
    }

    async fn cancel_order(&self, symbol: &str, order_id: &str) -> Result<()> {
        let body = serde_json::json!({
            "instId": symbol,
            "ordId": order_id,
        });
        let payload = body.to_string();
        let path = "/api/v5/trade/cancel-order";
        let headers = self.auth_headers("POST", path, &payload);
        let url = format!("{}{path}", self.base_url());

        let mut req = self.http.post(&url);
        for (k, v) in &headers {
            req = req.header(k, v);
        }

        let resp = req.body(payload).send().await.context("OKX cancel failed")?;
        let body_text = resp.text().await.unwrap_or_default();
        let response: OkxResponse<serde_json::Value> = serde_json::from_str(&body_text)?;

        if response.code != "0" {
            anyhow::bail!("OKX cancel failed: {}", response.msg);
        }
        Ok(())
    }

    async fn set_leverage(&self, symbol: &str, leverage: u32) -> Result<()> {
        let body = serde_json::json!({
            "instId": symbol,
            "lever": leverage.to_string(),
            "mgnMode": "cross",
        });
        let payload = body.to_string();
        let path = "/api/v5/account/set-leverage";
        let headers = self.auth_headers("POST", path, &payload);
        let url = format!("{}{path}", self.base_url());

        let mut req = self.http.post(&url);
        for (k, v) in &headers {
            req = req.header(k, v);
        }

        let resp = req.body(payload).send().await.context("OKX leverage failed")?;
        let body_text = resp.text().await.unwrap_or_default();
        let response: OkxResponse<serde_json::Value> = serde_json::from_str(&body_text)?;

        if response.code != "0" {
            anyhow::bail!("OKX set_leverage failed: {}", response.msg);
        }
        Ok(())
    }

    async fn get_balance(&self) -> Result<Decimal> {
        let path = "/api/v5/account/balance";
        let headers = self.auth_headers("GET", path, "");
        let url = format!("{}{path}", self.base_url());

        let mut req = self.http.get(&url);
        for (k, v) in &headers {
            req = req.header(k, v);
        }

        let resp = req.send().await.context("OKX get_balance failed")?;
        let body_text = resp.text().await.unwrap_or_default();
        let response: OkxResponse<OkxBalanceData> = serde_json::from_str(&body_text)?;

        let data = response.data.first().context("No balance data")?;
        Decimal::from_str(&data.total_eq).context("Failed to parse OKX balance")
    }

    fn exchange(&self) -> Exchange {
        Exchange::OKX
    }
}
