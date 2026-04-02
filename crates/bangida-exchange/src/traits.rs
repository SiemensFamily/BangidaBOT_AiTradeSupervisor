use async_trait::async_trait;
use bangida_core::{
    AccountBalance, MarketEvent, OrderRequest, OrderResponse, OrderId, Position, Symbol,
};
use tokio::sync::broadcast;

/// Provides a live market data stream from an exchange via WebSocket.
#[async_trait]
pub trait MarketDataFeed: Send + Sync {
    /// Subscribe to market data for the given symbols.
    /// Returns a broadcast receiver that emits `MarketEvent` variants.
    async fn subscribe(
        &self,
        symbols: &[Symbol],
    ) -> anyhow::Result<broadcast::Receiver<MarketEvent>>;

    /// Shut down the feed gracefully.
    async fn shutdown(&self) -> anyhow::Result<()>;
}

/// Manages orders and account state on an exchange.
#[async_trait]
pub trait OrderManager: Send + Sync {
    /// Place an order on the exchange.
    async fn place_order(&self, request: &OrderRequest) -> anyhow::Result<OrderResponse>;

    /// Cancel a single order by its exchange order id.
    async fn cancel_order(
        &self,
        symbol: &Symbol,
        order_id: &OrderId,
    ) -> anyhow::Result<OrderResponse>;

    /// Cancel all open orders for a symbol.
    async fn cancel_all_orders(&self, symbol: &Symbol) -> anyhow::Result<Vec<OrderResponse>>;

    /// Get current position for a symbol.
    async fn get_position(&self, symbol: &Symbol) -> anyhow::Result<Position>;

    /// Get the account balance summary.
    async fn get_account_balance(&self) -> anyhow::Result<AccountBalance>;

    /// Set leverage for a symbol.
    async fn set_leverage(&self, symbol: &Symbol, leverage: u32) -> anyhow::Result<()>;
}
