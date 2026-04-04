use rust_decimal::Decimal;
use rust_decimal_macros::dec;

/// Simulated exchange for backtesting — fills orders with configurable slippage.
pub struct SimExchange {
    balance: f64,
    slippage_bps: f64,
    maker_fee_bps: f64,
    taker_fee_bps: f64,
}

/// Result of a simulated fill.
#[derive(Debug, Clone)]
pub struct SimFill {
    pub fill_price: f64,
    pub quantity: f64,
    pub fee: f64,
    pub slippage: f64,
}

impl SimExchange {
    pub fn new(initial_balance: f64, slippage_bps: f64, maker_fee_bps: f64, taker_fee_bps: f64) -> Self {
        Self {
            balance: initial_balance,
            slippage_bps,
            maker_fee_bps,
            taker_fee_bps,
        }
    }

    /// Simulate a market order fill with slippage.
    pub fn fill_market(&mut self, price: f64, quantity: f64, is_buy: bool) -> SimFill {
        let slippage_mult = self.slippage_bps / 10_000.0;
        let fill_price = if is_buy {
            price * (1.0 + slippage_mult)
        } else {
            price * (1.0 - slippage_mult)
        };

        let notional = fill_price * quantity;
        let fee = notional * (self.taker_fee_bps / 10_000.0);
        let slippage = (fill_price - price).abs() * quantity;

        SimFill {
            fill_price,
            quantity,
            fee,
            slippage,
        }
    }

    /// Simulate a limit order fill (no slippage, maker fee).
    pub fn fill_limit(&mut self, price: f64, quantity: f64) -> SimFill {
        let notional = price * quantity;
        let fee = notional * (self.maker_fee_bps / 10_000.0);

        SimFill {
            fill_price: price,
            quantity,
            fee,
            slippage: 0.0,
        }
    }

    pub fn balance(&self) -> f64 {
        self.balance
    }

    pub fn update_balance(&mut self, pnl: f64, fee: f64) {
        self.balance += pnl - fee;
    }
}

impl Default for SimExchange {
    fn default() -> Self {
        // Default: 2 bps slippage, -2 bps maker rebate, 4 bps taker fee
        Self::new(100.0, 2.0, -2.0, 4.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn market_buy_has_positive_slippage() {
        let mut sim = SimExchange::default();
        let fill = sim.fill_market(50000.0, 0.001, true);
        assert!(fill.fill_price > 50000.0);
        assert!(fill.fee > 0.0);
    }

    #[test]
    fn limit_fill_no_slippage() {
        let mut sim = SimExchange::default();
        let fill = sim.fill_limit(50000.0, 0.001);
        assert_eq!(fill.fill_price, 50000.0);
        assert_eq!(fill.slippage, 0.0);
    }

    #[test]
    fn balance_updates() {
        let mut sim = SimExchange::new(1000.0, 2.0, -2.0, 4.0);
        sim.update_balance(50.0, 2.0);
        assert_eq!(sim.balance(), 1048.0);
    }
}
