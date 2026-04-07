//! Heuristic auto-tuner for strategy parameters.
//!
//! Periodically analyzes recent trade history and adjusts config to improve
//! profitability. Writes changes to the shared config and persists to disk
//! at config/default.toml. Also writes a dedicated log at logs/auto_tuner.log.
//!
//! Strategies are constructed once at startup, so config changes only take
//! effect after the bot is restarted.

use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, RwLock};

use scalper_core::config::ScalperConfig;

use crate::dashboard::{ConsoleLog, TradeRecord};

const TUNE_INTERVAL_SECS: u64 = 300;
const MIN_TRADES_FOR_TUNING: usize = 5;
const RECENT_TRADE_WINDOW: usize = 20;
const LOG_PATH: &str = "logs/auto_tuner.log";

/// Public state shared with the dashboard so the UI can show whether the
/// auto-tuner has run, when it last ran, and what it last did.
#[derive(Debug, Default, Clone)]
pub struct AutoTunerState {
    pub last_run_ms: u64,
    pub total_runs: u64,
    pub total_changes: u64,
    pub last_summary: String,
    pub last_changes: Vec<String>,
}

#[derive(Debug, Default)]
struct TradeMetrics {
    total: usize,
    wins: usize,
    losses: usize,
    win_rate: f64,
    profit_factor: f64,
    r_multiple: f64,
    net_pnl: f64,
}

fn compute_metrics(trades: &[TradeRecord]) -> TradeMetrics {
    let mut total_win_pnl = 0.0;
    let mut total_loss_pnl = 0.0;
    let mut wins = 0usize;
    let mut losses = 0usize;
    for t in trades {
        if t.pnl > 0.0 {
            wins += 1;
            total_win_pnl += t.pnl;
        } else if t.pnl < 0.0 {
            losses += 1;
            total_loss_pnl += t.pnl.abs();
        }
    }
    let total = trades.len();
    TradeMetrics {
        total,
        wins,
        losses,
        win_rate: if total > 0 { wins as f64 / total as f64 } else { 0.0 },
        profit_factor: if total_loss_pnl > 0.0 { total_win_pnl / total_loss_pnl } else { 0.0 },
        r_multiple: if losses > 0 && total_loss_pnl > 0.0 {
            (total_win_pnl / wins.max(1) as f64) / (total_loss_pnl / losses as f64)
        } else { 0.0 },
        net_pnl: total_win_pnl - total_loss_pnl,
    }
}

/// Apply heuristic tuning rules. Returns a list of human-readable change descriptions.
fn apply_rules(cfg: &mut ScalperConfig, metrics: &TradeMetrics) -> Vec<String> {
    let mut changes = Vec::new();

    // R-multiple too low → widen TP
    if metrics.r_multiple < 1.0 && metrics.losses >= 3 {
        let ob = &mut cfg.strategy.ob_imbalance;
        if ob.take_profit_ticks < 20 {
            ob.take_profit_ticks += 1;
            changes.push(format!(
                "ob_imbalance.take_profit_ticks → {} (R-multiple {:.2} < 1.0)",
                ob.take_profit_ticks, metrics.r_multiple
            ));
        }
    }

    // R-multiple very good → can tighten SL slightly
    if metrics.r_multiple > 2.5 && metrics.wins >= 5 {
        let ob = &mut cfg.strategy.ob_imbalance;
        if ob.stop_loss_ticks > 3 {
            ob.stop_loss_ticks -= 1;
            changes.push(format!(
                "ob_imbalance.stop_loss_ticks → {} (R-multiple {:.2} > 2.5)",
                ob.stop_loss_ticks, metrics.r_multiple
            ));
        }
    }

    // Win rate too low → raise imbalance threshold (more selective)
    if metrics.win_rate < 0.30 && metrics.total >= 10 {
        let ob = &mut cfg.strategy.ob_imbalance;
        if ob.imbalance_threshold < 0.70 {
            ob.imbalance_threshold = (ob.imbalance_threshold + 0.05).min(0.70);
            changes.push(format!(
                "ob_imbalance.imbalance_threshold → {:.2} (win rate {:.0}%)",
                ob.imbalance_threshold,
                metrics.win_rate * 100.0
            ));
        }
    }

    // Win rate high → can be slightly more aggressive
    if metrics.win_rate > 0.65 && metrics.total >= 10 {
        let ob = &mut cfg.strategy.ob_imbalance;
        if ob.imbalance_threshold > 0.30 {
            ob.imbalance_threshold = (ob.imbalance_threshold - 0.02).max(0.30);
            changes.push(format!(
                "ob_imbalance.imbalance_threshold → {:.2} (win rate {:.0}%)",
                ob.imbalance_threshold,
                metrics.win_rate * 100.0
            ));
        }
    }

    // Profit factor poor → reduce risk
    if metrics.profit_factor < 0.6 && metrics.total >= 10 {
        if cfg.risk.max_risk_per_trade_pct > 0.5 {
            cfg.risk.max_risk_per_trade_pct =
                (cfg.risk.max_risk_per_trade_pct - 0.25).max(0.5);
            changes.push(format!(
                "risk.max_risk_per_trade_pct → {:.2}% (profit factor {:.2})",
                cfg.risk.max_risk_per_trade_pct, metrics.profit_factor
            ));
        }
    }

    // Profit factor good → size up gently
    if metrics.profit_factor > 1.5 && metrics.total >= 10 {
        if cfg.risk.max_risk_per_trade_pct < 2.0 {
            cfg.risk.max_risk_per_trade_pct =
                (cfg.risk.max_risk_per_trade_pct + 0.1).min(2.0);
            changes.push(format!(
                "risk.max_risk_per_trade_pct → {:.2}% (profit factor {:.2})",
                cfg.risk.max_risk_per_trade_pct, metrics.profit_factor
            ));
        }
    }

    changes
}

async fn persist_config(cfg: &ScalperConfig, path: &str) -> anyhow::Result<()> {
    let toml_str = toml::to_string_pretty(cfg)?;
    tokio::fs::write(path, toml_str).await?;
    Ok(())
}

/// Append a line to the auto-tuner log file. Creates the parent directory
/// and the file if they don't exist. Logged separately from the general
/// console log so the user can review tuning history independently.
async fn append_log(line: &str) -> anyhow::Result<()> {
    if let Some(parent) = std::path::Path::new(LOG_PATH).parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(LOG_PATH)
        .await?;
    let ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    file.write_all(format!("{} {}\n", ts, line).as_bytes()).await?;
    Ok(())
}

pub async fn run_auto_tuner(
    config: Arc<RwLock<ScalperConfig>>,
    trade_history: Arc<Mutex<Vec<TradeRecord>>>,
    console_log: Arc<Mutex<ConsoleLog>>,
    state: Arc<Mutex<AutoTunerState>>,
) {
    // Initial heartbeat to the log file so the user can verify the agent
    // is alive even before any tuning cycle has run.
    let _ = append_log("startup auto-tuner task started").await;

    let mut interval =
        tokio::time::interval(tokio::time::Duration::from_secs(TUNE_INTERVAL_SECS));
    // Skip the first immediate tick — let the bot collect data first
    interval.tick().await;

    loop {
        interval.tick().await;
        let now_ms = chrono::Utc::now().timestamp_millis() as u64;

        // Compute metrics inside the lock scope to avoid cloning trades
        let metrics_result: Result<TradeMetrics, usize> = {
            let history = trade_history.lock().await;
            let n = history.len().min(RECENT_TRADE_WINDOW);
            let slice = &history[history.len() - n..];
            if slice.len() < MIN_TRADES_FOR_TUNING {
                Err(slice.len())
            } else {
                Ok(compute_metrics(slice))
            }
        };

        let metrics = match metrics_result {
            Ok(m) => m,
            Err(count) => {
                let _ = append_log(&format!(
                    "skip insufficient_trades count={} need={}",
                    count, MIN_TRADES_FOR_TUNING
                ))
                .await;
                console_log.lock().await.push(format!(
                    "Auto-tuner: skipping (only {} recent trades, need {})",
                    count, MIN_TRADES_FOR_TUNING
                ));
                let mut s = state.lock().await;
                s.last_run_ms = now_ms;
                s.total_runs += 1;
                s.last_summary = format!("skip: {}/{} trades", count, MIN_TRADES_FOR_TUNING);
                s.last_changes.clear();
                continue;
            }
        };

        let mut new_cfg = config.read().await.clone();
        let changes = apply_rules(&mut new_cfg, &metrics);

        let persist_err = if changes.is_empty() {
            None
        } else {
            *config.write().await = new_cfg;
            persist_config(&*config.read().await, "config/default.toml")
                .await
                .err()
                .map(|e| e.to_string())
        };

        let summary = format!(
            "n={} wins={} losses={} win_rate={:.0}% R={:.2} PF={:.2} net=${:.2}",
            metrics.total,
            metrics.wins,
            metrics.losses,
            metrics.win_rate * 100.0,
            metrics.r_multiple,
            metrics.profit_factor,
            metrics.net_pnl,
        );

        // Write to dedicated log file (one line per event)
        let _ = append_log(&format!("metrics {}", summary)).await;
        for c in &changes {
            let _ = append_log(&format!("change {}", c)).await;
        }
        if let Some(ref e) = persist_err {
            let _ = append_log(&format!("persist_failed {}", e)).await;
        }
        if changes.is_empty() {
            let _ = append_log("no_changes").await;
        }

        // Update shared dashboard state
        {
            let mut s = state.lock().await;
            s.last_run_ms = now_ms;
            s.total_runs += 1;
            s.total_changes += changes.len() as u64;
            s.last_summary = summary.clone();
            s.last_changes = changes.clone();
        }

        // Mirror to console_log (batched in a single lock acquisition)
        let mut log = console_log.lock().await;
        log.push(format!("Auto-tuner: {}", summary));
        if changes.is_empty() {
            log.push("Auto-tuner: no changes (metrics within target range)".to_string());
        } else {
            for c in &changes {
                log.push(format!("Auto-tuner: {}", c));
            }
            if let Some(e) = persist_err {
                log.push(format!("Auto-tuner: persist failed: {}", e));
            }
        }
    }
}
