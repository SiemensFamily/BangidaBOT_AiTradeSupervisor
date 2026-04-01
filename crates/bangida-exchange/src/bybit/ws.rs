use anyhow::Result;
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use rust_decimal::Decimal;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::time::{interval, Instant};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info, warn};

use bangida_core::{AggTrade, Exchange, Kline, MarketEvent, Symbol};

use super::models::{
    BybitWsMessage, KlineData, OrderBookSnapshot, PublicTrade, TickerData,
    parse_decimal,
};
use super::rest::BybitClient;
use crate::traits::MarketDataFeed;

/// Maximum exponential backoff delay for reconnection.
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);
/// Initial backoff delay.
const INITIAL_RECONNECT_DELAY: Duration = Duration::from_millis(500);
/// Bybit requires a heartbeat ping every 20 seconds.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(20);
/// Broadcast channel capacity.
const CHANNEL_CAPACITY: usize = 8192;

/// Bybit V5 WebSocket feed.
pub struct BybitWsFeed {
    client: BybitClient,
    tx: broadcast::Sender<MarketEvent>,
    shutdown: Arc<tokio::sync::Notify>,
}

impl BybitWsFeed {
    pub fn new(client: BybitClient) -> Self {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            client,
            tx,
            shutdown: Arc::new(tokio::sync::Notify::new()),
        }
    }

    /// Build subscription messages for the given symbols.
    fn build_subscribe_msg(symbols: &[Symbol]) -> serde_json::Value {
        let mut args: Vec<String> = Vec::new();
        for sym in symbols {
            let s = &sym.0;
            args.push(format!("orderbook.50.{s}"));
            args.push(format!("publicTrade.{s}"));
            args.push(format!("kline.1.{s}"));
            args.push(format!("tickers.{s}"));
        }
        serde_json::json!({
            "op": "subscribe",
            "args": args,
        })
    }

    /// Build the WebSocket auth message for the private channel.
    fn build_auth_msg(auth: &super::auth::BybitAuth) -> serde_json::Value {
        let expires = chrono::Utc::now().timestamp_millis() + 10_000;
        let signature = auth.ws_auth_signature(expires as u64);
        serde_json::json!({
            "op": "auth",
            "args": [auth.api_key(), expires, signature],
        })
    }

    /// Build the heartbeat ping message.
    fn build_ping_msg() -> serde_json::Value {
        serde_json::json!({
            "op": "ping",
        })
    }

    /// Core event loop.
    async fn run_loop(
        client: BybitClient,
        symbols: Vec<Symbol>,
        tx: broadcast::Sender<MarketEvent>,
        shutdown: Arc<tokio::sync::Notify>,
    ) {
        let mut backoff = INITIAL_RECONNECT_DELAY;

        loop {
            // Bybit uses separate URLs for public and private. For simplicity
            // we connect to the public linear endpoint and authenticate on it.
            let url = format!("{}/v5/public/linear", client.ws_base_url());

            info!(url = %url, "Connecting to Bybit WebSocket");

            let ws_stream = match connect_async(&url).await {
                Ok((stream, _)) => {
                    info!("Bybit WebSocket connected");
                    backoff = INITIAL_RECONNECT_DELAY;
                    stream
                }
                Err(e) => {
                    error!(error = %e, "Bybit WebSocket connection failed");
                    tokio::select! {
                        _ = tokio::time::sleep(backoff) => {}
                        _ = shutdown.notified() => {
                            info!("Shutdown during Bybit reconnect backoff");
                            return;
                        }
                    }
                    backoff = (backoff * 2).min(MAX_RECONNECT_DELAY);
                    continue;
                }
            };

            let (mut write, mut read) = ws_stream.split();

            // Authenticate for private topics (order/position updates)
            let auth_msg = Self::build_auth_msg(client.auth());
            if let Err(e) = write
                .send(Message::Text(serde_json::to_string(&auth_msg).unwrap().into()))
                .await
            {
                warn!(error = %e, "Failed to send Bybit auth message");
            }

            // Subscribe to topics
            let sub_msg = Self::build_subscribe_msg(&symbols);
            if let Err(e) = write
                .send(Message::Text(serde_json::to_string(&sub_msg).unwrap().into()))
                .await
            {
                error!(error = %e, "Failed to send Bybit subscribe message");
                continue;
            }

            let mut heartbeat_timer = interval(HEARTBEAT_INTERVAL);
            heartbeat_timer.reset();
            let mut last_msg = Instant::now();

            loop {
                tokio::select! {
                    msg = read.next() => {
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                last_msg = Instant::now();
                                Self::handle_text_message(&text, &tx);
                            }
                            Some(Ok(Message::Ping(data))) => {
                                last_msg = Instant::now();
                                if let Err(e) = write.send(Message::Pong(data)).await {
                                    warn!(error = %e, "Failed to send pong");
                                    break;
                                }
                            }
                            Some(Ok(Message::Pong(_))) => {
                                last_msg = Instant::now();
                            }
                            Some(Ok(Message::Close(frame))) => {
                                info!(?frame, "Bybit WebSocket closed by server");
                                break;
                            }
                            Some(Ok(_)) => {}
                            Some(Err(e)) => {
                                error!(error = %e, "Bybit WebSocket read error");
                                break;
                            }
                            None => {
                                warn!("Bybit WebSocket stream ended");
                                break;
                            }
                        }
                    }
                    _ = heartbeat_timer.tick() => {
                        if last_msg.elapsed() > Duration::from_secs(60) {
                            warn!("No Bybit messages for 60s, reconnecting");
                            break;
                        }
                        let ping_msg = Self::build_ping_msg();
                        if let Err(e) = write
                            .send(Message::Text(serde_json::to_string(&ping_msg).unwrap().into()))
                            .await
                        {
                            warn!(error = %e, "Failed to send Bybit heartbeat");
                            break;
                        }
                        debug!("Sent heartbeat ping to Bybit");
                    }
                    _ = shutdown.notified() => {
                        info!("Shutdown requested, closing Bybit WebSocket");
                        let _ = write.send(Message::Close(None)).await;
                        return;
                    }
                }
            }

            warn!(delay_ms = backoff.as_millis() as u64, "Reconnecting Bybit WebSocket");
            tokio::select! {
                _ = tokio::time::sleep(backoff) => {}
                _ = shutdown.notified() => {
                    info!("Shutdown during Bybit reconnect backoff");
                    return;
                }
            }
            backoff = (backoff * 2).min(MAX_RECONNECT_DELAY);
        }
    }

    fn handle_text_message(text: &str, tx: &broadcast::Sender<MarketEvent>) {
        let msg: BybitWsMessage = match serde_json::from_str(text) {
            Ok(m) => m,
            Err(e) => {
                debug!(error = %e, "Failed to parse Bybit WS message");
                return;
            }
        };

        // Handle operational responses (pong, subscribe confirmations)
        if let Some(ref op) = msg.op {
            match op.as_str() {
                "pong" => {
                    debug!("Received Bybit pong");
                    return;
                }
                "subscribe" => {
                    if msg.success == Some(true) {
                        debug!("Bybit subscription confirmed");
                    } else {
                        warn!(ret_msg = ?msg.ret_msg, "Bybit subscription failed");
                    }
                    return;
                }
                "auth" => {
                    if msg.success == Some(true) {
                        info!("Bybit WebSocket authenticated");
                    } else {
                        warn!(ret_msg = ?msg.ret_msg, "Bybit WebSocket auth failed");
                    }
                    return;
                }
                _ => return,
            }
        }

        let topic = match msg.topic {
            Some(ref t) => t.as_str(),
            None => return,
        };

        let data = match msg.data {
            Some(ref d) => d,
            None => return,
        };

        let ts = msg.ts.unwrap_or(0);

        if topic.starts_with("orderbook.") {
            Self::handle_orderbook(topic, data, ts, msg.msg_type.as_deref(), tx);
        } else if topic.starts_with("publicTrade.") {
            Self::handle_public_trade(data, tx);
        } else if topic.starts_with("kline.") {
            Self::handle_kline(topic, data, tx);
        } else if topic.starts_with("tickers.") {
            Self::handle_ticker(data, tx);
        } else {
            debug!(topic, "Unknown Bybit topic");
        }
    }

    fn handle_orderbook(
        _topic: &str,
        data: &serde_json::Value,
        ts: u64,
        _msg_type: Option<&str>,
        tx: &broadcast::Sender<MarketEvent>,
    ) {
        // Both snapshot and delta have the same shape for our purposes
        let ob: OrderBookSnapshot = match serde_json::from_value(data.clone()) {
            Ok(o) => o,
            Err(e) => {
                debug!(error = %e, "Failed to parse Bybit orderbook");
                return;
            }
        };

        let bids: Vec<(Decimal, Decimal)> = ob
            .b
            .iter()
            .filter_map(|b| {
                let price = parse_decimal(&b[0]).ok()?;
                let qty = parse_decimal(&b[1]).ok()?;
                Some((price, qty))
            })
            .collect();

        let asks: Vec<(Decimal, Decimal)> = ob
            .a
            .iter()
            .filter_map(|a| {
                let price = parse_decimal(&a[0]).ok()?;
                let qty = parse_decimal(&a[1]).ok()?;
                Some((price, qty))
            })
            .collect();

        let event = MarketEvent::OrderBookUpdate {
            exchange: Exchange::Bybit,
            symbol: Symbol::new(&ob.s),
            bids,
            asks,
            timestamp_ms: ts,
        };
        let _ = tx.send(event);
    }

    fn handle_public_trade(data: &serde_json::Value, tx: &broadcast::Sender<MarketEvent>) {
        // Bybit sends trades as an array
        let trades: Vec<PublicTrade> = match serde_json::from_value(data.clone()) {
            Ok(t) => t,
            Err(_) => {
                // Try single trade
                match serde_json::from_value::<PublicTrade>(data.clone()) {
                    Ok(t) => vec![t],
                    Err(e) => {
                        debug!(error = %e, "Failed to parse Bybit trade");
                        return;
                    }
                }
            }
        };

        for trade in trades {
            let price = match parse_decimal(&trade.p) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let qty = match parse_decimal(&trade.v) {
                Ok(q) => q,
                Err(_) => continue,
            };
            let is_buyer_maker = trade.side == "Sell";

            let event = MarketEvent::Trade(AggTrade {
                symbol: Symbol::new(&trade.s),
                price,
                quantity: qty,
                timestamp_ms: trade.timestamp,
                is_buyer_maker,
            });
            let _ = tx.send(event);
        }
    }

    fn handle_kline(
        topic: &str,
        data: &serde_json::Value,
        tx: &broadcast::Sender<MarketEvent>,
    ) {
        let klines: Vec<KlineData> = match serde_json::from_value(data.clone()) {
            Ok(k) => k,
            Err(_) => {
                match serde_json::from_value::<KlineData>(data.clone()) {
                    Ok(k) => vec![k],
                    Err(e) => {
                        debug!(error = %e, "Failed to parse Bybit kline");
                        return;
                    }
                }
            }
        };

        // Extract symbol from topic: "kline.1.BTCUSDT"
        let symbol = topic
            .rsplit('.')
            .next()
            .unwrap_or("UNKNOWN");

        for kline in klines {
            if !kline.confirm {
                continue;
            }

            let event = MarketEvent::KlineClose(Kline {
                symbol: Symbol::new(symbol),
                open: parse_decimal(&kline.open).unwrap_or_default(),
                high: parse_decimal(&kline.high).unwrap_or_default(),
                low: parse_decimal(&kline.low).unwrap_or_default(),
                close: parse_decimal(&kline.close).unwrap_or_default(),
                volume: parse_decimal(&kline.volume).unwrap_or_default(),
                open_time_ms: kline.start,
                close_time_ms: kline.end,
            });
            let _ = tx.send(event);
        }
    }

    fn handle_ticker(data: &serde_json::Value, tx: &broadcast::Sender<MarketEvent>) {
        let ticker: TickerData = match serde_json::from_value(data.clone()) {
            Ok(t) => t,
            Err(e) => {
                debug!(error = %e, "Failed to parse Bybit ticker");
                return;
            }
        };

        let mark_price = ticker
            .mark_price
            .as_deref()
            .and_then(|s| parse_decimal(s).ok())
            .unwrap_or_default();
        let funding_rate = ticker
            .funding_rate
            .as_deref()
            .and_then(|s| parse_decimal(s).ok())
            .unwrap_or_default();
        let next_funding = ticker
            .next_funding_time
            .as_deref()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        if mark_price == Decimal::ZERO {
            return;
        }

        let event = MarketEvent::MarkPrice {
            symbol: Symbol::new(&ticker.symbol),
            mark_price,
            funding_rate,
            next_funding_time_ms: next_funding,
        };
        let _ = tx.send(event);
    }
}

#[async_trait]
impl MarketDataFeed for BybitWsFeed {
    async fn subscribe(
        &self,
        symbols: &[Symbol],
    ) -> Result<broadcast::Receiver<MarketEvent>> {
        let rx = self.tx.subscribe();
        let client = self.client.clone();
        let symbols = symbols.to_vec();
        let tx = self.tx.clone();
        let shutdown = self.shutdown.clone();

        tokio::spawn(async move {
            Self::run_loop(client, symbols, tx, shutdown).await;
        });

        info!("Bybit WebSocket feed subscribed");
        Ok(rx)
    }

    async fn shutdown(&self) -> Result<()> {
        info!("Shutting down Bybit WebSocket feed");
        self.shutdown.notify_waiters();
        Ok(())
    }
}
