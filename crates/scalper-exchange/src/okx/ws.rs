use anyhow::Result;
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use rust_decimal::Decimal;
use scalper_core::config::OkxExchangeConfig;
use scalper_core::types::{Exchange, MarketEvent, Side};
use std::str::FromStr;
use tokio::sync::broadcast;
use tokio::time::{self, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

use super::models::*;
use crate::traits::MarketDataFeed;

/// OKX V5 WebSocket feed.
pub struct OkxWsFeed {
    config: OkxExchangeConfig,
}

impl OkxWsFeed {
    pub fn new(config: OkxExchangeConfig) -> Self {
        Self { config }
    }

    fn ws_url(&self) -> String {
        format!("{}/ws/v5/public", self.config.base_url_ws)
    }

    fn parse_message(channel: &str, inst_id: &str, data: &serde_json::Value) -> Option<MarketEvent> {
        match channel {
            "books5" => {
                let book: WsBookData = serde_json::from_value(data.clone()).ok()?;
                let ts = book.ts.parse::<u64>().ok()?;
                let bids = book.bids.iter().filter_map(|[p, s, _, _]| {
                    Some((Decimal::from_str(p).ok()?, Decimal::from_str(s).ok()?))
                }).collect();
                let asks = book.asks.iter().filter_map(|[p, s, _, _]| {
                    Some((Decimal::from_str(p).ok()?, Decimal::from_str(s).ok()?))
                }).collect();
                Some(MarketEvent::OrderBookUpdate {
                    exchange: Exchange::OKX,
                    symbol: inst_id.to_string(),
                    bids,
                    asks,
                    timestamp_ms: ts,
                })
            }
            "trades" => {
                let trade: WsTradeData = serde_json::from_value(data.clone()).ok()?;
                let is_buyer_maker = trade.side == "sell";
                Some(MarketEvent::Trade {
                    exchange: Exchange::OKX,
                    symbol: trade.inst_id,
                    price: Decimal::from_str(&trade.px).ok()?,
                    quantity: Decimal::from_str(&trade.sz).ok()?,
                    is_buyer_maker,
                    timestamp_ms: trade.ts.parse().ok()?,
                })
            }
            "mark-price" => {
                let ticker: WsTickerData = serde_json::from_value(data.clone()).ok()?;
                Some(MarketEvent::MarkPrice {
                    exchange: Exchange::OKX,
                    symbol: ticker.inst_id,
                    mark_price: Decimal::from_str(ticker.mark_px.as_deref()?).ok()?,
                    funding_rate: ticker.funding_rate.as_deref().and_then(|r| Decimal::from_str(r).ok()).unwrap_or_default(),
                    next_funding_time: ticker.next_funding_time.as_deref().and_then(|t| t.parse().ok()).unwrap_or(0),
                })
            }
            "liquidation-orders" => {
                let liq: WsLiquidationData = serde_json::from_value(data.clone()).ok()?;
                let side = if liq.side == "buy" { Side::Buy } else { Side::Sell };
                Some(MarketEvent::LiquidationEvent {
                    exchange: Exchange::OKX,
                    symbol: liq.inst_id,
                    side,
                    quantity: Decimal::from_str(&liq.sz).ok()?,
                    price: Decimal::from_str(&liq.bk_px).ok()?,
                    timestamp_ms: liq.ts.parse().ok()?,
                })
            }
            _ => None,
        }
    }
}

#[async_trait]
impl MarketDataFeed for OkxWsFeed {
    async fn subscribe(
        &self,
        symbols: &[String],
        tx: broadcast::Sender<MarketEvent>,
    ) -> Result<()> {
        let url = self.ws_url();
        info!("OKX WS connecting to {url}");

        let mut backoff_ms = 500u64;

        loop {
            match connect_async(&url).await {
                Ok((ws_stream, _)) => {
                    info!("OKX WS connected");
                    backoff_ms = 500;

                    let (mut write, mut read) = ws_stream.split();

                    // Subscribe to channels
                    let args: Vec<serde_json::Value> = symbols.iter().flat_map(|s| {
                        vec![
                            serde_json::json!({"channel": "books5", "instId": s}),
                            serde_json::json!({"channel": "trades", "instId": s}),
                            serde_json::json!({"channel": "mark-price", "instId": s}),
                            serde_json::json!({"channel": "liquidation-orders", "instType": "SWAP"}),
                        ]
                    }).collect();

                    let sub = serde_json::json!({"op": "subscribe", "args": args});
                    if let Err(e) = write.send(Message::Text(sub.to_string())).await {
                        error!("OKX WS subscribe failed: {e}");
                        continue;
                    }

                    let mut ping_interval = time::interval(Duration::from_secs(25));

                    loop {
                        tokio::select! {
                            msg = read.next() => {
                                match msg {
                                    Some(Ok(Message::Text(text))) => {
                                        if let Ok(ws_msg) = serde_json::from_str::<WsPushMessage>(&text) {
                                            if let (Some(arg), Some(data_arr)) = (&ws_msg.arg, &ws_msg.data) {
                                                let inst_id = arg.inst_id.as_deref().unwrap_or("");
                                                for item in data_arr {
                                                    if let Some(event) = Self::parse_message(&arg.channel, inst_id, item) {
                                                        let _ = tx.send(event);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Some(Ok(Message::Ping(data))) => {
                                        let _ = write.send(Message::Pong(data)).await;
                                    }
                                    Some(Ok(Message::Close(_))) | None => {
                                        warn!("OKX WS disconnected");
                                        break;
                                    }
                                    Some(Err(e)) => {
                                        error!("OKX WS error: {e}");
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                            _ = ping_interval.tick() => {
                                if write.send(Message::Text("ping".to_string())).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("OKX WS connection failed: {e}");
                }
            }

            time::sleep(Duration::from_millis(backoff_ms)).await;
            backoff_ms = (backoff_ms * 2).min(30_000);
        }
    }
}
