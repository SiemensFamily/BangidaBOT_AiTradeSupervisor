# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Run Commands

```bash
cargo build                          # Debug build
cargo build --release                # Optimized build (thin LTO, stripped)
cargo run                            # Run bot (reads config/default.toml + mode overlay)
cargo run --bin backtest             # Offline backtesting harness
cargo run --bin backtest_sweep       # Parametric sweep optimization
cargo run -p scalper-desktop         # Launch Egui desktop GUI

cargo test                           # All workspace tests
cargo test -p scalper-data           # Single crate tests
cargo test -p scalper-backtest       # Backtest engine tests
cargo test -- test_name              # Single test by name

RUST_LOG=info cargo run              # Control log verbosity
RUST_LOG=debug,scalper_exchange=trace cargo run  # Per-crate log levels
```

## Architecture

**Event-driven pipeline** — exchange WebSocket feeds flow through a series of async tasks connected by Tokio channels:

```
Exchange WS feeds → broadcast(8192) → Data Aggregator → MarketContext snapshot
    → Ensemble Strategy Evaluator → mpsc(256) → Risk Manager → mpsc(256) → Executor → Exchange API
```

### Workspace Crates

| Crate | Purpose |
|---|---|
| `scalper-core` | Foundation types (`Signal`, `Position`, `AccountBalance`), config structs, error types |
| `scalper-data` | Technical indicators (EMA/RSI/MACD/BB/ATR/VWAP/ADX/Supertrend/etc.), orderbook, candle aggregation, regime detection |
| `scalper-exchange` | Exchange integrations — each exchange (Binance, Bybit, OKX, Kraken) has `auth.rs`, `rest.rs`, `ws.rs`, `models.rs`. Implements `MarketDataFeed` and `OrderManager` traits |
| `scalper-strategy` | 7 strategies (momentum, ob_imbalance, liquidation_wick, funding_bias, mean_reversion, donchian, ma_cross) + regime-adaptive ensemble voter in `ensemble.rs` |
| `scalper-risk` | Circuit breaker, position sizer (fixed fractional / volatility-adjusted / Kelly), PnL tracker, performance tracker |
| `scalper-execution` | Strength-based order preparation (market vs limit vs PostOnly) and order lifecycle tracking |
| `scalper-backtest` | Historical data fetching, candle replay with cost model, simulated exchange, trade report generation |
| `scalper-learning` | Evolutionary optimization with SQLite-backed population persistence |
| `scalper-desktop` | Egui GUI with dashboard, settings, and trade review tabs; connects to bot via WebSocket |

### Key Traits

- **`Strategy`** (`scalper-strategy/src/traits.rs`) — `name()`, `weight()`, `evaluate(&MarketContext) -> Option<Signal>`. All strategies implement this.
- **`MarketDataFeed`** / **`OrderManager`** (`scalper-exchange/src/lib.rs`) — Exchange abstraction for data feeds and order management.

### Main Orchestration

`src/main.rs` spawns ~10 async tasks (labeled A–J in code) coordinated through `Arc<Mutex<T>>` shared state and Tokio channels. The dashboard state struct holds all runtime metrics (equity curve, trade history, strategy votes, system metrics).

### Configuration

TOML-based with layered loading: `config/default.toml` → mode overlay (`paper.toml` or `live.toml`) → env vars (`SCALPER__SECTION__KEY`). Key sections: `[general]`, `[exchanges.*]`, `[trading]`, `[risk]`, `[strategy.*]`.

Paper mode uses exchange testnets (Binance, Bybit) with a local paper simulator (`src/paper_sim.rs`).

### Execution Logic

Order type selection is strength-based:
- Strength ≤ 0.0 → Market order (stop-loss urgency)
- Strength > 0.8 → Limit crossing spread, IOC
- 0.3–0.8 → Limit at best bid/ask, PostOnly
- < 0.3 → Limit at best bid/ask, PostOnly

### Risk Pipeline

Signals pass through: circuit breaker check → position count check → hourly trade rate check → position sizing → max loss validation. Circuit breaker halts on consecutive losses, daily loss limit, or extreme volatility regime.

## Conventions

- Financial math uses `rust_decimal::Decimal` — never use `f64` for money/prices.
- Shared mutable state pattern: `Arc<Mutex<T>>` with `parking_lot` mutexes; `DashMap` for concurrent maps.
- All exchange API auth uses HMAC-SHA256 signing (in each exchange's `auth.rs`).
- Strategies are enabled/disabled and weighted via config TOML, not code changes.
- `MarketContext` is the universal snapshot (~80 fields) passed to all strategy evaluators.
- Tests are inline `#[cfg(test)]` modules within source files, no separate `tests/` directory.
- Release profile: opt-level 3, thin LTO, codegen-units 1, stripped — optimized for latency.
