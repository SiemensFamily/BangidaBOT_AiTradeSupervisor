//! SQLite persistence for the learning system.
//!
//! Tables:
//!   generations(id, ts, population_size, best_fitness, avg_fitness)
//!   candidates(id, gen_id, candidate_id, genome_json, pnl, wins, losses, fitness)

use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;

use crate::Population;

pub struct LearningDb {
    conn: Connection,
}

impl LearningDb {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS generations (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                ts              INTEGER NOT NULL,
                generation      INTEGER NOT NULL,
                population_size INTEGER NOT NULL,
                best_fitness    REAL NOT NULL,
                avg_fitness     REAL NOT NULL,
                best_pnl        REAL NOT NULL
            );
            CREATE TABLE IF NOT EXISTS candidates (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                gen_id          INTEGER NOT NULL,
                candidate_id    INTEGER NOT NULL,
                genome_json     TEXT NOT NULL,
                wins            INTEGER NOT NULL,
                losses          INTEGER NOT NULL,
                pnl             REAL NOT NULL,
                fitness         REAL NOT NULL,
                FOREIGN KEY (gen_id) REFERENCES generations(id)
            );
            CREATE INDEX IF NOT EXISTS idx_candidates_gen ON candidates(gen_id);
            CREATE INDEX IF NOT EXISTS idx_generations_ts ON generations(ts);
            "#,
        )?;
        Ok(Self { conn })
    }

    /// Persist a complete generation snapshot. Returns the generation row id.
    pub fn save_generation(&mut self, pop: &Population) -> Result<i64> {
        let ts = chrono::Utc::now().timestamp_millis();
        let best_fitness = pop.best().map(|c| c.fitness()).unwrap_or(0.0);
        let best_pnl = pop.best().map(|c| c.net_pnl).unwrap_or(0.0);
        let avg = pop.avg_fitness();

        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO generations (ts, generation, population_size, best_fitness, avg_fitness, best_pnl) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![ts, pop.generation as i64, pop.candidates.len() as i64, best_fitness, avg, best_pnl],
        )?;
        let gen_id = tx.last_insert_rowid();
        for c in &pop.candidates {
            let json = serde_json::to_string(&c.genome).unwrap_or_default();
            tx.execute(
                "INSERT INTO candidates (gen_id, candidate_id, genome_json, wins, losses, pnl, fitness) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![gen_id, c.id as i64, json, c.wins as i64, c.losses as i64, c.net_pnl, c.fitness()],
            )?;
        }
        tx.commit()?;
        Ok(gen_id)
    }

    /// Return the (timestamp_ms, best_fitness) series for charting recent
    /// generations. Limited to the last `limit` rows.
    pub fn fitness_history(&self, limit: usize) -> Result<Vec<(i64, f64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT ts, best_fitness FROM generations ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
        })?;
        let mut out: Vec<(i64, f64)> = Vec::new();
        for r in rows {
            out.push(r?);
        }
        out.reverse();
        Ok(out)
    }

    /// Return the top N candidates from the most recent generation as
    /// (candidate_id, genome_json, pnl, fitness).
    pub fn top_candidates(&self, n: usize) -> Result<Vec<(u32, String, f64, f64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT candidate_id, genome_json, pnl, fitness
             FROM candidates
             WHERE gen_id = (SELECT MAX(id) FROM generations)
             ORDER BY fitness DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![n as i64], |r| {
            Ok((
                r.get::<_, i64>(0)? as u32,
                r.get::<_, String>(1)?,
                r.get::<_, f64>(2)?,
                r.get::<_, f64>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_round_trip() {
        let tmp = std::env::temp_dir().join(format!("learning_test_{}.db", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        let mut db = LearningDb::open(&tmp).unwrap();
        let pop = Population::new(5);
        let gen_id = db.save_generation(&pop).unwrap();
        assert!(gen_id > 0);
        let history = db.fitness_history(10).unwrap();
        assert_eq!(history.len(), 1);
        let _ = std::fs::remove_file(&tmp);
    }
}
