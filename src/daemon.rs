//! Main supervision loop and daemon lifecycle (architecture.md §5.2, §6).

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tracing::{debug, info, trace, warn};

use crate::actions::{ActionResult, ActionStatus};
use crate::config::AppConfig;
use crate::explain;
use crate::history::{HistoryRecord, HistoryStore};
use crate::logging::{AuditEvent, AuditKind, AuditWriter};
use crate::metrics::{self, CpuSample};
use crate::policy;
use crate::pressure::{self, PressureLevel, SystemState};
use crate::processes;
use crate::profiles::{self, ProfileConfig};
use crate::rollback::RollbackStore;
use crate::safety::Capabilities;
use crate::services;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Result of one supervision tick; returned by [`single_tick`].
pub struct TickResult {
    pub state: SystemState,
    /// Pressure level from this tick (append to the hysteresis trend).
    pub pressure_level: PressureLevel,
    pub results: Vec<ActionResult>,
}

// ---------------------------------------------------------------------------
// adaptive_interval
// ---------------------------------------------------------------------------

/// Compute the adaptive polling interval for the current state (architecture.md §6 step 16).
///
/// Uses `profile` intervals as the baseline; clamps to
/// `[config.polling.min_interval_secs, config.polling.max_interval_secs]`.
#[must_use]
pub fn adaptive_interval(
    state: SystemState,
    profile: &ProfileConfig,
    config: &AppConfig,
) -> Duration {
    let secs = match state {
        SystemState::ModeratePressure
        | SystemState::HighPressure
        | SystemState::CriticalPressure => profile.pressure_interval_secs,
        _ => profile.idle_interval_secs,
    };
    let bounded = secs
        .max(config.polling.min_interval_secs)
        .min(config.polling.max_interval_secs);
    Duration::from_secs(bounded)
}

// ---------------------------------------------------------------------------
// single_tick
// ---------------------------------------------------------------------------

/// Execute one full supervision tick (architecture.md §6 steps 5–16).
///
/// Accepts a pre-collected `MetricsSnapshot` so that tests can inject specific
/// system states without depending on live `/proc` or `/sys` content.
///
/// All actions are dry-run in v0.1 — [`crate::actions::simulate`] is always used;
/// no system state is changed.
pub fn single_tick(
    config: &AppConfig,
    caps: &Capabilities,
    metrics_snap: &metrics::MetricsSnapshot,
    profile: &ProfileConfig,
    pressure_trend: &[PressureLevel],
    audit: &AuditWriter,
    history: &mut HistoryStore,
) -> TickResult {
    let metrics_caps = metrics::Capabilities {
        has_psi: caps.has_psi,
        has_cgroup_v2: caps.has_cgroup_v2,
        has_systemd: caps.has_systemd,
        is_root: caps.is_root,
        has_zram: caps.has_zram,
    };

    // Steps 6–8: pressure, process, and service analysis.
    let pressure_snap = pressure::compute(&metrics_caps, metrics_snap, config, pressure_trend);
    let processes = processes::analyze(config);
    let services = services::analyze(config);

    // Step 9: classify system state.
    let state = pressure::classify_state(&pressure_snap, &processes, &services, &metrics_caps);

    // Step 10: policy decision.
    let decision = policy::decide(state, profile, &processes, &services);

    // Step 11: plan actions.
    let planned = crate::actions::plan(&decision, profile, &processes);

    // Steps 12–13: safety gate + dry-run simulation (v0.1: always simulate).
    //
    // `actions::simulate` calls `safety::evaluate` internally and returns
    // `Blocked` or `Simulated` — no execution path exists in v0.1.
    let results: Vec<ActionResult> = planned
        .iter()
        .map(|action| {
            let result = crate::actions::simulate(action, config, profile, caps);
            let audit_kind = if matches!(result.status, ActionStatus::Blocked) {
                AuditKind::Block
            } else {
                AuditKind::Action
            };
            audit.append(&AuditEvent::new(
                audit_kind,
                format!("{state:?}"),
                format!("{:?}", pressure_snap.level),
                format!("{:?}", action.kind),
                result.message.clone(),
            ));
            result
        })
        .collect();

    // Step 14: audit the decision.
    audit.append(&AuditEvent::new(
        AuditKind::Decision,
        format!("{state:?}"),
        format!("{:?}", pressure_snap.level),
        decision.rationale.clone(),
        format!("{} action(s)", results.len()),
    ));

    // Step 15: explain at trace level.
    let explanation = explain::build(state, &pressure_snap, &decision, &results);
    trace!("{}", explanation.summary);

    // Step 15: persist history record to JSONL.
    let simulated_count = results
        .iter()
        .filter(|r| matches!(r.status, ActionStatus::Simulated))
        .count();
    let blocked_count = results
        .iter()
        .filter(|r| matches!(r.status, ActionStatus::Blocked))
        .count();
    let psi_summary = format!(
        "cpu={:.1} mem={:.1} io={:.1}",
        pressure_snap.cpu.some_avg10, pressure_snap.memory.some_avg10, pressure_snap.io.some_avg10,
    );
    let outcomes: Vec<String> = planned
        .iter()
        .zip(results.iter())
        .map(|(a, r)| format!("{:?}:{:?}", a.kind, r.status))
        .collect();
    history.append(HistoryRecord {
        schema_version: crate::history::SCHEMA_VERSION,
        timestamp: metrics_snap.timestamp,
        pressure_level: pressure_snap.level,
        psi_summary,
        state: format!("{state:?}"),
        action_count: results.len(),
        simulated_count,
        blocked_count,
        outcomes,
    });

    TickResult {
        state,
        pressure_level: pressure_snap.level,
        results,
    }
}

// ---------------------------------------------------------------------------
// run
// ---------------------------------------------------------------------------

/// Run the supervision daemon loop (architecture.md §6).
///
/// Blocks until SIGTERM or SIGINT. Builds its own tokio runtime so that
/// `main` and `cli::dispatch` remain synchronous.
///
/// All actions are dry-run in v0.1 — no system state is changed.
///
/// # Errors
/// Returns an error if the tokio runtime cannot be built or if any
/// unrecoverable startup error occurs.
pub fn run(config: AppConfig) -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime")?;
    rt.block_on(run_async(config))
}

// ---------------------------------------------------------------------------
// Private: async daemon loop
// ---------------------------------------------------------------------------

async fn run_async(config: AppConfig) -> Result<()> {
    let caps = Capabilities::detect();
    let profile = profiles::resolve(&config.global.profile, &config);
    let audit = AuditWriter::new(&config.logging.audit_dir);
    let mut history = HistoryStore::open(&config).context("failed to open history store")?;
    // Rollback store is opened for v0.1 scaffolding; entries are written in v0.2+ execute paths.
    let _rollback = RollbackStore::open(&config).context("failed to open rollback store")?;
    let mut prev_cpu = CpuSample::default();

    // Hysteresis trend: last N pressure levels (oldest first).
    // Seeded from disk history so the trend survives daemon restarts.
    let trend_cap = config.polling.hysteresis_ticks as usize + 1;
    let mut pressure_trend: Vec<PressureLevel> = history.recent_levels(trend_cap);

    info!(
        dry_run = config.global.dry_run,
        profile = ?config.global.profile,
        "syswarden daemon starting",
    );

    // Install SIGTERM listener (unix only).
    #[cfg(unix)]
    let mut sigterm = {
        use tokio::signal::unix::{signal, SignalKind};
        signal(SignalKind::terminate()).context("failed to install SIGTERM handler")?
    };

    loop {
        let tick_start = Instant::now();

        // Step 5: collect metrics.
        let metrics_caps = metrics::Capabilities {
            has_psi: caps.has_psi,
            has_cgroup_v2: caps.has_cgroup_v2,
            has_systemd: caps.has_systemd,
            is_root: caps.is_root,
            has_zram: caps.has_zram,
        };
        let metrics_snap = match metrics::collect(&metrics_caps, &mut prev_cpu) {
            Ok(m) => m,
            Err(e) => {
                warn!("metrics collection error: {e}; skipping tick");
                tokio::time::sleep(Duration::from_secs(config.polling.min_interval_secs)).await;
                continue;
            }
        };

        // Steps 6–15: run one tick.
        let tick = single_tick(
            &config,
            &caps,
            &metrics_snap,
            &profile,
            &pressure_trend,
            &audit,
            &mut history,
        );

        // Update hysteresis trend.
        pressure_trend.push(tick.pressure_level);
        if pressure_trend.len() > trend_cap {
            pressure_trend.remove(0);
        }

        debug!(
            state = ?tick.state,
            actions = tick.results.len(),
            elapsed_ms = tick_start.elapsed().as_millis(),
            "tick complete",
        );

        // Step 16: adaptive sleep, interrupted by SIGTERM or SIGINT.
        let interval = adaptive_interval(tick.state, &profile, &config);
        let elapsed = tick_start.elapsed();
        let sleep_dur = interval.saturating_sub(elapsed);

        // Signal futures are re-polled each iteration.
        #[cfg(unix)]
        {
            tokio::select! {
                () = tokio::time::sleep(sleep_dur) => {}
                _ = sigterm.recv() => {
                    info!("received SIGTERM; shutting down");
                    break;
                }
                r = tokio::signal::ctrl_c() => {
                    if r.is_ok() {
                        info!("received SIGINT; shutting down");
                    }
                    break;
                }
            }
        }
        #[cfg(not(unix))]
        {
            tokio::select! {
                () = tokio::time::sleep(sleep_dur) => {}
                r = tokio::signal::ctrl_c() => {
                    if r.is_ok() {
                        info!("received SIGINT; shutting down");
                    }
                    break;
                }
            }
        }
    }

    // Step 17: graceful shutdown — flush is a no-op in Phase 14 (in-memory history).
    info!("syswarden daemon stopped");
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::actions::ActionStatus;
    use crate::config;
    use crate::metrics::memory::MemoryMetrics;
    use crate::metrics::{CpuMetrics, IoMetrics, MetricsSnapshot};
    use crate::pressure::SystemState;
    use crate::profiles;

    fn zeroed_snapshot() -> MetricsSnapshot {
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

    /// Smoke test: one tick with no PSI → Degraded state, dry-run only, history appended.
    ///
    /// `has_psi = false` makes `pressure::compute` return early without reading
    /// `/proc/pressure/*`, keeping this test host-independent (planning.md §7).
    #[test]
    fn smoke_single_tick_dry_run() {
        let mut config = config::defaults();
        // Use a temp dir so the test never needs root (default dir = /var/lib/…).
        let hist_dir = std::env::temp_dir().join("syswarden_smoke_history");
        let _ = std::fs::remove_dir_all(&hist_dir); // clear leftovers from prior runs
        config.history.dir = hist_dir.to_string_lossy().to_string();
        let caps = Capabilities {
            is_root: false,
            has_psi: false,
            has_cgroup_v2: false,
            has_systemd: false,
            has_zram: false,
        };
        let profile = profiles::resolve(&config.global.profile, &config);
        let audit = AuditWriter::new(std::env::temp_dir().join("syswarden_smoke_test_audit"));
        let mut history = HistoryStore::open(&config).expect("history open");
        let snap = zeroed_snapshot();

        let tick = single_tick(&config, &caps, &snap, &profile, &[], &audit, &mut history);

        // No PSI → Degraded state.
        assert_eq!(tick.state, SystemState::Degraded);

        // Dry-run: no action may be Executed.
        for r in &tick.results {
            assert!(
                !matches!(r.status, ActionStatus::Executed),
                "action executed in dry-run: {r:?}",
            );
        }

        // One history record appended.
        assert_eq!(history.len(), 1);
    }

    #[test]
    fn adaptive_interval_pressure_uses_pressure_secs() {
        let config = config::defaults();
        let profile = profiles::resolve(&config.global.profile, &config);
        let dur = adaptive_interval(SystemState::HighPressure, &profile, &config);
        assert_eq!(dur.as_secs(), profile.pressure_interval_secs);
    }

    #[test]
    fn adaptive_interval_idle_uses_idle_secs() {
        let config = config::defaults();
        let profile = profiles::resolve(&config.global.profile, &config);
        let dur = adaptive_interval(SystemState::Idle, &profile, &config);
        assert_eq!(dur.as_secs(), profile.idle_interval_secs);
    }

    #[test]
    fn adaptive_interval_clamped_to_min() {
        let mut config = config::defaults();
        config.polling.min_interval_secs = 60;
        config.polling.max_interval_secs = 120; // keep min < max (valid config)
        let profile = profiles::resolve(&config.global.profile, &config);
        let dur = adaptive_interval(SystemState::CriticalPressure, &profile, &config);
        assert!(dur.as_secs() >= 60);
    }

    #[test]
    fn adaptive_interval_clamped_to_max() {
        let mut config = config::defaults();
        config.polling.max_interval_secs = 1;
        let profile = profiles::resolve(&config.global.profile, &config);
        let dur = adaptive_interval(SystemState::Idle, &profile, &config);
        assert_eq!(dur.as_secs(), 1);
    }
}
