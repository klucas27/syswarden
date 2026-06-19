//! Daemon loop smoke tests: single-tick determinism and clean shutdown (planning.md §7).

use chrono::Utc;

use syswarden::actions::ActionStatus;
use syswarden::config;
use syswarden::daemon::single_tick;
use syswarden::history::HistoryStore;
use syswarden::logging::AuditWriter;
use syswarden::metrics::memory::MemoryMetrics;
use syswarden::metrics::{CpuMetrics, IoMetrics, MetricsSnapshot};
use syswarden::pressure::SystemState;
use syswarden::profiles;
use syswarden::safety::Capabilities;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn zeroed_snap() -> MetricsSnapshot {
    MetricsSnapshot {
        timestamp: Utc::now(),
        memory: MemoryMetrics {
            total_kb: 8_000_000,
            available_kb: 4_000_000,
            free_kb: 1_000_000,
            buffers_kb: 500_000,
            cached_kb: 2_500_000,
            swap_total_kb: 0,
            swap_used_kb: 0,
            swap_in_rate: 0.0,
            swap_out_rate: 0.0,
        },
        cpu: CpuMetrics {
            utilization_pct: 0.0,
            load1: 0.0,
            load5: 0.0,
            load15: 0.0,
            num_cpus: 4,
        },
        io: IoMetrics { io_wait_pct: 0.0 },
    }
}

fn no_caps() -> Capabilities {
    Capabilities {
        is_root: false,
        has_psi: false,
        has_cgroup_v2: false,
        has_systemd: false,
        has_zram: false,
    }
}

fn open_test_history(tag: &str) -> (HistoryStore, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("syswarden_it_daemon_{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cfg = config::defaults();
    cfg.history.dir = dir.to_string_lossy().to_string();
    let store = HistoryStore::open(&cfg).expect("history open");
    (store, dir)
}

// ---------------------------------------------------------------------------
// single_tick: no PSI → Degraded state
// ---------------------------------------------------------------------------

#[test]
fn single_tick_no_psi_returns_degraded() {
    let mut cfg = config::defaults();
    let (mut history, hist_dir) = open_test_history("degraded");
    cfg.history.dir = hist_dir.to_string_lossy().to_string();

    let profile = profiles::resolve(&cfg.global.profile, &cfg);
    let audit = AuditWriter::new(std::env::temp_dir().join("syswarden_it_daemon_degraded_audit"));
    let snap = zeroed_snap();

    let tick = single_tick(&cfg, &no_caps(), &snap, &profile, &[], &audit, &mut history);

    assert_eq!(
        tick.state,
        SystemState::Degraded,
        "no PSI must produce Degraded state"
    );

    let _ = std::fs::remove_dir_all(&hist_dir);
}

// ---------------------------------------------------------------------------
// single_tick: dry-run — no action is ever Executed
// ---------------------------------------------------------------------------

#[test]
fn single_tick_dry_run_never_executes() {
    let mut cfg = config::defaults();
    let (mut history, hist_dir) = open_test_history("dryrun");
    cfg.history.dir = hist_dir.to_string_lossy().to_string();

    let profile = profiles::resolve(&cfg.global.profile, &cfg);
    let audit = AuditWriter::new(std::env::temp_dir().join("syswarden_it_daemon_dryrun_audit"));
    let snap = zeroed_snap();

    let tick = single_tick(&cfg, &no_caps(), &snap, &profile, &[], &audit, &mut history);

    for r in &tick.results {
        assert!(
            !matches!(r.status, ActionStatus::Executed),
            "dry-run must never produce Executed; got {:?}",
            r.status
        );
    }

    let _ = std::fs::remove_dir_all(&hist_dir);
}

// ---------------------------------------------------------------------------
// single_tick: history record is appended
// ---------------------------------------------------------------------------

#[test]
fn single_tick_appends_history_record() {
    let mut cfg = config::defaults();
    let (mut history, hist_dir) = open_test_history("hist");
    cfg.history.dir = hist_dir.to_string_lossy().to_string();

    let profile = profiles::resolve(&cfg.global.profile, &cfg);
    let audit = AuditWriter::new(std::env::temp_dir().join("syswarden_it_daemon_hist_audit"));
    let snap = zeroed_snap();

    assert_eq!(history.len(), 0);
    let _ = single_tick(&cfg, &no_caps(), &snap, &profile, &[], &audit, &mut history);
    assert_eq!(
        history.len(),
        1,
        "one tick must append exactly one history record"
    );

    let _ = std::fs::remove_dir_all(&hist_dir);
}

// ---------------------------------------------------------------------------
// single_tick: deterministic — same inputs → same state
// ---------------------------------------------------------------------------

#[test]
fn single_tick_is_deterministic() {
    let mut cfg = config::defaults();
    let (mut h1, d1) = open_test_history("det1");
    let (mut h2, d2) = open_test_history("det2");
    cfg.history.dir = d1.to_string_lossy().to_string();

    let profile = profiles::resolve(&cfg.global.profile, &cfg);
    let audit = AuditWriter::new(std::env::temp_dir().join("syswarden_it_daemon_det_audit"));
    let snap = zeroed_snap();

    let t1 = single_tick(&cfg, &no_caps(), &snap, &profile, &[], &audit, &mut h1);
    let t2 = single_tick(&cfg, &no_caps(), &snap, &profile, &[], &audit, &mut h2);

    assert_eq!(t1.state, t2.state, "state must be deterministic");
    assert_eq!(t1.pressure_level, t2.pressure_level);
    assert_eq!(t1.results.len(), t2.results.len());

    let _ = std::fs::remove_dir_all(&d1);
    let _ = std::fs::remove_dir_all(&d2);
}
