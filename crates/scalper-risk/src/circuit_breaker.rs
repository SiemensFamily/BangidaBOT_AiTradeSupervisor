use parking_lot::Mutex;
use scalper_core::config::RiskConfig;
use tracing::warn;

pub struct CircuitBreaker {
    inner: Mutex<CircuitBreakerInner>,
}

struct CircuitBreakerInner {
    consecutive_losses: u32,
    daily_loss: f64,
    peak_equity: f64,
    current_equity: f64,
    trades_this_hour: u32,
    cooldown_until_ms: u64,
    config: RiskConfig,
}

impl CircuitBreaker {
    pub fn new(config: RiskConfig, initial_equity: f64) -> Self {
        Self {
            inner: Mutex::new(CircuitBreakerInner {
                consecutive_losses: 0,
                daily_loss: 0.0,
                peak_equity: initial_equity,
                current_equity: initial_equity,
                trades_this_hour: 0,
                cooldown_until_ms: 0,
                config,
            }),
        }
    }

    /// Returns true if trading is allowed right now.
    /// Returns false if ANY circuit breaker condition is violated.
    pub fn can_trade(&self, now_ms: u64) -> bool {
        let inner = self.inner.lock();

        if inner.consecutive_losses >= inner.config.max_consecutive_losses {
            warn!(
                consecutive_losses = inner.consecutive_losses,
                "Circuit breaker: max consecutive losses reached"
            );
            return false;
        }

        let daily_loss_pct = if inner.peak_equity > 0.0 {
            inner.daily_loss / inner.peak_equity * 100.0
        } else {
            0.0
        };
        if daily_loss_pct >= inner.config.max_daily_loss_pct {
            warn!(daily_loss_pct, "Circuit breaker: max daily loss reached");
            return false;
        }

        let drawdown_pct = if inner.peak_equity > 0.0 {
            (inner.peak_equity - inner.current_equity) / inner.peak_equity * 100.0
        } else {
            0.0
        };
        if drawdown_pct >= inner.config.max_drawdown_pct {
            warn!(drawdown_pct, "Circuit breaker: max drawdown reached");
            return false;
        }

        if inner.current_equity < inner.config.min_equity {
            warn!(
                equity = inner.current_equity,
                min = inner.config.min_equity,
                "Circuit breaker: equity below minimum"
            );
            return false;
        }

        if inner.trades_this_hour >= inner.config.max_trades_per_hour {
            warn!(
                trades = inner.trades_this_hour,
                "Circuit breaker: max trades per hour reached"
            );
            return false;
        }

        if now_ms < inner.cooldown_until_ms {
            warn!(
                cooldown_until_ms = inner.cooldown_until_ms,
                "Circuit breaker: in cooldown period"
            );
            return false;
        }

        true
    }

    /// Record a trade result and update circuit breaker state.
    pub fn on_trade_result(&self, pnl: f64, now_ms: u64) {
        let mut inner = self.inner.lock();

        if pnl < 0.0 {
            inner.consecutive_losses += 1;
            inner.daily_loss += pnl.abs();

            if inner.consecutive_losses >= inner.config.max_consecutive_losses {
                let cooldown_ms =
                    inner.config.cooldown_minutes as u64 * 60 * 1000;
                inner.cooldown_until_ms = now_ms + cooldown_ms;
                warn!(
                    cooldown_until_ms = inner.cooldown_until_ms,
                    "Circuit breaker: cooldown activated"
                );
            }
        } else {
            inner.consecutive_losses = 0;
        }

        inner.current_equity += pnl;
        if inner.current_equity > inner.peak_equity {
            inner.peak_equity = inner.current_equity;
        }
        inner.trades_this_hour += 1;
    }

    /// Reset daily counters (call at start of each trading day).
    pub fn reset_daily(&self) {
        let mut inner = self.inner.lock();
        inner.daily_loss = 0.0;
        inner.consecutive_losses = 0;
        inner.cooldown_until_ms = 0;
        inner.trades_this_hour = 0;
    }

    /// Reset hourly trade counter.
    pub fn reset_hourly(&self) {
        let mut inner = self.inner.lock();
        inner.trades_this_hour = 0;
    }

    /// Get current equity.
    pub fn current_equity(&self) -> f64 {
        self.inner.lock().current_equity
    }

    /// Get current drawdown percentage from peak.
    pub fn drawdown_pct(&self) -> f64 {
        let inner = self.inner.lock();
        if inner.peak_equity > 0.0 {
            (inner.peak_equity - inner.current_equity) / inner.peak_equity * 100.0
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_normal_operation() {
        let cb = CircuitBreaker::new(test_config(), 1000.0);
        assert!(cb.can_trade(0));
        assert_eq!(cb.current_equity(), 1000.0);
        assert_eq!(cb.drawdown_pct(), 0.0);

        // Record a winning trade
        cb.on_trade_result(10.0, 100);
        assert!(cb.can_trade(200));
        assert_eq!(cb.current_equity(), 1010.0);
    }

    #[test]
    fn test_consecutive_losses_trigger_cooldown() {
        let cb = CircuitBreaker::new(test_config(), 1000.0);

        // 3 consecutive losses should trigger cooldown
        cb.on_trade_result(-5.0, 1000);
        assert!(cb.can_trade(1001));
        cb.on_trade_result(-5.0, 2000);
        assert!(cb.can_trade(2001));
        cb.on_trade_result(-5.0, 3000);

        // Now at 3 consecutive losses -- can_trade should be false
        assert!(!cb.can_trade(3001));

        // Cooldown is 30 minutes = 1_800_000 ms from now_ms=3000
        let cooldown_end = 3000 + 30 * 60 * 1000;
        assert!(!cb.can_trade(cooldown_end - 1));
        // A win resets consecutive_losses but cooldown_until_ms is still set
        // After cooldown expires, if we reset daily, can trade again
        cb.reset_daily();
        assert!(cb.can_trade(cooldown_end));
    }

    #[test]
    fn test_win_resets_consecutive_losses() {
        let cb = CircuitBreaker::new(test_config(), 1000.0);

        cb.on_trade_result(-5.0, 1000);
        cb.on_trade_result(-5.0, 2000);
        // 2 consecutive losses, then a win
        cb.on_trade_result(10.0, 3000);
        // Should reset consecutive losses to 0
        cb.on_trade_result(-5.0, 4000);
        cb.on_trade_result(-5.0, 5000);
        // Only 2 consecutive losses again, should still be tradeable
        assert!(cb.can_trade(5001));
    }

    #[test]
    fn test_daily_loss_limit() {
        let cb = CircuitBreaker::new(test_config(), 1000.0);

        // Max daily loss is 5% of peak = 50.0
        // Record losses that push daily_loss to >= 50
        cb.on_trade_result(-20.0, 1000);
        cb.on_trade_result(1.0, 2000); // reset consecutive
        cb.on_trade_result(-20.0, 3000);
        cb.on_trade_result(1.0, 4000);
        cb.on_trade_result(-15.0, 5000); // total daily_loss = 55

        // daily_loss_pct = 55/1002 * 100 ~= 5.49% >= 5%
        assert!(!cb.can_trade(5001));
    }

    #[test]
    fn test_min_equity_check() {
        let config = RiskConfig {
            min_equity: 900.0,
            max_consecutive_losses: 100, // high to not trigger
            ..test_config()
        };
        let cb = CircuitBreaker::new(config, 1000.0);

        cb.on_trade_result(-101.0, 1000);
        // equity = 899, below min_equity 900
        assert!(!cb.can_trade(1001));
    }

    #[test]
    fn test_max_trades_per_hour() {
        let config = RiskConfig {
            max_trades_per_hour: 2,
            max_consecutive_losses: 100,
            ..test_config()
        };
        let cb = CircuitBreaker::new(config, 1000.0);

        cb.on_trade_result(1.0, 1000);
        cb.on_trade_result(1.0, 2000);
        // 2 trades recorded, max is 2
        assert!(!cb.can_trade(2001));

        cb.reset_hourly();
        assert!(cb.can_trade(2002));
    }

    #[test]
    fn test_drawdown_limit() {
        let config = RiskConfig {
            max_drawdown_pct: 10.0,
            max_consecutive_losses: 100,
            min_equity: 0.0,
            ..test_config()
        };
        let cb = CircuitBreaker::new(config, 1000.0);

        // Peak is 1000, lose 100 => drawdown = 10%
        cb.on_trade_result(-100.0, 1000);
        assert!(!cb.can_trade(1001));
    }

    #[test]
    fn test_reset_daily() {
        let cb = CircuitBreaker::new(test_config(), 1000.0);

        cb.on_trade_result(-5.0, 1000);
        cb.on_trade_result(-5.0, 2000);
        cb.on_trade_result(-5.0, 3000);
        assert!(!cb.can_trade(3001));

        cb.reset_daily();
        assert!(cb.can_trade(3002));
    }
}
