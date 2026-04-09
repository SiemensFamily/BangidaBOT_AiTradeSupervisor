//! Genetic-algorithm parameter optimizer for strategy parameters.
//!
//! Each Candidate carries a Genome of trade parameters. Candidates run a
//! simplified shadow strategy fed by live MarketSnapshots — they don't
//! actually execute trades, they record what would have happened.
//!
//! Every K seconds the Population evolves: top half survives, bottom half
//! is replaced via mutation/crossover.
//!
//! Persistence is handled via SQLite (see `database` module).

pub mod database;

use rand::prelude::*;
use serde::{Deserialize, Serialize};

/// A parameter set that the genetic algorithm evolves.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Genome {
    pub imbalance_threshold: f64,    // 0.20 .. 0.80
    pub take_profit_ticks: u32,      // 3 .. 20
    pub stop_loss_ticks: u32,        // 3 .. 15
    pub rsi_oversold: f64,           // 15 .. 35
    pub rsi_overbought: f64,         // 65 .. 85
    pub adx_min: f64,                // 0 .. 30 (only trade when ADX > this)
    pub use_supertrend_filter: bool, // require supertrend agreement
    pub max_hold_secs: u32,          // 60 .. 900
}

impl Genome {
    pub fn random(rng: &mut impl Rng) -> Self {
        Self {
            imbalance_threshold: rng.gen_range(0.20..0.80),
            take_profit_ticks: rng.gen_range(3..=20),
            stop_loss_ticks: rng.gen_range(3..=15),
            rsi_oversold: rng.gen_range(15.0..35.0),
            rsi_overbought: rng.gen_range(65.0..85.0),
            adx_min: rng.gen_range(0.0..30.0),
            use_supertrend_filter: rng.gen_bool(0.5),
            max_hold_secs: rng.gen_range(60..=900),
        }
    }

    /// Average two parents (with random tie-break for booleans).
    pub fn crossover(a: &Self, b: &Self, rng: &mut impl Rng) -> Self {
        Self {
            imbalance_threshold: (a.imbalance_threshold + b.imbalance_threshold) / 2.0,
            take_profit_ticks: ((a.take_profit_ticks + b.take_profit_ticks) / 2).max(3),
            stop_loss_ticks: ((a.stop_loss_ticks + b.stop_loss_ticks) / 2).max(3),
            rsi_oversold: (a.rsi_oversold + b.rsi_oversold) / 2.0,
            rsi_overbought: (a.rsi_overbought + b.rsi_overbought) / 2.0,
            adx_min: (a.adx_min + b.adx_min) / 2.0,
            use_supertrend_filter: if rng.gen_bool(0.5) { a.use_supertrend_filter } else { b.use_supertrend_filter },
            max_hold_secs: (a.max_hold_secs + b.max_hold_secs) / 2,
        }
    }

    /// 10% chance per gene to perturb with gaussian noise (clamped to range).
    pub fn mutate(&mut self, rng: &mut impl Rng) {
        if rng.gen_bool(0.10) { self.imbalance_threshold = (self.imbalance_threshold + rng.gen_range(-0.05..0.05)).clamp(0.20, 0.80); }
        if rng.gen_bool(0.10) { self.take_profit_ticks = (self.take_profit_ticks as i32 + rng.gen_range(-2..=2)).clamp(3, 20) as u32; }
        if rng.gen_bool(0.10) { self.stop_loss_ticks = (self.stop_loss_ticks as i32 + rng.gen_range(-2..=2)).clamp(3, 15) as u32; }
        if rng.gen_bool(0.10) { self.rsi_oversold = (self.rsi_oversold + rng.gen_range(-3.0..3.0)).clamp(15.0, 35.0); }
        if rng.gen_bool(0.10) { self.rsi_overbought = (self.rsi_overbought + rng.gen_range(-3.0..3.0)).clamp(65.0, 85.0); }
        if rng.gen_bool(0.10) { self.adx_min = (self.adx_min + rng.gen_range(-3.0..3.0)).clamp(0.0, 30.0); }
        if rng.gen_bool(0.05) { self.use_supertrend_filter = !self.use_supertrend_filter; }
        if rng.gen_bool(0.10) { self.max_hold_secs = (self.max_hold_secs as i32 + rng.gen_range(-60..=60)).clamp(60, 900) as u32; }
    }
}

/// A simplified live snapshot of the market that the candidates evaluate against.
/// Built once per strategy engine tick from the existing MarketContext.
#[derive(Debug, Clone)]
pub struct MarketSnapshot {
    pub timestamp_ms: u64,
    pub mid_price: f64,
    pub spread: f64,
    pub imbalance_ratio: f64,
    pub rsi_14: f64,
    pub adx_14: f64,
    pub supertrend_up: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Side {
    Long,
    Short,
}

#[derive(Debug, Clone)]
struct OpenSimPosition {
    side: Side,
    entry_price: f64,
    entry_ms: u64,
    take_profit: f64,
    stop_loss: f64,
}

/// One candidate strategy under evaluation.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub id: u32,
    pub genome: Genome,
    pub age_ticks: u64,
    pub wins: u32,
    pub losses: u32,
    pub gross_profit: f64,
    pub gross_loss: f64,
    pub net_pnl: f64,
    pub max_drawdown: f64,
    peak_pnl: f64,
    open: Option<OpenSimPosition>,
}

impl Candidate {
    pub fn new(id: u32, genome: Genome) -> Self {
        Self {
            id,
            genome,
            age_ticks: 0,
            wins: 0,
            losses: 0,
            gross_profit: 0.0,
            gross_loss: 0.0,
            net_pnl: 0.0,
            max_drawdown: 0.0,
            peak_pnl: 0.0,
            open: None,
        }
    }

    /// Evaluate one tick. May open or close a simulated position.
    pub fn tick(&mut self, snap: &MarketSnapshot) {
        self.age_ticks += 1;

        // 1. Manage open position first (TP/SL/time exit)
        if let Some(ref pos) = self.open.clone() {
            let mut exit: Option<(f64, &'static str)> = None;
            match pos.side {
                Side::Long => {
                    if snap.mid_price >= pos.take_profit {
                        exit = Some((pos.take_profit, "TP"));
                    } else if snap.mid_price <= pos.stop_loss {
                        exit = Some((pos.stop_loss, "SL"));
                    }
                }
                Side::Short => {
                    if snap.mid_price <= pos.take_profit {
                        exit = Some((pos.take_profit, "TP"));
                    } else if snap.mid_price >= pos.stop_loss {
                        exit = Some((pos.stop_loss, "SL"));
                    }
                }
            }
            // Time exit
            if exit.is_none() {
                let age_ms = snap.timestamp_ms.saturating_sub(pos.entry_ms);
                if age_ms >= (self.genome.max_hold_secs as u64) * 1000 {
                    exit = Some((snap.mid_price, "TIME"));
                }
            }
            if let Some((exit_px, _reason)) = exit {
                let pnl = match pos.side {
                    Side::Long => exit_px - pos.entry_price,
                    Side::Short => pos.entry_price - exit_px,
                };
                self.net_pnl += pnl;
                if pnl > 0.0 {
                    self.wins += 1;
                    self.gross_profit += pnl;
                } else {
                    self.losses += 1;
                    self.gross_loss += pnl.abs();
                }
                if self.net_pnl > self.peak_pnl {
                    self.peak_pnl = self.net_pnl;
                }
                let dd = self.peak_pnl - self.net_pnl;
                if dd > self.max_drawdown {
                    self.max_drawdown = dd;
                }
                self.open = None;
            }
        }

        // 2. Open new position if entry conditions match
        if self.open.is_none() {
            // Skip if ADX too low
            if snap.adx_14 < self.genome.adx_min {
                return;
            }
            let tick_size = snap.spread.max(0.5);
            let tp_offset = self.genome.take_profit_ticks as f64 * tick_size;
            let sl_offset = self.genome.stop_loss_ticks as f64 * tick_size;

            // Long: imbalance > threshold AND RSI not overbought
            if snap.imbalance_ratio > self.genome.imbalance_threshold
                && snap.rsi_14 < self.genome.rsi_overbought
                && (!self.genome.use_supertrend_filter || snap.supertrend_up)
            {
                self.open = Some(OpenSimPosition {
                    side: Side::Long,
                    entry_price: snap.mid_price,
                    entry_ms: snap.timestamp_ms,
                    take_profit: snap.mid_price + tp_offset,
                    stop_loss: snap.mid_price - sl_offset,
                });
            }
            // Short: imbalance < -threshold AND RSI not oversold
            else if snap.imbalance_ratio < -self.genome.imbalance_threshold
                && snap.rsi_14 > self.genome.rsi_oversold
                && (!self.genome.use_supertrend_filter || !snap.supertrend_up)
            {
                self.open = Some(OpenSimPosition {
                    side: Side::Short,
                    entry_price: snap.mid_price,
                    entry_ms: snap.timestamp_ms,
                    take_profit: snap.mid_price - tp_offset,
                    stop_loss: snap.mid_price + sl_offset,
                });
            }
        }
    }

    pub fn total_trades(&self) -> u32 {
        self.wins + self.losses
    }

    pub fn win_rate(&self) -> f64 {
        let n = self.total_trades();
        if n == 0 { 0.0 } else { self.wins as f64 / n as f64 }
    }

    pub fn profit_factor(&self) -> f64 {
        if self.gross_loss > 0.0 {
            self.gross_profit / self.gross_loss
        } else if self.gross_profit > 0.0 {
            999.0
        } else {
            0.0
        }
    }

    /// Composite fitness used for ranking. Rewards profit factor and trade
    /// count, penalizes large drawdowns. Returns 0 if too few trades.
    pub fn fitness(&self) -> f64 {
        let n = self.total_trades();
        if n < 5 {
            return 0.0;
        }
        let pf = self.profit_factor().min(5.0);
        let trade_bonus = (n as f64).sqrt();
        let dd_penalty = if self.max_drawdown > 0.0 && self.peak_pnl > 0.0 {
            (1.0 - (self.max_drawdown / (self.peak_pnl + 1.0))).max(0.1)
        } else {
            1.0
        };
        pf * trade_bonus * dd_penalty
    }

    /// Reset metrics — used when a candidate is replaced or after evolution.
    pub fn reset_metrics(&mut self) {
        self.age_ticks = 0;
        self.wins = 0;
        self.losses = 0;
        self.gross_profit = 0.0;
        self.gross_loss = 0.0;
        self.net_pnl = 0.0;
        self.max_drawdown = 0.0;
        self.peak_pnl = 0.0;
        self.open = None;
    }
}

/// Population of candidates managed by the genetic algorithm.
pub struct Population {
    pub generation: u64,
    pub candidates: Vec<Candidate>,
    next_id: u32,
}

impl Population {
    pub fn new(size: usize) -> Self {
        let mut rng = thread_rng();
        let mut candidates = Vec::with_capacity(size);
        for i in 0..size {
            candidates.push(Candidate::new(i as u32, Genome::random(&mut rng)));
        }
        Self {
            generation: 0,
            candidates,
            next_id: size as u32,
        }
    }

    pub fn tick(&mut self, snap: &MarketSnapshot) {
        for c in &mut self.candidates {
            c.tick(snap);
        }
    }

    /// Run one evolution step: keep the top half, replace the bottom half via
    /// tournament-selected crossover + mutation. Bumps the generation counter.
    pub fn evolve(&mut self) {
        let mut rng = thread_rng();
        // Sort descending by fitness
        self.candidates.sort_by(|a, b| b.fitness().partial_cmp(&a.fitness()).unwrap_or(std::cmp::Ordering::Equal));

        let size = self.candidates.len();
        let keep = size / 2;
        // Replace bottom half
        for i in keep..size {
            // Tournament selection of 3 from the survivors
            let parents: Vec<&Candidate> = (0..3)
                .map(|_| &self.candidates[rng.gen_range(0..keep)])
                .collect();
            let p1 = parents.iter().max_by(|a, b| a.fitness().partial_cmp(&b.fitness()).unwrap_or(std::cmp::Ordering::Equal)).unwrap();
            let parents2: Vec<&Candidate> = (0..3)
                .map(|_| &self.candidates[rng.gen_range(0..keep)])
                .collect();
            let p2 = parents2.iter().max_by(|a, b| a.fitness().partial_cmp(&b.fitness()).unwrap_or(std::cmp::Ordering::Equal)).unwrap();
            let mut child_genome = Genome::crossover(&p1.genome, &p2.genome, &mut rng);
            child_genome.mutate(&mut rng);
            let id = self.next_id;
            self.next_id += 1;
            self.candidates[i] = Candidate::new(id, child_genome);
        }
        // Reset metrics on the survivors so the next generation evaluates from
        // a fresh slate (otherwise old fitness drowns out new market regimes)
        for c in &mut self.candidates[..keep] {
            c.reset_metrics();
        }
        self.generation += 1;
    }

    pub fn best(&self) -> Option<&Candidate> {
        self.candidates
            .iter()
            .max_by(|a, b| a.fitness().partial_cmp(&b.fitness()).unwrap_or(std::cmp::Ordering::Equal))
    }

    pub fn avg_fitness(&self) -> f64 {
        if self.candidates.is_empty() { return 0.0; }
        self.candidates.iter().map(|c| c.fitness()).sum::<f64>() / self.candidates.len() as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn population_evolves() {
        let mut pop = Population::new(20);
        let snap = MarketSnapshot {
            timestamp_ms: 1000,
            mid_price: 50000.0,
            spread: 5.0,
            imbalance_ratio: 0.6,
            rsi_14: 50.0,
            adx_14: 30.0,
            supertrend_up: true,
        };
        // Tick a bunch to build some fitness
        for i in 0..100 {
            let mut s = snap.clone();
            s.timestamp_ms = 1000 + i * 1000;
            s.mid_price = 50000.0 + (i as f64).sin() * 100.0;
            pop.tick(&s);
        }
        let g0 = pop.generation;
        pop.evolve();
        assert_eq!(pop.generation, g0 + 1);
    }

    #[test]
    fn genome_mutate_stays_in_bounds() {
        let mut rng = thread_rng();
        let mut g = Genome::random(&mut rng);
        for _ in 0..1000 {
            g.mutate(&mut rng);
            assert!(g.imbalance_threshold >= 0.20 && g.imbalance_threshold <= 0.80);
            assert!(g.take_profit_ticks >= 3 && g.take_profit_ticks <= 20);
            assert!(g.stop_loss_ticks >= 3 && g.stop_loss_ticks <= 15);
            assert!(g.rsi_oversold >= 15.0 && g.rsi_oversold <= 35.0);
            assert!(g.rsi_overbought >= 65.0 && g.rsi_overbought <= 85.0);
            assert!(g.adx_min >= 0.0 && g.adx_min <= 30.0);
            assert!(g.max_hold_secs >= 60 && g.max_hold_secs <= 900);
        }
    }
}
