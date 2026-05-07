//! System monitor — lightweight CPU & RAM usage sampling.
//!
//! Samples are taken at a configurable interval (default 1s) to avoid
//! the cost of querying sysinfo every frame. The latest values are
//! exposed via simple getters for the snapshot pipeline.

use std::time::{Duration, Instant};
use sysinfo::System;

/// How often to re-sample CPU/RAM (default: 1 second).
const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

/// Lightweight system resource monitor.
///
/// Call `update()` every frame — it only actually re-samples when
/// `SAMPLE_INTERVAL` has elapsed since the last sample.
pub struct SystemMonitor {
    sys: System,
    last_sample: Instant,
    /// CPU usage as a percentage (0–100), averaged across all cores.
    cpu_usage: f32,
    /// Total physical RAM in bytes.
    ram_total: u64,
    /// Used physical RAM in bytes.
    ram_used: u64,
}

impl SystemMonitor {
    pub fn new() -> Self {
        let mut sys = System::new();
        // Initial refresh to populate baseline
        sys.refresh_cpu_usage();
        sys.refresh_memory();

        let cpu_usage = Self::avg_cpu(&sys);
        let ram_total = sys.total_memory();
        let ram_used = sys.used_memory();

        Self {
            sys,
            last_sample: Instant::now(),
            cpu_usage,
            ram_total,
            ram_used,
        }
    }

    /// Call every frame. Only re-samples when the interval has elapsed.
    pub fn update(&mut self) {
        if self.last_sample.elapsed() >= SAMPLE_INTERVAL {
            self.sys.refresh_cpu_usage();
            self.sys.refresh_memory();
            self.cpu_usage = Self::avg_cpu(&self.sys);
            self.ram_total = self.sys.total_memory();
            self.ram_used = self.sys.used_memory();
            self.last_sample = Instant::now();
        }
    }

    /// CPU usage % (0–100), averaged across all cores.
    pub fn cpu_usage(&self) -> f32 {
        self.cpu_usage
    }

    /// Total physical RAM in bytes.
    pub fn ram_total(&self) -> u64 {
        self.ram_total
    }

    /// Used physical RAM in bytes.
    pub fn ram_used(&self) -> u64 {
        self.ram_used
    }

    /// RAM usage as a percentage (0–100).
    pub fn ram_usage_pct(&self) -> f32 {
        if self.ram_total > 0 {
            (self.ram_used as f64 / self.ram_total as f64 * 100.0) as f32
        } else {
            0.0
        }
    }

    fn avg_cpu(sys: &System) -> f32 {
        let cpus = sys.cpus();
        if cpus.is_empty() {
            return 0.0;
        }
        cpus.iter().map(|c| c.cpu_usage()).sum::<f32>() / cpus.len() as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_monitor_initial_values() {
        let mon = SystemMonitor::new();
        // CPU usage should be between 0 and 100
        assert!(mon.cpu_usage() >= 0.0);
        assert!(mon.cpu_usage() <= 100.0);
        // RAM total should be positive on any real machine
        assert!(mon.ram_total() > 0);
        // Used <= Total
        assert!(mon.ram_used() <= mon.ram_total());
        // Percentage should be in range
        assert!(mon.ram_usage_pct() >= 0.0);
        assert!(mon.ram_usage_pct() <= 100.0);
    }

    #[test]
    fn update_does_not_panic() {
        let mut mon = SystemMonitor::new();
        // Multiple updates should work fine
        for _ in 0..3 {
            mon.update();
        }
    }
}
