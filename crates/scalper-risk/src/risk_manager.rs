use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use scalper_core::config::RiskConfig;
use scalper_core::types::{Signal, ValidatedSignal, VolatilityRegime};
use tracing::{info, warn};

use crate::circuit_breaker::CircuitBreaker;
use crate::pnl_tracker::PnlTracker;
use crate::position_sizer::PositionSizer;

/// Central risk manager that ties together circuit breaking, position sizing,
/// and PnL tracking for the scalping bot.
pub struct RiskManager {
    config: RiskConfig,
    circuit_breaker: CircuitBreaker,
    pnl_tracker: PnlTracker,
}

impl RiskManager {
    pub fn new(config: RiskConfig, initial_equity: f64) -> Self {
        let circuit_breaker = CircuitBreaker::new(config.clone(), initial_equity);
        let pnl_tracker = PnlTracker::new(initial_equity);
        Self {
            config,
            circuit_breaker,
            pnl_tracker,
        }
    }

    /// Validate a trading signal through the full risk pipeline.
    ///
    /// Returns `None` if the trade should be rejected (circuit breaker tripped,
    /// extreme volatility, or position too small).
    ///
    /// Returns `Some(ValidatedSignal)` with computed quantity, leverage, and max loss.
    pub fn validate_signal(
        &self,
        signal: &Signal,
        regime: VolatilityRegime,
        atr: Option<f64>,
        price: f64,
        now_ms: u64,
    ) -> Option<ValidatedSignal> {
        // 1. Check circuit breaker
        if !self.circuit_breaker.can_trade(now_ms) {
            warn!("Risk manager: circuit breaker halted trading");
            return None;
        }

        // 2. Reject trading in extreme volatility
        if regime == VolatilityRegime::Extreme {
            warn!("Risk manager: extreme volatility regime, pausing all trading");
            return None;
        }

        let equity = self.circuit_breaker.current_equity();
        let risk_pct = self.config.max_risk_per_trade_pct;

        // 3. Calculate stop distance from signal or default 0.5%
        let stop_distance_pct = if let Some(stop_loss) = &signal.stop_loss {
            let stop_f64 = stop_loss.to_string().parse::<f64>().unwrap_or(0.0);
            if price > 0.0 && stop_f64 > 0.0 {
                ((price - stop_f64).abs() / price) * 100.0
            } else {
                0.5
            }
        } else {
            0.5
        };

        // 4. Size position based on available data
        let mut quantity = if let Some(atr_val) = atr {
            PositionSizer::volatility_adjusted(equity, risk_pct, atr_val, price)
        } else {
            let notional = PositionSizer::fixed_fractional(equity, risk_pct, stop_distance_pct);
            if price > 0.0 {
                notional / price
            } else {
                0.0
            }
        };

        // 5. Regime-based risk adjustment
        match regime {
            VolatilityRegime::Volatile => {
                quantity *= 0.5;
                info!("Risk manager: volatile regime, halving position size");
            }
            VolatilityRegime::Ranging => {
                quantity *= 1.2;
                info!("Risk manager: ranging regime, increasing position size by 20%");
            }
            VolatilityRegime::Normal => {}
            VolatilityRegime::Extreme => unreachable!(), // handled above
        }

        // 6. Enforce max leverage from config
        let max_notional = equity * self.config.max_leverage as f64;
        let notional = quantity * price;
        let leverage = if notional > 0.0 && equity > 0.0 {
            let raw_leverage = (notional / equity).ceil() as u32;
            raw_leverage.min(self.config.max_leverage).max(1)
        } else {
            1
        };

        // Cap quantity to max leverage
        if notional > max_notional && price > 0.0 {
            quantity = max_notional / price;
        }

        // Calculate max loss
        let max_loss = quantity * price * (stop_distance_pct / 100.0);

        if quantity <= 0.0 {
            warn!("Risk manager: computed quantity is zero or negative");
            return None;
        }

        let quantity_dec =
            Decimal::from_f64(quantity).unwrap_or_else(|| Decimal::new(0, 0));
        let max_loss_dec =
            Decimal::from_f64(max_loss).unwrap_or_else(|| Decimal::new(0, 0));

        Some(ValidatedSignal {
            signal: signal.clone(),
            quantity: quantity_dec,
            leverage,
            max_loss: max_loss_dec,
        })
    }

    /// Record a trade result, updating both circuit breaker and PnL tracker.
    pub fn on_trade_result(&mut self, pnl: f64, fees: f64, now_ms: u64) {
        let net = pnl - fees;
        self.circuit_breaker.on_trade_result(net, now_ms);
        self.pnl_tracker.record_trade(pnl, fees);
    }

    /// Access the circuit breaker.
    pub fn circuit_breaker(&self) -> &CircuitBreaker {
        &self.circuit_breaker
    }

    /// Access the PnL tracker.
    pub fn pnl_tracker(&self) -> &PnlTracker {
        &self.pnl_tracker
    }

    /// Reset daily counters on both circuit breaker and PnL tracker.
    pub fn reset_daily(&mut self) {
        self.circuit_breaker.reset_daily();
        self.pnl_tracker.reset_daily();
    }

    /// Reset hourly trade counter on the circuit breaker.
    pub fn reset_hourly(&mut self) {
        self.circuit_breaker.reset_hourly();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use scalper_core::types::{Exchange, Side};

    fn test_config() -> RiskConfig {
        RiskConfig {
            max_risk_per_trade_pct: 1.0,
            max_daily_loss_pct: 5.0,
            max_drawdown_pct: 10.0,
            max_consecutive_losses: 3,
            cooldown_minutes: 30,
            min_equity: 50.0,
            max_open_positions: 3,
            max_trades_per_hour: 10,
            max_leverage: 20,
        }
    }

    fn test_signal() -> Signal {
        Signal {
            strategy_name: "test".to_string(),
            symbol: "BTCUSDT".to_string(),
            exchange: Exchange::Binance,
            side: Side::Buy,
            strength: 0.8,
            confidence: 0.9,
            take_profit: Some(dec!(51000)),
            stop_loss: Some(dec!(49000)),
            timestamp_ms: 1000,
        }
    }

    #[test]
    fn test_validate_signal_normal_regime() {
        let rm = RiskManager::new(test_config(), 10000.0);
        let signal = test_signal();
        let price = 50000.0;

        let result = rm.validate_signal(&signal, VolatilityRegime::Normal, None, price, 1000);
        assert!(result.is_some());

        let vs = result.unwrap();
        assert!(vs.quantity > Decimal::ZERO);
        assert!(vs.leverage >= 1);
        assert!(vs.leverage <= 20);
        assert!(vs.max_loss > Decimal::ZERO);
    }

    #[test]
    fn test_validate_signal_extreme_regime_rejected() {
        let rm = RiskManager::new(test_config(), 10000.0);
        let signal = test_signal();

        let result =
            rm.validate_signal(&signal, VolatilityRegime::Extreme, None, 50000.0, 1000);
        assert!(result.is_none());
    }

    #[test]
    fn test_validate_signal_circuit_breaker_tripped() {
        let rm = RiskManager::new(test_config(), 10000.0);
        let signal = test_signal();

        // Trip the circuit breaker with consecutive losses
        rm.circuit_breaker.on_trade_result(-100.0, 1000);
        rm.circuit_breaker.on_trade_result(-100.0, 2000);
        rm.circuit_breaker.on_trade_result(-100.0, 3000);

        let result =
            rm.validate_signal(&signal, VolatilityRegime::Normal, None, 50000.0, 3001);
        assert!(result.is_none());
    }

    #[test]
    fn test_validate_signal_volatile_regime_halves_size() {
        let rm = RiskManager::new(test_config(), 10000.0);
        let signal = test_signal();
        let price = 50000.0;

        let normal = rm
            .validate_signal(&signal, VolatilityRegime::Normal, None, price, 1000)
            .unwrap();
        let volatile = rm
            .validate_signal(&signal, VolatilityRegime::Volatile, None, price, 1000)
            .unwrap();

        // Volatile quantity should be half of normal
        let normal_f: f64 = normal.quantity.to_string().parse().unwrap();
        let volatile_f: f64 = volatile.quantity.to_string().parse().unwrap();
        assert!((volatile_f - normal_f * 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_validate_signal_ranging_regime_increases_size() {
        let rm = RiskManager::new(test_config(), 10000.0);
        let signal = test_signal();
        let price = 50000.0;

        let normal = rm
            .validate_signal(&signal, VolatilityRegime::Normal, None, price, 1000)
            .unwrap();
        let ranging = rm
            .validate_signal(&signal, VolatilityRegime::Ranging, None, price, 1000)
            .unwrap();

        let normal_f: f64 = normal.quantity.to_string().parse().unwrap();
        let ranging_f: f64 = ranging.quantity.to_string().parse().unwrap();
        assert!((ranging_f - normal_f * 1.2).abs() < 1e-6);
    }

    #[test]
    fn test_validate_signal_with_atr() {
        let rm = RiskManager::new(test_config(), 10000.0);
        let signal = test_signal();
        let price = 50000.0;
        let atr = 500.0;

        let result = rm.validate_signal(
            &signal,
            VolatilityRegime::Normal,
            Some(atr),
            price,
            1000,
        );
        assert!(result.is_some());
        let vs = result.unwrap();
        assert!(vs.quantity > Decimal::ZERO);
    }

    #[test]
    fn test_on_trade_result_updates_both() {
        let mut rm = RiskManager::new(test_config(), 10000.0);

        rm.on_trade_result(50.0, 2.0, 1000);
        // PnL tracker: net = 48, equity = 10048
        assert!((rm.pnl_tracker().equity() - 10048.0).abs() < 1e-6);
        // Circuit breaker: net = 48, equity = 10048
        assert!((rm.circuit_breaker().current_equity() - 10048.0).abs() < 1e-6);
        assert_eq!(rm.pnl_tracker().total_trades(), 1);
    }

    #[test]
    fn test_reset_daily() {
        let mut rm = RiskManager::new(test_config(), 10000.0);
        rm.on_trade_result(-50.0, 2.0, 1000);
        rm.on_trade_result(-50.0, 2.0, 2000);
        rm.on_trade_result(-50.0, 2.0, 3000);

        // Circuit breaker should be tripped
        assert!(!rm.circuit_breaker().can_trade(3001));

        rm.reset_daily();
        // Should be able to trade again
        assert!(rm.circuit_breaker().can_trade(3002));
    }

    #[test]
    fn test_reset_hourly() {
        let config = RiskConfig {
            max_trades_per_hour: 2,
            max_consecutive_losses: 100,
            ..test_config()
        };
        let mut rm = RiskManager::new(config, 10000.0);
        rm.on_trade_result(1.0, 0.0, 1000);
        rm.on_trade_result(1.0, 0.0, 2000);
        assert!(!rm.circuit_breaker().can_trade(2001));

        rm.reset_hourly();
        assert!(rm.circuit_breaker().can_trade(2002));
    }

    #[test]
    fn test_signal_without_stop_loss_uses_default() {
        let rm = RiskManager::new(test_config(), 10000.0);
        let mut signal = test_signal();
        signal.stop_loss = None;

        let result =
            rm.validate_signal(&signal, VolatilityRegime::Normal, None, 50000.0, 1000);
        assert!(result.is_some());
        // Default stop distance is 0.5%, so max_loss should reflect that
        let vs = result.unwrap();
        assert!(vs.max_loss > Decimal::ZERO);
    }
}
