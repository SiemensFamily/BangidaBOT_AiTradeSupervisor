use rust_decimal::Decimal;
use scalper_core::types::{Exchange, OrderType, Side, TimeInForce, ValidatedSignal};

use crate::latency::LatencyTracker;

/// A prepared order ready to send to an exchange.
#[derive(Debug, Clone)]
pub struct PreparedOrder {
    pub symbol: String,
    pub exchange: Exchange,
    pub side: Side,
    pub order_type: OrderType,
    pub time_in_force: TimeInForce,
    pub price: Option<Decimal>,
    pub quantity: Decimal,
    pub reduce_only: bool,
}

/// Converts validated signals into exchange-ready orders.
pub struct Executor {
    latency: LatencyTracker,
}

impl Executor {
    pub fn new() -> Self {
        Self {
            latency: LatencyTracker::default(),
        }
    }

    /// Prepare an order from a validated signal and current market data.
    ///
    /// Logic based on signal strength:
    /// - strength > 0.8 (aggressive): Limit crossing spread by 1 tick, IOC
    /// - strength 0.3..=0.8 (normal): Limit at best bid/ask, PostOnly
    /// - strength < 0.3 (passive): Limit at best bid/ask, PostOnly
    /// - strength <= 0.0 (stop_loss): Market order, IOC
    pub fn prepare_order(
        &self,
        signal: &ValidatedSignal,
        best_bid: Decimal,
        best_ask: Decimal,
        tick_size: Decimal,
    ) -> PreparedOrder {
        let strength = signal.signal.strength;
        let side = signal.signal.side;

        if strength <= 0.0 {
            // Stop-loss urgency: market order
            return PreparedOrder {
                symbol: signal.signal.symbol.clone(),
                exchange: signal.signal.exchange,
                side,
                order_type: OrderType::Market,
                time_in_force: TimeInForce::IOC,
                price: None,
                quantity: signal.quantity,
                reduce_only: false,
            };
        }

        if strength > 0.8 {
            // Aggressive: cross the spread by 1 tick
            let price = match side {
                Side::Buy => best_ask + tick_size,
                Side::Sell => best_bid - tick_size,
            };
            PreparedOrder {
                symbol: signal.signal.symbol.clone(),
                exchange: signal.signal.exchange,
                side,
                order_type: OrderType::Limit,
                time_in_force: TimeInForce::IOC,
                price: Some(price),
                quantity: signal.quantity,
                reduce_only: false,
            }
        } else {
            // Normal (0.3..=0.8) and passive (< 0.3): post at best bid/ask
            let price = match side {
                Side::Buy => best_bid,
                Side::Sell => best_ask,
            };
            PreparedOrder {
                symbol: signal.signal.symbol.clone(),
                exchange: signal.signal.exchange,
                side,
                order_type: OrderType::Limit,
                time_in_force: TimeInForce::PostOnly,
                price: Some(price),
                quantity: signal.quantity,
                reduce_only: false,
            }
        }
    }

    /// Prepare a stop-loss order (StopMarket, reduce_only).
    pub fn prepare_stop_loss(
        &self,
        symbol: &str,
        exchange: Exchange,
        side: Side,
        quantity: Decimal,
        stop_price: Decimal,
    ) -> PreparedOrder {
        PreparedOrder {
            symbol: symbol.to_string(),
            exchange,
            side,
            order_type: OrderType::StopMarket,
            time_in_force: TimeInForce::GTC,
            price: Some(stop_price),
            quantity,
            reduce_only: true,
        }
    }

    /// Prepare a take-profit order (TakeProfitMarket, reduce_only).
    pub fn prepare_take_profit(
        &self,
        symbol: &str,
        exchange: Exchange,
        side: Side,
        quantity: Decimal,
        tp_price: Decimal,
    ) -> PreparedOrder {
        PreparedOrder {
            symbol: symbol.to_string(),
            exchange,
            side,
            order_type: OrderType::TakeProfitMarket,
            time_in_force: TimeInForce::GTC,
            price: Some(tp_price),
            quantity,
            reduce_only: true,
        }
    }

    /// Record an order execution latency sample in microseconds.
    pub fn record_latency(&mut self, latency_us: u64) {
        self.latency.record(latency_us);
    }

    /// Access latency statistics.
    pub fn latency_stats(&self) -> &LatencyTracker {
        &self.latency
    }
}

impl Default for Executor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use scalper_core::types::Signal;

    fn make_signal(side: Side, strength: f64, quantity: Decimal) -> ValidatedSignal {
        ValidatedSignal {
            signal: Signal {
                strategy_name: "test_strategy".to_string(),
                symbol: "BTCUSDT".to_string(),
                exchange: Exchange::Binance,
                side,
                strength,
                confidence: 0.9,
                take_profit: None,
                stop_loss: None,
                timestamp_ms: 1000,
            },
            quantity,
            leverage: 10,
            max_loss: dec!(100),
        }
    }

    #[test]
    fn test_aggressive_buy() {
        let executor = Executor::new();
        let signal = make_signal(Side::Buy, 0.9, dec!(0.5));
        let order = executor.prepare_order(&signal, dec!(50000), dec!(50010), dec!(0.1));

        assert_eq!(order.side, Side::Buy);
        assert_eq!(order.order_type, OrderType::Limit);
        assert_eq!(order.time_in_force, TimeInForce::IOC);
        assert_eq!(order.price, Some(dec!(50010.1))); // best_ask + tick
        assert_eq!(order.quantity, dec!(0.5));
        assert!(!order.reduce_only);
    }

    #[test]
    fn test_aggressive_sell() {
        let executor = Executor::new();
        let signal = make_signal(Side::Sell, 0.85, dec!(1.0));
        let order = executor.prepare_order(&signal, dec!(50000), dec!(50010), dec!(0.1));

        assert_eq!(order.side, Side::Sell);
        assert_eq!(order.order_type, OrderType::Limit);
        assert_eq!(order.time_in_force, TimeInForce::IOC);
        assert_eq!(order.price, Some(dec!(49999.9))); // best_bid - tick
    }

    #[test]
    fn test_normal_buy() {
        let executor = Executor::new();
        let signal = make_signal(Side::Buy, 0.5, dec!(0.5));
        let order = executor.prepare_order(&signal, dec!(50000), dec!(50010), dec!(0.1));

        assert_eq!(order.order_type, OrderType::Limit);
        assert_eq!(order.time_in_force, TimeInForce::PostOnly);
        assert_eq!(order.price, Some(dec!(50000))); // best_bid
    }

    #[test]
    fn test_normal_sell() {
        let executor = Executor::new();
        let signal = make_signal(Side::Sell, 0.6, dec!(1.0));
        let order = executor.prepare_order(&signal, dec!(50000), dec!(50010), dec!(0.1));

        assert_eq!(order.order_type, OrderType::Limit);
        assert_eq!(order.time_in_force, TimeInForce::PostOnly);
        assert_eq!(order.price, Some(dec!(50010))); // best_ask
    }

    #[test]
    fn test_passive_buy() {
        let executor = Executor::new();
        let signal = make_signal(Side::Buy, 0.2, dec!(0.5));
        let order = executor.prepare_order(&signal, dec!(50000), dec!(50010), dec!(0.1));

        assert_eq!(order.order_type, OrderType::Limit);
        assert_eq!(order.time_in_force, TimeInForce::PostOnly);
        assert_eq!(order.price, Some(dec!(50000))); // best_bid
    }

    #[test]
    fn test_stop_loss_signal_market_order() {
        let executor = Executor::new();
        let signal = make_signal(Side::Sell, -0.5, dec!(1.0));
        let order = executor.prepare_order(&signal, dec!(50000), dec!(50010), dec!(0.1));

        assert_eq!(order.order_type, OrderType::Market);
        assert_eq!(order.time_in_force, TimeInForce::IOC);
        assert_eq!(order.price, None);
    }

    #[test]
    fn test_zero_strength_is_market() {
        let executor = Executor::new();
        let signal = make_signal(Side::Sell, 0.0, dec!(1.0));
        let order = executor.prepare_order(&signal, dec!(50000), dec!(50010), dec!(0.1));

        assert_eq!(order.order_type, OrderType::Market);
        assert_eq!(order.time_in_force, TimeInForce::IOC);
    }

    #[test]
    fn test_boundary_strength_0_8() {
        let executor = Executor::new();
        let signal = make_signal(Side::Buy, 0.8, dec!(0.5));
        let order = executor.prepare_order(&signal, dec!(50000), dec!(50010), dec!(0.1));

        // 0.8 is not > 0.8, so it goes to the normal/passive branch
        assert_eq!(order.time_in_force, TimeInForce::PostOnly);
        assert_eq!(order.price, Some(dec!(50000)));
    }

    #[test]
    fn test_prepare_stop_loss() {
        let executor = Executor::new();
        let order = executor.prepare_stop_loss(
            "ETHUSDT",
            Exchange::Bybit,
            Side::Sell,
            dec!(2.0),
            dec!(3000),
        );

        assert_eq!(order.symbol, "ETHUSDT");
        assert_eq!(order.exchange, Exchange::Bybit);
        assert_eq!(order.side, Side::Sell);
        assert_eq!(order.order_type, OrderType::StopMarket);
        assert_eq!(order.time_in_force, TimeInForce::GTC);
        assert_eq!(order.price, Some(dec!(3000)));
        assert_eq!(order.quantity, dec!(2.0));
        assert!(order.reduce_only);
    }

    #[test]
    fn test_prepare_take_profit() {
        let executor = Executor::new();
        let order = executor.prepare_take_profit(
            "ETHUSDT",
            Exchange::OKX,
            Side::Sell,
            dec!(5.0),
            dec!(4000),
        );

        assert_eq!(order.symbol, "ETHUSDT");
        assert_eq!(order.exchange, Exchange::OKX);
        assert_eq!(order.side, Side::Sell);
        assert_eq!(order.order_type, OrderType::TakeProfitMarket);
        assert_eq!(order.time_in_force, TimeInForce::GTC);
        assert_eq!(order.price, Some(dec!(4000)));
        assert_eq!(order.quantity, dec!(5.0));
        assert!(order.reduce_only);
    }

    #[test]
    fn test_latency_recording() {
        let mut executor = Executor::new();
        executor.record_latency(100);
        executor.record_latency(200);
        executor.record_latency(300);

        let stats = executor.latency_stats();
        assert_eq!(stats.count(), 3);
        assert_eq!(stats.mean(), 200);
        assert_eq!(stats.min(), 100);
        assert_eq!(stats.max(), 300);
    }

    #[test]
    fn test_default_executor() {
        let executor = Executor::default();
        assert_eq!(executor.latency_stats().count(), 0);
    }
}
