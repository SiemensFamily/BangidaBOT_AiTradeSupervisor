//! Historical OHLCV data loader for Kraken Futures and Binance Futures.
//!
//! Supports two exchanges via a simple `Exchange` enum and one public
//! function (`load_candles`) that routes to the right fetcher. Both fetchers
//! paginate so `days` is the actual window fetched (previously Kraken was
//! capped at ~33 hours because the charts endpoint returns at most 2000
//! candles per call).
//!
//! Kraken endpoint:
//!   GET https://futures.kraken.com/api/charts/v1/trade/{symbol}/{resolution}
//!     ?from=<unix_sec>&to=<unix_sec>
//!   Returns up to 2000 candles per call. Pagination walks forward
//!   through time in 2000-candle windows.
//!
//! Binance endpoint:
//!   GET https://fapi.binance.com/fapi/v1/klines
//!     ?symbol=BTCUSDT&interval=1m&startTime=<ms>&endTime=<ms>&limit=1500
//!   Returns array-of-arrays format. Max 1500 candles per call.
//!
//! Caches to `data/history/{exchange}_{symbol}_{resolution}.json`.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// Which exchange to fetch historical candles from. Named `Venue` to avoid
/// clashing with `scalper_core::types::Exchange`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Venue {
    Kraken,
    Binance,
}

impl Venue {
    pub fn parse(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "kraken" => Ok(Venue::Kraken),
            "binance" => Ok(Venue::Binance),
            _ => Err(anyhow!("unknown venue: {} (expected kraken or binance)", s)),
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Venue::Kraken => "kraken",
            Venue::Binance => "binance",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candle {
    pub time_ms: u64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

// ---------- Kraken ----------

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

// ---------- Shared helpers ----------

/// Resolution string → seconds per candle. Used for pagination math and
/// cache-coverage checks.
fn resolution_seconds(resolution: &str) -> Result<i64> {
    match resolution {
        "1m" => Ok(60),
        "5m" => Ok(300),
        "15m" => Ok(900),
        "30m" => Ok(1800),
        "1h" => Ok(3600),
        "4h" => Ok(14_400),
        "1d" => Ok(86_400),
        other => Err(anyhow!("unsupported resolution: {}", other)),
    }
}

/// Kraken → Binance interval string mapping (they use the same strings for
/// the ones we care about, but we centralize it so adding more is easy).
fn binance_interval(resolution: &str) -> Result<&'static str> {
    match resolution {
        "1m" => Ok("1m"),
        "5m" => Ok("5m"),
        "15m" => Ok("15m"),
        "30m" => Ok("30m"),
        "1h" => Ok("1h"),
        "4h" => Ok("4h"),
        "1d" => Ok("1d"),
        other => Err(anyhow!("unsupported resolution: {}", other)),
    }
}

fn cache_path(venue: Venue, symbol: &str, resolution: &str) -> PathBuf {
    PathBuf::from(format!(
        "data/history/{}_{}_{}.json",
        venue.as_str(),
        symbol,
        resolution
    ))
}

/// Deduplicate by timestamp and sort ascending. Kraken paginated chunks may
/// overlap at the boundaries.
fn dedupe_sort(mut candles: Vec<Candle>) -> Vec<Candle> {
    candles.sort_by_key(|c| c.time_ms);
    candles.dedup_by_key(|c| c.time_ms);
    candles
}

/// Top-level loader: tries cache first, refetches if coverage is insufficient.
///
/// `days` is the number of days back from now. `resolution` is a Kraken-style
/// resolution string: "1m", "5m", "15m", "30m", "1h", "4h", "1d".
pub async fn load_candles(symbol: &str, resolution: &str, days: u32) -> Result<Vec<Candle>> {
    load_candles_ex(Venue::Kraken, symbol, resolution, days).await
}

/// Venue-aware version. The Kraken `symbol` should be the Kraken symbol
/// (e.g., `PI_XBTUSD`); the Binance `symbol` should be the Binance symbol
/// (e.g., `BTCUSDT`).
pub async fn load_candles_ex(
    venue: Venue,
    symbol: &str,
    resolution: &str,
    days: u32,
) -> Result<Vec<Candle>> {
    let res_s = resolution_seconds(resolution)?;
    let path = cache_path(venue, symbol, resolution);

    if path.exists() {
        let bytes = std::fs::read(&path)
            .with_context(|| format!("read cache {}", path.display()))?;
        let cached: Vec<Candle> = serde_json::from_slice(&bytes)
            .with_context(|| format!("parse cache {}", path.display()))?;
        let now_ms = chrono::Utc::now().timestamp_millis() as u64;
        let cutoff_ms = now_ms.saturating_sub(days as u64 * 86_400 * 1000);
        let trimmed: Vec<Candle> = cached
            .into_iter()
            .filter(|c| c.time_ms >= cutoff_ms)
            .collect();

        // Coverage check: compare against expected candle count. A cache that
        // only has 33h of 1m data (2000 candles) when the user asks for 30d
        // (43200 candles) should be refetched. We use 70% as the threshold
        // since real data has weekend gaps, outages, etc.
        let expected = ((days as i64 * 86_400) / res_s) as usize;
        let coverage = if expected > 0 {
            (trimmed.len() * 100) / expected
        } else {
            100
        };
        if coverage >= 70 {
            tracing::info!(
                "Loaded {} cached candles from {} ({}% coverage)",
                trimmed.len(),
                path.display(),
                coverage
            );
            return Ok(trimmed);
        } else {
            tracing::info!(
                "Cache has only {}% coverage ({} candles, expected {}) — refetching",
                coverage,
                trimmed.len(),
                expected
            );
        }
    }

    tracing::info!(
        "Fetching {} {} {} candles for {} days...",
        venue.as_str(),
        symbol,
        resolution,
        days
    );
    let candles = match venue {
        Venue::Kraken => fetch_kraken(symbol, resolution, days).await?,
        Venue::Binance => fetch_binance(symbol, resolution, days).await?,
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let json = serde_json::to_vec_pretty(&candles)?;
    std::fs::write(&path, json).with_context(|| format!("write cache {}", path.display()))?;
    tracing::info!("Cached {} candles to {}", candles.len(), path.display());

    Ok(candles)
}

// ---------- Kraken fetcher (paginated) ----------

async fn fetch_kraken(symbol: &str, resolution: &str, days: u32) -> Result<Vec<Candle>> {
    let res_s = resolution_seconds(resolution)?;
    let now_s = chrono::Utc::now().timestamp();
    let start_s = now_s - days as i64 * 86_400;

    // Kraken returns at most 2000 candles per call. Step forward in chunks
    // of 1900 * resolution seconds so the windows never overflow the cap
    // even if Kraken is filling gaps generously.
    let chunk_seconds = 1900 * res_s;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    let mut all: Vec<Candle> = Vec::new();
    let mut cursor = start_s;
    let mut page = 0_usize;

    while cursor < now_s {
        let chunk_end = (cursor + chunk_seconds).min(now_s);
        let url = format!(
            "https://futures.kraken.com/api/charts/v1/trade/{}/{}?from={}&to={}",
            symbol, resolution, cursor, chunk_end
        );
        tracing::debug!("Kraken page {}: GET {}", page, url);

        let resp = client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("kraken page {} GET failed", page))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Kraken page {} returned {}: {}",
                page,
                status,
                body
            ));
        }
        let body: KrakenResponse = resp
            .json()
            .await
            .with_context(|| format!("parse Kraken page {}", page))?;
        let candles: Vec<Candle> = body.candles.into_iter().map(Candle::from).collect();

        let got = candles.len();
        tracing::debug!(
            "Kraken page {} got {} candles (cursor {} → {})",
            page,
            got,
            cursor,
            chunk_end
        );

        if got == 0 {
            // No data in this window — just advance. Happens on weekends or
            // low-liquidity symbols.
            cursor = chunk_end + 1;
            page += 1;
            continue;
        }

        // Track the last returned timestamp so we can advance past it.
        let last_ts = candles.last().map(|c| c.time_ms).unwrap_or(0) as i64 / 1000;
        all.extend(candles);

        // Advance cursor. If Kraken gave us a full chunk, jump past it; if
        // it gave us less (near present or sparse data), step just past the
        // last candle we got so we don't loop forever.
        let next_cursor = (last_ts + res_s).max(chunk_end + 1);
        if next_cursor <= cursor {
            // Defensive: Kraken returned stale data — bail to avoid infinite loop
            tracing::warn!(
                "Kraken page {} didn't advance (cursor {} → {}), stopping",
                page,
                cursor,
                next_cursor
            );
            break;
        }
        cursor = next_cursor;
        page += 1;

        // Be polite — Kraken has rate limits.
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Safety valve: hard cap on pages to prevent runaway.
        if page > 500 {
            tracing::warn!("Kraken pagination hit 500-page safety cap, stopping");
            break;
        }
    }

    let all = dedupe_sort(all);
    if all.is_empty() {
        return Err(anyhow!(
            "Kraken returned zero candles for {} {} across {} pages",
            symbol,
            resolution,
            page
        ));
    }
    tracing::info!(
        "Kraken fetch complete: {} candles across {} pages",
        all.len(),
        page
    );
    Ok(all)
}

// ---------- Binance fetcher (paginated) ----------

async fn fetch_binance(symbol: &str, resolution: &str, days: u32) -> Result<Vec<Candle>> {
    let interval = binance_interval(resolution)?;
    let res_s = resolution_seconds(resolution)?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let start_ms = now_ms - (days as i64) * 86_400 * 1000;

    // Binance futures klines: max 1500 per call. Use 1400 for safety.
    let limit = 1400_i64;
    let chunk_ms = limit * res_s * 1000;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    let mut all: Vec<Candle> = Vec::new();
    let mut cursor_ms = start_ms;
    let mut page = 0_usize;

    while cursor_ms < now_ms {
        let end_ms = (cursor_ms + chunk_ms).min(now_ms);
        let url = format!(
            "https://fapi.binance.com/fapi/v1/klines?symbol={}&interval={}&startTime={}&endTime={}&limit={}",
            symbol, interval, cursor_ms, end_ms, limit
        );
        tracing::debug!("Binance page {}: GET {}", page, url);

        let resp = client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("binance page {} GET failed", page))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Binance page {} returned {}: {}",
                page,
                status,
                body
            ));
        }

        // Binance returns [[openTime, open, high, low, close, volume, closeTime, ...], ...]
        let raw: Vec<Vec<serde_json::Value>> = resp
            .json()
            .await
            .with_context(|| format!("parse Binance page {}", page))?;

        let got = raw.len();
        tracing::debug!(
            "Binance page {} got {} candles (cursor {} → {})",
            page,
            got,
            cursor_ms,
            end_ms
        );

        if got == 0 {
            cursor_ms = end_ms + 1;
            page += 1;
            continue;
        }

        let mut last_open_ms: i64 = 0;
        for row in raw {
            if row.len() < 6 {
                continue;
            }
            let open_ms = row[0].as_i64().unwrap_or(0);
            let open = parse_num(&row[1]);
            let high = parse_num(&row[2]);
            let low = parse_num(&row[3]);
            let close = parse_num(&row[4]);
            let volume = parse_num(&row[5]);
            last_open_ms = open_ms;
            all.push(Candle {
                time_ms: open_ms as u64,
                open,
                high,
                low,
                close,
                volume,
            });
        }

        // Advance past the last candle's open time by one resolution.
        let next_cursor = (last_open_ms + res_s * 1000).max(end_ms + 1);
        if next_cursor <= cursor_ms {
            tracing::warn!(
                "Binance page {} didn't advance (cursor {} → {}), stopping",
                page,
                cursor_ms,
                next_cursor
            );
            break;
        }
        cursor_ms = next_cursor;
        page += 1;

        // Binance allows 2400 weight/min on public klines — 150ms is safe.
        tokio::time::sleep(Duration::from_millis(150)).await;

        if page > 500 {
            tracing::warn!("Binance pagination hit 500-page safety cap, stopping");
            break;
        }
    }

    let all = dedupe_sort(all);
    if all.is_empty() {
        return Err(anyhow!(
            "Binance returned zero candles for {} {} across {} pages",
            symbol,
            resolution,
            page
        ));
    }
    tracing::info!(
        "Binance fetch complete: {} candles across {} pages",
        all.len(),
        page
    );
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_seconds_known_values() {
        assert_eq!(resolution_seconds("1m").unwrap(), 60);
        assert_eq!(resolution_seconds("5m").unwrap(), 300);
        assert_eq!(resolution_seconds("15m").unwrap(), 900);
        assert_eq!(resolution_seconds("1h").unwrap(), 3600);
        assert_eq!(resolution_seconds("4h").unwrap(), 14_400);
        assert_eq!(resolution_seconds("1d").unwrap(), 86_400);
        assert!(resolution_seconds("bogus").is_err());
    }

    #[test]
    fn venue_parse() {
        assert_eq!(Venue::parse("kraken").unwrap(), Venue::Kraken);
        assert_eq!(Venue::parse("KRAKEN").unwrap(), Venue::Kraken);
        assert_eq!(Venue::parse("binance").unwrap(), Venue::Binance);
        assert!(Venue::parse("gdax").is_err());
    }

    #[test]
    fn dedupe_sort_removes_duplicates() {
        let candles = vec![
            Candle { time_ms: 3000, open: 3.0, high: 3.0, low: 3.0, close: 3.0, volume: 1.0 },
            Candle { time_ms: 1000, open: 1.0, high: 1.0, low: 1.0, close: 1.0, volume: 1.0 },
            Candle { time_ms: 2000, open: 2.0, high: 2.0, low: 2.0, close: 2.0, volume: 1.0 },
            Candle { time_ms: 1000, open: 1.0, high: 1.0, low: 1.0, close: 1.0, volume: 1.0 },
        ];
        let sorted = dedupe_sort(candles);
        assert_eq!(sorted.len(), 3);
        assert_eq!(sorted[0].time_ms, 1000);
        assert_eq!(sorted[1].time_ms, 2000);
        assert_eq!(sorted[2].time_ms, 3000);
    }
}
