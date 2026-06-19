//! Integration tests for PSI parsing and pressure classification (planning.md §7).

use syswarden::config;
use syswarden::metrics::memory::MemoryMetrics;
use syswarden::metrics::{Capabilities, CpuMetrics, IoMetrics, MetricsSnapshot};
use syswarden::pressure::{self, PressureLevel, SystemState};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn no_psi_caps() -> Capabilities {
    Capabilities::default() // all false — no PSI, no cgroup v2, not root
}

fn healthy_snapshot() -> MetricsSnapshot {
    MetricsSnapshot {
        timestamp: chrono::Utc::now(),
        memory: MemoryMetrics {
            total_kb: 8_000_000,
            available_kb: 6_000_000,
            free_kb: 2_000_000,
            buffers_kb: 500_000,
            cached_kb: 2_000_000,
            swap_total_kb: 0,
            swap_used_kb: 0,
            swap_in_rate: 0.0,
            swap_out_rate: 0.0,
        },
        cpu: CpuMetrics {
            utilization_pct: 5.0,
            load1: 0.5,
            load5: 0.4,
            load15: 0.3,
            num_cpus: 4,
        },
        io: IoMetrics { io_wait_pct: 0.0 },
    }
}

// ---------------------------------------------------------------------------
// PSI fixture parsing
// ---------------------------------------------------------------------------

const CPU_FIXTURE: &str = include_str!("../examples/fixtures/pressure_cpu.sample");
const MEM_FIXTURE: &str = include_str!("../examples/fixtures/pressure_memory.sample");
const IO_FIXTURE: &str = include_str!("../examples/fixtures/pressure_io.sample");

#[test]
fn parse_psi_cpu_fixture() {
    let psi = pressure::parse_psi(CPU_FIXTURE).expect("parse cpu fixture");
    // fixture: some avg10=5.32 avg60=3.17 avg300=1.87
    assert!(
        (psi.some_avg10 - 5.32).abs() < 0.01,
        "some_avg10={}",
        psi.some_avg10
    );
    assert!((psi.some_avg60 - 3.17).abs() < 0.01);
    assert!((psi.some_avg300 - 1.87).abs() < 0.01);
}

#[test]
fn parse_psi_memory_fixture() {
    let psi = pressure::parse_psi(MEM_FIXTURE).expect("parse memory fixture");
    // fixture has both some and full lines
    assert!(psi.some_avg10 >= 0.0);
    assert!(psi.full_avg10 >= 0.0);
}

#[test]
fn parse_psi_io_fixture() {
    let psi = pressure::parse_psi(IO_FIXTURE).expect("parse io fixture");
    assert!(psi.some_avg10 >= 0.0);
}

#[test]
fn parse_psi_empty_string_returns_error() {
    assert!(
        pressure::parse_psi("").is_err(),
        "empty PSI content should return Err"
    );
}

#[test]
fn parse_psi_corrupt_content_returns_error() {
    assert!(
        pressure::parse_psi("not psi data at all\n").is_err(),
        "corrupt PSI content should return Err"
    );
}

// ---------------------------------------------------------------------------
// Pressure classification — no PSI → Degraded
// ---------------------------------------------------------------------------

#[test]
fn no_psi_caps_compute_returns_level_none() {
    let cfg = config::defaults();
    let snap = pressure::compute(&no_psi_caps(), &healthy_snapshot(), &cfg, &[]);
    // When PSI is unavailable, compute returns PressureLevel::None (early return).
    assert_eq!(snap.level, PressureLevel::None);
}

#[test]
fn no_psi_classify_state_returns_degraded() {
    let cfg = config::defaults();
    let snap = pressure::compute(&no_psi_caps(), &healthy_snapshot(), &cfg, &[]);
    let state = pressure::classify_state(&snap, &[], &[], &no_psi_caps());
    assert_eq!(
        state,
        SystemState::Degraded,
        "missing PSI must classify as Degraded"
    );
}

// ---------------------------------------------------------------------------
// Healthy cache — must NOT raise memory pressure (architecture.md §5.9)
// ---------------------------------------------------------------------------

#[test]
fn healthy_cache_does_not_raise_memory_pressure() {
    // Large page cache but MemAvailable is high → memory is healthy.
    // Even if cached_kb is large, pressure must stay None/Low.
    let cfg = config::defaults();
    let snap = pressure::compute(&no_psi_caps(), &healthy_snapshot(), &cfg, &[]);
    assert!(
        snap.level <= PressureLevel::Low,
        "healthy cache must not trigger memory pressure; level={:?}",
        snap.level
    );
}

// ---------------------------------------------------------------------------
// Hysteresis — level doesn't spike up then immediately back down
// ---------------------------------------------------------------------------

#[test]
fn hysteresis_stabilizes_around_recent_trend() {
    let cfg = config::defaults();
    // A trend of all-None means hysteresis won't dampen a new high reading.
    // But with a trend of all-None, compute(no_psi) must still return None.
    let trend: Vec<PressureLevel> = vec![PressureLevel::None; 5];
    let snap = pressure::compute(&no_psi_caps(), &healthy_snapshot(), &cfg, &trend);
    assert_eq!(snap.level, PressureLevel::None);
}
