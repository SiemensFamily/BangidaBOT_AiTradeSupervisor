use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use rust_decimal::Decimal;
use scalper_core::config::ExchangeConfig;
use scalper_core::types::{Exchange, OrderType, Side, TimeInForce};
use std::str::FromStr;
use tracing::{debug, warn};

use super::auth;
use super::models::{BinanceAccountInfo, BinanceOrderResponse, ListenKeyResponse};
use crate::traits::{OrderManager, OrderResponse};

/// Binance Futures (USD-M) REST client.
pub struct BinanceClient {
    config: ExchangeConfig,
    http: Client,
}

impl BinanceClient {
    pub fn new(config: ExchangeConfig) -> Self {
        Self {
            config,
            http: Client::new(),
        }
    }

    fn base_url(&self) -> &str {
        &self.config.base_url_rest
    }

    fn signed_request(&self, query: &str) -> (String, String) {
        let ts = auth::timestamp_ms();
        let full_query = if query.is_empty() {
            format!("timestamp={ts}")
        } else {
            format!("{query}&timestamp={ts}")
        };
        let signature = auth::sign(&self.config.api_secret, &full_query);
        (full_query, signature)
    }

    /// Create a listen key for the user data stream.
    pub async fn create_listen_key(&self) -> Result<String> {
        let url = format!("{}/fapi/v1/listenKey", self.base_url());
        let resp = self
            .http
            .post(&url)
            .header("X-MBX-APIKEY", &self.config.api_key)
            .send()
            .await
            .context("Failed to create listen key")?;

        let body: ListenKeyResponse = resp.json().await?;
        Ok(body.listen_key)
    }

    /// Renew the listen key (should be called every 30 minutes).
    pub async fn renew_listen_key(&self) -> Result<()> {
        let url = format!("{}/fapi/v1/listenKey", self.base_url());
        self.http
            .put(&url)
            .header("X-MBX-APIKEY", &self.config.api_key)
            .send()
            .await
            .context("Failed to renew listen key")?;
        Ok(())
    }
}

#[async_trait]
impl OrderManager for BinanceClient {
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
            Side::Buy => "BUY",
            Side::Sell => "SELL",
        };
        let type_str = match order_type {
            OrderType::Market => "MARKET",
            OrderType::Limit => "LIMIT",
            OrderType::StopMarket => "STOP_MARKET",
            OrderType::TakeProfitMarket => "TAKE_PROFIT_MARKET",
        };
        let tif_str = match time_in_force {
            TimeInForce::GTC => "GTC",
            TimeInForce::IOC => "IOC",
            TimeInForce::FOK => "FOK",
            TimeInForce::PostOnly => "GTX",
        };

        let mut query_parts = vec![
            format!("symbol={symbol}"),
            format!("side={side_str}"),
            format!("type={type_str}"),
            format!("quantity={quantity}"),
            format!("newOrderRespType=RESULT"),
        ];

        if order_type == OrderType::Limit {
            query_parts.push(format!("timeInForce={tif_str}"));
            if let Some(p) = price {
                query_parts.push(format!("price={p}"));
            }
        }
        if matches!(
            order_type,
            OrderType::StopMarket | OrderType::TakeProfitMarket
        ) {
            if let Some(p) = price {
                query_parts.push(format!("stopPrice={p}"));
            }
        }
        if reduce_only {
            query_parts.push("reduceOnly=true".to_string());
        }

        let query = query_parts.join("&");
        let (signed_query, signature) = self.signed_request(&query);
        let url = format!(
            "{}/fapi/v1/order?{signed_query}&signature={signature}",
            self.base_url()
        );

        debug!("Binance place_order: {symbol} {side_str} {type_str} qty={quantity}");

        let resp = self
            .http
            .post(&url)
            .header("X-MBX-APIKEY", &self.config.api_key)
            .send()
            .await
            .context("Binance place_order request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!("Binance order error {status}: {body}");
            anyhow::bail!("Binance order failed ({status}): {body}");
        }

        let body: BinanceOrderResponse = resp.json().await?;
        Ok(OrderResponse {
            order_id: body.order_id.to_string(),
            exchange: Exchange::Binance,
            symbol: body.symbol,
            status: body.status,
        })
    }

    async fn cancel_order(&self, symbol: &str, order_id: &str) -> Result<()> {
        let query = format!("symbol={symbol}&orderId={order_id}");
        let (signed_query, signature) = self.signed_request(&query);
        let url = format!(
            "{}/fapi/v1/order?{signed_query}&signature={signature}",
            self.base_url()
        );

        let resp = self
            .http
            .delete(&url)
            .header("X-MBX-APIKEY", &self.config.api_key)
            .send()
            .await
            .context("Binance cancel_order failed")?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!("Binance cancel error: {body}");
            anyhow::bail!("Binance cancel failed: {body}");
        }
        Ok(())
    }

    async fn set_leverage(&self, symbol: &str, leverage: u32) -> Result<()> {
        let query = format!("symbol={symbol}&leverage={leverage}");
        let (signed_query, signature) = self.signed_request(&query);
        let url = format!(
            "{}/fapi/v1/leverage?{signed_query}&signature={signature}",
            self.base_url()
        );

        let resp = self
            .http
            .post(&url)
            .header("X-MBX-APIKEY", &self.config.api_key)
            .send()
            .await
            .context("Binance set_leverage failed")?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!("Binance leverage error: {body}");
            anyhow::bail!("Binance set_leverage failed: {body}");
        }
        Ok(())
    }

    async fn get_balance(&self) -> Result<Decimal> {
        let (signed_query, signature) = self.signed_request("");
        let url = format!(
            "{}/fapi/v2/account?{signed_query}&signature={signature}",
            self.base_url()
        );

        let resp = self
            .http
            .get(&url)
            .header("X-MBX-APIKEY", &self.config.api_key)
            .send()
            .await
            .context("Binance get_balance failed")?;

        let info: BinanceAccountInfo = resp.json().await?;
        Decimal::from_str(&info.available_balance)
            .context("Failed to parse Binance balance")
    }

    fn exchange(&self) -> Exchange {
        Exchange::Binance
    }
}
