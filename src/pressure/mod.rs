//! PSI parsing, pressure classification, and system-state derivation (architecture.md §5.5, §7, §8).
#![allow(dead_code)]

use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::AppConfig;
use crate::error::SyswardenError;
use crate::metrics::{Capabilities, MetricsSnapshot};
use crate::processes::ProcessInfo;
use crate::services::ServiceInfo;

// ---------------------------------------------------------------------------
// PsiMetrics (Phase 6)
// ---------------------------------------------------------------------------

/// Parsed PSI metrics for one resource (`cpu`, `memory`, or `io`) (architecture.md §15).
///
/// CPU has no `full` line — `full_*` fields are 0.0 for CPU.
/// `total_us` is from the `some` line (microseconds of any stall since boot).
#[derive(Debug, Clone, Default)]
pub struct PsiMetrics {
    pub some_avg10: f64,
    pub some_avg60: f64,
    pub some_avg300: f64,
    pub full_avg10: f64,
    pub full_avg60: f64,
    pub full_avg300: f64,
    pub total_us: u64,
}

/// Parse the text content of a `/proc/pressure/{cpu,memory,io}` file into [`PsiMetrics`].
///
/// Accepts files with just a `some` line (CPU) or both `some` and `full` (memory, io).
/// Returns `Err(SyswardenError::Parse)` if the `some` line is missing or malformed.
pub fn parse_psi(content: &str) -> Result<PsiMetrics, SyswardenError> {
    let mut metrics = PsiMetrics::default();
    let mut found_some = false;

    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("some ") {
            let kv = parse_kv(rest);
            metrics.some_avg10 = get_f64(&kv, "avg10")?;
            metrics.some_avg60 = get_f64(&kv, "avg60")?;
            metrics.some_avg300 = get_f64(&kv, "avg300")?;
            metrics.total_us = get_u64(&kv, "total")?;
            found_some = true;
        } else if let Some(rest) = line.strip_prefix("full ") {
            let kv = parse_kv(rest);
            metrics.full_avg10 = get_f64(&kv, "avg10")?;
            metrics.full_avg60 = get_f64(&kv, "avg60")?;
            metrics.full_avg300 = get_f64(&kv, "avg300")?;
        }
    }

    if !found_some {
        return Err(SyswardenError::Parse(
            "missing 'some' line in PSI file".into(),
        ));
    }

    Ok(metrics)
}

/// Read and parse a PSI file at `path`.
///
/// Returns `Err(SyswardenError::Io(_))` with `kind() == NotFound` when PSI is unavailable
/// (kernel built without `CONFIG_PSI`). Callers match on this to degrade gracefully.
pub fn read_psi(path: &Path) -> Result<PsiMetrics, SyswardenError> {
    let content = std::fs::read_to_string(path)?;
    parse_psi(&content)
}

fn parse_kv(s: &str) -> std::collections::HashMap<&str, &str> {
    s.split_whitespace()
        .filter_map(|token| token.split_once('='))
        .collect()
}

fn get_f64(kv: &std::collections::HashMap<&str, &str>, key: &str) -> Result<f64, SyswardenError> {
    kv.get(key)
        .ok_or_else(|| SyswardenError::Parse(format!("missing PSI field '{key}'")))?
        .parse::<f64>()
        .map_err(|_| SyswardenError::Parse(format!("bad f64 for PSI field '{key}'")))
}

fn get_u64(kv: &std::collections::HashMap<&str, &str>, key: &str) -> Result<u64, SyswardenError> {
    kv.get(key)
        .ok_or_else(|| SyswardenError::Parse(format!("missing PSI field '{key}'")))?
        .parse::<u64>()
        .map_err(|_| SyswardenError::Parse(format!("bad u64 for PSI field '{key}'")))
}

// ---------------------------------------------------------------------------
// PressureLevel (Phase 9)
// ---------------------------------------------------------------------------

/// Scalar pressure classification (architecture.md §8, §15).
///
/// Declaration order defines `Ord`: `None < Low < Moderate < High < Critical`.
/// Use `.max()` to combine sub-levels from CPU/memory/IO.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub enum PressureLevel {
    #[default]
    None,
    Low,
    Moderate,
    High,
    Critical,
}

// ---------------------------------------------------------------------------
// PressureSnapshot (Phase 9)
// ---------------------------------------------------------------------------

/// Classified pressure for one collection tick (architecture.md §15).
///
/// `level` is the final classification after cross-checks and hysteresis.
/// `contributors` names the sub-systems that drove the level above `Low`.
#[derive(Debug, Clone)]
pub struct PressureSnapshot {
    pub timestamp: DateTime<Utc>,
    pub cpu: PsiMetrics,
    pub memory: PsiMetrics,
    pub io: PsiMetrics,
    pub level: PressureLevel,
    pub contributors: Vec<String>,
}

// ---------------------------------------------------------------------------
// SystemState (Phase 9)
// ---------------------------------------------------------------------------

/// Supervision state machine (architecture.md §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemState {
    Initializing,
    Idle,
    Healthy,
    ModeratePressure,
    HighPressure,
    CriticalPressure,
    Recovery,
    Degraded,
    ProtectedMode,
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Map a single PSI `avg10` value to a [`PressureLevel`] using the given thresholds.
fn classify_level(avg10: f64, moderate: f64, high: f64, critical: f64) -> PressureLevel {
    if avg10 >= critical {
        PressureLevel::Critical
    } else if avg10 >= high {
        PressureLevel::High
    } else if avg10 >= moderate {
        PressureLevel::Moderate
    } else if avg10 > 0.0 {
        PressureLevel::Low
    } else {
        PressureLevel::None
    }
}

/// Classify memory pressure with the §8 cross-check.
///
/// High `MemUsed` / low `MemFree` alone **never** raises pressure above `Low`.
/// Escalation requires both elevated PSI **and** low `MemAvailable` (below
/// `thresholds.mem_available_low_pct`%) and/or rising swap-in activity.
/// This encodes the invariant: Linux page-cache use is normal, not a crisis.
#[allow(clippy::cast_precision_loss)]
fn classify_memory_level(
    psi: &PsiMetrics,
    metrics: &MetricsSnapshot,
    thresholds: &crate::config::PressureThresholds,
) -> PressureLevel {
    let mem_available_pct = if metrics.memory.total_kb > 0 {
        metrics.memory.available_kb as f64 / metrics.memory.total_kb as f64 * 100.0
    } else {
        100.0 // unknown → assume healthy
    };

    let mem_constrained = mem_available_pct < thresholds.mem_available_low_pct;
    let swap_rising = metrics.memory.swap_in_rate > 0.1; // pages/sec

    // If memory is genuinely available and swap isn't rising, cap at Low.
    // This is the core "healthy cache ≠ pressure" invariant (architecture.md §8).
    if !mem_constrained && !swap_rising {
        return if psi.some_avg10 > 0.0 {
            PressureLevel::Low
        } else {
            PressureLevel::None
        };
    }

    // Memory IS constrained — classify using full PSI (all-tasks-stalled) as the
    // primary signal for High/Critical, some PSI for Moderate.
    if psi.full_avg10 >= thresholds.mem_full_critical {
        PressureLevel::Critical
    } else if psi.full_avg10 >= thresholds.mem_full_high {
        PressureLevel::High
    } else if psi.some_avg10 >= thresholds.mem_some_moderate {
        PressureLevel::Moderate
    } else if psi.some_avg10 > 0.0 {
        PressureLevel::Low
    } else {
        PressureLevel::None
    }
}

/// Apply hysteresis to prevent pressure-level flapping (architecture.md §7, §8).
///
/// `trend` is a slice of recent computed levels (oldest first). Escalation (moving
/// to a higher level) requires at least `ticks` consecutive readings at the new level;
/// de-escalation is always immediate. If `trend` is empty, `raw` is returned as-is.
fn apply_hysteresis(raw: PressureLevel, trend: &[PressureLevel], ticks: u32) -> PressureLevel {
    if ticks == 0 || trend.is_empty() {
        return raw;
    }
    let prev = *trend.last().unwrap_or(&PressureLevel::None);
    if raw <= prev {
        // De-escalation or stable: apply immediately.
        return raw;
    }
    // Escalation: need `ticks` consecutive readings >= raw (this tick + ticks-1 prior).
    let required_prev = (ticks as usize).saturating_sub(1);
    if trend.len() < required_prev {
        return prev; // not enough history yet
    }
    let recent = &trend[trend.len().saturating_sub(required_prev)..];
    if recent.iter().all(|&l| l >= raw) {
        raw
    } else {
        prev
    }
}

// ---------------------------------------------------------------------------
// Public API (Phase 9)
// ---------------------------------------------------------------------------

/// Read live PSI files, classify per-resource sub-levels, apply cross-checks and
/// hysteresis, and return a [`PressureSnapshot`] (architecture.md §8).
///
/// When `caps.has_psi` is false (or PSI files are unreadable), returns a snapshot
/// with `level = None` and a "degraded" contributor — the daemon continues without
/// PSI classification.
///
/// `trend` is a slice of recent pressure levels (oldest first) used for hysteresis.
/// Pass an empty slice on the first tick.
pub fn compute(
    caps: &Capabilities,
    metrics: &MetricsSnapshot,
    config: &AppConfig,
    trend: &[PressureLevel],
) -> PressureSnapshot {
    let thresholds = &config.pressure.thresholds;

    if !caps.has_psi {
        return PressureSnapshot {
            timestamp: Utc::now(),
            cpu: PsiMetrics::default(),
            memory: PsiMetrics::default(),
            io: PsiMetrics::default(),
            level: PressureLevel::None,
            contributors: vec!["degraded: PSI unavailable (no CONFIG_PSI)".into()],
        };
    }

    let cpu_psi = read_psi(Path::new("/proc/pressure/cpu")).unwrap_or_default();
    let mem_psi = read_psi(Path::new("/proc/pressure/memory")).unwrap_or_default();
    let io_psi = read_psi(Path::new("/proc/pressure/io")).unwrap_or_default();

    let cpu_level = classify_level(
        cpu_psi.some_avg10,
        thresholds.cpu_moderate,
        thresholds.cpu_high,
        thresholds.cpu_critical,
    );
    let mem_level = classify_memory_level(&mem_psi, metrics, thresholds);
    let io_level = classify_level(
        io_psi.some_avg10,
        thresholds.io_moderate,
        thresholds.io_high,
        thresholds.io_critical,
    );

    let mut contributors = Vec::new();
    if cpu_level >= PressureLevel::Moderate {
        contributors.push(format!("cpu: some_avg10={:.1}%", cpu_psi.some_avg10));
    }
    if mem_level >= PressureLevel::Moderate {
        contributors.push(format!(
            "memory: some_avg10={:.1}% full_avg10={:.1}%",
            mem_psi.some_avg10, mem_psi.full_avg10
        ));
    }
    if io_level >= PressureLevel::Moderate {
        contributors.push(format!("io: some_avg10={:.1}%", io_psi.some_avg10));
    }

    let raw = cpu_level.max(mem_level).max(io_level);
    let level = apply_hysteresis(raw, trend, config.polling.hysteresis_ticks);

    PressureSnapshot {
        timestamp: Utc::now(),
        cpu: cpu_psi,
        memory: mem_psi,
        io: io_psi,
        level,
        contributors,
    }
}

/// Classify the current [`SystemState`] from pressure, findings, and capabilities
/// (architecture.md §7).
///
/// `Recovery` is not derivable from a single tick; it is set by the daemon loop
/// (Phase 14) when pressure has been falling for N consecutive ticks. This function
/// never returns `Recovery`, `Initializing`, or `ProtectedMode` — those are set
/// externally by the caller.
pub fn classify_state(
    pressure: &PressureSnapshot,
    processes: &[ProcessInfo],
    services: &[ServiceInfo],
    caps: &Capabilities,
) -> SystemState {
    if !caps.has_psi {
        return SystemState::Degraded;
    }

    let has_anomaly = processes.iter().any(|p| !p.flags.is_empty())
        || services.iter().any(|s| !s.flags.is_empty());

    match pressure.level {
        PressureLevel::None => {
            if has_anomaly {
                SystemState::Healthy // anomaly found; trigger recommend path
            } else {
                SystemState::Idle
            }
        }
        PressureLevel::Low => SystemState::Healthy,
        PressureLevel::Moderate => SystemState::ModeratePressure,
        PressureLevel::High => SystemState::HighPressure,
        PressureLevel::Critical => SystemState::CriticalPressure,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::metrics::memory::MemoryMetrics;
    use crate::metrics::{Capabilities, CpuMetrics, IoMetrics, MetricsSnapshot};

    const CPU_FIXTURE: &str = include_str!("../../examples/fixtures/pressure_cpu.sample");
    const MEMORY_FIXTURE: &str = include_str!("../../examples/fixtures/pressure_memory.sample");
    const IO_FIXTURE: &str = include_str!("../../examples/fixtures/pressure_io.sample");

    // ---- Phase 6 tests (unchanged) ----------------------------------------

    #[test]
    fn parse_cpu_psi_some_only() {
        let m = parse_psi(CPU_FIXTURE).expect("cpu fixture should parse");
        assert!(
            (m.some_avg10 - 5.32).abs() < f64::EPSILON,
            "got {}",
            m.some_avg10
        );
        assert!(
            (m.some_avg60 - 3.17).abs() < f64::EPSILON,
            "got {}",
            m.some_avg60
        );
        assert!(
            (m.some_avg300 - 1.87).abs() < f64::EPSILON,
            "got {}",
            m.some_avg300
        );
        assert_eq!(m.total_us, 12_345_678);
        assert!(m.full_avg10 < f64::EPSILON);
        assert!(m.full_avg60 < f64::EPSILON);
    }

    #[test]
    fn parse_memory_psi_some_and_full() {
        let m = parse_psi(MEMORY_FIXTURE).expect("memory fixture should parse");
        assert!((m.some_avg10 - 2.11).abs() < f64::EPSILON);
        assert!((m.some_avg60 - 0.94).abs() < f64::EPSILON);
        assert!((m.some_avg300 - 0.42).abs() < f64::EPSILON);
        assert_eq!(m.total_us, 5_678_901);
        assert!(
            (m.full_avg10 - 0.02).abs() < f64::EPSILON,
            "got {}",
            m.full_avg10
        );
        assert!((m.full_avg60 - 0.01).abs() < f64::EPSILON);
        assert!(m.full_avg300 < f64::EPSILON);
    }

    #[test]
    fn parse_io_psi_some_and_full() {
        let m = parse_psi(IO_FIXTURE).expect("io fixture should parse");
        assert!((m.some_avg10 - 8.75).abs() < f64::EPSILON);
        assert!((m.full_avg10 - 0.51).abs() < f64::EPSILON);
        assert_eq!(m.total_us, 9_876_543);
    }

    #[test]
    fn parse_error_on_missing_some_line() {
        let err = parse_psi("full avg10=0.00 avg60=0.00 avg300=0.00 total=0\n").unwrap_err();
        assert!(matches!(err, SyswardenError::Parse(_)));
    }

    #[test]
    fn parse_error_on_empty_content() {
        let err = parse_psi("").unwrap_err();
        assert!(matches!(err, SyswardenError::Parse(_)));
    }

    #[test]
    fn parse_error_on_bad_float() {
        let err = parse_psi("some avg10=bad avg60=0.00 avg300=0.00 total=0\n").unwrap_err();
        assert!(matches!(err, SyswardenError::Parse(_)));
    }

    #[test]
    fn read_psi_not_found_is_io_error() {
        let err = read_psi(Path::new("/proc/pressure/__nonexistent__")).unwrap_err();
        assert!(
            matches!(err, SyswardenError::Io(ref e) if e.kind() == std::io::ErrorKind::NotFound),
            "expected NotFound Io error, got {err:?}"
        );
    }

    #[test]
    #[ignore = "reads live /proc/pressure files; requires Linux with CONFIG_PSI"]
    fn read_psi_live_cpu() {
        let m = read_psi(Path::new("/proc/pressure/cpu")).expect("live PSI should be readable");
        assert!(m.some_avg10 >= 0.0);
        assert!(m.some_avg10 <= 100.0);
    }

    // ---- Phase 9 tests ----------------------------------------------------

    fn make_memory_metrics(total_kb: u64, available_kb: u64, swap_in_rate: f64) -> MemoryMetrics {
        MemoryMetrics {
            total_kb,
            available_kb,
            free_kb: 0,
            buffers_kb: 0,
            cached_kb: 0,
            swap_total_kb: 0,
            swap_used_kb: 0,
            swap_in_rate,
            swap_out_rate: 0.0,
        }
    }

    fn make_snapshot(memory: MemoryMetrics) -> MetricsSnapshot {
        MetricsSnapshot {
            timestamp: Utc::now(),
            memory,
            cpu: CpuMetrics {
                utilization_pct: 0.0,
                load1: 0.0,
                load5: 0.0,
                load15: 0.0,
                num_cpus: 1,
            },
            io: IoMetrics { io_wait_pct: 0.0 },
        }
    }

    #[test]
    fn classify_level_boundaries() {
        // Defaults: moderate=15, high=35, critical=60
        let cfg = AppConfig::default();
        let t = &cfg.pressure.thresholds;
        assert_eq!(
            classify_level(0.0, t.cpu_moderate, t.cpu_high, t.cpu_critical),
            PressureLevel::None
        );
        assert_eq!(
            classify_level(1.0, t.cpu_moderate, t.cpu_high, t.cpu_critical),
            PressureLevel::Low
        );
        assert_eq!(
            classify_level(15.0, t.cpu_moderate, t.cpu_high, t.cpu_critical),
            PressureLevel::Moderate
        );
        assert_eq!(
            classify_level(35.0, t.cpu_moderate, t.cpu_high, t.cpu_critical),
            PressureLevel::High
        );
        assert_eq!(
            classify_level(60.0, t.cpu_moderate, t.cpu_high, t.cpu_critical),
            PressureLevel::Critical
        );
    }

    #[test]
    fn memory_healthy_cache_not_flagged() {
        // MemAvailable is 50% — healthy cache use — must NOT escalate above Low
        // even when some_avg10 is above moderate threshold.
        let cfg = AppConfig::default();
        let psi = PsiMetrics {
            some_avg10: 25.0, // above moderate threshold (10%)
            full_avg10: 10.0, // above full_high threshold (5%)
            ..PsiMetrics::default()
        };
        // 8 GiB available out of 16 GiB = 50% — well above mem_available_low_pct=10%
        let metrics = make_snapshot(make_memory_metrics(
            16_000_000, 8_000_000, 0.0, // no swap-in
        ));
        let level = classify_memory_level(&psi, &metrics, &cfg.pressure.thresholds);
        assert!(
            level <= PressureLevel::Low,
            "healthy cache must not exceed Low; got {level:?}"
        );
    }

    #[test]
    fn memory_low_available_escalates_to_high() {
        // MemAvailable is 5% (below 10% threshold) — genuine pressure
        let cfg = AppConfig::default();
        let psi = PsiMetrics {
            some_avg10: 15.0,
            full_avg10: 7.0, // above full_high=5.0
            ..PsiMetrics::default()
        };
        // 800 MB available out of 16 GB = 5%
        let metrics = make_snapshot(make_memory_metrics(16_000_000, 800_000, 0.0));
        let level = classify_memory_level(&psi, &metrics, &cfg.pressure.thresholds);
        assert_eq!(level, PressureLevel::High);
    }

    #[test]
    fn memory_swap_rising_escalates_without_low_available() {
        // MemAvailable is 20% (above 10% threshold) but swap-in is active
        let cfg = AppConfig::default();
        let psi = PsiMetrics {
            some_avg10: 12.0, // above mem_some_moderate=10.0
            full_avg10: 0.0,
            ..PsiMetrics::default()
        };
        let metrics = make_snapshot(make_memory_metrics(
            16_000_000, 3_200_000, // 20% — above low threshold
            5.0,       // swap-in rising
        ));
        let level = classify_memory_level(&psi, &metrics, &cfg.pressure.thresholds);
        assert!(
            level >= PressureLevel::Moderate,
            "swap-in + some_psi should produce at least Moderate; got {level:?}"
        );
    }

    #[test]
    fn hysteresis_holds_on_single_spike() {
        // First elevated reading should not immediately escalate with hysteresis_ticks=3.
        // trend has one prev=None, raw=High → need 2 prior Highs → hold at None.
        let trend = [PressureLevel::None];
        let result = apply_hysteresis(PressureLevel::High, &trend, 3);
        assert_eq!(
            result,
            PressureLevel::None,
            "single spike must not escalate"
        );
    }

    #[test]
    fn hysteresis_escalates_after_enough_ticks() {
        // 3 prior High readings → 4th tick should escalate.
        let trend = [
            PressureLevel::None,
            PressureLevel::High,
            PressureLevel::High,
        ];
        let result = apply_hysteresis(PressureLevel::High, &trend, 3);
        assert_eq!(
            result,
            PressureLevel::High,
            "sustained High must escalate after N ticks"
        );
    }

    #[test]
    fn hysteresis_deescalates_immediately() {
        // Pressure drops from High to Low → allow immediately.
        let trend = [PressureLevel::High, PressureLevel::High];
        let result = apply_hysteresis(PressureLevel::Low, &trend, 3);
        assert_eq!(result, PressureLevel::Low);
    }

    #[test]
    fn hysteresis_empty_trend_returns_raw() {
        let result = apply_hysteresis(PressureLevel::Critical, &[], 3);
        assert_eq!(result, PressureLevel::Critical);
    }

    #[test]
    fn compute_degrades_without_psi_capability() {
        let caps = Capabilities {
            has_psi: false,
            ..Capabilities::default()
        };
        let cfg = AppConfig::default();
        let metrics = make_snapshot(make_memory_metrics(16_000_000, 8_000_000, 0.0));
        let snap = compute(&caps, &metrics, &cfg, &[]);
        assert_eq!(snap.level, PressureLevel::None);
        assert!(
            snap.contributors.iter().any(|c| c.contains("degraded")),
            "must report degraded contributor"
        );
    }

    #[test]
    fn classify_state_idle_no_psi_anomaly() {
        let caps = Capabilities {
            has_psi: true,
            ..Capabilities::default()
        };
        let snap = PressureSnapshot {
            timestamp: Utc::now(),
            cpu: PsiMetrics::default(),
            memory: PsiMetrics::default(),
            io: PsiMetrics::default(),
            level: PressureLevel::None,
            contributors: vec![],
        };
        assert_eq!(classify_state(&snap, &[], &[], &caps), SystemState::Idle);
    }

    #[test]
    fn classify_state_healthy_on_low_pressure() {
        let caps = Capabilities {
            has_psi: true,
            ..Capabilities::default()
        };
        let snap = PressureSnapshot {
            timestamp: Utc::now(),
            cpu: PsiMetrics::default(),
            memory: PsiMetrics::default(),
            io: PsiMetrics::default(),
            level: PressureLevel::Low,
            contributors: vec![],
        };
        assert_eq!(classify_state(&snap, &[], &[], &caps), SystemState::Healthy);
    }

    #[test]
    fn classify_state_moderate_pressure() {
        let caps = Capabilities {
            has_psi: true,
            ..Capabilities::default()
        };
        let snap = PressureSnapshot {
            timestamp: Utc::now(),
            cpu: PsiMetrics::default(),
            memory: PsiMetrics::default(),
            io: PsiMetrics::default(),
            level: PressureLevel::Moderate,
            contributors: vec!["cpu: some_avg10=20.0%".into()],
        };
        assert_eq!(
            classify_state(&snap, &[], &[], &caps),
            SystemState::ModeratePressure
        );
    }

    #[test]
    fn classify_state_critical_pressure() {
        let caps = Capabilities {
            has_psi: true,
            ..Capabilities::default()
        };
        let snap = PressureSnapshot {
            timestamp: Utc::now(),
            cpu: PsiMetrics::default(),
            memory: PsiMetrics::default(),
            io: PsiMetrics::default(),
            level: PressureLevel::Critical,
            contributors: vec!["memory: some_avg10=65.0% full_avg10=25.0%".into()],
        };
        assert_eq!(
            classify_state(&snap, &[], &[], &caps),
            SystemState::CriticalPressure
        );
    }

    #[test]
    fn classify_state_degraded_without_psi() {
        let caps = Capabilities {
            has_psi: false,
            ..Capabilities::default()
        };
        let snap = PressureSnapshot {
            timestamp: Utc::now(),
            cpu: PsiMetrics::default(),
            memory: PsiMetrics::default(),
            io: PsiMetrics::default(),
            level: PressureLevel::None,
            contributors: vec!["degraded: PSI unavailable (no CONFIG_PSI)".into()],
        };
        assert_eq!(
            classify_state(&snap, &[], &[], &caps),
            SystemState::Degraded
        );
    }
}
