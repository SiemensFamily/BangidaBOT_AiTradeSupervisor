//! Historical bar-replay engine with realistic fill simulation.
//!
//! For each OHLCV bar:
//!   1. Update the indicator stack (RSI, MACD, BB, ATR, Stoch, etc.)
//!   2. Build a `MarketContext` using the bar's close + derived signals.
//!      Order book fields (imbalance_ratio, depth) are approximated since
//!      OHLCV doesn't carry book data — see caveats in `synth_ctx`.
//!   3. If a position is open, check TP/SL against the bar's high/low
//!      and the time-based exit against `max_hold_bars`. Exit wins if the
//!      bar touched the target (conservative: SL takes precedence over TP
//!      when both would trigger in the same bar — the pessimistic side).
//!   4. If no position is open, ask the ensemble for a signal. On a
//!      signal, enter at the bar's close with 2 bps slippage, set
//!      TP/SL from the signal's levels, record entry bar index.
//!
//! Fees: 0.05% per leg (Kraken Futures taker). Slippage: 2 bps per leg.
//!
//! Caveats:
//!   • Order book imbalance can't be recovered from OHLCV. We approximate
//!     it as a function of (close - midrange) / halfrange — a rough proxy
//!     for "did the bar close near its high or low". This is a weakness,
//!     but it's consistent across all backtest bars so relative performance
//!     of different parameter sets remains comparable.
//!   • The live strategy engine runs at 100ms with real bid/ask flow.
//!     The backtest runs at the bar cadence (e.g. 1m). Signals that would
//!     have fired between bars are missed. This under-counts trades but
//!     doesn't bias win rate.

use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use scalper_core::types::{Exchange, Side, Trend, VolatilityRegime};
use scalper_data::indicators::*;
use scalper_strategy::ensemble::EnsembleStrategy;
use scalper_strategy::traits::MarketContext;

use crate::historical::Candle;
use crate::report::{BacktestReport, ReportBuilder};

/// Fees per leg, in basis points (Kraken taker is 5 bps).
pub const FEE_BPS: f64 = 5.0;
/// Slippage per leg, in basis points.
pub const SLIPPAGE_BPS: f64 = 2.0;
/// Maximum bars to hold a position before forcing an exit.
pub const DEFAULT_MAX_HOLD_BARS: usize = 10;

struct OpenPosition {
    side: Side,
    entry_price: f64,
    quantity: f64,
    take_profit: f64,
    stop_loss: f64,
    entry_bar: usize,
}

/// Indicator stack — mirrors the live bot's IndicatorState but owned
/// by the backtest so it's isolated from live state.
struct IndicatorStack {
    rsi: RSI,
    ema_9: EMA,
    ema_21: EMA,
    macd: MACD,
    bb: BollingerBands,
    vwap: VWAP,
    atr: ATR,
    obv: OBV,
    stoch: Stochastic,
    stoch_rsi: StochRSI,
    cci: CCI,
    adx: ADX,
    psar: ParabolicSAR,
    supertrend: Supertrend,
    last_close: Option<f64>,
}

impl IndicatorStack {
    fn new() -> Self {
        Self {
            rsi: RSI::new(14),
            ema_9: EMA::new(9),
            ema_21: EMA::new(21),
            macd: MACD::new(12, 26, 9),
            bb: BollingerBands::new(20, 2.0),
            vwap: VWAP::new(),
            atr: ATR::new(14),
            obv: OBV::new(),
            stoch: Stochastic::new(14, 3, 3),
            stoch_rsi: StochRSI::new(14, 14),
            cci: CCI::new(20),
            adx: ADX::new(14),
            psar: ParabolicSAR::new(),
            supertrend: Supertrend::new(10, 3.0),
            last_close: None,
        }
    }

    fn update(&mut self, c: &Candle) {
        // Price-only indicators use close
        self.rsi.update(c.close);
        self.ema_9.update(c.close);
        self.ema_21.update(c.close);
        self.macd.update(c.close);
        self.bb.update(c.close);
        self.stoch_rsi.update(c.close);

        // OHLCV-based indicators
        let prev_close = self.last_close.unwrap_or(c.close);
        self.atr.update_ohlc(c.high, c.low, prev_close);
        self.obv.update_with_price(c.close, c.volume);
        self.vwap.update_with_volume(c.close, c.volume);
        self.stoch.update_ohlc(c.high, c.low, c.close);
        self.cci.update_ohlc(c.high, c.low, c.close);
        self.adx.update_ohlc(c.high, c.low, c.close);
        self.psar.update_hl(c.high, c.low);
        self.supertrend.update_ohlc(c.high, c.low, c.close, prev_close);

        self.last_close = Some(c.close);
    }

    fn is_ready(&self) -> bool {
        self.rsi.is_ready()
            && self.macd.is_ready()
            && self.bb.is_ready()
            && self.atr.is_ready()
    }
}

/// Rolling 60-bar window of highs/lows for highest_high/lowest_low.
struct HighLowWindow {
    highs: std::collections::VecDeque<f64>,
    lows: std::collections::VecDeque<f64>,
    volumes: std::collections::VecDeque<f64>,
    period: usize,
}

impl HighLowWindow {
    fn new(period: usize) -> Self {
        Self {
            highs: std::collections::VecDeque::with_capacity(period),
            lows: std::collections::VecDeque::with_capacity(period),
            volumes: std::collections::VecDeque::with_capacity(period),
            period,
        }
    }
    fn push(&mut self, h: f64, l: f64, v: f64) {
        self.highs.push_back(h);
        self.lows.push_back(l);
        self.volumes.push_back(v);
        while self.highs.len() > self.period {
            self.highs.pop_front();
            self.lows.pop_front();
            self.volumes.pop_front();
        }
    }
    fn highest(&self) -> f64 {
        self.highs.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
    }
    fn lowest(&self) -> f64 {
        self.lows.iter().cloned().fold(f64::INFINITY, f64::min)
    }
    fn avg_volume(&self) -> f64 {
        if self.volumes.is_empty() {
            return 0.0;
        }
        self.volumes.iter().sum::<f64>() / self.volumes.len() as f64
    }
}

/// Build a synthetic MarketContext from the current bar + indicator state.
///
/// Caveat: order book depth fields are zeroed, imbalance_ratio is a
/// position-in-range proxy, funding_rate/liquidation are zero.
fn synth_ctx(
    symbol: &str,
    c: &Candle,
    ind: &IndicatorStack,
    window: &HighLowWindow,
) -> MarketContext {
    let mid = c.close;
    let spread = (c.high - c.low).max(c.close * 0.0001); // at least 1 bp
    let half_spread = spread / 2.0;
    // Approximate best_bid/best_ask as close ± half spread
    let best_bid = mid - half_spread;
    let best_ask = mid + half_spread;

    // Position-in-range proxy for orderbook imbalance: if close is near
    // the high of the bar, buyers won the bar → bullish imbalance (+).
    let bar_range = (c.high - c.low).max(1e-9);
    let pos_in_range = (c.close - c.low) / bar_range; // 0..1
    let imbalance_ratio = (pos_in_range - 0.5) * 2.0; // -1..1

    let (_macd, _sig, hist) = if ind.macd.is_ready() {
        ind.macd.lines()
    } else {
        (0.0, 0.0, 0.0)
    };
    let (bb_upper, bb_mid, bb_lower) = if ind.bb.is_ready() {
        ind.bb.bands()
    } else {
        (mid, mid, mid)
    };

    let tf_trend = if ind.ema_9.value() > ind.ema_21.value() {
        Trend::Up
    } else if ind.ema_9.value() < ind.ema_21.value() {
        Trend::Down
    } else {
        Trend::Neutral
    };

    // Simple regime from ATR relative to price
    let atr_pct = if mid > 0.0 { ind.atr.value() / mid } else { 0.0 };
    let regime = if atr_pct > 0.02 {
        VolatilityRegime::Volatile
    } else if atr_pct < 0.003 {
        VolatilityRegime::Ranging
    } else {
        VolatilityRegime::Normal
    };

    MarketContext {
        symbol: symbol.to_string(),
        exchange: Exchange::Kraken,
        last_price: Decimal::from_f64(mid).unwrap_or_default(),
        best_bid: Decimal::from_f64(best_bid).unwrap_or_default(),
        best_ask: Decimal::from_f64(best_ask).unwrap_or_default(),
        spread: Decimal::from_f64(spread).unwrap_or_default(),
        tick_size: Decimal::from_f64(spread).unwrap_or_default(),
        imbalance_ratio,
        bid_depth_10: Decimal::ZERO,
        ask_depth_10: Decimal::ZERO,
        rsi_14: ind.rsi.value(),
        ema_9: ind.ema_9.value(),
        ema_21: ind.ema_21.value(),
        macd_histogram: hist,
        bollinger_upper: bb_upper,
        bollinger_lower: bb_lower,
        bollinger_middle: bb_mid,
        vwap: ind.vwap.value(),
        atr_14: ind.atr.value(),
        obv: ind.obv.value(),
        stoch_k: ind.stoch.k(),
        stoch_d: ind.stoch.d(),
        stoch_rsi: ind.stoch_rsi.value(),
        cci_20: ind.cci.value(),
        adx_14: ind.adx.value(),
        psar: ind.psar.value(),
        psar_long: ind.psar.is_long(),
        supertrend: ind.supertrend.value(),
        supertrend_up: ind.supertrend.trend_up(),
        // CVD proxy: a bullish bar (close > open) contributes positive
        // volume to CVD, a bearish bar negative. Scaled by the magnitude
        // of the bar so big moves dominate. Not perfect, but gives the
        // strategies a directional signal consistent with OHLCV data.
        cvd: if c.close > c.open {
            c.volume
        } else if c.close < c.open {
            -c.volume
        } else {
            0.0
        },
        volume_ratio: if window.avg_volume() > 0.0 {
            c.volume / window.avg_volume()
        } else {
            1.0
        },
        liquidation_volume_1m: 0.0,
        tf_5m_trend: tf_trend,
        tf_15m_trend: tf_trend,
        volatility_regime: regime,
        highest_high_60s: window.highest(),
        lowest_low_60s: window.lowest(),
        avg_volume_60s: window.avg_volume(),
        current_volume: c.volume,
        funding_rate: 0.0,
        funding_rate_secondary: 0.0,
        open_interest: None,
        price_velocity_30s: 0.0,
        timestamp_ms: c.time_ms,
    }
}

/// Replay a sequence of candles through the given ensemble and return a report.
///
/// `notional` is the dollar amount per trade (constant for the whole run —
/// we aren't using the live risk manager's equity curve because that would
/// make drawdown a function of starting balance rather than a pure
/// strategy metric).
pub fn replay(
    symbol: &str,
    candles: &[Candle],
    ensemble: &EnsembleStrategy,
    notional: f64,
    max_hold_bars: usize,
) -> BacktestReport {
    let mut ind = IndicatorStack::new();
    let mut window = HighLowWindow::new(60);
    let mut report = ReportBuilder::new(notional);
    let mut open: Option<OpenPosition> = None;

    let fee_rate = FEE_BPS / 10_000.0;
    let slip = SLIPPAGE_BPS / 10_000.0;

    for (i, c) in candles.iter().enumerate() {
        ind.update(c);
        window.push(c.high, c.low, c.volume);
        if !ind.is_ready() {
            continue;
        }

        // 1. Manage open position against this bar's high/low
        if let Some(ref pos) = open {
            let held = i - pos.entry_bar;
            let mut exit: Option<(f64, bool)> = None; // (price, is_sl)

            match pos.side {
                Side::Buy => {
                    // Pessimistic: if both SL and TP would trigger inside the
                    // bar, assume SL first (worse outcome).
                    if c.low <= pos.stop_loss {
                        exit = Some((pos.stop_loss, true));
                    } else if c.high >= pos.take_profit {
                        exit = Some((pos.take_profit, false));
                    }
                }
                Side::Sell => {
                    if c.high >= pos.stop_loss {
                        exit = Some((pos.stop_loss, true));
                    } else if c.low <= pos.take_profit {
                        exit = Some((pos.take_profit, false));
                    }
                }
            }

            if exit.is_none() && held >= max_hold_bars {
                exit = Some((c.close, false));
            }

            if let Some((exit_price, _is_sl)) = exit {
                let exit_with_slip = match pos.side {
                    Side::Buy => exit_price * (1.0 - slip),
                    Side::Sell => exit_price * (1.0 + slip),
                };
                let pnl = match pos.side {
                    Side::Buy => (exit_with_slip - pos.entry_price) * pos.quantity,
                    Side::Sell => (pos.entry_price - exit_with_slip) * pos.quantity,
                };
                let fee = (pos.entry_price + exit_with_slip) * pos.quantity * fee_rate;
                report.record_trade(pnl, fee);
                open = None;
            }
        }

        // 2. Look for a new entry if no position is open
        if open.is_none() {
            let ctx = synth_ctx(symbol, c, &ind, &window);
            if let Some(signal) = ensemble.evaluate(&ctx) {
                let entry_slipped = match signal.side {
                    Side::Buy => c.close * (1.0 + slip),
                    Side::Sell => c.close * (1.0 - slip),
                };
                let qty = notional / entry_slipped;
                let tp = signal
                    .take_profit
                    .and_then(|d| d.to_string().parse::<f64>().ok())
                    .unwrap_or_else(|| match signal.side {
                        Side::Buy => entry_slipped * 1.005,
                        Side::Sell => entry_slipped * 0.995,
                    });
                let sl = signal
                    .stop_loss
                    .and_then(|d| d.to_string().parse::<f64>().ok())
                    .unwrap_or_else(|| match signal.side {
                        Side::Buy => entry_slipped * 0.9975,
                        Side::Sell => entry_slipped * 1.0025,
                    });
                open = Some(OpenPosition {
                    side: signal.side,
                    entry_price: entry_slipped,
                    quantity: qty,
                    take_profit: tp,
                    stop_loss: sl,
                    entry_bar: i,
                });
            }
        }
    }

    // Close any remaining position at the last close
    if let Some(pos) = open {
        let last = candles.last().map(|c| c.close).unwrap_or(pos.entry_price);
        let pnl = match pos.side {
            Side::Buy => (last - pos.entry_price) * pos.quantity,
            Side::Sell => (pos.entry_price - last) * pos.quantity,
        };
        let fee = (pos.entry_price + last) * pos.quantity * fee_rate;
        report.record_trade(pnl, fee);
    }

    report.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use scalper_strategy::traits::Strategy;

    /// Build a synthetic candle series: a sine wave around a base price.
    fn sine_candles(n: usize, base: f64, amplitude: f64, period_bars: f64) -> Vec<Candle> {
        (0..n)
            .map(|i| {
                let t = (i as f64) * 2.0 * std::f64::consts::PI / period_bars;
                let mid = base + amplitude * t.sin();
                let range = amplitude * 0.1;
                Candle {
                    time_ms: 1_700_000_000_000 + (i as u64) * 60_000,
                    open: mid - range * 0.5,
                    high: mid + range,
                    low: mid - range,
                    close: mid,
                    volume: 100.0 + (i as f64 % 10.0) * 10.0,
                }
            })
            .collect()
    }

    #[test]
    fn replay_runs_without_panic_no_strategies() {
        let candles = sine_candles(200, 50_000.0, 500.0, 40.0);
        let ensemble = EnsembleStrategy::new(Vec::<Box<dyn Strategy>>::new(), 0.20);
        let report = replay("TEST", &candles, &ensemble, 5000.0, 10);
        // With no strategies, no trades should fire
        assert_eq!(report.total_trades, 0);
    }

    #[test]
    fn replay_handles_sparse_candles() {
        // Fewer than 60 candles — indicator stack won't warm up
        let candles = sine_candles(30, 50_000.0, 500.0, 10.0);
        let ensemble = EnsembleStrategy::new(Vec::<Box<dyn Strategy>>::new(), 0.20);
        let report = replay("TEST", &candles, &ensemble, 5000.0, 10);
        assert_eq!(report.total_trades, 0);
    }
}
