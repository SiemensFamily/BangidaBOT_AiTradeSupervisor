use anyhow::Result;
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use rust_decimal::Decimal;
use scalper_core::config::ExchangeConfig;
use scalper_core::types::{Exchange, MarketEvent, Side};
use std::str::FromStr;
use tokio::sync::broadcast;
use tokio::time::{self, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

use super::models::*;
use crate::traits::MarketDataFeed;

/// Bybit V5 linear WebSocket feed.
pub struct BybitWsFeed {
    config: ExchangeConfig,
}

impl BybitWsFeed {
    pub fn new(config: ExchangeConfig) -> Self {
        Self { config }
    }

    fn ws_url(&self) -> String {
        format!("{}/v5/public/linear", self.config.base_url_ws)
    }

    fn parse_topic(topic: &str, data: &serde_json::Value, ts: u64) -> Option<MarketEvent> {
        if topic.starts_with("orderbook") {
            let ob: WsOrderBookData = serde_json::from_value(data.clone()).ok()?;
            let bids = ob.b.iter().filter_map(|[p, q]| {
                Some((Decimal::from_str(p).ok()?, Decimal::from_str(q).ok()?))
            }).collect();
            let asks = ob.a.iter().filter_map(|[p, q]| {
                Some((Decimal::from_str(p).ok()?, Decimal::from_str(q).ok()?))
            }).collect();
            Some(MarketEvent::OrderBookUpdate {
                exchange: Exchange::Bybit,
                symbol: ob.s,
                bids,
                asks,
                timestamp_ms: ts,
            })
        } else if topic.starts_with("publicTrade") {
            let trades: Vec<WsTradeItem> = serde_json::from_value(data.clone()).ok()?;
            let t = trades.first()?;
            let is_buyer_maker = t.side == "Sell"; // Bybit: taker side, so Sell = buyer was maker
            Some(MarketEvent::Trade {
                exchange: Exchange::Bybit,
                symbol: t.s.clone(),
                price: Decimal::from_str(&t.p).ok()?,
                quantity: Decimal::from_str(&t.v).ok()?,
                is_buyer_maker,
                timestamp_ms: t.timestamp,
            })
        } else if topic.starts_with("kline") {
            let klines: Vec<WsKlineItem> = serde_json::from_value(data.clone()).ok()?;
            let k = klines.first()?;
            if !k.confirm {
                return None;
            }
            // Extract symbol from topic: "kline.1.BTCUSDT" -> "BTCUSDT"
            let symbol = topic.split('.').nth(2)?.to_string();
            Some(MarketEvent::KlineClose {
                exchange: Exchange::Bybit,
                symbol,
                open: Decimal::from_str(&k.open).ok()?,
                high: Decimal::from_str(&k.high).ok()?,
                low: Decimal::from_str(&k.low).ok()?,
                close: Decimal::from_str(&k.close).ok()?,
                volume: Decimal::from_str(&k.volume).ok()?,
                timestamp_ms: k.end,
            })
        } else if topic.starts_with("tickers") {
            let ticker: WsTickerData = serde_json::from_value(data.clone()).ok()?;
            let mark_price = ticker.mark_price.as_ref().and_then(|p| Decimal::from_str(p).ok())?;
            let funding_rate = ticker.funding_rate.as_ref().and_then(|r| Decimal::from_str(r).ok()).unwrap_or_default();
            let next_funding = ticker.next_funding_time.as_ref().and_then(|t| t.parse::<u64>().ok()).unwrap_or(0);
            Some(MarketEvent::MarkPrice {
                exchange: Exchange::Bybit,
                symbol: ticker.symbol,
                mark_price,
                funding_rate,
                next_funding_time: next_funding,
            })
        } else {
            None
        }
    }
}

#[async_trait]
impl MarketDataFeed for BybitWsFeed {
    async fn subscribe(
        &self,
        symbols: &[String],
        tx: broadcast::Sender<MarketEvent>,
    ) -> Result<()> {
        let url = self.ws_url();
        info!("Bybit WS connecting to {url}");

        let mut backoff_ms = 500u64;
        let max_backoff = 30_000u64;

        loop {
            match connect_async(&url).await {
                Ok((ws_stream, _)) => {
                    info!("Bybit WS connected");
                    backoff_ms = 500;

                    let (mut write, mut read) = ws_stream.split();

                    // Subscribe to topics
                    let args: Vec<String> = symbols.iter().flat_map(|s| {
                        vec![
                            format!("orderbook.50.{s}"),
                            format!("publicTrade.{s}"),
                            format!("kline.1.{s}"),
                            format!("tickers.{s}"),
                        ]
                    }).collect();

                    let sub_msg = serde_json::json!({
                        "op": "subscribe",
                        "args": args
                    });
                    if let Err(e) = write.send(Message::Text(sub_msg.to_string())).await {
                        error!("Bybit WS subscribe failed: {e}");
                        continue;
                    }

                    let mut ping_interval = time::interval(Duration::from_secs(20));

                    loop {
                        tokio::select! {
                            msg = read.next() => {
                                match msg {
                                    Some(Ok(Message::Text(text))) => {
                                        if let Ok(ws_msg) = serde_json::from_str::<WsMessage>(&text) {
                                            if let (Some(topic), Some(data)) = (&ws_msg.topic, &ws_msg.data) {
                                                let ts = ws_msg.ts.unwrap_or(0);
                                                if let Some(event) = Self::parse_topic(topic, data, ts) {
                                                    let _ = tx.send(event);
                                                }
                                            }
                                        }
                                    }
                                    Some(Ok(Message::Ping(data))) => {
                                        let _ = write.send(Message::Pong(data)).await;
                                    }
                                    Some(Ok(Message::Close(_))) | None => {
                                        warn!("Bybit WS disconnected");
                                        break;
                                    }
                                    Some(Err(e)) => {
                                        error!("Bybit WS error: {e}");
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                            _ = ping_interval.tick() => {
                                let ping = serde_json::json!({"op": "ping"});
                                if write.send(Message::Text(ping.to_string())).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("Bybit WS connection failed: {e}");
                }
            }

            warn!("Bybit WS reconnecting in {backoff_ms}ms...");
            time::sleep(Duration::from_millis(backoff_ms)).await;
            backoff_ms = (backoff_ms * 2).min(max_backoff);
        }
    }
}
