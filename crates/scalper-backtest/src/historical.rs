//! Historical OHLCV data loader for Kraken Futures.
//!
//! Fetches candles from the public charts endpoint with no auth required.
//! Caches to `data/history/{symbol}_{resolution}.json` so repeat backtests
//! don't hit the network.
//!
//! Endpoint format:
//!   GET https://futures.kraken.com/api/charts/v1/trade/{symbol}/{resolution}
//!     ?from=<unix_sec>&to=<unix_sec>
//!
//! Response format (simplified):
//!   { "candles": [{"time":ms, "open":"x", "high":"x", "low":"x",
//!                  "close":"x", "volume":"x"}, ...] }

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candle {
    pub time_ms: u64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

#[derive(Debug, Deserialize)]
struct KrakenCandle {
    time: u64,
    open: serde_json::Value,
    high: serde_json::Value,
    low: serde_json::Value,
    close: serde_json::Value,
    volume: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct KrakenResponse {
    candles: Vec<KrakenCandle>,
}

fn parse_num(v: &serde_json::Value) -> f64 {
    match v {
        serde_json::Value::String(s) => s.parse().unwrap_or(0.0),
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
        _ => 0.0,
    }
}

impl From<KrakenCandle> for Candle {
    fn from(k: KrakenCandle) -> Self {
        Self {
            time_ms: k.time,
            open: parse_num(&k.open),
            high: parse_num(&k.high),
            low: parse_num(&k.low),
            close: parse_num(&k.close),
            volume: parse_num(&k.volume),
        }
    }
}

fn cache_path(symbol: &str, resolution: &str) -> PathBuf {
    PathBuf::from(format!("data/history/{}_{}.json", symbol, resolution))
}

/// Load candles from disk cache if present, otherwise fetch from Kraken.
///
/// `days` is the number of days back from now. `resolution` is Kraken's
/// resolution string: "1m", "5m", "15m", "1h", "4h", "1d".
pub async fn load_candles(symbol: &str, resolution: &str, days: u32) -> Result<Vec<Candle>> {
    let path = cache_path(symbol, resolution);
    if path.exists() {
        let bytes = std::fs::read(&path)
            .with_context(|| format!("read cache {}", path.display()))?;
        let cached: Vec<Candle> = serde_json::from_slice(&bytes)
            .with_context(|| format!("parse cache {}", path.display()))?;
        let now_ms = chrono::Utc::now().timestamp_millis() as u64;
        let cutoff_ms = now_ms.saturating_sub(days as u64 * 86_400 * 1000);
        let trimmed: Vec<Candle> = cached.into_iter().filter(|c| c.time_ms >= cutoff_ms).collect();
        if !trimmed.is_empty() {
            tracing::info!("Loaded {} cached candles from {}", trimmed.len(), path.display());
            return Ok(trimmed);
        }
    }

    tracing::info!("Fetching {} {} candles for {} days from Kraken...", symbol, resolution, days);
    let candles = fetch_kraken(symbol, resolution, days).await?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let json = serde_json::to_vec_pretty(&candles)?;
    std::fs::write(&path, json).with_context(|| format!("write cache {}", path.display()))?;
    tracing::info!("Cached {} candles to {}", candles.len(), path.display());

    Ok(candles)
}

async fn fetch_kraken(symbol: &str, resolution: &str, days: u32) -> Result<Vec<Candle>> {
    let now_s = chrono::Utc::now().timestamp();
    let from_s = now_s - days as i64 * 86_400;
    let url = format!(
        "https://futures.kraken.com/api/charts/v1/trade/{}/{}?from={}&to={}",
        symbol, resolution, from_s, now_s
    );
    tracing::debug!("GET {}", url);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let resp = client.get(&url).send().await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("Kraken returned {}: {}", status, body));
    }
    let body: KrakenResponse = resp.json().await.context("parse Kraken response")?;
    let candles: Vec<Candle> = body.candles.into_iter().map(Candle::from).collect();
    if candles.is_empty() {
        return Err(anyhow!("Kraken returned zero candles for {} {}", symbol, resolution));
    }
    Ok(candles)
}
