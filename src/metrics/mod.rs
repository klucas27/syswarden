//! Metrics collection coordinator; produces `MetricsSnapshot` (architecture.md §5.4, §15).
#![allow(dead_code)]

pub mod cpu;
pub mod io;
pub mod memory;

pub use cpu::{CpuMetrics, CpuSample};
pub use io::IoMetrics;
pub use memory::MemoryMetrics;

use std::path::Path;

use chrono::{DateTime, Utc};

use crate::error::SyswardenError;

/// Runtime capabilities detected at daemon startup (architecture.md §6).
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default)]
pub struct Capabilities {
    pub has_psi: bool,
    pub has_cgroup_v2: bool,
    pub has_systemd: bool,
    pub is_root: bool,
    pub has_zram: bool,
}

/// One complete metrics collection tick (architecture.md §15).
#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    pub timestamp: DateTime<Utc>,
    pub memory: MemoryMetrics,
    pub cpu: CpuMetrics,
    pub io: IoMetrics,
}

/// Collect one [`MetricsSnapshot`].
///
/// `prev_cpu` is updated in-place so the next call computes accurate deltas.
/// On the very first call (with a default/zeroed `prev_cpu`), CPU utilization
/// reflects the since-boot average — accurate on subsequent calls.
#[allow(clippy::similar_names)]
pub fn collect(
    _caps: &Capabilities,
    prev_cpu: &mut CpuSample,
) -> Result<MetricsSnapshot, SyswardenError> {
    // Memory
    let mut mem = memory::read(Path::new("/proc/meminfo"))?;

    // CPU stat
    let stat_content = std::fs::read_to_string("/proc/stat")?;
    let mut curr = cpu::parse_stat(&stat_content)?;

    // Swap counters from vmstat (best-effort; silently skipped if absent)
    if let Ok(vmstat) = std::fs::read_to_string("/proc/vmstat") {
        cpu::update_swap(&mut curr, &vmstat);
    }

    let utilization_pct = cpu::utilization(prev_cpu, &curr);
    let io_wait_pct = cpu::iowait_pct(prev_cpu, &curr);
    mem.swap_in_rate = cpu::swap_in_rate(prev_cpu, &curr);
    mem.swap_out_rate = cpu::swap_out_rate(prev_cpu, &curr);
    let num_cpus = cpu::count_cpus(&stat_content);
    *prev_cpu = curr;

    // Loadavg
    let loadavg_content = std::fs::read_to_string("/proc/loadavg")?;
    let (avg1, avg5, avg15) = cpu::parse_loadavg(&loadavg_content)?;

    Ok(MetricsSnapshot {
        timestamp: Utc::now(),
        memory: mem,
        cpu: CpuMetrics {
            utilization_pct,
            load1: avg1,
            load5: avg5,
            load15: avg15,
            num_cpus,
        },
        io: IoMetrics { io_wait_pct },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "reads live /proc files; requires Linux"]
    fn collect_returns_valid_snapshot() {
        let caps = Capabilities::default();
        let mut prev = CpuSample::default();
        let snap = collect(&caps, &mut prev).expect("collect should succeed");
        assert!(snap.memory.total_kb > 0, "total memory must be > 0");
        assert!(snap.memory.available_kb > 0, "available memory must be > 0");
        assert!(snap.cpu.num_cpus > 0, "must have at least one CPU");
        assert!(snap.cpu.load1 >= 0.0);
        assert!(snap.cpu.utilization_pct >= 0.0);
        assert!(snap.cpu.utilization_pct <= 100.0);
        assert!(snap.io.io_wait_pct >= 0.0);
    }
}
