//! Learning mode bridge — owns the Population and runs the evolution loop.
//!
//! The strategy engine ticks the population per evaluation cycle. A separate
//! task runs evolution every EVOLVE_INTERVAL_SECS and persists to SQLite.

use std::sync::Arc;
use tokio::sync::Mutex;

use scalper_learning::database::LearningDb;
use scalper_learning::{MarketSnapshot, Population};

/// Population size — N candidates evaluated in parallel.
pub const POPULATION_SIZE: usize = 24;
/// How often to run a generation step (seconds).
const EVOLVE_INTERVAL_SECS: u64 = 300; // 5 minutes
/// Path for the SQLite database.
pub const DB_PATH: &str = "data/learning.db";

/// Shared learning state held in main.rs and accessed by the strategy engine
/// task (for ticking) and the dashboard (for status).
pub struct LearningState {
    pub population: Population,
    pub enabled: bool,
    pub last_evolve_ms: u64,
    pub total_ticks: u64,
}

impl LearningState {
    pub fn new() -> Self {
        Self {
            population: Population::new(POPULATION_SIZE),
            enabled: true,
            last_evolve_ms: 0,
            total_ticks: 0,
        }
    }

    pub fn tick(&mut self, snap: &MarketSnapshot) {
        if !self.enabled {
            return;
        }
        self.population.tick(snap);
        self.total_ticks += 1;
    }
}

impl Default for LearningState {
    fn default() -> Self {
        Self::new()
    }
}

/// Background task — runs evolution and persists results every 5 minutes.
pub async fn run_learning_evolver(state: Arc<Mutex<LearningState>>) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(EVOLVE_INTERVAL_SECS));
    interval.tick().await; // skip first

    let db_open: Option<LearningDb> = match LearningDb::open(DB_PATH) {
        Ok(db) => Some(db),
        Err(e) => {
            tracing::error!("Failed to open learning DB at {}: {}", DB_PATH, e);
            None
        }
    };
    let db_mutex = db_open.map(|db| std::sync::Mutex::new(db));

    loop {
        interval.tick().await;

        let mut s = state.lock().await;
        if !s.enabled {
            continue;
        }
        s.population.evolve();
        s.last_evolve_ms = chrono::Utc::now().timestamp_millis() as u64;

        // Persist to SQLite if available
        if let Some(ref db) = db_mutex {
            if let Ok(mut guard) = db.lock() {
                if let Err(e) = guard.save_generation(&s.population) {
                    tracing::warn!("learning: save_generation failed: {}", e);
                } else {
                    tracing::info!(
                        "learning: generation {} saved (best_fitness={:.2})",
                        s.population.generation,
                        s.population.best().map(|c| c.fitness()).unwrap_or(0.0)
                    );
                }
            }
        }
    }
}
