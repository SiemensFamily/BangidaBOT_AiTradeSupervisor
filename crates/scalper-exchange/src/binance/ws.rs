use anyhow::{Context, Result};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use rust_decimal::Decimal;
use scalper_core::config::ExchangeConfig;
use scalper_core::types::{Exchange, MarketEvent, Side};
use std::str::FromStr;
use tokio::sync::broadcast;
use tokio::time::{self, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

use super::models::*;
use crate::traits::MarketDataFeed;

/// Binance Futures WebSocket feed.
pub struct BinanceWsFeed {
    config: ExchangeConfig,
}

impl BinanceWsFeed {
    pub fn new(config: ExchangeConfig) -> Self {
        Self { config }
    }

    fn build_stream_url(&self, symbols: &[String]) -> String {
        let streams: Vec<String> = symbols
            .iter()
            .flat_map(|s| {
                let lower = s.to_lowercase();
                vec![
                    format!("{lower}@depth@100ms"),
                    format!("{lower}@aggTrade"),
                    format!("{lower}@kline_1m"),
                    format!("{lower}@markPrice@1s"),
                    format!("{lower}@forceOrder"),
                ]
            })
            .collect();
        let joined = streams.join("/");
        format!("{}/stream?streams={joined}", self.config.base_url_ws)
    }

    fn parse_event(stream: &str, data: &serde_json::Value) -> Option<MarketEvent> {
        if stream.contains("@depth") {
            let update: WsDepthUpdate = serde_json::from_value(data.clone()).ok()?;
            let bids = update
                .b
                .iter()
                .filter_map(|[p, q]| {
                    Some((Decimal::from_str(p).ok()?, Decimal::from_str(q).ok()?))
                })
                .collect();
            let asks = update
                .a
                .iter()
                .filter_map(|[p, q]| {
                    Some((Decimal::from_str(p).ok()?, Decimal::from_str(q).ok()?))
                })
                .collect();
            Some(MarketEvent::OrderBookUpdate {
                exchange: Exchange::Binance,
                symbol: update.s,
                bids,
                asks,
                timestamp_ms: update.event_time,
            })
        } else if stream.contains("@aggTrade") {
            let trade: WsAggTrade = serde_json::from_value(data.clone()).ok()?;
            Some(MarketEvent::Trade {
                exchange: Exchange::Binance,
                symbol: trade.s,
                price: Decimal::from_str(&trade.p).ok()?,
                quantity: Decimal::from_str(&trade.q).ok()?,
                is_buyer_maker: trade.m,
                timestamp_ms: trade.event_time,
            })
        } else if stream.contains("@kline") {
            let kline: WsKline = serde_json::from_value(data.clone()).ok()?;
            if !kline.k.x {
                return None; // Only emit on candle close
            }
            Some(MarketEvent::KlineClose {
                exchange: Exchange::Binance,
                symbol: kline.s,
                open: Decimal::from_str(&kline.k.o).ok()?,
                high: Decimal::from_str(&kline.k.h).ok()?,
                low: Decimal::from_str(&kline.k.l).ok()?,
                close: Decimal::from_str(&kline.k.c).ok()?,
                volume: Decimal::from_str(&kline.k.v).ok()?,
                timestamp_ms: kline.k.close_time,
            })
        } else if stream.contains("@markPrice") {
            let mp: WsMarkPrice = serde_json::from_value(data.clone()).ok()?;
            Some(MarketEvent::MarkPrice {
                exchange: Exchange::Binance,
                symbol: mp.s,
                mark_price: Decimal::from_str(&mp.p).ok()?,
                funding_rate: Decimal::from_str(&mp.r).ok()?,
                next_funding_time: mp.next_funding_time,
            })
        } else if stream.contains("@forceOrder") {
            let fo: WsForceOrder = serde_json::from_value(data.clone()).ok()?;
            let side = if fo.o.side == "BUY" {
                Side::Buy
            } else {
                Side::Sell
            };
            Some(MarketEvent::LiquidationEvent {
                exchange: Exchange::Binance,
                symbol: fo.o.s,
                side,
                quantity: Decimal::from_str(&fo.o.q).ok()?,
                price: Decimal::from_str(&fo.o.p).ok()?,
                timestamp_ms: fo.o.trade_time,
            })
        } else {
            None
        }
    }
}

#[async_trait]
impl MarketDataFeed for BinanceWsFeed {
    async fn subscribe(
        &self,
        symbols: &[String],
        tx: broadcast::Sender<MarketEvent>,
    ) -> Result<()> {
        let url = self.build_stream_url(symbols);
        info!("Binance WS connecting to {url}");

        let mut backoff_ms = 500u64;
        let max_backoff_ms = 30_000u64;

        loop {
            match connect_async(&url).await {
                Ok((ws_stream, _)) => {
                    info!("Binance WS connected");
                    backoff_ms = 500;

                    let (mut write, mut read) = ws_stream.split();
                    let mut ping_interval = time::interval(Duration::from_secs(180));

                    loop {
                        tokio::select! {
                            msg = read.next() => {
                                match msg {
                                    Some(Ok(Message::Text(text))) => {
                                        if let Ok(wrapper) = serde_json::from_str::<WsStreamMessage>(&text) {
                                            if let Some(event) = Self::parse_event(&wrapper.stream, &wrapper.data) {
                                                let _ = tx.send(event);
                                            }
                                        }
                                    }
                                    Some(Ok(Message::Ping(data))) => {
                                        let _ = write.send(Message::Pong(data)).await;
                                    }
                                    Some(Ok(Message::Close(_))) | None => {
                                        warn!("Binance WS disconnected, reconnecting...");
                                        break;
                                    }
                                    Some(Err(e)) => {
                                        error!("Binance WS error: {e}");
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                            _ = ping_interval.tick() => {
                                if write.send(Message::Ping(vec![])).await.is_err() {
                                    warn!("Binance WS ping failed, reconnecting...");
                                    break;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("Binance WS connection failed: {e}");
                }
            }

            warn!("Binance WS reconnecting in {backoff_ms}ms...");
            time::sleep(Duration::from_millis(backoff_ms)).await;
            backoff_ms = (backoff_ms * 2).min(max_backoff_ms);
        }
    }
}
