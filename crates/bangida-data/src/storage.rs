use anyhow::{Context, Result};
use rust_decimal::Decimal;
use rusqlite::{params, Connection};
use tracing::{debug, info};

use bangida_core::{Exchange, Side};

// ---------------------------------------------------------------------------
// Database
// ---------------------------------------------------------------------------

/// Lightweight SQLite wrapper for the trade journal and equity snapshots.
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Open (or create) the database at `path` and run migrations.
    pub fn new(path: &str) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open database at {path}"))?;

        // Performance pragmas suitable for a local journal.
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous  = NORMAL;
             PRAGMA foreign_keys = ON;",
        )?;

        let db = Self { conn };
        db.run_migrations()?;
        info!("database initialized at {path}");
        Ok(db)
    }

    /// Create an in-memory database (useful for tests).
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self { conn };
        db.run_migrations()?;
        Ok(db)
    }

    // ------------------------------------------------------------------
    // Migrations
    // ------------------------------------------------------------------

    fn run_migrations(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS trades (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp_ms    INTEGER NOT NULL,
                exchange        TEXT    NOT NULL,
                symbol          TEXT    NOT NULL,
                side            TEXT    NOT NULL,
                entry_price     TEXT    NOT NULL,
                exit_price      TEXT,
                quantity        TEXT    NOT NULL,
                leverage        INTEGER NOT NULL DEFAULT 1,
                pnl             TEXT,
                fees            TEXT,
                strategy        TEXT    NOT NULL DEFAULT '',
                hold_time_ms    INTEGER,
                status          TEXT    NOT NULL DEFAULT 'OPEN'
            );

            CREATE TABLE IF NOT EXISTS equity_snapshots (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp_ms    INTEGER NOT NULL,
                equity          TEXT    NOT NULL,
                unrealized_pnl  TEXT    NOT NULL DEFAULT '0',
                drawdown_pct    REAL    NOT NULL DEFAULT 0.0
            );

            CREATE INDEX IF NOT EXISTS idx_trades_timestamp
                ON trades(timestamp_ms);
            CREATE INDEX IF NOT EXISTS idx_trades_symbol
                ON trades(symbol);
            CREATE INDEX IF NOT EXISTS idx_equity_timestamp
                ON equity_snapshots(timestamp_ms);",
        )?;
        debug!("database migrations applied");
        Ok(())
    }

    // ------------------------------------------------------------------
    // Trade journal
    // ------------------------------------------------------------------

    /// Insert a new trade record. Returns the row id.
    pub fn record_trade(
        &self,
        timestamp_ms: u64,
        exchange: &Exchange,
        symbol: &str,
        side: &Side,
        entry_price: Decimal,
        quantity: Decimal,
        leverage: u32,
        strategy: &str,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO trades (timestamp_ms, exchange, symbol, side, entry_price, quantity, leverage, strategy)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                timestamp_ms as i64,
                exchange.to_string(),
                symbol,
                side.to_string(),
                entry_price.to_string(),
                quantity.to_string(),
                leverage,
                strategy,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Update a trade with exit information.
    pub fn update_trade_exit(
        &self,
        trade_id: i64,
        exit_price: Decimal,
        pnl: Decimal,
        fees: Decimal,
        hold_time_ms: u64,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE trades SET exit_price = ?1, pnl = ?2, fees = ?3, hold_time_ms = ?4, status = 'CLOSED'
             WHERE id = ?5",
            params![
                exit_price.to_string(),
                pnl.to_string(),
                fees.to_string(),
                hold_time_ms as i64,
                trade_id,
            ],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Equity snapshots
    // ------------------------------------------------------------------

    /// Record an equity snapshot.
    pub fn record_equity_snapshot(
        &self,
        timestamp_ms: u64,
        equity: Decimal,
        unrealized_pnl: Decimal,
        drawdown_pct: f64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO equity_snapshots (timestamp_ms, equity, unrealized_pnl, drawdown_pct)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                timestamp_ms as i64,
                equity.to_string(),
                unrealized_pnl.to_string(),
                drawdown_pct,
            ],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Queries
    // ------------------------------------------------------------------

    /// Fetch the most recent `limit` trades (newest first).
    pub fn get_recent_trades(&self, limit: u32) -> Result<Vec<TradeRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, timestamp_ms, exchange, symbol, side, entry_price, exit_price,
                    quantity, leverage, pnl, fees, strategy, hold_time_ms, status
             FROM trades ORDER BY timestamp_ms DESC LIMIT ?1",
        )?;

        let rows = stmt.query_map(params![limit], |row| {
            Ok(TradeRow {
                id: row.get(0)?,
                timestamp_ms: row.get::<_, i64>(1)? as u64,
                exchange: row.get(2)?,
                symbol: row.get(3)?,
                side: row.get(4)?,
                entry_price: row.get(5)?,
                exit_price: row.get(6)?,
                quantity: row.get(7)?,
                leverage: row.get(8)?,
                pnl: row.get(9)?,
                fees: row.get(10)?,
                strategy: row.get(11)?,
                hold_time_ms: row.get::<_, Option<i64>>(12)?.map(|v| v as u64),
                status: row.get(13)?,
            })
        })?;

        let mut trades = Vec::new();
        for row in rows {
            trades.push(row?);
        }
        Ok(trades)
    }

    /// Sum of PnL for trades closed today (UTC day boundary based on
    /// `timestamp_ms`). Returns `Decimal::ZERO` if there are no closed trades
    /// today.
    pub fn get_daily_pnl(&self) -> Result<Decimal> {
        let now_ms = chrono::Utc::now().timestamp_millis() as u64;
        // Midnight UTC today.
        let day_start_ms = now_ms - (now_ms % 86_400_000);

        let result: Option<String> = self.conn.query_row(
            "SELECT COALESCE(SUM(CAST(pnl AS REAL)), 0) FROM trades
             WHERE status = 'CLOSED' AND timestamp_ms >= ?1",
            params![day_start_ms as i64],
            |row| row.get(0),
        )?;

        match result {
            Some(s) => Ok(s.parse::<Decimal>().unwrap_or(Decimal::ZERO)),
            None => Ok(Decimal::ZERO),
        }
    }
}

// ---------------------------------------------------------------------------
// Row types
// ---------------------------------------------------------------------------

/// A row from the `trades` table.
#[derive(Debug, Clone)]
pub struct TradeRow {
    pub id: i64,
    pub timestamp_ms: u64,
    pub exchange: String,
    pub symbol: String,
    pub side: String,
    pub entry_price: String,
    pub exit_price: Option<String>,
    pub quantity: String,
    pub leverage: u32,
    pub pnl: Option<String>,
    pub fees: Option<String>,
    pub strategy: String,
    pub hold_time_ms: Option<u64>,
    pub status: String,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn test_db() -> Database {
        Database::in_memory().expect("in-memory db")
    }

    #[test]
    fn record_and_query_trade() {
        let db = test_db();
        let id = db
            .record_trade(
                1_700_000_000_000,
                &Exchange::Binance,
                "BTCUSDT",
                &Side::Buy,
                dec!(42000),
                dec!(0.01),
                10,
                "scalp_v1",
            )
            .unwrap();
        assert!(id > 0);

        db.update_trade_exit(id, dec!(42100), dec!(10), dec!(0.84), 5_000)
            .unwrap();

        let trades = db.get_recent_trades(10).unwrap();
        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].status, "CLOSED");
        assert_eq!(trades[0].exit_price, Some("42100".to_string()));
    }

    #[test]
    fn equity_snapshot() {
        let db = test_db();
        db.record_equity_snapshot(1_700_000_000_000, dec!(10000), dec!(50), 1.5)
            .unwrap();
        // Just verify it doesn't panic; full query coverage is out of scope.
    }
}
