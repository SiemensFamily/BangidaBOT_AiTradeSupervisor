use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Type alias for trading symbol strings.
pub type Symbol = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderType {
    Market,
    Limit,
    StopMarket,
    TakeProfitMarket,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeInForce {
    GTC,
    IOC,
    FOK,
    PostOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Exchange {
    Binance,
    Bybit,
    OKX,
    Kraken,
}

impl fmt::Display for Exchange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Exchange::Binance => write!(f, "Binance"),
            Exchange::Bybit => write!(f, "Bybit"),
            Exchange::OKX => write!(f, "OKX"),
            Exchange::Kraken => write!(f, "Kraken"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VolatilityRegime {
    Ranging,
    Normal,
    Volatile,
    Extreme,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Trend {
    Up,
    Neutral,
    Down,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MarketEvent {
    OrderBookUpdate {
        exchange: Exchange,
        symbol: String,
        bids: Vec<(Decimal, Decimal)>,
        asks: Vec<(Decimal, Decimal)>,
        timestamp_ms: u64,
    },
    Trade {
        exchange: Exchange,
        symbol: String,
        price: Decimal,
        quantity: Decimal,
        is_buyer_maker: bool,
        timestamp_ms: u64,
    },
    KlineClose {
        exchange: Exchange,
        symbol: String,
        open: Decimal,
        high: Decimal,
        low: Decimal,
        close: Decimal,
        volume: Decimal,
        timestamp_ms: u64,
    },
    MarkPrice {
        exchange: Exchange,
        symbol: String,
        mark_price: Decimal,
        funding_rate: Decimal,
        next_funding_time: u64,
    },
    LiquidationEvent {
        exchange: Exchange,
        symbol: String,
        side: Side,
        quantity: Decimal,
        price: Decimal,
        timestamp_ms: u64,
    },
    OrderUpdate {
        exchange: Exchange,
        symbol: String,
        order_id: String,
        status: String,
        filled_qty: Decimal,
        avg_price: Decimal,
    },
    PositionUpdate {
        exchange: Exchange,
        symbol: String,
        side: Side,
        quantity: Decimal,
        entry_price: Decimal,
        unrealized_pnl: Decimal,
    },
    BalanceUpdate {
        exchange: Exchange,
        currency: String,
        available: Decimal,
        total: Decimal,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signal {
    pub strategy_name: String,
    pub symbol: String,
    pub exchange: Exchange,
    pub side: Side,
    pub strength: f64,
    pub confidence: f64,
    pub take_profit: Option<Decimal>,
    pub stop_loss: Option<Decimal>,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatedSignal {
    pub signal: Signal,
    pub quantity: Decimal,
    pub leverage: u32,
    pub max_loss: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub symbol: String,
    pub exchange: Exchange,
    pub side: Side,
    pub quantity: Decimal,
    pub entry_price: Decimal,
    pub leverage: u32,
    pub unrealized_pnl: Decimal,
    pub margin: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountBalance {
    pub exchange: Exchange,
    pub total_equity: Decimal,
    pub available_balance: Decimal,
    pub margin_used: Decimal,
}
