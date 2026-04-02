use bangida_core::config::RiskConfig;
use bangida_core::error::BangidaError;
use bangida_core::types::{Signal, ValidatedSignal};
use rust_decimal::Decimal;
use tracing::{info, warn};

use crate::position_sizer::PositionSizer;

/// Pre-trade risk validation gate.
///
/// Every signal must pass through the `RiskManager` before an order is placed.
/// It enforces position limits, equity requirements, per-trade risk caps,
/// and leverage constraints, then computes the appropriate position size.
pub struct RiskManager {
    config: RiskConfig,
}

impl RiskManager {
    pub fn new(config: RiskConfig) -> Self {
        Self { config }
    }

    /// Validate a trading signal against current account state and risk limits.
    ///
    /// Returns a `ValidatedSignal` with computed quantity and max loss on success,
    /// or a `BangidaError` describing why the signal was rejected.
    pub fn validate_signal(
        &self,
        signal: &Signal,
        equity: Decimal,
        open_positions: usize,
        leverage: u32,
    ) -> Result<ValidatedSignal, BangidaError> {
        // Check minimum equity
        let min_equity = Decimal::try_from(self.config.min_equity)
            .unwrap_or(Decimal::ZERO);
        if equity < min_equity {
            warn!(
                %equity,
                %min_equity,
                "equity below minimum"
            );
            return Err(BangidaError::RiskCheck(format!(
                "equity {} below minimum {}",
                equity, min_equity
            )));
        }

        // Check max open positions
        if open_positions >= self.config.max_open_positions as usize {
            warn!(
                open_positions,
                max = self.config.max_open_positions,
                "max open positions reached"
            );
            return Err(BangidaError::RiskCheck(format!(
                "max open positions reached: {}/{}",
                open_positions, self.config.max_open_positions
            )));
        }

        // Check leverage within limits
        let effective_leverage = leverage.min(self.config.max_leverage);
        if leverage > self.config.max_leverage {
            warn!(
                leverage,
                max = self.config.max_leverage,
                "leverage exceeds maximum, clamping"
            );
        }

        // Compute stop distance from signal
        let stop_distance_pct = if let Some(stop_loss) = &signal.stop_loss {
            // Estimate stop distance as a fraction of entry (using mid_price ~ last traded)
            // The signal doesn't carry entry price, so we estimate from the stop_loss field.
            // If no stop loss is set, use the max_risk_per_trade_pct as the stop distance.
            let price_estimate = *stop_loss; // approximate
            if price_estimate.is_zero() {
                self.config.max_risk_per_trade_pct
            } else {
                self.config.max_risk_per_trade_pct
            }
        } else {
            self.config.max_risk_per_trade_pct
        };

        // Check risk per trade within limits
        if self.config.max_risk_per_trade_pct <= 0.0 {
            return Err(BangidaError::RiskCheck(
                "max_risk_per_trade_pct must be positive".to_string(),
            ));
        }

        // Compute position size via fixed fractional
        let quantity = PositionSizer::fixed_fractional(
            equity,
            self.config.max_risk_per_trade_pct,
            stop_distance_pct,
        );

        if quantity.is_zero() {
            return Err(BangidaError::RiskCheck(
                "computed position size is zero".to_string(),
            ));
        }

        // Max loss for this trade
        let max_loss = equity
            * Decimal::try_from(self.config.max_risk_per_trade_pct).unwrap_or(Decimal::ZERO);

        info!(
            symbol = %signal.symbol,
            side = %signal.side,
            %quantity,
            leverage = effective_leverage,
            %max_loss,
            "signal validated"
        );

        Ok(ValidatedSignal {
            signal: signal.clone(),
            quantity,
            leverage: effective_leverage,
            max_loss,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bangida_core::types::{Side, Symbol};
    use rust_decimal_macros::dec;

    fn test_config() -> RiskConfig {
        RiskConfig {
            max_risk_per_trade_pct: 0.01,
            max_daily_loss_pct: 0.05,
            max_drawdown_pct: 0.10,
            max_consecutive_losses: 5,
            cooldown_minutes: 15,
            min_equity: 100.0,
            max_open_positions: 3,
            max_trades_per_hour: 60,
            max_leverage: 20,
        }
    }

    fn test_signal() -> Signal {
        Signal {
            symbol: Symbol::new("BTCUSDT"),
            side: Side::Buy,
            strength: 0.75,
            confidence: 0.8,
            source: "test".to_string(),
            take_profit: None,
            stop_loss: None,
            timestamp_ms: 0,
        }
    }

    #[test]
    fn test_validate_signal_ok() {
        let rm = RiskManager::new(test_config());
        let result = rm.validate_signal(&test_signal(), dec!(10000), 0, 10);
        assert!(result.is_ok());
        let vs = result.unwrap();
        assert!(vs.quantity > Decimal::ZERO);
        assert!(vs.leverage <= 10);
    }

    #[test]
    fn test_reject_low_equity() {
        let rm = RiskManager::new(test_config());
        let result = rm.validate_signal(&test_signal(), dec!(50), 0, 10);
        assert!(result.is_err());
    }

    #[test]
    fn test_reject_max_positions() {
        let rm = RiskManager::new(test_config());
        let result = rm.validate_signal(&test_signal(), dec!(10000), 3, 10);
        assert!(result.is_err());
    }
}
