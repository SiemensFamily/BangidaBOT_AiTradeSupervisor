use bangida_core::config::RiskConfig;
use bangida_core::error::BangidaError;
use parking_lot::Mutex;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use tracing::{error, info};

/// Tracks trading activity and enforces hard limits that halt trading
/// when the bot is in a losing streak or drawdown exceeds thresholds.
pub struct CircuitBreaker {
    inner: Mutex<CircuitBreakerInner>,
    config: RiskConfig,
}

struct CircuitBreakerInner {
    consecutive_losses: u32,
    daily_loss: Decimal,
    peak_equity: Decimal,
    current_equity: Decimal,
    trades_this_hour: u32,
    /// Timestamp (ms) of the last loss — used for cooldown calculation.
    last_loss_time: Option<u64>,
    is_halted: bool,
    /// Timestamp (ms) when cooldown started (after a loss streak).
    cooldown_until: Option<u64>,
}

impl CircuitBreaker {
    pub fn new(config: RiskConfig, initial_equity: Decimal) -> Self {
        Self {
            inner: Mutex::new(CircuitBreakerInner {
                consecutive_losses: 0,
                daily_loss: Decimal::ZERO,
                peak_equity: initial_equity,
                current_equity: initial_equity,
                trades_this_hour: 0,
                last_loss_time: None,
                is_halted: false,
                cooldown_until: None,
            }),
            config,
        }
    }

    /// Record the result of a completed trade. Negative `pnl` is a loss.
    pub fn on_trade_result(&self, pnl: Decimal) {
        let mut inner = self.inner.lock();
        inner.trades_this_hour += 1;

        if pnl < Decimal::ZERO {
            inner.consecutive_losses += 1;
            inner.daily_loss += pnl; // pnl is negative, daily_loss becomes more negative
            inner.last_loss_time = Some(bangida_core::time::now_ms());

            // Start cooldown after a loss streak
            if inner.consecutive_losses >= self.config.max_consecutive_losses {
                let cooldown_ms = self.config.cooldown_minutes as u64 * 60 * 1000;
                let until = bangida_core::time::now_ms() + cooldown_ms;
                inner.cooldown_until = Some(until);
                inner.is_halted = true;
                error!(
                    consecutive_losses = inner.consecutive_losses,
                    cooldown_minutes = self.config.cooldown_minutes,
                    "circuit breaker: loss streak cooldown activated"
                );
            }
        } else {
            // Reset consecutive losses on a winning trade
            inner.consecutive_losses = 0;
        }

        // Update equity
        inner.current_equity += pnl;
        if inner.current_equity > inner.peak_equity {
            inner.peak_equity = inner.current_equity;
        }
    }

    /// Check whether trading is currently permitted.
    ///
    /// Returns `Ok(())` if trading is allowed, or `Err(BangidaError::CircuitBreaker)`
    /// with a human-readable reason if trading should halt.
    pub fn can_trade(&self) -> Result<(), BangidaError> {
        let inner = self.inner.lock();

        // Check cooldown expiry
        if let Some(until) = inner.cooldown_until {
            let now = bangida_core::time::now_ms();
            if now < until {
                return Err(BangidaError::CircuitBreaker(format!(
                    "in cooldown until {}ms ({}s remaining)",
                    until,
                    (until - now) / 1000
                )));
            }
            // Cooldown expired — we'll clear halted state below if no other triggers fire
        }

        // Check consecutive losses (without cooldown having expired)
        if inner.consecutive_losses >= self.config.max_consecutive_losses {
            if let Some(until) = inner.cooldown_until {
                if bangida_core::time::now_ms() < until {
                    return Err(BangidaError::CircuitBreaker(format!(
                        "consecutive losses {} >= max {}",
                        inner.consecutive_losses, self.config.max_consecutive_losses
                    )));
                }
            }
        }

        // Check daily loss
        if !inner.daily_loss.is_zero() && !inner.peak_equity.is_zero() {
            let daily_loss_pct = (inner.daily_loss / inner.peak_equity)
                .to_f64()
                .unwrap_or(0.0)
                .abs();
            if daily_loss_pct > self.config.max_daily_loss_pct {
                return Err(BangidaError::CircuitBreaker(format!(
                    "daily loss {:.2}% exceeds max {:.2}%",
                    daily_loss_pct * 100.0,
                    self.config.max_daily_loss_pct * 100.0
                )));
            }
        }

        // Check drawdown
        if !inner.peak_equity.is_zero() {
            let drawdown = (inner.peak_equity - inner.current_equity) / inner.peak_equity;
            let drawdown_pct = drawdown.to_f64().unwrap_or(0.0);
            if drawdown_pct > self.config.max_drawdown_pct {
                return Err(BangidaError::CircuitBreaker(format!(
                    "drawdown {:.2}% exceeds max {:.2}%",
                    drawdown_pct * 100.0,
                    self.config.max_drawdown_pct * 100.0
                )));
            }
        }

        // Check minimum equity
        let min_equity = Decimal::try_from(self.config.min_equity).unwrap_or(Decimal::ZERO);
        if inner.current_equity < min_equity {
            return Err(BangidaError::CircuitBreaker(format!(
                "equity {} below minimum {}",
                inner.current_equity, min_equity
            )));
        }

        // Check trades per hour
        if inner.trades_this_hour >= self.config.max_trades_per_hour {
            return Err(BangidaError::CircuitBreaker(format!(
                "trades this hour {} >= max {}",
                inner.trades_this_hour, self.config.max_trades_per_hour
            )));
        }

        Ok(())
    }

    /// Reset daily counters. Should be called at UTC midnight.
    pub fn reset_daily(&self) {
        let mut inner = self.inner.lock();
        inner.daily_loss = Decimal::ZERO;
        inner.trades_this_hour = 0;
        inner.consecutive_losses = 0;
        inner.cooldown_until = None;
        inner.is_halted = false;
        info!("circuit breaker: daily counters reset");
    }

    /// Reset the hourly trade counter. Should be called every hour.
    pub fn reset_hourly(&self) {
        let mut inner = self.inner.lock();
        inner.trades_this_hour = 0;
    }

    /// Update equity externally (e.g., from balance updates).
    pub fn update_equity(&self, equity: Decimal) {
        let mut inner = self.inner.lock();
        inner.current_equity = equity;
        if equity > inner.peak_equity {
            inner.peak_equity = equity;
        }
    }

    /// Returns whether the circuit breaker is currently in halted state.
    pub fn is_halted(&self) -> bool {
        self.inner.lock().is_halted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn test_config() -> RiskConfig {
        RiskConfig {
            max_risk_per_trade_pct: 0.01,
            max_daily_loss_pct: 0.05,
            max_drawdown_pct: 0.10,
            max_consecutive_losses: 3,
            cooldown_minutes: 5,
            min_equity: 100.0,
            max_open_positions: 3,
            max_trades_per_hour: 60,
            max_leverage: 20,
        }
    }

    #[test]
    fn test_can_trade_initially() {
        let cb = CircuitBreaker::new(test_config(), dec!(10000));
        assert!(cb.can_trade().is_ok());
    }

    #[test]
    fn test_consecutive_losses_trigger() {
        let cb = CircuitBreaker::new(test_config(), dec!(10000));
        cb.on_trade_result(dec!(-10));
        cb.on_trade_result(dec!(-10));
        cb.on_trade_result(dec!(-10)); // 3rd loss = max
        assert!(cb.can_trade().is_err());
    }

    #[test]
    fn test_winning_trade_resets_streak() {
        let cb = CircuitBreaker::new(test_config(), dec!(10000));
        cb.on_trade_result(dec!(-10));
        cb.on_trade_result(dec!(-10));
        cb.on_trade_result(dec!(20)); // win resets streak
        assert!(cb.can_trade().is_ok());
    }

    #[test]
    fn test_low_equity_trigger() {
        let cb = CircuitBreaker::new(test_config(), dec!(150));
        // Drop below min_equity of 100
        cb.on_trade_result(dec!(-60));
        assert!(cb.can_trade().is_err());
    }

    #[test]
    fn test_reset_daily() {
        let cb = CircuitBreaker::new(test_config(), dec!(10000));
        cb.on_trade_result(dec!(-10));
        cb.on_trade_result(dec!(-10));
        cb.on_trade_result(dec!(-10));
        assert!(cb.can_trade().is_err());
        cb.reset_daily();
        assert!(cb.can_trade().is_ok());
    }
}
