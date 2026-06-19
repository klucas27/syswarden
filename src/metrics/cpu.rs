//! CPU metrics from `/proc/stat`, `/proc/loadavg`, `/proc/vmstat`; produces `CpuMetrics` (architecture.md §5.4, §15).
#![allow(dead_code, clippy::module_name_repetitions)]

use std::time::Instant;

use crate::error::SyswardenError;

/// Raw CPU tick counts and swap counters from one reading of `/proc/stat` + `/proc/vmstat`.
///
/// Two consecutive [`CpuSample`]s are needed to compute utilization and swap rates.
#[derive(Debug, Clone)]
pub struct CpuSample {
    pub user: u64,
    pub nice: u64,
    pub system: u64,
    pub idle: u64,
    pub iowait: u64,
    pub irq: u64,
    pub softirq: u64,
    pub steal: u64,
    /// Cumulative pages swapped in (from `/proc/vmstat` `pswpin`).
    pub swap_in: u64,
    /// Cumulative pages swapped out (from `/proc/vmstat` `pswpout`).
    pub swap_out: u64,
    pub taken_at: Instant,
}

impl CpuSample {
    /// Sum of all CPU tick fields.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.user
            .saturating_add(self.nice)
            .saturating_add(self.system)
            .saturating_add(self.idle)
            .saturating_add(self.iowait)
            .saturating_add(self.irq)
            .saturating_add(self.softirq)
            .saturating_add(self.steal)
    }

    /// Combined idle + iowait ticks.
    #[must_use]
    pub fn idle_total(&self) -> u64 {
        self.idle.saturating_add(self.iowait)
    }
}

impl Default for CpuSample {
    fn default() -> Self {
        Self {
            user: 0,
            nice: 0,
            system: 0,
            idle: 0,
            iowait: 0,
            irq: 0,
            softirq: 0,
            steal: 0,
            swap_in: 0,
            swap_out: 0,
            taken_at: Instant::now(),
        }
    }
}

/// CPU metrics for one tick (architecture.md §15).
#[derive(Debug, Clone)]
pub struct CpuMetrics {
    pub utilization_pct: f64,
    pub load1: f64,
    pub load5: f64,
    pub load15: f64,
    pub num_cpus: u32,
}

/// Parse the aggregate `cpu` line from `/proc/stat` into a [`CpuSample`].
///
/// Swap counters default to 0; call [`update_swap`] after to fill them in.
///
/// # Errors
/// Returns `Err` if the `cpu` aggregate line is missing or unparseable.
pub fn parse_stat(content: &str) -> Result<CpuSample, SyswardenError> {
    let line = content
        .lines()
        .find(|l| {
            let rest = l.strip_prefix("cpu").unwrap_or("");
            rest.starts_with(' ')
        })
        .ok_or_else(|| SyswardenError::Parse("missing 'cpu ' line in /proc/stat".into()))?;

    let parts: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .map(|s| s.parse().unwrap_or(0))
        .collect();

    if parts.len() < 8 {
        return Err(SyswardenError::Parse(format!(
            "/proc/stat cpu line has {} fields, need ≥8",
            parts.len()
        )));
    }

    Ok(CpuSample {
        user: parts[0],
        nice: parts[1],
        system: parts[2],
        idle: parts[3],
        iowait: parts[4],
        irq: parts[5],
        softirq: parts[6],
        steal: parts[7],
        swap_in: 0,
        swap_out: 0,
        taken_at: Instant::now(),
    })
}

/// Fill `pswpin`/`pswpout` into an existing [`CpuSample`] from `/proc/vmstat` content.
///
/// Silently leaves counters at 0 if the fields are absent (degraded gracefully).
pub fn update_swap(sample: &mut CpuSample, vmstat_content: &str) {
    for line in vmstat_content.lines() {
        let mut parts = line.split_whitespace();
        match (parts.next(), parts.next()) {
            (Some("pswpin"), Some(v)) => sample.swap_in = v.parse().unwrap_or(0),
            (Some("pswpout"), Some(v)) => sample.swap_out = v.parse().unwrap_or(0),
            _ => {}
        }
    }
}

/// Count the number of logical CPUs from `/proc/stat` (`cpu0`, `cpu1`, … lines).
#[must_use]
pub fn count_cpus(stat_content: &str) -> u32 {
    let count = stat_content
        .lines()
        .filter(|l| {
            let rest = l.strip_prefix("cpu").unwrap_or("");
            rest.starts_with(|c: char| c.is_ascii_digit())
        })
        .count();
    u32::try_from(count).unwrap_or(1)
}

/// Parse `/proc/loadavg` content → `(load1, load5, load15)`.
///
/// # Errors
/// Returns `Err` if any of the three load-average fields are missing or non-numeric.
#[allow(clippy::similar_names)]
pub fn parse_loadavg(content: &str) -> Result<(f64, f64, f64), SyswardenError> {
    let mut parts = content.split_whitespace();
    let parse_f = |s: Option<&str>, label: &str| -> Result<f64, SyswardenError> {
        s.ok_or_else(|| SyswardenError::Parse(format!("missing {label} in /proc/loadavg")))?
            .parse::<f64>()
            .map_err(|_| SyswardenError::Parse(format!("bad {label} in /proc/loadavg")))
    };
    let avg1 = parse_f(parts.next(), "load1")?;
    let avg5 = parse_f(parts.next(), "load5")?;
    let avg15 = parse_f(parts.next(), "load15")?;
    Ok((avg1, avg5, avg15))
}

/// Overall CPU utilization (%) from two consecutive samples.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn utilization(prev: &CpuSample, curr: &CpuSample) -> f64 {
    let total = curr.total().saturating_sub(prev.total());
    if total == 0 {
        return 0.0;
    }
    let idle = curr.idle_total().saturating_sub(prev.idle_total());
    let busy = total.saturating_sub(idle);
    (busy as f64 / total as f64) * 100.0
}

/// I/O wait percentage from two consecutive samples.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn iowait_pct(prev: &CpuSample, curr: &CpuSample) -> f64 {
    let total = curr.total().saturating_sub(prev.total());
    if total == 0 {
        return 0.0;
    }
    let iowait = curr.iowait.saturating_sub(prev.iowait);
    (iowait as f64 / total as f64) * 100.0
}

/// Swap-in rate in pages/second from two consecutive samples.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn swap_in_rate(prev: &CpuSample, curr: &CpuSample) -> f64 {
    let elapsed = curr.taken_at.duration_since(prev.taken_at).as_secs_f64();
    if elapsed < 0.001 {
        return 0.0;
    }
    curr.swap_in.saturating_sub(prev.swap_in) as f64 / elapsed
}

/// Swap-out rate in pages/second from two consecutive samples.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn swap_out_rate(prev: &CpuSample, curr: &CpuSample) -> f64 {
    let elapsed = curr.taken_at.duration_since(prev.taken_at).as_secs_f64();
    if elapsed < 0.001 {
        return 0.0;
    }
    curr.swap_out.saturating_sub(prev.swap_out) as f64 / elapsed
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAT_FIXTURE: &str = include_str!("../../examples/fixtures/proc_stat.sample");
    const LOADAVG_FIXTURE: &str = include_str!("../../examples/fixtures/proc_loadavg.sample");

    #[test]
    fn parse_stat_reads_cpu_line() {
        let s = parse_stat(STAT_FIXTURE).expect("fixture should parse");
        assert_eq!(s.user, 100_000);
        assert_eq!(s.nice, 0);
        assert_eq!(s.system, 50_000);
        assert_eq!(s.idle, 850_000);
        assert_eq!(s.iowait, 10_000);
        assert_eq!(s.irq, 0);
        assert_eq!(s.softirq, 5_000);
        assert_eq!(s.steal, 0);
    }

    #[test]
    fn utilization_from_two_samples() {
        let prev = CpuSample::default(); // all zeros
        let curr = parse_stat(STAT_FIXTURE).expect("fixture");
        let util = utilization(&prev, &curr);
        // total=1_015_000, idle_total=860_000, busy=155_000, util=155/1015*100≈15.27%
        assert!((util - 15.27).abs() < 0.1, "got {util}");
    }

    #[test]
    fn utilization_zero_on_no_delta() {
        let s = parse_stat(STAT_FIXTURE).expect("fixture");
        assert!(utilization(&s, &s) < f64::EPSILON);
    }

    #[test]
    fn iowait_from_two_samples() {
        let prev = CpuSample::default();
        let curr = parse_stat(STAT_FIXTURE).expect("fixture");
        let iow = iowait_pct(&prev, &curr);
        // iowait=10_000 / total=1_015_000 * 100 ≈ 0.985%
        assert!((iow - 0.985).abs() < 0.01, "got {iow}");
    }

    #[test]
    fn count_cpus_from_fixture() {
        assert_eq!(count_cpus(STAT_FIXTURE), 4);
    }

    #[test]
    fn parse_loadavg_fixture() {
        let (l1, l5, l15) = parse_loadavg(LOADAVG_FIXTURE).expect("fixture");
        assert!((l1 - 0.72).abs() < f64::EPSILON);
        assert!((l5 - 0.61).abs() < f64::EPSILON);
        assert!((l15 - 0.55).abs() < f64::EPSILON);
    }

    #[test]
    fn update_swap_parses_vmstat() {
        let vmstat = "nr_free_pages 12345\npswpin 500\npswpout 250\nnr_active_anon 9999\n";
        let mut sample = CpuSample::default();
        update_swap(&mut sample, vmstat);
        assert_eq!(sample.swap_in, 500);
        assert_eq!(sample.swap_out, 250);
    }

    #[test]
    fn swap_rate_zero_on_identical_timestamp() {
        let s = CpuSample {
            swap_in: 100,
            swap_out: 50,
            ..CpuSample::default()
        };
        // same taken_at → elapsed ≈ 0 → rate = 0.0
        assert!(swap_in_rate(&s, &s) < f64::EPSILON);
        assert!(swap_out_rate(&s, &s) < f64::EPSILON);
    }

    #[test]
    fn parse_stat_error_on_missing_cpu_line() {
        let err = parse_stat("intr 123\nctxt 456\n").unwrap_err();
        assert!(matches!(err, SyswardenError::Parse(_)));
    }
}
