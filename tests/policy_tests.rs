//! Integration tests for the policy decision engine (planning.md §7).

use syswarden::config::{self, AppConfig, ProfileName};
use syswarden::policy::{decide, DecisionIntent};
use syswarden::pressure::SystemState;
use syswarden::profiles;

fn conservative() -> (syswarden::profiles::ProfileConfig, AppConfig) {
    let cfg = config::defaults();
    let profile = profiles::resolve(&ProfileName::Conservative, &cfg);
    (profile, cfg)
}

fn balanced() -> (syswarden::profiles::ProfileConfig, AppConfig) {
    let cfg = config::defaults();
    let profile = profiles::resolve(&ProfileName::Balanced, &cfg);
    (profile, cfg)
}

// ---------------------------------------------------------------------------
// Per-state intent coverage (conservative profile, no anomalies)
// ---------------------------------------------------------------------------

#[test]
fn initializing_returns_observe_only() {
    let (p, _) = conservative();
    let d = decide(SystemState::Initializing, &p, &[], &[]);
    assert_eq!(d.intent, DecisionIntent::ObserveOnly);
}

#[test]
fn idle_returns_do_nothing() {
    let (p, _) = conservative();
    let d = decide(SystemState::Idle, &p, &[], &[]);
    assert_eq!(d.intent, DecisionIntent::DoNothing);
}

#[test]
fn healthy_no_anomaly_returns_observe_only() {
    let (p, _) = conservative();
    let d = decide(SystemState::Healthy, &p, &[], &[]);
    assert_eq!(d.intent, DecisionIntent::ObserveOnly);
}

#[test]
fn degraded_returns_log_only() {
    let (p, _) = conservative();
    let d = decide(SystemState::Degraded, &p, &[], &[]);
    assert_eq!(d.intent, DecisionIntent::LogOnly);
}

#[test]
fn protected_mode_returns_log_only() {
    let (p, _) = conservative();
    let d = decide(SystemState::ProtectedMode, &p, &[], &[]);
    assert_eq!(d.intent, DecisionIntent::LogOnly);
}

#[test]
fn recovery_returns_recommend() {
    let (p, _) = conservative();
    let d = decide(SystemState::Recovery, &p, &[], &[]);
    assert_eq!(d.intent, DecisionIntent::Recommend);
}

// ---------------------------------------------------------------------------
// Moderate / High / Critical — conservative profile → Recommend or Alert
// ---------------------------------------------------------------------------

#[test]
fn moderate_pressure_conservative_no_anomaly_returns_recommend() {
    let (p, _) = conservative();
    let d = decide(SystemState::ModeratePressure, &p, &[], &[]);
    // Conservative profile with no anomalies → Recommend (no actual action).
    assert_eq!(d.intent, DecisionIntent::Recommend);
}

#[test]
fn high_pressure_conservative_no_anomaly_returns_alert() {
    let (p, _) = conservative();
    let d = decide(SystemState::HighPressure, &p, &[], &[]);
    assert_eq!(d.intent, DecisionIntent::Alert);
}

#[test]
fn critical_pressure_conservative_no_anomaly_returns_alert() {
    let (p, _) = conservative();
    let d = decide(SystemState::CriticalPressure, &p, &[], &[]);
    assert_eq!(d.intent, DecisionIntent::Alert);
}

// ---------------------------------------------------------------------------
// Balanced profile — moderate pressure → may attempt cgroup/process actions
// ---------------------------------------------------------------------------

#[test]
fn moderate_pressure_balanced_no_anomaly_returns_recommend() {
    let (p, _) = balanced();
    let d = decide(SystemState::ModeratePressure, &p, &[], &[]);
    // Balanced with no flagged targets → Recommend.
    assert_eq!(d.intent, DecisionIntent::Recommend);
}

// ---------------------------------------------------------------------------
// rationale is always non-empty
// ---------------------------------------------------------------------------

#[test]
fn all_states_produce_non_empty_rationale() {
    let (p, _) = conservative();
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
        let d = decide(state, &p, &[], &[]);
        assert!(
            !d.rationale.is_empty(),
            "rationale must not be empty for state {state:?}"
        );
    }
}
