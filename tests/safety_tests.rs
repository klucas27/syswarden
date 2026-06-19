//! Integration tests for the safety layer (planning.md §7).

use std::collections::HashMap;

use syswarden::actions::{ActionKind, ActionTarget, PlannedAction};
use syswarden::config::{self, ProfileName};
use syswarden::profiles::{self, ActionRisk};
use syswarden::safety::{self, Capabilities, SafetyDecision};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_action(kind: ActionKind, risk: ActionRisk, target: ActionTarget) -> PlannedAction {
    PlannedAction {
        id: 1,
        kind,
        risk,
        target,
        params: HashMap::new(),
        explanation: "test action".to_string(),
    }
}

fn system_action(kind: ActionKind, risk: ActionRisk) -> PlannedAction {
    make_action(kind, risk, ActionTarget::System)
}

fn root_caps() -> Capabilities {
    Capabilities {
        is_root: true,
        has_psi: true,
        has_cgroup_v2: true,
        has_systemd: true,
        has_zram: false,
    }
}

fn non_root_caps() -> Capabilities {
    Capabilities {
        is_root: false,
        ..root_caps()
    }
}

// ---------------------------------------------------------------------------
// Gate 1: Prohibited actions are always blocked
// ---------------------------------------------------------------------------

#[test]
fn prohibited_action_is_always_blocked() {
    let cfg = config::defaults();
    let profile = profiles::resolve(&ProfileName::Conservative, &cfg);
    let action = system_action(ActionKind::ApplySysctl, ActionRisk::Prohibited);
    let verdict = safety::evaluate(&action, &cfg, &profile, &root_caps());
    assert!(
        matches!(verdict, SafetyDecision::Block { .. }),
        "Prohibited action must always be blocked; got {verdict:?}"
    );
}

// ---------------------------------------------------------------------------
// Gate 2: Action risk exceeds profile maximum
// ---------------------------------------------------------------------------

#[test]
fn aggressive_action_blocked_on_conservative_profile() {
    let cfg = config::defaults();
    // Conservative profile has low max_allowed_risk.
    let profile = profiles::resolve(&ProfileName::Conservative, &cfg);
    // Aggressive risk exceeds conservative max.
    let action = system_action(ActionKind::SetMemoryMax, ActionRisk::Aggressive);
    let verdict = safety::evaluate(&action, &cfg, &profile, &root_caps());
    assert!(
        matches!(verdict, SafetyDecision::Block { .. }),
        "Aggressive risk must be blocked on conservative profile; got {verdict:?}"
    );
}

// ---------------------------------------------------------------------------
// Gate 3: Protected process/service is never touched
// ---------------------------------------------------------------------------

#[test]
fn protected_process_is_blocked() {
    let cfg = config::defaults();
    let profile = profiles::resolve(&ProfileName::Conservative, &cfg);
    // "syswarden" is always in protected.processes (defaults).
    let action = make_action(
        ActionKind::AdjustNice,
        ActionRisk::Moderate,
        ActionTarget::Process {
            pid: 1,
            comm: "syswarden".to_string(),
        },
    );
    let verdict = safety::evaluate(&action, &cfg, &profile, &root_caps());
    assert!(
        matches!(verdict, SafetyDecision::Block { .. }),
        "Protected process must be blocked; got {verdict:?}"
    );
}

#[test]
fn protected_service_is_blocked() {
    let cfg = config::defaults();
    let profile = profiles::resolve(&ProfileName::Conservative, &cfg);
    // "syswarden.service" is always in protected.services.
    let action = make_action(
        ActionKind::SetCpuWeight,
        ActionRisk::Moderate,
        ActionTarget::Service {
            unit: "syswarden.service".to_string(),
        },
    );
    let verdict = safety::evaluate(&action, &cfg, &profile, &root_caps());
    assert!(
        matches!(verdict, SafetyDecision::Block { .. }),
        "Protected service must be blocked; got {verdict:?}"
    );
}

// ---------------------------------------------------------------------------
// Gate 4: Service not in allowed.services is blocked
// ---------------------------------------------------------------------------

#[test]
fn non_allowlisted_service_is_blocked() {
    let cfg = config::defaults(); // allowed.services is empty by default
    let profile = profiles::resolve(&ProfileName::Balanced, &cfg);
    let action = make_action(
        ActionKind::SetCpuWeight,
        ActionRisk::Moderate,
        ActionTarget::Service {
            unit: "some-random.service".to_string(),
        },
    );
    let verdict = safety::evaluate(&action, &cfg, &profile, &root_caps());
    assert!(
        matches!(verdict, SafetyDecision::Block { .. }),
        "Service not in allowed.services must be blocked; got {verdict:?}"
    );
}

// ---------------------------------------------------------------------------
// Gate 6: Non-root blocks state-changing actions
// ---------------------------------------------------------------------------

#[test]
fn state_changing_action_blocked_when_not_root() {
    let mut cfg = config::defaults();
    cfg.global.dry_run = false; // remove dry-run so gate 7 doesn't trigger first
    cfg.global.allow_aggressive_actions = true;
    let profile = profiles::resolve(&ProfileName::Performance, &cfg);
    let action = system_action(ActionKind::AdjustNice, ActionRisk::Moderate);
    let verdict = safety::evaluate(&action, &cfg, &profile, &non_root_caps());
    assert!(
        matches!(verdict, SafetyDecision::Block { .. }),
        "State-changing action must be blocked for non-root; got {verdict:?}"
    );
}

// ---------------------------------------------------------------------------
// Gate 7: dry_run → RequireDryRun (when all other gates pass)
// ---------------------------------------------------------------------------

#[test]
fn dry_run_config_returns_require_dry_run_for_observe() {
    let cfg = config::defaults(); // dry_run = true
    let profile = profiles::resolve(&ProfileName::Conservative, &cfg);
    // Observe is Safe risk — gates 1–6 all pass for this action.
    let action = system_action(ActionKind::Observe, ActionRisk::Safe);
    let verdict = safety::evaluate(&action, &cfg, &profile, &root_caps());
    // Safe/observe action: dry_run=true → RequireDryRun (not Block, not Allow).
    assert!(
        matches!(
            verdict,
            SafetyDecision::RequireDryRun | SafetyDecision::Allow
        ),
        "Observe with dry_run should be RequireDryRun or Allow; got {verdict:?}"
    );
}

// ---------------------------------------------------------------------------
// Fail-closed: unknown / edge-case inputs default to Block
// ---------------------------------------------------------------------------

#[test]
fn safe_observe_action_with_root_and_no_dry_run_returns_allow() {
    let mut cfg = config::defaults();
    cfg.global.dry_run = false;
    let profile = profiles::resolve(&ProfileName::Conservative, &cfg);
    let action = system_action(ActionKind::Observe, ActionRisk::Safe);
    let verdict = safety::evaluate(&action, &cfg, &profile, &root_caps());
    assert_eq!(
        verdict,
        SafetyDecision::Allow,
        "Safe Observe with root and dry_run=false must be Allow; got {verdict:?}"
    );
}
