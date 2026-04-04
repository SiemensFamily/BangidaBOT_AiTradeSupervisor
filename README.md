# Crypto Scalper

Ultra-low-latency crypto futures scalping bot built in Rust. Designed for rapid growth from small accounts ($100+) with survival-first risk management.

## Features

- **4 Exchange Support**: Binance, Bybit, OKX, Kraken (futures/perpetuals)
- **4 Trading Strategies**: Momentum Breakout, Order Book Imbalance, Liquidation Wick Reversal, Funding Rate Bias
- **Regime-Adaptive Ensemble**: Dynamically adjusts strategy weights based on ATR volatility regime
- **Multi-Timeframe Confirmation**: Filters 1m entries through 5m/15m trend alignment
- **PostOnly Execution**: Captures maker rebates on every entry (critical for small accounts)
- **Circuit Breaker**: Auto-halts on consecutive losses, daily loss limits, or extreme volatility
- **Kelly + ATR Position Sizing**: Optimal geometric growth with volatility-scaled positions

## Architecture

```
┌─────────────────────┐
│  Exchange WS Feeds   │  Binance, Bybit, OKX, Kraken
└──────────┬──────────┘
           │ broadcast
     ┌─────▼─────┐
     │  Data Agg  │  OrderBook, Indicators (EMA, RSI, BB, MACD, ATR, OBV, VWAP)
     │  + Regime   │  Candles (1m/5m/15m), CVD, Liquidation Flow
     └─────┬─────┘
           │
  ┌────────▼────────┐
  │ Ensemble Strategy │  Regime-adaptive weighted voting
  │  (4 strategies)   │  Minimum 2 must agree, threshold 0.20
  └────────┬────────┘
           │ Signal
    ┌──────▼──────┐
    │ Risk Pipeline │  Circuit breaker → Position sizing → Leverage check
    └──────┬──────┘
           │ ValidatedSignal
     ┌─────▼─────┐
     │  Executor   │  PostOnly limit orders (maker rebates)
     └─────┬─────┘
           │
     ┌─────▼─────┐
     │ Exchange API │  REST order placement
     └───────────┘
```

## Quick Start

```bash
# 1. Clone and configure
cp .env.example .env
# Edit .env with your API keys

# 2. Paper trading (testnet)
cargo run

# 3. Live trading
SCALPER__GENERAL__MODE=live cargo run

# 4. Docker
docker compose up -d
```

## Configuration

Edit `config/default.toml` for base settings. Mode-specific overrides in `config/live.toml` and `config/paper.toml`. Environment variables with `SCALPER__` prefix override all.

### Risk Defaults ($100 Account)

| Parameter | Value | Reasoning |
|-----------|-------|-----------|
| Risk per trade | 3% | $3 on $100 — meaningful position size |
| Daily loss limit | 10% | $10 max — preserves capital |
| Max drawdown | 25% | Hard stop before ruin |
| Consecutive losses | 3 | Triggers 15min cooldown |
| Max leverage | 20x | Enough for small accounts |
| Max positions | 1 | Capital constraint |

## Crate Structure

| Crate | Purpose |
|-------|---------|
| `scalper-core` | Types, config, errors |
| `scalper-data` | Indicators, orderbook, regime detection |
| `scalper-exchange` | 4 exchange REST + WebSocket clients |
| `scalper-strategy` | 4 strategies + regime-adaptive ensemble |
| `scalper-risk` | Circuit breaker, position sizing, PnL tracking |
| `scalper-execution` | Order execution, tracking, latency monitoring |
| `scalper-backtest` | Backtesting engine with simulated fills |

## Testing

```bash
cargo test           # Run all tests
cargo test -p scalper-strategy  # Test specific crate
```

## Disclaimer

This software is for educational purposes only. Crypto futures trading involves substantial risk of loss. Past performance does not guarantee future results. Use at your own risk.
