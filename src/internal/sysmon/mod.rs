//! CPU and RAM usage, sampled once per `SAMPLE_INTERVAL` because querying
//! sysinfo every frame is expensive.

use std::time::{Duration, Instant};
use sysinfo::System;

const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

/// System resource monitor. Call `update()` every frame; it re-samples only
/// after `SAMPLE_INTERVAL`.
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

impl Default for SystemMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemMonitor {
    pub fn new() -> Self {
        let mut sys = System::new();
        // Baseline for the first CPU delta.
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

    /// Re-samples if `SAMPLE_INTERVAL` has elapsed.
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

    /// CPU usage in percent (0–100), averaged across cores.
    pub fn cpu_usage(&self) -> f32 {
        self.cpu_usage
    }

    /// Bytes.
    pub fn ram_total(&self) -> u64 {
        self.ram_total
    }

    /// Bytes.
    pub fn ram_used(&self) -> u64 {
        self.ram_used
    }

    /// Percent (0–100).
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
        cpus.iter().map(sysinfo::Cpu::cpu_usage).sum::<f32>() / cpus.len() as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_monitor_initial_values() {
        let mon = SystemMonitor::new();
        assert!(mon.cpu_usage() >= 0.0);
        assert!(mon.cpu_usage() <= 100.0);
        assert!(mon.ram_total() > 0);
        assert!(mon.ram_used() <= mon.ram_total());
        assert!(mon.ram_usage_pct() >= 0.0);
        assert!(mon.ram_usage_pct() <= 100.0);
    }

    #[test]
    fn update_does_not_panic() {
        let mut mon = SystemMonitor::new();
        for _ in 0..3 {
            mon.update();
        }
    }
}
