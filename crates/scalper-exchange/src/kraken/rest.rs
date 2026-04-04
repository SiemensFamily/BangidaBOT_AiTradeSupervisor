use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use rust_decimal::Decimal;
use scalper_core::config::ExchangeConfig;
use scalper_core::types::{Exchange, OrderType, Side, TimeInForce};
use std::str::FromStr;
use tracing::{debug, warn};

use super::auth;
use crate::traits::{OrderManager, OrderResponse};

/// Kraken Futures REST client.
pub struct KrakenClient {
    config: ExchangeConfig,
    http: Client,
}

impl KrakenClient {
    pub fn new(config: ExchangeConfig) -> Self {
        Self {
            config,
            http: Client::new(),
        }
    }

    fn base_url(&self) -> &str {
        &self.config.base_url_rest
    }
}

#[async_trait]
impl OrderManager for KrakenClient {
    async fn place_order(
        &self,
        symbol: &str,
        side: Side,
        order_type: OrderType,
        _time_in_force: TimeInForce,
        quantity: Decimal,
        price: Option<Decimal>,
        reduce_only: bool,
    ) -> Result<OrderResponse> {
        let side_str = match side {
            Side::Buy => "buy",
            Side::Sell => "sell",
        };
        let type_str = match order_type {
            OrderType::Market | OrderType::StopMarket | OrderType::TakeProfitMarket => "mkt",
            OrderType::Limit => "lmt",
        };

        let nonce = auth::nonce();
        let mut post_data = format!(
            "orderType={type_str}&symbol={symbol}&side={side_str}&size={quantity}"
        );
        if let Some(p) = price {
            if order_type == OrderType::Limit {
                post_data.push_str(&format!("&limitPrice={p}"));
            }
            if matches!(order_type, OrderType::StopMarket | OrderType::TakeProfitMarket) {
                post_data.push_str(&format!("&stopPrice={p}"));
            }
        }
        if reduce_only {
            post_data.push_str("&reduceOnly=true");
        }

        let path = "/derivatives/api/v3/sendorder";
        let sig = auth::sign(&self.config.api_secret, &nonce, path, &post_data);
        let url = format!("{}{path}", self.base_url());

        debug!("Kraken place_order: {symbol} {side_str} {type_str} qty={quantity}");

        let resp = self
            .http
            .post(&url)
            .header("APIKey", &self.config.api_key)
            .header("Nonce", &nonce)
            .header("Authent", &sig)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(post_data)
            .send()
            .await
            .context("Kraken place_order failed")?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!("Kraken order error: {body}");
            anyhow::bail!("Kraken order failed: {body}");
        }

        let body_text = resp.text().await.unwrap_or_default();
        let response: serde_json::Value = serde_json::from_str(&body_text)?;

        let order_id = response["sendStatus"]["order_id"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        let status = response["sendStatus"]["status"]
            .as_str()
            .unwrap_or("placed")
            .to_string();

        Ok(OrderResponse {
            order_id,
            exchange: Exchange::Kraken,
            symbol: symbol.to_string(),
            status,
        })
    }

    async fn cancel_order(&self, _symbol: &str, order_id: &str) -> Result<()> {
        let nonce = auth::nonce();
        let post_data = format!("order_id={order_id}");
        let path = "/derivatives/api/v3/cancelorder";
        let sig = auth::sign(&self.config.api_secret, &nonce, path, &post_data);
        let url = format!("{}{path}", self.base_url());

        let resp = self
            .http
            .post(&url)
            .header("APIKey", &self.config.api_key)
            .header("Nonce", &nonce)
            .header("Authent", &sig)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(post_data)
            .send()
            .await
            .context("Kraken cancel failed")?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Kraken cancel failed: {body}");
        }
        Ok(())
    }

    async fn set_leverage(&self, _symbol: &str, _leverage: u32) -> Result<()> {
        // Kraken Futures leverage is set per-account, not per-symbol via API
        // Leverage is managed through the web interface or account settings
        Ok(())
    }

    async fn get_balance(&self) -> Result<Decimal> {
        let nonce = auth::nonce();
        let path = "/derivatives/api/v3/accounts";
        let sig = auth::sign(&self.config.api_secret, &nonce, path, "");
        let url = format!("{}{path}", self.base_url());

        let resp = self
            .http
            .get(&url)
            .header("APIKey", &self.config.api_key)
            .header("Nonce", &nonce)
            .header("Authent", &sig)
            .send()
            .await
            .context("Kraken get_balance failed")?;

        let body_text = resp.text().await.unwrap_or_default();
        let response: serde_json::Value = serde_json::from_str(&body_text)?;

        // Extract flex balance from the accounts response
        let balance_str = response["accounts"]["flex"]["availableMargin"]
            .as_f64()
            .unwrap_or(0.0);
        Ok(Decimal::from_f64_retain(balance_str).unwrap_or_default())
    }

    fn exchange(&self) -> Exchange {
        Exchange::Kraken
    }
}
