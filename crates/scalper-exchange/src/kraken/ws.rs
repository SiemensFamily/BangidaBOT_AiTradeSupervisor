use anyhow::Result;
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use rust_decimal::Decimal;
use scalper_core::config::ExchangeConfig;
use scalper_core::types::{Exchange, MarketEvent};
use tokio::sync::broadcast;
use tokio::time::{self, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

use super::models::WsMessage;
use crate::traits::MarketDataFeed;

/// Kraken Futures WebSocket feed.
pub struct KrakenWsFeed {
    config: ExchangeConfig,
}

impl KrakenWsFeed {
    pub fn new(config: ExchangeConfig) -> Self {
        Self { config }
    }

    fn ws_url(&self) -> &str {
        &self.config.base_url_ws
    }

    fn parse_message(msg: &WsMessage) -> Option<MarketEvent> {
        let feed = msg.feed.as_deref()?;
        let symbol = msg.product_id.clone()?;

        match feed {
            "book" | "book_snapshot" => {
                let bids: Vec<(Decimal, Decimal)> = msg
                    .bids
                    .as_ref()?
                    .iter()
                    .map(|l| {
                        (
                            Decimal::from_f64_retain(l.price).unwrap_or_default(),
                            Decimal::from_f64_retain(l.qty).unwrap_or_default(),
                        )
                    })
                    .collect();
                let asks: Vec<(Decimal, Decimal)> = msg
                    .asks
                    .as_ref()?
                    .iter()
                    .map(|l| {
                        (
                            Decimal::from_f64_retain(l.price).unwrap_or_default(),
                            Decimal::from_f64_retain(l.qty).unwrap_or_default(),
                        )
                    })
                    .collect();
                Some(MarketEvent::OrderBookUpdate {
                    exchange: Exchange::Kraken,
                    symbol,
                    bids,
                    asks,
                    timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                })
            }
            "trade" | "trade_snapshot" => {
                let price = Decimal::from_f64_retain(msg.price?).unwrap_or_default();
                let quantity = Decimal::from_f64_retain(msg.qty?).unwrap_or_default();
                let is_buyer_maker = msg.side.as_deref() == Some("sell");
                Some(MarketEvent::Trade {
                    exchange: Exchange::Kraken,
                    symbol,
                    price,
                    quantity,
                    is_buyer_maker,
                    timestamp_ms: msg.time.unwrap_or(0),
                })
            }
            "ticker" => {
                let mark_price =
                    Decimal::from_f64_retain(msg.mark_price?).unwrap_or_default();
                let funding_rate =
                    Decimal::from_f64_retain(msg.funding_rate.unwrap_or(0.0)).unwrap_or_default();
                Some(MarketEvent::MarkPrice {
                    exchange: Exchange::Kraken,
                    symbol,
                    mark_price,
                    funding_rate,
                    next_funding_time: msg.next_funding_rate_time.unwrap_or(0),
                })
            }
            _ => None,
        }
    }
}

#[async_trait]
impl MarketDataFeed for KrakenWsFeed {
    async fn subscribe(
        &self,
        symbols: &[String],
        tx: broadcast::Sender<MarketEvent>,
    ) -> Result<()> {
        let url = self.ws_url();
        info!("Kraken WS connecting to {url}");

        let mut backoff_ms = 500u64;

        loop {
            match connect_async(url).await {
                Ok((ws_stream, _)) => {
                    info!("Kraken WS connected");
                    backoff_ms = 500;

                    let (mut write, mut read) = ws_stream.split();

                    // Subscribe to feeds
                    for symbol in symbols {
                        let sub_book = serde_json::json!({
                            "event": "subscribe",
                            "feed": "book",
                            "product_ids": [symbol]
                        });
                        let sub_trade = serde_json::json!({
                            "event": "subscribe",
                            "feed": "trade",
                            "product_ids": [symbol]
                        });
                        let sub_ticker = serde_json::json!({
                            "event": "subscribe",
                            "feed": "ticker",
                            "product_ids": [symbol]
                        });
                        let _ = write.send(Message::Text(sub_book.to_string())).await;
                        let _ = write.send(Message::Text(sub_trade.to_string())).await;
                        let _ = write.send(Message::Text(sub_ticker.to_string())).await;
                    }

                    let mut ping_interval = time::interval(Duration::from_secs(30));

                    let mut msg_count: u64 = 0;

                    loop {
                        tokio::select! {
                            msg = read.next() => {
                                match msg {
                                    Some(Ok(Message::Text(text))) => {
                                        msg_count += 1;
                                        // Log first 5 messages for diagnostics
                                        if msg_count <= 5 {
                                            info!("Kraken WS msg #{}: {}", msg_count, &text[..text.len().min(300)]);
                                        }
                                        if let Ok(ws_msg) = serde_json::from_str::<WsMessage>(&text) {
                                            if let Some(event) = Self::parse_message(&ws_msg) {
                                                let _ = tx.send(event);
                                            }
                                        } else {
                                            warn!("Kraken WS: failed to parse: {}", &text[..text.len().min(200)]);
                                        }
                                    }
                                    Some(Ok(Message::Ping(data))) => {
                                        let _ = write.send(Message::Pong(data)).await;
                                    }
                                    Some(Ok(Message::Close(_))) | None => {
                                        warn!("Kraken WS disconnected");
                                        break;
                                    }
                                    Some(Err(e)) => {
                                        error!("Kraken WS error: {e}");
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                            _ = ping_interval.tick() => {
                                if write.send(Message::Ping(vec![])).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("Kraken WS connection failed: {e}");
                }
            }

            time::sleep(Duration::from_millis(backoff_ms)).await;
            backoff_ms = (backoff_ms * 2).min(30_000);
        }
    }
}
