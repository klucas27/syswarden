//! Action planner, simulator, and (v0.2+) executor; every path gates through `safety` (architecture.md §5.12, §10).
#![allow(dead_code)]

use std::collections::HashMap;

use crate::config::AppConfig;
use crate::policy::{DecisionIntent, PolicyDecision, Target};
use crate::processes::ProcessInfo;
use crate::profiles::{ActionRisk, ProfileConfig};
use crate::safety::{self, Capabilities, SafetyDecision};

// ---------------------------------------------------------------------------
// ActionKind
// ---------------------------------------------------------------------------

/// Concrete action type (architecture.md §15).
///
/// Declaration order matches §10's risk grouping: observe → moderate → aggressive → prohibited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionKind {
    // Safe — no system state change.
    Observe,
    Log,
    Report,
    Recommend,
    // Moderate — requires profile permission + allowed targets.
    CreateBackup,
    AdjustNice,
    AdjustIonice,
    SetCpuWeight,
    SetIoWeight,
    SetMemoryHigh,
    // Aggressive — requires `allow_aggressive_actions` + specific flags.
    SetMemoryMax,
    RestartService,
    StopService,
    ApplyZram,
    ApplySysctl,
}

// ---------------------------------------------------------------------------
// ActionTarget
// ---------------------------------------------------------------------------

/// What a `PlannedAction` operates on (architecture.md §15).
///
/// Carries identifying information so the safety layer can check protected lists
/// and allowlists without trusting the policy engine's own flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionTarget {
    Process { pid: u32, comm: String },
    Service { unit: String },
    System,
}

// ---------------------------------------------------------------------------
// ActionStatus
// ---------------------------------------------------------------------------

/// Lifecycle state of an action (architecture.md §15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionStatus {
    Planned,
    Simulated,
    Blocked,
    Executed,
    Failed,
    RolledBack,
}

// ---------------------------------------------------------------------------
// PlannedAction / ActionResult
// ---------------------------------------------------------------------------

/// A concrete intended change, ready for safety evaluation and execution (architecture.md §15).
#[derive(Debug, Clone)]
pub struct PlannedAction {
    pub id: u64,
    pub kind: ActionKind,
    pub risk: ActionRisk,
    pub target: ActionTarget,
    pub params: HashMap<String, String>,
    pub explanation: String,
}

/// Outcome of a planned action (architecture.md §15).
#[derive(Debug, Clone)]
pub struct ActionResult {
    pub action_id: u64,
    pub status: ActionStatus,
    pub message: String,
    pub rollback_id: Option<u64>,
}

// ---------------------------------------------------------------------------
// Planner
// ---------------------------------------------------------------------------

/// Translate a `PolicyDecision` into a sequence of `PlannedAction`s (architecture.md §5.12).
///
/// Pure mapping — no side effects, no safety checks. Each result must be passed through
/// `simulate` (v0.1) or `execute` (v0.2+) before anything touches the system.
#[must_use]
#[allow(clippy::too_many_lines)] // Exhaustive match over all DecisionIntent variants (planning.md §5).
pub fn plan(
    decision: &PolicyDecision,
    profile: &ProfileConfig,
    processes: &[ProcessInfo],
) -> Vec<PlannedAction> {
    let mut out: Vec<PlannedAction> = Vec::new();
    let mut id: u64 = 1;

    match decision.intent {
        DecisionIntent::DoNothing => {}

        DecisionIntent::ObserveOnly => {
            out.push(safe_action(
                id,
                ActionKind::Observe,
                ActionTarget::System,
                &decision.rationale,
            ));
        }

        DecisionIntent::LogOnly
        | DecisionIntent::EnterProtectedMode
        | DecisionIntent::BlockAction => {
            out.push(safe_action(
                id,
                ActionKind::Log,
                ActionTarget::System,
                &decision.rationale,
            ));
        }

        DecisionIntent::Alert => {
            out.push(safe_action(
                id,
                ActionKind::Report,
                ActionTarget::System,
                &decision.rationale,
            ));
        }

        DecisionIntent::Recommend | DecisionIntent::RecommendZram => {
            for target in &decision.targets {
                out.push(safe_action(
                    id,
                    ActionKind::Recommend,
                    policy_target_to_action_target(target, processes),
                    &decision.rationale,
                ));
                id += 1;
            }
        }

        DecisionIntent::AdjustProcessPriority => {
            for target in &decision.targets {
                if let Target::Process(pid) = target {
                    let act_target = ActionTarget::Process {
                        pid: *pid,
                        comm: lookup_comm(*pid, processes),
                    };
                    if profile.allow_nice {
                        let mut params = HashMap::new();
                        params.insert("nice".into(), "5".into());
                        out.push(PlannedAction {
                            id,
                            kind: ActionKind::AdjustNice,
                            risk: ActionRisk::Moderate,
                            target: act_target.clone(),
                            params,
                            explanation: decision.rationale.clone(),
                        });
                        id += 1;
                    }
                    if profile.allow_ionice {
                        let mut params = HashMap::new();
                        params.insert("class".into(), "best-effort".into());
                        params.insert("level".into(), "4".into());
                        out.push(PlannedAction {
                            id,
                            kind: ActionKind::AdjustIonice,
                            risk: ActionRisk::Moderate,
                            target: act_target,
                            params,
                            explanation: decision.rationale.clone(),
                        });
                        id += 1;
                    }
                }
            }
        }

        DecisionIntent::ApplyCgroupSystemdLimit => {
            for target in &decision.targets {
                if let Target::Service(unit) = target {
                    let act_target = ActionTarget::Service { unit: unit.clone() };
                    if profile.allow_cpu_weight {
                        let mut params = HashMap::new();
                        params.insert("weight".into(), "50".into());
                        out.push(PlannedAction {
                            id,
                            kind: ActionKind::SetCpuWeight,
                            risk: ActionRisk::Moderate,
                            target: act_target.clone(),
                            params,
                            explanation: decision.rationale.clone(),
                        });
                        id += 1;
                    }
                    if profile.allow_memory_high {
                        let mut params = HashMap::new();
                        params.insert("limit".into(), "auto".into());
                        out.push(PlannedAction {
                            id,
                            kind: ActionKind::SetMemoryHigh,
                            risk: ActionRisk::Moderate,
                            target: act_target.clone(),
                            params,
                            explanation: decision.rationale.clone(),
                        });
                        id += 1;
                    }
                    if profile.allow_io_weight {
                        let mut params = HashMap::new();
                        params.insert("weight".into(), "50".into());
                        out.push(PlannedAction {
                            id,
                            kind: ActionKind::SetIoWeight,
                            risk: ActionRisk::Moderate,
                            target: act_target,
                            params,
                            explanation: decision.rationale.clone(),
                        });
                        id += 1;
                    }
                }
            }
        }

        DecisionIntent::ApplyZram => {
            out.push(PlannedAction {
                id,
                kind: ActionKind::ApplyZram,
                risk: ActionRisk::Aggressive,
                target: ActionTarget::System,
                params: HashMap::new(),
                explanation: decision.rationale.clone(),
            });
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Simulator
// ---------------------------------------------------------------------------

/// Evaluate and simulate a `PlannedAction` without executing it (v0.1 only).
///
/// Always calls `safety::evaluate` first (architecture.md §5.13, §17). Both `Allow`
/// and `RequireDryRun` verdicts produce `Simulated` — no system state is changed.
#[must_use]
pub fn simulate(
    action: &PlannedAction,
    config: &AppConfig,
    profile: &ProfileConfig,
    caps: &Capabilities,
) -> ActionResult {
    match safety::evaluate(action, config, profile, caps) {
        SafetyDecision::Block { reason } => ActionResult {
            action_id: action.id,
            status: ActionStatus::Blocked,
            message: reason,
            rollback_id: None,
        },
        SafetyDecision::Allow | SafetyDecision::RequireDryRun => ActionResult {
            action_id: action.id,
            status: ActionStatus::Simulated,
            message: format!(
                "[DRY-RUN] {:?} on {:?} — {}",
                action.kind, action.target, action.explanation
            ),
            rollback_id: None,
        },
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn safe_action(
    id: u64,
    kind: ActionKind,
    target: ActionTarget,
    explanation: &str,
) -> PlannedAction {
    PlannedAction {
        id,
        kind,
        risk: ActionRisk::Safe,
        target,
        params: HashMap::new(),
        explanation: explanation.to_string(),
    }
}

fn lookup_comm(pid: u32, processes: &[ProcessInfo]) -> String {
    processes
        .iter()
        .find(|p| p.pid == pid)
        .map_or_else(|| format!("pid:{pid}"), |p| p.comm.clone())
}

fn policy_target_to_action_target(target: &Target, processes: &[ProcessInfo]) -> ActionTarget {
    match target {
        Target::Process(pid) => ActionTarget::Process {
            pid: *pid,
            comm: lookup_comm(*pid, processes),
        },
        Target::Service(unit) => ActionTarget::Service { unit: unit.clone() },
        Target::System => ActionTarget::System,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppConfig, ProfileName};
    use crate::policy::{DecisionIntent, PolicyDecision, Target};
    use crate::processes::{ProcessFlag, ProcessInfo};
    use crate::profiles::resolve;
    use crate::safety::Capabilities;

    // --- Helpers ---

    fn decision(intent: DecisionIntent, targets: Vec<Target>) -> PolicyDecision {
        PolicyDecision {
            intent,
            targets,
            rationale: "test rationale".into(),
        }
    }

    fn make_process(pid: u32, comm: &str) -> ProcessInfo {
        ProcessInfo {
            pid,
            comm: comm.to_string(),
            cmdline: String::new(),
            cpu_pct: 0.0,
            rss_kb: 0,
            io_read_rate: 0.0,
            io_write_rate: 0.0,
            nice: 0,
            is_protected: false,
            flags: vec![ProcessFlag::HighCpu],
        }
    }

    fn balanced() -> ProfileConfig {
        resolve(&ProfileName::Balanced, &AppConfig::default())
    }

    fn developer() -> ProfileConfig {
        resolve(&ProfileName::Developer, &AppConfig::default())
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

    fn nonroot_caps() -> Capabilities {
        Capabilities {
            is_root: false,
            ..root_caps()
        }
    }

    fn open_config(unit: &str) -> AppConfig {
        let mut cfg = AppConfig::default();
        cfg.global.dry_run = false;
        cfg.global.allow_aggressive_actions = true;
        cfg.global.allow_zram_apply = true;
        cfg.allowed.services.push(unit.to_string());
        cfg
    }

    fn permissive_profile() -> ProfileConfig {
        ProfileConfig {
            idle_interval_secs: 6,
            pressure_interval_secs: 2,
            max_allowed_risk: ActionRisk::Aggressive,
            allow_nice: true,
            allow_ionice: true,
            allow_cpu_weight: true,
            allow_io_weight: true,
            allow_memory_high: true,
            allow_memory_max: true,
            allow_service_restart: true,
            allow_service_stop: true,
            allow_zram_apply: true,
        }
    }

    // --- plan: safe/observe intents ---

    #[test]
    fn plan_do_nothing_returns_empty() {
        let d = decision(DecisionIntent::DoNothing, vec![]);
        assert!(plan(&d, &balanced(), &[]).is_empty());
    }

    #[test]
    fn plan_observe_only_returns_one_observe_action() {
        let d = decision(DecisionIntent::ObserveOnly, vec![Target::System]);
        let actions = plan(&d, &balanced(), &[]);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].kind, ActionKind::Observe);
        assert_eq!(actions[0].risk, ActionRisk::Safe);
        assert_eq!(actions[0].target, ActionTarget::System);
    }

    #[test]
    fn plan_log_only_returns_log_action() {
        let d = decision(DecisionIntent::LogOnly, vec![Target::System]);
        let actions = plan(&d, &balanced(), &[]);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].kind, ActionKind::Log);
        assert_eq!(actions[0].risk, ActionRisk::Safe);
    }

    #[test]
    fn plan_alert_returns_report_action() {
        let d = decision(DecisionIntent::Alert, vec![Target::System]);
        let actions = plan(&d, &balanced(), &[]);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].kind, ActionKind::Report);
        assert_eq!(actions[0].risk, ActionRisk::Safe);
    }

    #[test]
    fn plan_enter_protected_mode_returns_log_action() {
        let d = decision(DecisionIntent::EnterProtectedMode, vec![Target::System]);
        let actions = plan(&d, &balanced(), &[]);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].kind, ActionKind::Log);
    }

    #[test]
    fn plan_block_action_returns_log_action() {
        let d = decision(DecisionIntent::BlockAction, vec![Target::System]);
        let actions = plan(&d, &balanced(), &[]);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].kind, ActionKind::Log);
    }

    // --- plan: recommend ---

    #[test]
    fn plan_recommend_maps_each_target_to_recommend_action() {
        let d = decision(
            DecisionIntent::Recommend,
            vec![Target::Process(100), Target::Process(200)],
        );
        let procs = vec![make_process(100, "proc100"), make_process(200, "proc200")];
        let actions = plan(&d, &balanced(), &procs);
        assert_eq!(actions.len(), 2);
        assert!(actions.iter().all(|a| a.kind == ActionKind::Recommend));
        assert!(actions.iter().all(|a| a.risk == ActionRisk::Safe));
    }

    #[test]
    fn plan_recommend_zram_produces_recommend_on_system() {
        let d = decision(DecisionIntent::RecommendZram, vec![Target::System]);
        let actions = plan(&d, &balanced(), &[]);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].kind, ActionKind::Recommend);
        assert_eq!(actions[0].target, ActionTarget::System);
    }

    // --- plan: AdjustProcessPriority ---

    #[test]
    fn plan_adjust_priority_nice_only() {
        let mut profile = permissive_profile();
        profile.allow_ionice = false;
        let d = decision(
            DecisionIntent::AdjustProcessPriority,
            vec![Target::Process(42)],
        );
        let procs = vec![make_process(42, "myapp")];
        let actions = plan(&d, &profile, &procs);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].kind, ActionKind::AdjustNice);
        assert_eq!(actions[0].params["nice"], "5");
    }

    #[test]
    fn plan_adjust_priority_ionice_only() {
        let mut profile = permissive_profile();
        profile.allow_nice = false;
        let d = decision(
            DecisionIntent::AdjustProcessPriority,
            vec![Target::Process(42)],
        );
        let procs = vec![make_process(42, "myapp")];
        let actions = plan(&d, &profile, &procs);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].kind, ActionKind::AdjustIonice);
        assert_eq!(actions[0].params["class"], "best-effort");
    }

    #[test]
    fn plan_adjust_priority_both_flags_emits_two_actions_per_process() {
        let d = decision(
            DecisionIntent::AdjustProcessPriority,
            vec![Target::Process(10), Target::Process(20)],
        );
        let procs = vec![make_process(10, "a"), make_process(20, "b")];
        let actions = plan(&d, &permissive_profile(), &procs);
        assert_eq!(actions.len(), 4); // 2 procs × (nice + ionice)
        assert_eq!(
            actions
                .iter()
                .filter(|a| a.kind == ActionKind::AdjustNice)
                .count(),
            2
        );
        assert_eq!(
            actions
                .iter()
                .filter(|a| a.kind == ActionKind::AdjustIonice)
                .count(),
            2
        );
    }

    #[test]
    fn plan_adjust_priority_no_permissions_returns_empty() {
        let mut profile = permissive_profile();
        profile.allow_nice = false;
        profile.allow_ionice = false;
        let d = decision(
            DecisionIntent::AdjustProcessPriority,
            vec![Target::Process(42)],
        );
        assert!(plan(&d, &profile, &[]).is_empty());
    }

    // --- plan: ApplyCgroupSystemdLimit ---

    #[test]
    fn plan_cgroup_cpu_weight_only() {
        let mut profile = permissive_profile();
        profile.allow_memory_high = false;
        profile.allow_io_weight = false;
        let d = decision(
            DecisionIntent::ApplyCgroupSystemdLimit,
            vec![Target::Service("app.service".into())],
        );
        let actions = plan(&d, &profile, &[]);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].kind, ActionKind::SetCpuWeight);
        assert_eq!(actions[0].params["weight"], "50");
    }

    #[test]
    fn plan_cgroup_memory_high_only() {
        let mut profile = permissive_profile();
        profile.allow_cpu_weight = false;
        profile.allow_io_weight = false;
        let d = decision(
            DecisionIntent::ApplyCgroupSystemdLimit,
            vec![Target::Service("app.service".into())],
        );
        let actions = plan(&d, &profile, &[]);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].kind, ActionKind::SetMemoryHigh);
        assert_eq!(actions[0].params["limit"], "auto");
    }

    #[test]
    fn plan_cgroup_all_three_flags_emits_three_actions() {
        let d = decision(
            DecisionIntent::ApplyCgroupSystemdLimit,
            vec![Target::Service("app.service".into())],
        );
        let actions = plan(&d, &permissive_profile(), &[]);
        assert_eq!(actions.len(), 3);
        let kinds: Vec<_> = actions.iter().map(|a| &a.kind).collect();
        assert!(kinds.contains(&&ActionKind::SetCpuWeight));
        assert!(kinds.contains(&&ActionKind::SetMemoryHigh));
        assert!(kinds.contains(&&ActionKind::SetIoWeight));
    }

    #[test]
    fn plan_cgroup_two_services_emits_actions_for_each() {
        let d = decision(
            DecisionIntent::ApplyCgroupSystemdLimit,
            vec![
                Target::Service("a.service".into()),
                Target::Service("b.service".into()),
            ],
        );
        let mut profile = permissive_profile();
        profile.allow_io_weight = false;
        profile.allow_memory_high = false;
        let actions = plan(&d, &profile, &[]);
        assert_eq!(actions.len(), 2); // 2 services × cpu_weight only
    }

    // --- plan: ApplyZram ---

    #[test]
    fn plan_apply_zram_returns_aggressive_action_on_system() {
        let d = decision(DecisionIntent::ApplyZram, vec![Target::System]);
        let actions = plan(&d, &permissive_profile(), &[]);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].kind, ActionKind::ApplyZram);
        assert_eq!(actions[0].risk, ActionRisk::Aggressive);
        assert_eq!(actions[0].target, ActionTarget::System);
    }

    // --- plan: structural invariants ---

    #[test]
    fn plan_ids_are_unique_and_start_at_one() {
        let d = decision(
            DecisionIntent::AdjustProcessPriority,
            vec![Target::Process(1), Target::Process(2), Target::Process(3)],
        );
        let procs = vec![
            make_process(1, "a"),
            make_process(2, "b"),
            make_process(3, "c"),
        ];
        let actions = plan(&d, &permissive_profile(), &procs);
        assert!(!actions.is_empty());
        let ids: Vec<u64> = actions.iter().map(|a| a.id).collect();
        assert_eq!(ids[0], 1);
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len(), "duplicate action IDs");
    }

    #[test]
    fn plan_comm_lookup_from_process_list() {
        let d = decision(
            DecisionIntent::AdjustProcessPriority,
            vec![Target::Process(77)],
        );
        let procs = vec![make_process(77, "chromium")];
        let mut profile = permissive_profile();
        profile.allow_ionice = false;
        let actions = plan(&d, &profile, &procs);
        assert_eq!(actions.len(), 1);
        assert_eq!(
            actions[0].target,
            ActionTarget::Process {
                pid: 77,
                comm: "chromium".into()
            }
        );
    }

    #[test]
    fn plan_comm_fallback_for_unknown_pid() {
        let d = decision(
            DecisionIntent::AdjustProcessPriority,
            vec![Target::Process(999)],
        );
        let mut profile = permissive_profile();
        profile.allow_ionice = false;
        let actions = plan(&d, &profile, &[]); // no process list
        assert_eq!(actions.len(), 1);
        assert_eq!(
            actions[0].target,
            ActionTarget::Process {
                pid: 999,
                comm: "pid:999".into()
            }
        );
    }

    #[test]
    fn plan_all_actions_have_non_empty_explanation() {
        let d = decision(
            DecisionIntent::Recommend,
            vec![Target::System, Target::Process(1)],
        );
        for action in plan(&d, &balanced(), &[]) {
            assert!(!action.explanation.is_empty());
        }
    }

    // --- simulate ---

    #[test]
    fn simulate_blocked_action_returns_blocked_status() {
        // Prohibited risk → gate 1 → Block.
        let action = PlannedAction {
            id: 1,
            kind: ActionKind::AdjustNice,
            risk: ActionRisk::Prohibited,
            target: ActionTarget::System,
            params: HashMap::new(),
            explanation: "test".into(),
        };
        let result = simulate(&action, &AppConfig::default(), &balanced(), &root_caps());
        assert_eq!(result.status, ActionStatus::Blocked);
        assert!(!result.message.is_empty());
        assert_eq!(result.action_id, 1);
        assert!(result.rollback_id.is_none());
    }

    #[test]
    fn simulate_safe_action_nonroot_dryrun_returns_simulated() {
        let action = PlannedAction {
            id: 5,
            kind: ActionKind::Observe,
            risk: ActionRisk::Safe,
            target: ActionTarget::System,
            params: HashMap::new(),
            explanation: "test".into(),
        };
        // Safe actions pass all gates (no state change → root/dry_run gates don't apply).
        let result = simulate(&action, &AppConfig::default(), &balanced(), &nonroot_caps());
        assert_eq!(result.status, ActionStatus::Simulated);
        assert_eq!(result.action_id, 5);
        assert!(result.rollback_id.is_none());
    }

    #[test]
    fn simulate_state_changing_action_with_dry_run_returns_simulated() {
        // Gate 7: dry_run=true → RequireDryRun → Simulated.
        let action = PlannedAction {
            id: 2,
            kind: ActionKind::AdjustNice,
            risk: ActionRisk::Moderate,
            target: ActionTarget::Process {
                pid: 100,
                comm: "myapp".into(),
            },
            params: HashMap::new(),
            explanation: "test".into(),
        };
        let mut cfg = AppConfig::default();
        cfg.global.dry_run = true;
        let result = simulate(&action, &cfg, &permissive_profile(), &root_caps());
        assert_eq!(result.status, ActionStatus::Simulated);
    }

    #[test]
    fn simulate_preserves_action_id_in_result() {
        let action = PlannedAction {
            id: 42,
            kind: ActionKind::Log,
            risk: ActionRisk::Safe,
            target: ActionTarget::System,
            params: HashMap::new(),
            explanation: "test".into(),
        };
        let result = simulate(&action, &AppConfig::default(), &balanced(), &nonroot_caps());
        assert_eq!(result.action_id, 42);
    }

    #[test]
    fn simulate_blocked_message_is_non_empty() {
        let action = PlannedAction {
            id: 1,
            kind: ActionKind::AdjustNice,
            risk: ActionRisk::Moderate,
            target: ActionTarget::Process {
                pid: 1,
                comm: "syswarden".into(),
            }, // protected
            params: HashMap::new(),
            explanation: "test".into(),
        };
        let result = simulate(
            &action,
            &open_config("any.service"),
            &permissive_profile(),
            &root_caps(),
        );
        assert_eq!(result.status, ActionStatus::Blocked);
        assert!(!result.message.is_empty());
    }

    #[test]
    fn simulate_produces_no_side_effects_on_dry_run_path() {
        // Construct a realistic plan → simulate pipeline and assert only Simulated/Blocked.
        let policy_decision = decision(
            DecisionIntent::AdjustProcessPriority,
            vec![Target::Process(500)],
        );
        let procs = vec![make_process(500, "heavyapp")];
        let cfg = AppConfig::default(); // dry_run=true by default
        let profile = developer(); // allow_nice=true, allow_ionice=true

        let actions = plan(&policy_decision, &profile, &procs);
        for action in &actions {
            let result = simulate(action, &cfg, &profile, &root_caps());
            assert!(
                matches!(
                    result.status,
                    ActionStatus::Simulated | ActionStatus::Blocked
                ),
                "unexpected status {:?} for {:?}",
                result.status,
                action.kind
            );
        }
    }
}
