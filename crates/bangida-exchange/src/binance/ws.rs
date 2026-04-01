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
    BinanceAggTrade, BinanceKlineEvent, BinanceMarkPrice, CombinedStreamMessage, DepthUpdate,
    UserDataEvent, parse_decimal,
};
use super::rest::BinanceClient;
use crate::traits::MarketDataFeed;

/// Maximum exponential backoff delay for reconnection.
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);
/// Initial backoff delay.
const INITIAL_RECONNECT_DELAY: Duration = Duration::from_millis(500);
/// WebSocket ping interval (Binance disconnects after 5 min of silence).
const PING_INTERVAL: Duration = Duration::from_secs(180);
/// Listen key renewal interval.
const LISTEN_KEY_RENEWAL_INTERVAL: Duration = Duration::from_secs(30 * 60);
/// Broadcast channel capacity.
const CHANNEL_CAPACITY: usize = 8192;

/// Binance Futures WebSocket feed.
///
/// Connects to the combined stream endpoint, subscribes to the requested
/// symbols, and forwards parsed `MarketEvent` messages on a broadcast channel.
pub struct BinanceWsFeed {
    client: BinanceClient,
    tx: broadcast::Sender<MarketEvent>,
    shutdown: Arc<tokio::sync::Notify>,
}

impl BinanceWsFeed {
    /// Create a new WebSocket feed backed by the given `BinanceClient`.
    pub fn new(client: BinanceClient) -> Self {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            client,
            tx,
            shutdown: Arc::new(tokio::sync::Notify::new()),
        }
    }

    /// Build the combined stream URL for the given symbols.
    /// Core event loop: connect, read messages, reconnect on failure.
    async fn run_loop(
        client: BinanceClient,
        symbols: Vec<Symbol>,
        tx: broadcast::Sender<MarketEvent>,
        shutdown: Arc<tokio::sync::Notify>,
    ) {
        let mut backoff = INITIAL_RECONNECT_DELAY;

        loop {
            // Obtain a listen key for the user data stream.
            let listen_key = match client.create_listen_key().await {
                Ok(lk) => Some(lk),
                Err(e) => {
                    warn!(error = %e, "Failed to create listen key, continuing without user data stream");
                    None
                }
            };

            let url = Self::build_stream_url_static(
                client.ws_base_url(),
                &symbols,
                listen_key.as_deref(),
            );

            info!(url = %url, "Connecting to Binance WebSocket");

            let ws_stream = match connect_async(&url).await {
                Ok((stream, _)) => {
                    info!("Binance WebSocket connected");
                    backoff = INITIAL_RECONNECT_DELAY;
                    stream
                }
                Err(e) => {
                    error!(error = %e, "Binance WebSocket connection failed");
                    tokio::select! {
                        _ = tokio::time::sleep(backoff) => {}
                        _ = shutdown.notified() => {
                            info!("Shutdown requested during reconnect backoff");
                            return;
                        }
                    }
                    backoff = (backoff * 2).min(MAX_RECONNECT_DELAY);
                    continue;
                }
            };

            let (mut write, mut read) = ws_stream.split();

            let mut ping_timer = interval(PING_INTERVAL);
            ping_timer.reset();
            let mut listen_key_timer = interval(LISTEN_KEY_RENEWAL_INTERVAL);
            listen_key_timer.reset();
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
                                info!(?frame, "Binance WebSocket closed by server");
                                break;
                            }
                            Some(Ok(_)) => {}
                            Some(Err(e)) => {
                                error!(error = %e, "Binance WebSocket read error");
                                break;
                            }
                            None => {
                                warn!("Binance WebSocket stream ended");
                                break;
                            }
                        }
                    }
                    _ = ping_timer.tick() => {
                        // Check if we haven't received anything for too long
                        if last_msg.elapsed() > Duration::from_secs(300) {
                            warn!("No messages received for 5 minutes, reconnecting");
                            break;
                        }
                        if let Err(e) = write.send(Message::Ping(vec![].into())).await {
                            warn!(error = %e, "Failed to send ping");
                            break;
                        }
                        debug!("Sent ping to Binance WebSocket");
                    }
                    _ = listen_key_timer.tick() => {
                        if let Some(ref lk) = listen_key {
                            if let Err(e) = client.renew_listen_key(lk).await {
                                warn!(error = %e, "Failed to renew listen key");
                            }
                        }
                    }
                    _ = shutdown.notified() => {
                        info!("Shutdown requested, closing Binance WebSocket");
                        let _ = write.send(Message::Close(None)).await;
                        return;
                    }
                }
            }

            // Reconnect with backoff
            warn!(delay_ms = backoff.as_millis() as u64, "Reconnecting Binance WebSocket");
            tokio::select! {
                _ = tokio::time::sleep(backoff) => {}
                _ = shutdown.notified() => {
                    info!("Shutdown requested during reconnect backoff");
                    return;
                }
            }
            backoff = (backoff * 2).min(MAX_RECONNECT_DELAY);
        }
    }

    fn build_stream_url_static(
        ws_base_url: &str,
        symbols: &[Symbol],
        listen_key: Option<&str>,
    ) -> String {
        let mut streams: Vec<String> = Vec::new();
        for sym in symbols {
            let s = sym.0.to_lowercase();
            streams.push(format!("{s}@depth@100ms"));
            streams.push(format!("{s}@aggTrade"));
            streams.push(format!("{s}@kline_1m"));
            streams.push(format!("{s}@markPrice@1s"));
        }
        if let Some(lk) = listen_key {
            streams.push(lk.to_string());
        }
        let joined = streams.join("/");
        format!("{ws_base_url}/stream?streams={joined}")
    }

    /// Parse a single text message from the combined stream.
    fn handle_text_message(text: &str, tx: &broadcast::Sender<MarketEvent>) {
        let combined: CombinedStreamMessage = match serde_json::from_str(text) {
            Ok(m) => m,
            Err(_) => {
                // Might be a user data event (not wrapped in combined stream format).
                Self::try_parse_user_data(text, tx);
                return;
            }
        };

        let stream = &combined.stream;

        if stream.ends_with("@depth@100ms") {
            Self::handle_depth(&combined.data, tx);
        } else if stream.ends_with("@aggTrade") {
            Self::handle_agg_trade(&combined.data, tx);
        } else if stream.contains("@kline_") {
            Self::handle_kline(&combined.data, tx);
        } else if stream.contains("@markPrice") {
            Self::handle_mark_price(&combined.data, tx);
        } else {
            // Could be a user data stream event
            Self::try_parse_user_data_value(&combined.data, tx);
        }
    }

    fn handle_depth(data: &serde_json::Value, tx: &broadcast::Sender<MarketEvent>) {
        let depth: DepthUpdate = match serde_json::from_value(data.clone()) {
            Ok(d) => d,
            Err(e) => {
                debug!(error = %e, "Failed to parse depth update");
                return;
            }
        };

        let bids: Vec<(Decimal, Decimal)> = depth
            .bids
            .iter()
            .filter_map(|b| {
                let price = parse_decimal(&b[0]).ok()?;
                let qty = parse_decimal(&b[1]).ok()?;
                Some((price, qty))
            })
            .collect();

        let asks: Vec<(Decimal, Decimal)> = depth
            .asks
            .iter()
            .filter_map(|a| {
                let price = parse_decimal(&a[0]).ok()?;
                let qty = parse_decimal(&a[1]).ok()?;
                Some((price, qty))
            })
            .collect();

        let event = MarketEvent::OrderBookUpdate {
            exchange: Exchange::Binance,
            symbol: Symbol::new(&depth.symbol),
            bids,
            asks,
            timestamp_ms: depth.transaction_time,
        };
        let _ = tx.send(event);
    }

    fn handle_agg_trade(data: &serde_json::Value, tx: &broadcast::Sender<MarketEvent>) {
        let trade: BinanceAggTrade = match serde_json::from_value(data.clone()) {
            Ok(t) => t,
            Err(e) => {
                debug!(error = %e, "Failed to parse aggTrade");
                return;
            }
        };

        let price = match parse_decimal(&trade.price) {
            Ok(p) => p,
            Err(_) => return,
        };
        let quantity = match parse_decimal(&trade.quantity) {
            Ok(q) => q,
            Err(_) => return,
        };

        let event = MarketEvent::Trade(AggTrade {
            symbol: Symbol::new(&trade.symbol),
            price,
            quantity,
            timestamp_ms: trade.trade_time,
            is_buyer_maker: trade.is_buyer_maker,
        });
        let _ = tx.send(event);
    }

    fn handle_kline(data: &serde_json::Value, tx: &broadcast::Sender<MarketEvent>) {
        let kline_event: BinanceKlineEvent = match serde_json::from_value(data.clone()) {
            Ok(k) => k,
            Err(e) => {
                debug!(error = %e, "Failed to parse kline");
                return;
            }
        };

        // Only emit on candle close
        if !kline_event.kline.is_closed {
            return;
        }

        let k = &kline_event.kline;
        let event = MarketEvent::KlineClose(Kline {
            symbol: Symbol::new(&kline_event.symbol),
            open: parse_decimal(&k.open).unwrap_or_default(),
            high: parse_decimal(&k.high).unwrap_or_default(),
            low: parse_decimal(&k.low).unwrap_or_default(),
            close: parse_decimal(&k.close).unwrap_or_default(),
            volume: parse_decimal(&k.volume).unwrap_or_default(),
            open_time_ms: k.open_time,
            close_time_ms: k.close_time,
        });
        let _ = tx.send(event);
    }

    fn handle_mark_price(data: &serde_json::Value, tx: &broadcast::Sender<MarketEvent>) {
        let mp: BinanceMarkPrice = match serde_json::from_value(data.clone()) {
            Ok(m) => m,
            Err(e) => {
                debug!(error = %e, "Failed to parse markPrice");
                return;
            }
        };

        let event = MarketEvent::MarkPrice {
            symbol: Symbol::new(&mp.symbol),
            mark_price: parse_decimal(&mp.mark_price).unwrap_or_default(),
            funding_rate: parse_decimal(&mp.funding_rate).unwrap_or_default(),
            next_funding_time_ms: mp.next_funding_time,
        };
        let _ = tx.send(event);
    }

    fn try_parse_user_data(text: &str, tx: &broadcast::Sender<MarketEvent>) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(text) {
            Self::try_parse_user_data_value(&val, tx);
        }
    }

    fn try_parse_user_data_value(data: &serde_json::Value, tx: &broadcast::Sender<MarketEvent>) {
        let event: UserDataEvent = match serde_json::from_value(data.clone()) {
            Ok(e) => e,
            Err(_) => return,
        };

        match event.event_type.as_str() {
            "ORDER_TRADE_UPDATE" => {
                if let Some(ref o) = event.order {
                    use bangida_core::{OrderResponse, OrderStatus, OrderType, Side};

                    let side = match o.side.as_str() {
                        "BUY" => Side::Buy,
                        _ => Side::Sell,
                    };
                    let order_type = match o.order_type.as_str() {
                        "MARKET" => OrderType::Market,
                        "STOP_MARKET" => OrderType::StopMarket,
                        "TAKE_PROFIT_MARKET" => OrderType::TakeProfitMarket,
                        _ => OrderType::Limit,
                    };
                    let status = match o.order_status.as_str() {
                        "NEW" => OrderStatus::New,
                        "PARTIALLY_FILLED" => OrderStatus::PartiallyFilled,
                        "FILLED" => OrderStatus::Filled,
                        "CANCELED" => OrderStatus::Canceled,
                        "REJECTED" => OrderStatus::Rejected,
                        "EXPIRED" => OrderStatus::Expired,
                        _ => OrderStatus::New,
                    };
                    let resp = OrderResponse {
                        order_id: o.order_id.to_string(),
                        client_order_id: o.client_order_id.clone(),
                        symbol: Symbol::new(&o.symbol),
                        side,
                        order_type,
                        quantity: parse_decimal(&o.original_quantity).unwrap_or_default(),
                        price: parse_decimal(&o.original_price).ok().filter(|p| *p != Decimal::ZERO),
                        status,
                        timestamp_ms: o.order_trade_time,
                    };
                    let _ = tx.send(MarketEvent::OrderUpdate(resp));
                }
            }
            "ACCOUNT_UPDATE" => {
                if let Some(ref a) = event.account {
                    // Emit position updates
                    for p in &a.positions {
                        let qty = parse_decimal(&p.position_amount).unwrap_or_default();
                        if qty == Decimal::ZERO {
                            continue;
                        }
                        use bangida_core::{Position, Side};
                        let side = if qty >= Decimal::ZERO {
                            Side::Buy
                        } else {
                            Side::Sell
                        };
                        let pos = Position {
                            symbol: Symbol::new(&p.symbol),
                            side,
                            quantity: qty.abs(),
                            entry_price: parse_decimal(&p.entry_price).unwrap_or_default(),
                            unrealized_pnl: parse_decimal(&p.unrealized_pnl).unwrap_or_default(),
                            leverage: 0, // Not available in user data stream
                            margin: Decimal::ZERO,
                        };
                        let _ = tx.send(MarketEvent::PositionUpdate(pos));
                    }

                    // Emit balance update from the first USDT balance
                    for b in &a.balances {
                        if b.asset == "USDT" {
                            use bangida_core::AccountBalance;
                            let total = parse_decimal(&b.wallet_balance).unwrap_or_default();
                            let cross = parse_decimal(&b.cross_wallet_balance).unwrap_or_default();
                            let balance = AccountBalance {
                                total_balance: total,
                                available_balance: cross,
                                unrealized_pnl: Decimal::ZERO,
                                margin_used: total - cross,
                            };
                            let _ = tx.send(MarketEvent::BalanceUpdate(balance));
                            break;
                        }
                    }
                }
            }
            _ => {
                debug!(event_type = %event.event_type, "Unknown user data event type");
            }
        }
    }
}

#[async_trait]
impl MarketDataFeed for BinanceWsFeed {
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

        info!("Binance WebSocket feed subscribed");
        Ok(rx)
    }

    async fn shutdown(&self) -> Result<()> {
        info!("Shutting down Binance WebSocket feed");
        self.shutdown.notify_waiters();
        Ok(())
    }
}
