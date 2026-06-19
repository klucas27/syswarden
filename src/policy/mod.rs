//! Pure policy engine: maps `SystemState` + `ProfileConfig` to `PolicyDecision` (architecture.md §5.11, §9).
#![allow(dead_code)]

use std::collections::HashSet;

use crate::pressure::SystemState;
use crate::processes::{ProcessFlag, ProcessInfo};
use crate::profiles::{ActionRisk, ProfileConfig};
use crate::services::ServiceInfo;

// ---------------------------------------------------------------------------
// DecisionIntent
// ---------------------------------------------------------------------------

/// The primary intent of a policy decision (architecture.md §9).
///
/// Matched exhaustively in all callers — no catch-all `_` in safety-critical code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionIntent {
    ObserveOnly,
    LogOnly,
    Recommend,
    Alert,
    AdjustProcessPriority,
    ApplyCgroupSystemdLimit,
    RecommendZram,
    ApplyZram,
    EnterProtectedMode,
    BlockAction,
    DoNothing,
}

// ---------------------------------------------------------------------------
// Target
// ---------------------------------------------------------------------------

/// The entity a `PolicyDecision` applies to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A specific process identified by PID.
    Process(u32),
    /// A specific systemd service unit by name.
    Service(String),
    /// Whole-system or unspecific target.
    System,
}

// ---------------------------------------------------------------------------
// PolicyDecision
// ---------------------------------------------------------------------------

/// Output of the policy engine: primary intent + targets + human rationale
/// (architecture.md §15).
///
/// This is a pure *intent*; the safety layer and action planner translate it
/// into concrete, gated `PlannedAction`s. Whether an action is *safe to execute*
/// is never decided here — that is the safety layer's sole responsibility.
#[derive(Debug, Clone)]
pub struct PolicyDecision {
    pub intent: DecisionIntent,
    pub targets: Vec<Target>,
    pub rationale: String,
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn has_process_anomaly(processes: &[ProcessInfo]) -> bool {
    processes.iter().any(|p| !p.flags.is_empty())
}

fn has_service_anomaly(services: &[ServiceInfo]) -> bool {
    services.iter().any(|s| !s.flags.is_empty())
}

/// Flagged, non-protected processes and services as `Recommend` targets.
/// Falls back to `Target::System` when nothing specific is flagged.
fn flagged_targets(processes: &[ProcessInfo], services: &[ServiceInfo]) -> Vec<Target> {
    let mut targets: Vec<Target> = processes
        .iter()
        .filter(|p| !p.is_protected && !p.flags.is_empty())
        .map(|p| Target::Process(p.pid))
        .collect();
    targets.extend(
        services
            .iter()
            .filter(|s| !s.is_protected && !s.flags.is_empty())
            .map(|s| Target::Service(s.unit.clone())),
    );
    if targets.is_empty() {
        targets.push(Target::System);
    }
    targets
}

/// Non-protected processes eligible for nice/ionice adjustment, deduplicated by PID.
///
/// - `allow_nice`: HighCpu-flagged processes.
/// - `allow_ionice`: any-flagged processes (planner picks the I/O variant).
fn collect_process_targets(processes: &[ProcessInfo], profile: &ProfileConfig) -> Vec<Target> {
    let mut seen: HashSet<u32> = HashSet::new();
    let mut targets = Vec::new();
    if profile.allow_nice {
        for p in processes
            .iter()
            .filter(|p| !p.is_protected && p.flags.contains(&ProcessFlag::HighCpu))
        {
            if seen.insert(p.pid) {
                targets.push(Target::Process(p.pid));
            }
        }
    }
    if profile.allow_ionice {
        for p in processes
            .iter()
            .filter(|p| !p.is_protected && !p.flags.is_empty())
        {
            if seen.insert(p.pid) {
                targets.push(Target::Process(p.pid));
            }
        }
    }
    targets
}

/// Allowed, non-protected, flagged services eligible for cgroup limit adjustment.
/// Returns empty when no cgroup permission is set on the profile.
fn collect_cgroup_targets(services: &[ServiceInfo], profile: &ProfileConfig) -> Vec<Target> {
    if !profile.allow_memory_high && !profile.allow_cpu_weight && !profile.allow_io_weight {
        return vec![];
    }
    services
        .iter()
        .filter(|s| s.is_allowed && !s.is_protected && !s.flags.is_empty())
        .map(|s| Target::Service(s.unit.clone()))
        .collect()
}

// ---------------------------------------------------------------------------
// Per-state decision builders (architecture.md §9 tables)
// ---------------------------------------------------------------------------

fn decide_moderate(
    profile: &ProfileConfig,
    processes: &[ProcessInfo],
    services: &[ServiceInfo],
) -> PolicyDecision {
    if profile.max_allowed_risk >= ActionRisk::Moderate {
        let cg = collect_cgroup_targets(services, profile);
        if !cg.is_empty() {
            return PolicyDecision {
                intent: DecisionIntent::ApplyCgroupSystemdLimit,
                targets: cg,
                rationale: "Moderate pressure; applying cgroup limits to allowed services.".into(),
            };
        }
        let proc = collect_process_targets(processes, profile);
        if !proc.is_empty() {
            return PolicyDecision {
                intent: DecisionIntent::AdjustProcessPriority,
                targets: proc,
                rationale:
                    "Moderate pressure; adjusting priority of heavy non-protected processes.".into(),
            };
        }
    }
    PolicyDecision {
        intent: DecisionIntent::Recommend,
        targets: flagged_targets(processes, services),
        rationale: "Moderate pressure; recommending conservative adjustments.".into(),
    }
}

fn decide_high(
    profile: &ProfileConfig,
    processes: &[ProcessInfo],
    services: &[ServiceInfo],
) -> PolicyDecision {
    if profile.max_allowed_risk >= ActionRisk::Moderate {
        let cg = collect_cgroup_targets(services, profile);
        if !cg.is_empty() {
            return PolicyDecision {
                intent: DecisionIntent::ApplyCgroupSystemdLimit,
                targets: cg,
                rationale:
                    "High pressure; applying conservative cgroup limits to allowed services.".into(),
            };
        }
        let proc = collect_process_targets(processes, profile);
        if !proc.is_empty() {
            return PolicyDecision {
                intent: DecisionIntent::AdjustProcessPriority,
                targets: proc,
                rationale: "High pressure; adjusting priority of heavy non-protected processes."
                    .into(),
            };
        }
    }
    PolicyDecision {
        intent: DecisionIntent::Alert,
        targets: vec![Target::System],
        rationale:
            "High pressure; alerting — no safe actionable targets within current permissions."
                .into(),
    }
}

fn decide_critical(
    profile: &ProfileConfig,
    processes: &[ProcessInfo],
    services: &[ServiceInfo],
) -> PolicyDecision {
    // Aggressive: zram apply is the strongest allowed action (architecture.md §9 critical row).
    if profile.allow_zram_apply && profile.max_allowed_risk >= ActionRisk::Aggressive {
        return PolicyDecision {
            intent: DecisionIntent::ApplyZram,
            targets: vec![Target::System],
            rationale: "Critical pressure; applying zram within permitted limits.".into(),
        };
    }
    if profile.max_allowed_risk >= ActionRisk::Moderate {
        let cg = collect_cgroup_targets(services, profile);
        if !cg.is_empty() {
            return PolicyDecision {
                intent: DecisionIntent::ApplyCgroupSystemdLimit,
                targets: cg,
                rationale:
                    "Critical pressure; applying strongest allowed cgroup limits to allowed services."
                        .into(),
            };
        }
        let proc = collect_process_targets(processes, profile);
        if !proc.is_empty() {
            return PolicyDecision {
                intent: DecisionIntent::AdjustProcessPriority,
                targets: proc,
                rationale:
                    "Critical pressure; adjusting process priorities within permitted limits."
                        .into(),
            };
        }
    }
    PolicyDecision {
        intent: DecisionIntent::Alert,
        targets: vec![Target::System],
        rationale:
            "Critical pressure; alerting — no safe actionable targets within current permissions."
                .into(),
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Pure policy function: `(SystemState, ProfileConfig, findings) → PolicyDecision`.
///
/// Total over all inputs — never returns an error. Does not enforce safety
/// invariants; the safety layer (Phase 12) decides whether each resulting
/// action may actually execute (architecture.md §5.11, §5.13).
#[must_use]
pub fn decide(
    state: SystemState,
    profile: &ProfileConfig,
    processes: &[ProcessInfo],
    services: &[ServiceInfo],
) -> PolicyDecision {
    match &state {
        SystemState::Initializing => PolicyDecision {
            intent: DecisionIntent::ObserveOnly,
            targets: vec![Target::System],
            rationale: "Starting up and validating environment.".into(),
        },
        SystemState::Idle => PolicyDecision {
            intent: DecisionIntent::DoNothing,
            targets: vec![],
            rationale: "System is idle; nothing to do.".into(),
        },
        SystemState::Healthy => {
            if has_process_anomaly(processes) || has_service_anomaly(services) {
                PolicyDecision {
                    intent: DecisionIntent::Recommend,
                    targets: flagged_targets(processes, services),
                    rationale: "System healthy with minor anomalies; recommending review.".into(),
                }
            } else {
                PolicyDecision {
                    intent: DecisionIntent::ObserveOnly,
                    targets: vec![Target::System],
                    rationale: "System healthy; observing only.".into(),
                }
            }
        }
        SystemState::ModeratePressure => decide_moderate(profile, processes, services),
        SystemState::HighPressure => decide_high(profile, processes, services),
        SystemState::CriticalPressure => decide_critical(profile, processes, services),
        SystemState::Recovery => PolicyDecision {
            intent: DecisionIntent::Recommend,
            targets: vec![Target::System],
            rationale: "Pressure decreasing; relaxing temporary measures.".into(),
        },
        SystemState::Degraded => PolicyDecision {
            intent: DecisionIntent::LogOnly,
            targets: vec![Target::System],
            rationale: "Running with reduced capabilities; observe-only for missing features."
                .into(),
        },
        SystemState::ProtectedMode => PolicyDecision {
            intent: DecisionIntent::LogOnly,
            targets: vec![Target::System],
            rationale: "Protected mode: observing only until the flagged condition is resolved."
                .into(),
        },
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppConfig, ProfileName};
    use crate::processes::{ProcessFlag, ProcessInfo};
    use crate::profiles::resolve;
    use crate::services::{ServiceFlag, ServiceInfo};

    fn make_process(pid: u32, is_protected: bool, flags: Vec<ProcessFlag>) -> ProcessInfo {
        ProcessInfo {
            pid,
            comm: format!("proc{pid}"),
            cmdline: String::new(),
            cpu_pct: 0.0,
            rss_kb: 0,
            io_read_rate: 0.0,
            io_write_rate: 0.0,
            nice: 0,
            is_protected,
            flags,
        }
    }

    fn make_service(
        unit: &str,
        is_protected: bool,
        is_allowed: bool,
        flags: Vec<ServiceFlag>,
    ) -> ServiceInfo {
        ServiceInfo {
            unit: unit.to_string(),
            active_state: "active".to_string(),
            sub_state: "running".to_string(),
            is_protected,
            is_allowed,
            cpu_usage: 0,
            memory_current: 0,
            restarts: 0,
            flags,
        }
    }

    fn balanced() -> ProfileConfig {
        resolve(&ProfileName::Balanced, &AppConfig::default())
    }

    fn conservative() -> ProfileConfig {
        resolve(&ProfileName::Conservative, &AppConfig::default())
    }

    fn performance_aggressive() -> ProfileConfig {
        let mut cfg = AppConfig::default();
        cfg.global.allow_aggressive_actions = true;
        cfg.global.allow_zram_apply = true;
        resolve(&ProfileName::Performance, &cfg)
    }

    // ---- Terminal / low-activity states ----

    #[test]
    fn initializing_returns_observe_only() {
        let d = decide(SystemState::Initializing, &balanced(), &[], &[]);
        assert_eq!(d.intent, DecisionIntent::ObserveOnly);
    }

    #[test]
    fn idle_returns_do_nothing_with_empty_targets() {
        let d = decide(SystemState::Idle, &balanced(), &[], &[]);
        assert_eq!(d.intent, DecisionIntent::DoNothing);
        assert!(d.targets.is_empty());
    }

    #[test]
    fn recovery_returns_recommend() {
        let d = decide(SystemState::Recovery, &balanced(), &[], &[]);
        assert_eq!(d.intent, DecisionIntent::Recommend);
    }

    #[test]
    fn degraded_returns_log_only() {
        let d = decide(SystemState::Degraded, &balanced(), &[], &[]);
        assert_eq!(d.intent, DecisionIntent::LogOnly);
    }

    #[test]
    fn protected_mode_returns_log_only() {
        let d = decide(SystemState::ProtectedMode, &balanced(), &[], &[]);
        assert_eq!(d.intent, DecisionIntent::LogOnly);
    }

    // ---- Healthy ----

    #[test]
    fn healthy_no_anomaly_returns_observe_only() {
        let d = decide(SystemState::Healthy, &balanced(), &[], &[]);
        assert_eq!(d.intent, DecisionIntent::ObserveOnly);
    }

    #[test]
    fn healthy_with_process_anomaly_returns_recommend() {
        let procs = vec![make_process(100, false, vec![ProcessFlag::HighCpu])];
        let d = decide(SystemState::Healthy, &balanced(), &procs, &[]);
        assert_eq!(d.intent, DecisionIntent::Recommend);
        assert!(d.targets.contains(&Target::Process(100)));
    }

    #[test]
    fn healthy_with_service_anomaly_returns_recommend() {
        let svcs = vec![make_service(
            "myapp.service",
            false,
            true,
            vec![ServiceFlag::Failing],
        )];
        let d = decide(SystemState::Healthy, &balanced(), &[], &svcs);
        assert_eq!(d.intent, DecisionIntent::Recommend);
    }

    #[test]
    fn healthy_only_protected_anomaly_targets_system() {
        // Protected process flagged: Recommend is still returned (anomaly detected)
        // but the target falls back to System since no non-protected flagged procs exist.
        let procs = vec![make_process(1, true, vec![ProcessFlag::HighCpu])];
        let d = decide(SystemState::Healthy, &balanced(), &procs, &[]);
        assert_eq!(d.intent, DecisionIntent::Recommend);
        assert!(d.targets.contains(&Target::System));
    }

    // ---- ModeratePressure ----

    #[test]
    fn moderate_conservative_returns_recommend() {
        let d = decide(SystemState::ModeratePressure, &conservative(), &[], &[]);
        assert_eq!(d.intent, DecisionIntent::Recommend);
    }

    #[test]
    fn moderate_balanced_no_targets_returns_recommend() {
        let d = decide(SystemState::ModeratePressure, &balanced(), &[], &[]);
        assert_eq!(d.intent, DecisionIntent::Recommend);
    }

    #[test]
    fn moderate_balanced_allowed_service_returns_cgroup_limit() {
        let svcs = vec![make_service(
            "build.service",
            false,
            true,
            vec![ServiceFlag::HighMemory],
        )];
        let d = decide(SystemState::ModeratePressure, &balanced(), &[], &svcs);
        assert_eq!(d.intent, DecisionIntent::ApplyCgroupSystemdLimit);
        assert!(d.targets.contains(&Target::Service("build.service".into())));
    }

    #[test]
    fn moderate_protected_service_not_targeted_for_cgroup() {
        let svcs = vec![make_service(
            "syswarden.service",
            true,
            true,
            vec![ServiceFlag::HighMemory],
        )];
        let d = decide(SystemState::ModeratePressure, &balanced(), &[], &svcs);
        assert_ne!(d.intent, DecisionIntent::ApplyCgroupSystemdLimit);
        assert!(!d
            .targets
            .contains(&Target::Service("syswarden.service".into())));
    }

    #[test]
    fn moderate_non_allowed_service_not_targeted_for_cgroup() {
        let svcs = vec![make_service(
            "other.service",
            false,
            false,
            vec![ServiceFlag::HighMemory],
        )];
        let d = decide(SystemState::ModeratePressure, &balanced(), &[], &svcs);
        assert_ne!(d.intent, DecisionIntent::ApplyCgroupSystemdLimit);
    }

    #[test]
    fn moderate_balanced_heavy_cpu_process_returns_adjust_priority() {
        let procs = vec![make_process(200, false, vec![ProcessFlag::HighCpu])];
        let d = decide(SystemState::ModeratePressure, &balanced(), &procs, &[]);
        assert_eq!(d.intent, DecisionIntent::AdjustProcessPriority);
        assert!(d.targets.contains(&Target::Process(200)));
    }

    #[test]
    fn moderate_protected_process_never_targeted() {
        let procs = vec![make_process(1, true, vec![ProcessFlag::HighCpu])];
        let d = decide(SystemState::ModeratePressure, &balanced(), &procs, &[]);
        assert_ne!(d.intent, DecisionIntent::AdjustProcessPriority);
        assert!(!d.targets.contains(&Target::Process(1)));
    }

    #[test]
    fn moderate_conservative_never_adjusts_priority() {
        // Conservative profile: max_allowed_risk=Safe → AdjustProcessPriority blocked.
        let procs = vec![make_process(200, false, vec![ProcessFlag::HighCpu])];
        let d = decide(SystemState::ModeratePressure, &conservative(), &procs, &[]);
        assert_eq!(d.intent, DecisionIntent::Recommend);
    }

    // ---- HighPressure ----

    #[test]
    fn high_conservative_no_targets_returns_alert() {
        let d = decide(SystemState::HighPressure, &conservative(), &[], &[]);
        assert_eq!(d.intent, DecisionIntent::Alert);
    }

    #[test]
    fn high_balanced_no_targets_returns_alert() {
        let d = decide(SystemState::HighPressure, &balanced(), &[], &[]);
        assert_eq!(d.intent, DecisionIntent::Alert);
    }

    #[test]
    fn high_balanced_allowed_service_returns_cgroup_limit() {
        let svcs = vec![make_service(
            "build.service",
            false,
            true,
            vec![ServiceFlag::HighMemory],
        )];
        let d = decide(SystemState::HighPressure, &balanced(), &[], &svcs);
        assert_eq!(d.intent, DecisionIntent::ApplyCgroupSystemdLimit);
    }

    #[test]
    fn high_balanced_heavy_process_returns_adjust_priority() {
        let procs = vec![make_process(200, false, vec![ProcessFlag::HighCpu])];
        let d = decide(SystemState::HighPressure, &balanced(), &procs, &[]);
        assert_eq!(d.intent, DecisionIntent::AdjustProcessPriority);
        assert!(d.targets.contains(&Target::Process(200)));
    }

    // ---- CriticalPressure ----

    #[test]
    fn critical_conservative_returns_alert() {
        let d = decide(SystemState::CriticalPressure, &conservative(), &[], &[]);
        assert_eq!(d.intent, DecisionIntent::Alert);
    }

    #[test]
    fn critical_balanced_no_targets_returns_alert() {
        let d = decide(SystemState::CriticalPressure, &balanced(), &[], &[]);
        assert_eq!(d.intent, DecisionIntent::Alert);
    }

    #[test]
    fn critical_performance_zram_allowed_returns_apply_zram() {
        let d = decide(
            SystemState::CriticalPressure,
            &performance_aggressive(),
            &[],
            &[],
        );
        assert_eq!(d.intent, DecisionIntent::ApplyZram);
    }

    #[test]
    fn critical_balanced_allowed_service_returns_cgroup_limit() {
        let svcs = vec![make_service(
            "build.service",
            false,
            true,
            vec![ServiceFlag::HighMemory],
        )];
        let d = decide(SystemState::CriticalPressure, &balanced(), &[], &svcs);
        assert_eq!(d.intent, DecisionIntent::ApplyCgroupSystemdLimit);
    }

    #[test]
    fn critical_balanced_heavy_process_returns_adjust_priority() {
        let procs = vec![make_process(200, false, vec![ProcessFlag::HighCpu])];
        let d = decide(SystemState::CriticalPressure, &balanced(), &procs, &[]);
        assert_eq!(d.intent, DecisionIntent::AdjustProcessPriority);
    }

    // ---- Allowlist/protection invariants under pressure ----

    #[test]
    fn cgroup_targets_exclude_protected_and_non_allowed() {
        let svcs = vec![
            make_service(
                "allowed.service",
                false,
                true,
                vec![ServiceFlag::HighMemory],
            ),
            make_service(
                "protected.service",
                true,
                true,
                vec![ServiceFlag::HighMemory],
            ),
            make_service(
                "not-allowed.service",
                false,
                false,
                vec![ServiceFlag::HighMemory],
            ),
        ];
        let d = decide(SystemState::HighPressure, &balanced(), &[], &svcs);
        assert_eq!(d.intent, DecisionIntent::ApplyCgroupSystemdLimit);
        assert!(d
            .targets
            .contains(&Target::Service("allowed.service".into())));
        assert!(!d
            .targets
            .contains(&Target::Service("protected.service".into())));
        assert!(!d
            .targets
            .contains(&Target::Service("not-allowed.service".into())));
    }

    #[test]
    fn process_targets_exclude_protected() {
        let procs = vec![
            make_process(100, false, vec![ProcessFlag::HighCpu]),
            make_process(1, true, vec![ProcessFlag::HighCpu]),
        ];
        let d = decide(SystemState::HighPressure, &balanced(), &procs, &[]);
        assert_eq!(d.intent, DecisionIntent::AdjustProcessPriority);
        assert!(d.targets.contains(&Target::Process(100)));
        assert!(!d.targets.contains(&Target::Process(1)));
    }

    #[test]
    fn all_decisions_have_non_empty_rationale() {
        let states = [
            SystemState::Initializing,
            SystemState::Idle,
            SystemState::Healthy,
            SystemState::ModeratePressure,
            SystemState::HighPressure,
            SystemState::CriticalPressure,
            SystemState::Recovery,
            SystemState::Degraded,
            SystemState::ProtectedMode,
        ];
        for state in states {
            let d = decide(state, &balanced(), &[], &[]);
            assert!(!d.rationale.is_empty(), "empty rationale for {state:?}");
        }
    }
}
