use bangida_core::types::{
    OrderId, OrderRequest, OrderResponse, OrderStatus, OrderType, Price, Quantity, Side, Symbol,
};
use rust_decimal::Decimal;
use tracing::debug;

/// A simulated open order on the simulated exchange.
#[derive(Debug, Clone)]
struct SimOrder {
    _order_id: OrderId,
    symbol: Symbol,
    side: Side,
    order_type: OrderType,
    price: Option<Price>,
    quantity: Quantity,
    stop_price: Option<Price>,
    _timestamp_ms: u64,
}

/// A simulated open position.
#[derive(Debug, Clone)]
struct SimPosition {
    symbol: Symbol,
    side: Side,
    quantity: Quantity,
    entry_price: Price,
}

/// Simulated exchange for backtesting.
///
/// Fills limit orders when price crosses (not touches) the order price.
/// Market orders fill immediately with configurable slippage.
pub struct SimulatedExchange {
    balance: Decimal,
    fee_rate: Decimal,
    current_price: Price,
    open_orders: Vec<SimOrder>,
    positions: Vec<SimPosition>,
    next_order_id: u64,
    /// Slippage in ticks for market orders (applied to the current price).
    slippage_ticks: Decimal,
}

impl SimulatedExchange {
    pub fn new(initial_balance: Decimal, fee_rate: Decimal) -> Self {
        Self {
            balance: initial_balance,
            fee_rate,
            current_price: Decimal::ZERO,
            open_orders: Vec::new(),
            positions: Vec::new(),
            next_order_id: 1,
            slippage_ticks: Decimal::new(1, 1), // 0.1 default slippage
        }
    }

    /// Set slippage in price units for market orders.
    pub fn set_slippage(&mut self, slippage: Decimal) {
        self.slippage_ticks = slippage;
    }

    /// Update the current market price (called on each tick/trade).
    pub fn update_price(&mut self, price: Price) {
        self.current_price = price;
    }

    /// Current market price.
    pub fn current_price(&self) -> Price {
        self.current_price
    }

    /// Number of open positions.
    pub fn open_position_count(&self) -> usize {
        self.positions.len()
    }

    /// Current balance (cash, excluding unrealized PnL).
    pub fn balance(&self) -> Decimal {
        self.balance
    }

    /// Place an order on the simulated exchange.
    ///
    /// Market orders fill immediately. Limit orders are queued.
    pub fn place_order(
        &mut self,
        req: OrderRequest,
        current_price: Price,
    ) -> OrderResponse {
        let order_id = format!("sim_{}", self.next_order_id);
        self.next_order_id += 1;

        match req.order_type {
            OrderType::Market => {
                // Fill immediately with slippage
                let fill_price = match req.side {
                    Side::Buy => current_price + self.slippage_ticks,
                    Side::Sell => current_price - self.slippage_ticks,
                };
                let fee = req.quantity * fill_price * self.fee_rate;

                // Update balance and positions
                match req.side {
                    Side::Buy => {
                        let cost = req.quantity * fill_price + fee;
                        self.balance -= cost;
                        self.add_position(req.symbol.clone(), Side::Buy, req.quantity, fill_price);
                    }
                    Side::Sell => {
                        let proceeds = req.quantity * fill_price - fee;
                        self.balance += proceeds;
                        self.close_position(&req.symbol, req.quantity, fill_price);
                    }
                }

                debug!(
                    %order_id,
                    %fill_price,
                    %fee,
                    "market order filled"
                );

                OrderResponse {
                    order_id,
                    client_order_id: String::new(),
                    symbol: req.symbol,
                    side: req.side,
                    order_type: req.order_type,
                    quantity: req.quantity,
                    price: Some(fill_price),
                    status: OrderStatus::Filled,
                    timestamp_ms: bangida_core::time::now_ms(),
                }
            }
            OrderType::Limit => {
                // Queue the limit order
                self.open_orders.push(SimOrder {
                    _order_id: order_id.clone(),
                    symbol: req.symbol.clone(),
                    side: req.side,
                    order_type: req.order_type,
                    price: req.price,
                    quantity: req.quantity,
                    stop_price: req.stop_price,
                    _timestamp_ms: bangida_core::time::now_ms(),
                });

                debug!(%order_id, price = ?req.price, "limit order queued");

                OrderResponse {
                    order_id,
                    client_order_id: String::new(),
                    symbol: req.symbol,
                    side: req.side,
                    order_type: req.order_type,
                    quantity: req.quantity,
                    price: req.price,
                    status: OrderStatus::New,
                    timestamp_ms: bangida_core::time::now_ms(),
                }
            }
            OrderType::StopMarket => {
                // Queue as a stop order
                self.open_orders.push(SimOrder {
                    _order_id: order_id.clone(),
                    symbol: req.symbol.clone(),
                    side: req.side,
                    order_type: req.order_type,
                    price: None,
                    quantity: req.quantity,
                    stop_price: req.stop_price,
                    _timestamp_ms: bangida_core::time::now_ms(),
                });

                OrderResponse {
                    order_id,
                    client_order_id: String::new(),
                    symbol: req.symbol,
                    side: req.side,
                    order_type: req.order_type,
                    quantity: req.quantity,
                    price: None,
                    status: OrderStatus::New,
                    timestamp_ms: bangida_core::time::now_ms(),
                }
            }
            OrderType::TakeProfitMarket => {
                self.open_orders.push(SimOrder {
                    _order_id: order_id.clone(),
                    symbol: req.symbol.clone(),
                    side: req.side,
                    order_type: req.order_type,
                    price: None,
                    quantity: req.quantity,
                    stop_price: req.stop_price,
                    _timestamp_ms: bangida_core::time::now_ms(),
                });

                OrderResponse {
                    order_id,
                    client_order_id: String::new(),
                    symbol: req.symbol,
                    side: req.side,
                    order_type: req.order_type,
                    quantity: req.quantity,
                    price: None,
                    status: OrderStatus::New,
                    timestamp_ms: bangida_core::time::now_ms(),
                }
            }
        }
    }

    /// Check if any open orders should fill at the given market price.
    ///
    /// Limit orders fill when price crosses (strictly passes through) the order price.
    /// Returns a vec of (pnl, fees) for each fill.
    pub fn check_fills(&mut self, market_price: Price) -> Vec<(Decimal, Decimal)> {
        // First pass: collect fill information without mutating self
        struct FillInfo {
            index: usize,
            symbol: Symbol,
            side: Side,
            quantity: Quantity,
            fill_price: Price,
            fee: Decimal,
        }

        let mut fills_to_apply: Vec<FillInfo> = Vec::new();

        for (i, order) in self.open_orders.iter().enumerate() {
            let should_fill = match order.order_type {
                OrderType::Limit => {
                    if let Some(order_price) = order.price {
                        match order.side {
                            Side::Buy => market_price < order_price,
                            Side::Sell => market_price > order_price,
                        }
                    } else {
                        false
                    }
                }
                OrderType::StopMarket => {
                    if let Some(stop_price) = order.stop_price {
                        match order.side {
                            Side::Buy => market_price > stop_price,
                            Side::Sell => market_price < stop_price,
                        }
                    } else {
                        false
                    }
                }
                OrderType::TakeProfitMarket => {
                    if let Some(stop_price) = order.stop_price {
                        match order.side {
                            Side::Buy => market_price < stop_price,
                            Side::Sell => market_price > stop_price,
                        }
                    } else {
                        false
                    }
                }
                _ => false,
            };

            if should_fill {
                let fill_price = match order.order_type {
                    OrderType::Limit => order.price.unwrap_or(market_price),
                    _ => market_price,
                };
                let fee = order.quantity * fill_price * self.fee_rate;

                fills_to_apply.push(FillInfo {
                    index: i,
                    symbol: order.symbol.clone(),
                    side: order.side,
                    quantity: order.quantity,
                    fill_price,
                    fee,
                });
            }
        }

        // Second pass: apply fills and collect results
        let mut results = Vec::new();
        let mut indices_to_remove: Vec<usize> = Vec::new();

        for fill in &fills_to_apply {
            let pnl = self.calculate_fill_pnl(&fill.symbol, fill.side, fill.quantity, fill.fill_price);

            match fill.side {
                Side::Buy => {
                    let cost = fill.quantity * fill.fill_price + fill.fee;
                    self.balance -= cost;
                    self.add_position(fill.symbol.clone(), Side::Buy, fill.quantity, fill.fill_price);
                }
                Side::Sell => {
                    let proceeds = fill.quantity * fill.fill_price - fill.fee;
                    self.balance += proceeds;
                    self.close_position(&fill.symbol, fill.quantity, fill.fill_price);
                }
            }

            debug!(
                %fill.fill_price,
                %fill.fee,
                %pnl,
                "simulated fill"
            );

            results.push((pnl, fill.fee));
            indices_to_remove.push(fill.index);
        }

        // Remove filled orders in reverse order to preserve indices
        for i in indices_to_remove.into_iter().rev() {
            self.open_orders.remove(i);
        }

        results
    }

    fn add_position(&mut self, symbol: Symbol, side: Side, quantity: Quantity, price: Price) {
        // Check if we already have a position in this symbol
        if let Some(pos) = self.positions.iter_mut().find(|p| p.symbol == symbol && p.side == side) {
            // Average into existing position
            let total_qty = pos.quantity + quantity;
            if !total_qty.is_zero() {
                pos.entry_price = (pos.entry_price * pos.quantity + price * quantity) / total_qty;
            }
            pos.quantity = total_qty;
        } else {
            self.positions.push(SimPosition {
                symbol,
                side,
                quantity,
                entry_price: price,
            });
        }
    }

    fn close_position(&mut self, symbol: &Symbol, quantity: Quantity, _price: Price) {
        self.positions.retain(|p| {
            if p.symbol == *symbol {
                // Simplified: remove if fully closed
                p.quantity > quantity
            } else {
                true
            }
        });
    }

    fn calculate_fill_pnl(
        &self,
        symbol: &Symbol,
        side: Side,
        quantity: Quantity,
        fill_price: Price,
    ) -> Decimal {
        // PnL only on position-closing trades
        if side == Side::Sell {
            if let Some(pos) = self.positions.iter().find(|p| p.symbol == *symbol && p.side == Side::Buy) {
                let pnl = (fill_price - pos.entry_price) * quantity;
                return pnl;
            }
        } else if side == Side::Buy {
            if let Some(pos) = self.positions.iter().find(|p| p.symbol == *symbol && p.side == Side::Sell) {
                let pnl = (pos.entry_price - fill_price) * quantity;
                return pnl;
            }
        }
        Decimal::ZERO
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bangida_core::types::{OrderType, TimeInForce};
    use rust_decimal_macros::dec;

    #[test]
    fn test_market_order_buy() {
        let mut exchange = SimulatedExchange::new(dec!(10000), dec!(0.0004));
        exchange.update_price(dec!(50000));

        let req = OrderRequest {
            symbol: Symbol::new("BTCUSDT"),
            side: Side::Buy,
            order_type: OrderType::Market,
            quantity: dec!(0.1),
            price: None,
            stop_price: None,
            time_in_force: TimeInForce::Ioc,
            reduce_only: false,
        };

        let resp = exchange.place_order(req, dec!(50000));
        assert_eq!(resp.status, OrderStatus::Filled);
        assert!(exchange.balance() < dec!(10000));
        assert_eq!(exchange.open_position_count(), 1);
    }

    #[test]
    fn test_limit_order_queued() {
        let mut exchange = SimulatedExchange::new(dec!(10000), dec!(0.0004));
        exchange.update_price(dec!(50000));

        let req = OrderRequest {
            symbol: Symbol::new("BTCUSDT"),
            side: Side::Buy,
            order_type: OrderType::Limit,
            quantity: dec!(0.1),
            price: Some(dec!(49900)),
            stop_price: None,
            time_in_force: TimeInForce::Gtc,
            reduce_only: false,
        };

        let resp = exchange.place_order(req, dec!(50000));
        assert_eq!(resp.status, OrderStatus::New);
    }

    #[test]
    fn test_limit_order_fills_on_cross() {
        let mut exchange = SimulatedExchange::new(dec!(10000), dec!(0.0004));
        exchange.update_price(dec!(50000));

        let req = OrderRequest {
            symbol: Symbol::new("BTCUSDT"),
            side: Side::Buy,
            order_type: OrderType::Limit,
            quantity: dec!(0.1),
            price: Some(dec!(49900)),
            stop_price: None,
            time_in_force: TimeInForce::Gtc,
            reduce_only: false,
        };

        exchange.place_order(req, dec!(50000));

        // Price touches but doesn't cross - should NOT fill
        let fills = exchange.check_fills(dec!(49900));
        assert!(fills.is_empty());

        // Price crosses below - should fill
        let fills = exchange.check_fills(dec!(49800));
        assert_eq!(fills.len(), 1);
    }
}
