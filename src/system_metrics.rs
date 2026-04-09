//! Process-level system metrics for the dashboard "HUD" overlay.
//!
//! Sampled every 2 seconds by a background task. Cheap — sysinfo only
//! refreshes the current process via Pid filtering.

use std::sync::Arc;
use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Default)]
pub struct SystemMetrics {
    pub cpu_percent: f32,
    pub memory_mb: f64,
    pub virtual_memory_mb: f64,
    pub uptime_secs: u64,
    /// Number of active dashboard WebSocket subscribers.
    pub dashboard_subscribers: usize,
    /// Number of active market-event broadcast subscribers (data aggregator etc.).
    pub market_subscribers: usize,
}

pub type RxCounter = Arc<dyn Fn() -> usize + Send + Sync>;

pub async fn run_metrics_sampler(
    state: Arc<Mutex<SystemMetrics>>,
    dashboard_rx_count: RxCounter,
    market_rx_count: RxCounter,
    start_ms: u64,
) {
    let pid = Pid::from_u32(std::process::id());
    let mut sys = System::new_with_specifics(
        RefreshKind::new().with_processes(ProcessRefreshKind::new().with_cpu().with_memory()),
    );
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(2));
    loop {
        interval.tick().await;
        sys.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::new().with_cpu().with_memory(),
        );
        let (cpu, mem, vmem) = if let Some(p) = sys.process(pid) {
            (
                p.cpu_usage(),
                p.memory() as f64 / 1024.0 / 1024.0,
                p.virtual_memory() as f64 / 1024.0 / 1024.0,
            )
        } else {
            (0.0, 0.0, 0.0)
        };

        let now_ms = chrono::Utc::now().timestamp_millis() as u64;
        let uptime = now_ms.saturating_sub(start_ms) / 1000;

        let mut s = state.lock().await;
        s.cpu_percent = cpu;
        s.memory_mb = mem;
        s.virtual_memory_mb = vmem;
        s.uptime_secs = uptime;
        s.dashboard_subscribers = dashboard_rx_count();
        s.market_subscribers = market_rx_count();
    }
}
