use async_trait::async_trait;
use rust_decimal::Decimal;
use scalper_core::types::{Exchange, MarketEvent, OrderType, Side, TimeInForce};
use tokio::sync::broadcast;

/// Response from placing an order on an exchange.
#[derive(Debug, Clone)]
pub struct OrderResponse {
    pub order_id: String,
    pub exchange: Exchange,
    pub symbol: String,
    pub status: String,
}

/// Abstraction over exchange REST APIs for order management.
#[async_trait]
pub trait OrderManager: Send + Sync {
    /// Place a new order.
    async fn place_order(
        &self,
        symbol: &str,
        side: Side,
        order_type: OrderType,
        time_in_force: TimeInForce,
        quantity: Decimal,
        price: Option<Decimal>,
        reduce_only: bool,
    ) -> anyhow::Result<OrderResponse>;

    /// Cancel an open order.
    async fn cancel_order(&self, symbol: &str, order_id: &str) -> anyhow::Result<()>;

    /// Set leverage for a symbol.
    async fn set_leverage(&self, symbol: &str, leverage: u32) -> anyhow::Result<()>;

    /// Get current account balance.
    async fn get_balance(&self) -> anyhow::Result<Decimal>;

    /// Which exchange this manager represents.
    fn exchange(&self) -> Exchange;
}

/// Abstraction over exchange WebSocket feeds for market data.
#[async_trait]
pub trait MarketDataFeed: Send + Sync {
    /// Subscribe to market data for the given symbols and stream events into the sender.
    async fn subscribe(
        &self,
        symbols: &[String],
        tx: broadcast::Sender<MarketEvent>,
    ) -> anyhow::Result<()>;
}
