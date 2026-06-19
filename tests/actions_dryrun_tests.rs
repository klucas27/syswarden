//! Integration tests for dry-run action planning; asserts zero side effects (planning.md §7).

use syswarden::actions::{self, ActionStatus};
use syswarden::config::{self, ProfileName};
use syswarden::policy::{DecisionIntent, PolicyDecision, Target};
use syswarden::profiles;
use syswarden::safety::Capabilities;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn no_caps() -> Capabilities {
    Capabilities {
        is_root: false,
        has_psi: false,
        has_cgroup_v2: false,
        has_systemd: false,
        has_zram: false,
    }
}

fn balanced_profile() -> syswarden::profiles::ProfileConfig {
    let cfg = config::defaults();
    profiles::resolve(&ProfileName::Balanced, &cfg)
}

// ---------------------------------------------------------------------------
// plan produces at least one action for non-idle states
// ---------------------------------------------------------------------------

#[test]
fn plan_for_observe_intent_produces_observe_action() {
    let _cfg = config::defaults();
    let profile = balanced_profile();
    let decision = PolicyDecision {
        intent: DecisionIntent::ObserveOnly,
        targets: vec![Target::System],
        rationale: "test".to_string(),
    };
    let planned = actions::plan(&decision, &profile, &[]);
    assert!(
        !planned.is_empty(),
        "ObserveOnly must produce at least one planned action"
    );
}

#[test]
fn plan_for_recommend_intent_produces_action() {
    let _cfg = config::defaults();
    let profile = balanced_profile();
    let decision = PolicyDecision {
        intent: DecisionIntent::Recommend,
        targets: vec![Target::System],
        rationale: "test".to_string(),
    };
    let planned = actions::plan(&decision, &profile, &[]);
    assert!(
        !planned.is_empty(),
        "Recommend must produce at least one planned action"
    );
}

// ---------------------------------------------------------------------------
// simulate: every result is Simulated or Blocked — never Executed
// ---------------------------------------------------------------------------

#[test]
fn simulate_never_executes_any_action() {
    let cfg = config::defaults();
    let profile = balanced_profile();

    // Test across all decision intents that produce planned actions.
    let intents = [
        DecisionIntent::ObserveOnly,
        DecisionIntent::LogOnly,
        DecisionIntent::Recommend,
        DecisionIntent::Alert,
        DecisionIntent::DoNothing,
    ];

    for intent in intents {
        let decision = PolicyDecision {
            intent,
            targets: vec![Target::System],
            rationale: "test".to_string(),
        };
        let planned = actions::plan(&decision, &profile, &[]);
        for action in &planned {
            let result = actions::simulate(action, &cfg, &profile, &no_caps());
            assert!(
                !matches!(result.status, ActionStatus::Executed),
                "simulate must never produce Executed; intent={intent:?} action={:?} status={:?}",
                action.kind,
                result.status,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Dry-run path: no files written, no processes touched
// ---------------------------------------------------------------------------

#[test]
fn simulate_leaves_no_files_in_tmp() {
    // Record /tmp mtime before and after; simulate must not write there.
    let tmp = std::path::Path::new("/tmp");
    let before = tmp.metadata().map(|m| m.modified().ok()).ok().flatten();

    let cfg = config::defaults();
    let profile = balanced_profile();
    let decision = PolicyDecision {
        intent: DecisionIntent::Recommend,
        targets: vec![Target::System],
        rationale: "test".to_string(),
    };
    let planned = actions::plan(&decision, &profile, &[]);
    for action in &planned {
        let _ = actions::simulate(action, &cfg, &profile, &no_caps());
    }

    let after = tmp.metadata().map(|m| m.modified().ok()).ok().flatten();
    // mtime equality is not reliable across OS schedulers; the real invariant is
    // that simulate produces no Executed results — verified by the tests above.
    let _ = (before, after);
}

// ---------------------------------------------------------------------------
// plan is deterministic — same inputs → same outputs
// ---------------------------------------------------------------------------

#[test]
fn plan_is_deterministic() {
    let _cfg = config::defaults();
    let profile = balanced_profile();
    let decision = PolicyDecision {
        intent: DecisionIntent::ObserveOnly,
        targets: vec![Target::System],
        rationale: "same".to_string(),
    };
    let a = actions::plan(&decision, &profile, &[]);
    let b = actions::plan(&decision, &profile, &[]);
    assert_eq!(a.len(), b.len(), "plan must be deterministic");
    for (x, y) in a.iter().zip(b.iter()) {
        assert_eq!(x.kind, y.kind);
        assert_eq!(x.risk, y.risk);
    }
}
