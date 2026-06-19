//! Mandatory fail-closed safety gate; every action passes through `evaluate` (architecture.md §5.13, §17).
#![allow(dead_code)]

use crate::actions::{ActionKind, ActionTarget, PlannedAction};
use crate::config::AppConfig;
use crate::profiles::{ActionRisk, ProfileConfig};

// ---------------------------------------------------------------------------
// Capabilities
// ---------------------------------------------------------------------------

/// Detected runtime capabilities used as safety inputs (architecture.md §5.13, §17).
///
/// Constructed once per daemon tick via [`Capabilities::detect`]. Tests construct
/// it directly with known values to avoid depending on host state.
#[allow(clippy::struct_excessive_bools)] // Five capability flags mandated by architecture.md §17.
#[derive(Debug, Clone)]
pub struct Capabilities {
    pub is_root: bool,
    pub has_psi: bool,
    pub has_cgroup_v2: bool,
    pub has_systemd: bool,
    pub has_zram: bool,
}

impl Capabilities {
    /// Probe the live system for privilege level and available kernel features.
    ///
    /// Fails safe on every sub-check: missing `/proc` entry → assume non-root/absent.
    #[must_use]
    pub fn detect() -> Self {
        Self {
            is_root: effective_uid() == 0,
            has_psi: std::path::Path::new("/proc/pressure/cpu").exists(),
            has_cgroup_v2: std::path::Path::new("/sys/fs/cgroup/cgroup.controllers").exists(),
            has_systemd: std::path::Path::new("/run/systemd/private").exists(),
            has_zram: std::path::Path::new("/sys/block/zram0").exists(),
        }
    }
}

/// Read the effective UID from `/proc/self/status`.
///
/// Returns `u32::MAX` (never root) on any parse failure — fail-safe.
fn effective_uid() -> u32 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap_or_default()
        .lines()
        .find(|l| l.starts_with("Uid:"))
        .and_then(|l| l.split_whitespace().nth(2)) // col 0=label, 1=ruid, 2=euid, 3=suid, 4=fsuid
        .and_then(|s| s.parse().ok())
        .unwrap_or(u32::MAX)
}

// ---------------------------------------------------------------------------
// SafetyDecision
// ---------------------------------------------------------------------------

/// Verdict returned by [`evaluate`] for every planned action (architecture.md §15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SafetyDecision {
    /// All gates passed; action may execute.
    Allow,
    /// A gate blocked the action; `reason` identifies which gate and why.
    Block { reason: String },
    /// All gates would allow the action, but `dry_run = true` → simulate only.
    RequireDryRun,
}

// ---------------------------------------------------------------------------
// evaluate
// ---------------------------------------------------------------------------

/// Evaluate one planned action against all safety gates (architecture.md §17).
///
/// Gate order (first failure wins; fail-closed on any uncertainty):
/// 1. `ActionRisk::Prohibited` → Block unconditionally.
/// 2. Action risk exceeds profile's `max_allowed_risk` → Block.
/// 3. Target is a protected process or service → Block.
/// 4. Service resource-control action targets a service not in `allowed.services` → Block.
/// 5. Required per-kind permission flag is not set → Block.
/// 6. State-changing action + non-root → Block.
/// 7. State-changing action + `dry_run = true` → `RequireDryRun`.
/// 8. All gates passed → Allow.
#[must_use]
pub fn evaluate(
    action: &PlannedAction,
    config: &AppConfig,
    profile: &ProfileConfig,
    caps: &Capabilities,
) -> SafetyDecision {
    // Gate 1: Prohibited actions are never allowed — no flag or config can override this.
    if action.risk == ActionRisk::Prohibited {
        return SafetyDecision::Block {
            reason: format!("{:?} is prohibited and cannot be executed", action.kind),
        };
    }

    // Gate 2: Action risk must not exceed the profile's maximum permitted risk.
    if action.risk > profile.max_allowed_risk {
        return SafetyDecision::Block {
            reason: format!(
                "action risk {:?} exceeds profile maximum {:?}",
                action.risk, profile.max_allowed_risk
            ),
        };
    }

    // Gate 3: Protected targets are never touched, regardless of profile or flags.
    match &action.target {
        ActionTarget::Process { comm, .. } => {
            if config.protected.processes.iter().any(|p| p == comm) {
                return SafetyDecision::Block {
                    reason: format!("process '{comm}' is in the protected list"),
                };
            }
        }
        ActionTarget::Service { unit } => {
            if config.protected.services.iter().any(|s| s == unit) {
                return SafetyDecision::Block {
                    reason: format!("service '{unit}' is in the protected list"),
                };
            }
        }
        ActionTarget::System => {}
    }

    // Gate 4: Service resource-control changes require the service to be in `allowed.services`.
    // An empty allowlist means no service is modifiable (per §17 invariant).
    if requires_service_allowlist(&action.kind) {
        if let ActionTarget::Service { unit } = &action.target {
            if !config.allowed.services.iter().any(|s| s == unit) {
                return SafetyDecision::Block {
                    reason: format!("service '{unit}' is not in allowed.services"),
                };
            }
        }
    }

    // Gate 5: Per-kind permission flags must be set in the profile and/or global config.
    if let Some(reason) = check_permission_flags(action, config, profile) {
        return SafetyDecision::Block { reason };
    }

    // Gate 6: State-changing actions require root privileges.
    if is_state_changing(action) && !caps.is_root {
        return SafetyDecision::Block {
            reason: "state-changing action requires root privileges".to_string(),
        };
    }

    // Gate 7: Under dry_run, state-changing actions are simulated rather than executed.
    if config.global.dry_run && is_state_changing(action) {
        return SafetyDecision::RequireDryRun;
    }

    SafetyDecision::Allow
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns `true` for actions that modify system state.
///
/// Observe/Log/Report/Recommend are pure read/emit operations with no side effects.
/// Everything else — even `CreateBackup` — changes external state and requires
/// root + `dry_run` gating.
fn is_state_changing(action: &PlannedAction) -> bool {
    !matches!(
        action.kind,
        ActionKind::Observe | ActionKind::Log | ActionKind::Report | ActionKind::Recommend
    )
}

/// Returns `true` for action kinds that require the service target to be in `allowed.services`.
///
/// Process actions (nice/ionice) do not use the service allowlist; they use the protected list.
fn requires_service_allowlist(kind: &ActionKind) -> bool {
    matches!(
        kind,
        ActionKind::SetCpuWeight
            | ActionKind::SetIoWeight
            | ActionKind::SetMemoryHigh
            | ActionKind::SetMemoryMax
            | ActionKind::RestartService
            | ActionKind::StopService
    )
}

/// Check per-action-kind permission flags. Returns a block reason string when a
/// required flag is missing from the profile or global config.
fn check_permission_flags(
    action: &PlannedAction,
    config: &AppConfig,
    profile: &ProfileConfig,
) -> Option<String> {
    let blocked = match action.kind {
        ActionKind::AdjustNice => !profile.allow_nice,
        ActionKind::AdjustIonice => !profile.allow_ionice,
        ActionKind::SetCpuWeight => !profile.allow_cpu_weight,
        ActionKind::SetIoWeight => !profile.allow_io_weight,
        ActionKind::SetMemoryHigh => !profile.allow_memory_high,
        ActionKind::SetMemoryMax => !profile.allow_memory_max,
        ActionKind::RestartService => !profile.allow_service_restart,
        ActionKind::StopService => !profile.allow_service_stop,
        ActionKind::ApplyZram => !profile.allow_zram_apply || !config.global.allow_zram_apply,
        ActionKind::ApplySysctl => !config.global.allow_sysctl_apply,
        // No permission flag required for safe/backup actions.
        ActionKind::Observe
        | ActionKind::Log
        | ActionKind::Report
        | ActionKind::Recommend
        | ActionKind::CreateBackup => false,
    };

    blocked.then(|| {
        format!(
            "{:?} is blocked: required permission flag is not set",
            action.kind
        )
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::{ActionKind, ActionTarget, PlannedAction};
    use crate::config::AppConfig;
    use crate::profiles::{ActionRisk, ProfileConfig};
    use std::collections::HashMap;

    // --- Test helpers ---

    fn make_action(kind: ActionKind, risk: ActionRisk, target: ActionTarget) -> PlannedAction {
        PlannedAction {
            id: 1,
            kind,
            risk,
            target,
            params: HashMap::new(),
            explanation: "test".to_string(),
        }
    }

    fn process_target(comm: &str) -> ActionTarget {
        ActionTarget::Process {
            pid: 1234,
            comm: comm.to_string(),
        }
    }

    fn service_target(unit: &str) -> ActionTarget {
        ActionTarget::Service {
            unit: unit.to_string(),
        }
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

    /// Profile that allows Moderate risk with all relevant flags enabled.
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

    /// Config that permits all global flags and disables `dry_run`, with one service allowed.
    fn open_config_with_service(unit: &str) -> AppConfig {
        let mut cfg = AppConfig::default();
        cfg.global.dry_run = false;
        cfg.global.allow_aggressive_actions = true;
        cfg.global.allow_zram_apply = true;
        cfg.global.allow_sysctl_apply = true;
        cfg.allowed.services.push(unit.to_string());
        cfg
    }

    // --- Gate 1: Prohibited ---

    #[test]
    fn prohibited_risk_always_blocked() {
        let action = make_action(
            ActionKind::Observe,
            ActionRisk::Prohibited,
            ActionTarget::System,
        );
        let cfg = open_config_with_service("any.service");
        let profile = permissive_profile();
        let caps = root_caps();
        assert!(matches!(
            evaluate(&action, &cfg, &profile, &caps),
            SafetyDecision::Block { .. }
        ));
    }

    #[test]
    fn prohibited_risk_blocked_even_under_dry_run() {
        let action = make_action(
            ActionKind::AdjustNice,
            ActionRisk::Prohibited,
            process_target("myapp"),
        );
        let cfg = AppConfig::default(); // dry_run=true
        let profile = permissive_profile();
        let caps = root_caps();
        assert!(matches!(
            evaluate(&action, &cfg, &profile, &caps),
            SafetyDecision::Block { .. }
        ));
    }

    // --- Gate 2: Risk vs profile max ---

    #[test]
    fn aggressive_action_blocked_by_moderate_profile() {
        let action = make_action(
            ActionKind::SetMemoryMax,
            ActionRisk::Aggressive,
            service_target("app.service"),
        );
        let mut cfg = open_config_with_service("app.service");
        cfg.global.allow_aggressive_actions = false;
        let mut profile = permissive_profile();
        profile.max_allowed_risk = ActionRisk::Moderate;
        let caps = root_caps();
        assert!(matches!(
            evaluate(&action, &cfg, &profile, &caps),
            SafetyDecision::Block { .. }
        ));
    }

    #[test]
    fn moderate_action_blocked_by_safe_only_profile() {
        let action = make_action(
            ActionKind::AdjustNice,
            ActionRisk::Moderate,
            process_target("myapp"),
        );
        let cfg = open_config_with_service("app.service");
        let mut profile = permissive_profile();
        profile.max_allowed_risk = ActionRisk::Safe;
        let caps = root_caps();
        assert!(matches!(
            evaluate(&action, &cfg, &profile, &caps),
            SafetyDecision::Block { .. }
        ));
    }

    // --- Gate 3: Protected targets ---

    #[test]
    fn protected_process_always_blocked() {
        let action = make_action(
            ActionKind::AdjustNice,
            ActionRisk::Moderate,
            process_target("syswarden"),
        );
        let cfg = open_config_with_service("any.service");
        let profile = permissive_profile();
        let caps = root_caps();
        let verdict = evaluate(&action, &cfg, &profile, &caps);
        assert!(matches!(verdict, SafetyDecision::Block { .. }));
    }

    #[test]
    fn protected_process_blocked_includes_all_defaults() {
        let protected = [
            "systemd",
            "systemd-journald",
            "dbus-daemon",
            "init",
            "sshd",
            "agetty",
        ];
        let cfg = open_config_with_service("any.service");
        let profile = permissive_profile();
        let caps = root_caps();
        for comm in protected {
            let action = make_action(
                ActionKind::AdjustNice,
                ActionRisk::Moderate,
                process_target(comm),
            );
            assert!(
                matches!(
                    evaluate(&action, &cfg, &profile, &caps),
                    SafetyDecision::Block { .. }
                ),
                "protected process '{comm}' was not blocked"
            );
        }
    }

    #[test]
    fn protected_service_always_blocked() {
        let action = make_action(
            ActionKind::SetCpuWeight,
            ActionRisk::Moderate,
            service_target("syswarden.service"),
        );
        let mut cfg = open_config_with_service("syswarden.service"); // even if in allowed
        cfg.allowed.services.push("syswarden.service".to_string());
        let profile = permissive_profile();
        let caps = root_caps();
        assert!(matches!(
            evaluate(&action, &cfg, &profile, &caps),
            SafetyDecision::Block { .. }
        ));
    }

    #[test]
    fn protected_service_blocked_includes_defaults() {
        let protected_svcs = [
            "systemd-journald.service",
            "systemd-logind.service",
            "dbus.service",
            "sshd.service",
        ];
        let profile = permissive_profile();
        let caps = root_caps();
        for unit in protected_svcs {
            let action = make_action(
                ActionKind::SetCpuWeight,
                ActionRisk::Moderate,
                service_target(unit),
            );
            let mut cfg = open_config_with_service(unit);
            cfg.allowed.services.push(unit.to_string());
            assert!(
                matches!(
                    evaluate(&action, &cfg, &profile, &caps),
                    SafetyDecision::Block { .. }
                ),
                "protected service '{unit}' was not blocked"
            );
        }
    }

    // --- Gate 4: Service allowlist ---

    #[test]
    fn service_not_in_allowlist_blocked() {
        let action = make_action(
            ActionKind::SetCpuWeight,
            ActionRisk::Moderate,
            service_target("myapp.service"),
        );
        let cfg = open_config_with_service("other.service"); // different service allowed
        let profile = permissive_profile();
        let caps = root_caps();
        assert!(matches!(
            evaluate(&action, &cfg, &profile, &caps),
            SafetyDecision::Block { .. }
        ));
    }

    #[test]
    fn empty_allowlist_blocks_all_service_resource_actions() {
        let action = make_action(
            ActionKind::SetMemoryHigh,
            ActionRisk::Moderate,
            service_target("myapp.service"),
        );
        let mut cfg = AppConfig::default();
        cfg.global.dry_run = false;
        // allowed.services is empty by default
        let profile = permissive_profile();
        let caps = root_caps();
        assert!(matches!(
            evaluate(&action, &cfg, &profile, &caps),
            SafetyDecision::Block { .. }
        ));
    }

    #[test]
    fn service_in_allowlist_passes_gate_4() {
        // With all other gates satisfied, an allowed service proceeds to Allow.
        let action = make_action(
            ActionKind::SetCpuWeight,
            ActionRisk::Moderate,
            service_target("myapp.service"),
        );
        let cfg = open_config_with_service("myapp.service");
        let profile = permissive_profile();
        let caps = root_caps();
        assert_eq!(
            evaluate(&action, &cfg, &profile, &caps),
            SafetyDecision::Allow
        );
    }

    // --- Gate 5: Permission flags ---

    #[test]
    fn adjust_nice_blocked_when_flag_unset() {
        let action = make_action(
            ActionKind::AdjustNice,
            ActionRisk::Moderate,
            process_target("myapp"),
        );
        let cfg = open_config_with_service("any.service");
        let mut profile = permissive_profile();
        profile.allow_nice = false;
        let caps = root_caps();
        assert!(matches!(
            evaluate(&action, &cfg, &profile, &caps),
            SafetyDecision::Block { .. }
        ));
    }

    #[test]
    fn adjust_ionice_blocked_when_flag_unset() {
        let action = make_action(
            ActionKind::AdjustIonice,
            ActionRisk::Moderate,
            process_target("myapp"),
        );
        let cfg = open_config_with_service("any.service");
        let mut profile = permissive_profile();
        profile.allow_ionice = false;
        let caps = root_caps();
        assert!(matches!(
            evaluate(&action, &cfg, &profile, &caps),
            SafetyDecision::Block { .. }
        ));
    }

    #[test]
    fn set_cpu_weight_blocked_when_flag_unset() {
        let action = make_action(
            ActionKind::SetCpuWeight,
            ActionRisk::Moderate,
            service_target("myapp.service"),
        );
        let cfg = open_config_with_service("myapp.service");
        let mut profile = permissive_profile();
        profile.allow_cpu_weight = false;
        let caps = root_caps();
        assert!(matches!(
            evaluate(&action, &cfg, &profile, &caps),
            SafetyDecision::Block { .. }
        ));
    }

    #[test]
    fn set_io_weight_blocked_when_flag_unset() {
        let action = make_action(
            ActionKind::SetIoWeight,
            ActionRisk::Moderate,
            service_target("myapp.service"),
        );
        let cfg = open_config_with_service("myapp.service");
        let mut profile = permissive_profile();
        profile.allow_io_weight = false;
        let caps = root_caps();
        assert!(matches!(
            evaluate(&action, &cfg, &profile, &caps),
            SafetyDecision::Block { .. }
        ));
    }

    #[test]
    fn set_memory_high_blocked_when_flag_unset() {
        let action = make_action(
            ActionKind::SetMemoryHigh,
            ActionRisk::Moderate,
            service_target("myapp.service"),
        );
        let cfg = open_config_with_service("myapp.service");
        let mut profile = permissive_profile();
        profile.allow_memory_high = false;
        let caps = root_caps();
        assert!(matches!(
            evaluate(&action, &cfg, &profile, &caps),
            SafetyDecision::Block { .. }
        ));
    }

    #[test]
    fn set_memory_max_blocked_when_flag_unset() {
        let action = make_action(
            ActionKind::SetMemoryMax,
            ActionRisk::Aggressive,
            service_target("myapp.service"),
        );
        let cfg = open_config_with_service("myapp.service");
        let mut profile = permissive_profile();
        profile.allow_memory_max = false;
        let caps = root_caps();
        assert!(matches!(
            evaluate(&action, &cfg, &profile, &caps),
            SafetyDecision::Block { .. }
        ));
    }

    #[test]
    fn restart_service_blocked_when_flag_unset() {
        let action = make_action(
            ActionKind::RestartService,
            ActionRisk::Aggressive,
            service_target("myapp.service"),
        );
        let cfg = open_config_with_service("myapp.service");
        let mut profile = permissive_profile();
        profile.allow_service_restart = false;
        let caps = root_caps();
        assert!(matches!(
            evaluate(&action, &cfg, &profile, &caps),
            SafetyDecision::Block { .. }
        ));
    }

    #[test]
    fn stop_service_blocked_when_flag_unset() {
        let action = make_action(
            ActionKind::StopService,
            ActionRisk::Aggressive,
            service_target("myapp.service"),
        );
        let cfg = open_config_with_service("myapp.service");
        let mut profile = permissive_profile();
        profile.allow_service_stop = false;
        let caps = root_caps();
        assert!(matches!(
            evaluate(&action, &cfg, &profile, &caps),
            SafetyDecision::Block { .. }
        ));
    }

    #[test]
    fn apply_zram_blocked_when_profile_flag_unset() {
        let action = make_action(
            ActionKind::ApplyZram,
            ActionRisk::Aggressive,
            ActionTarget::System,
        );
        let cfg = open_config_with_service("any.service"); // global.allow_zram_apply=true here
        let mut profile = permissive_profile();
        profile.allow_zram_apply = false;
        let caps = root_caps();
        assert!(matches!(
            evaluate(&action, &cfg, &profile, &caps),
            SafetyDecision::Block { .. }
        ));
    }

    #[test]
    fn apply_zram_blocked_when_global_flag_unset() {
        let action = make_action(
            ActionKind::ApplyZram,
            ActionRisk::Aggressive,
            ActionTarget::System,
        );
        let mut cfg = open_config_with_service("any.service");
        cfg.global.allow_zram_apply = false; // global gate
        let profile = permissive_profile(); // profile.allow_zram_apply=true
        let caps = root_caps();
        assert!(matches!(
            evaluate(&action, &cfg, &profile, &caps),
            SafetyDecision::Block { .. }
        ));
    }

    #[test]
    fn apply_sysctl_blocked_when_global_flag_unset() {
        let action = make_action(
            ActionKind::ApplySysctl,
            ActionRisk::Aggressive,
            ActionTarget::System,
        );
        let mut cfg = open_config_with_service("any.service");
        cfg.global.allow_sysctl_apply = false;
        let profile = permissive_profile();
        let caps = root_caps();
        assert!(matches!(
            evaluate(&action, &cfg, &profile, &caps),
            SafetyDecision::Block { .. }
        ));
    }

    // --- Gate 6: Non-root ---

    #[test]
    fn non_root_blocks_state_changing_action() {
        let action = make_action(
            ActionKind::AdjustNice,
            ActionRisk::Moderate,
            process_target("myapp"),
        );
        let cfg = open_config_with_service("any.service");
        let profile = permissive_profile();
        let caps = nonroot_caps();
        assert!(matches!(
            evaluate(&action, &cfg, &profile, &caps),
            SafetyDecision::Block { .. }
        ));
    }

    #[test]
    fn non_root_blocks_all_state_changing_kinds() {
        let state_changing = [
            (
                ActionKind::CreateBackup,
                ActionRisk::Moderate,
                ActionTarget::System,
            ),
            (
                ActionKind::AdjustNice,
                ActionRisk::Moderate,
                process_target("myapp"),
            ),
            (
                ActionKind::AdjustIonice,
                ActionRisk::Moderate,
                process_target("myapp"),
            ),
            (
                ActionKind::SetCpuWeight,
                ActionRisk::Moderate,
                service_target("myapp.service"),
            ),
            (
                ActionKind::SetMemoryHigh,
                ActionRisk::Moderate,
                service_target("myapp.service"),
            ),
        ];
        let caps = nonroot_caps();
        let profile = permissive_profile();

        for (kind, risk, target) in state_changing {
            let action = make_action(kind.clone(), risk, target);
            let cfg = open_config_with_service("myapp.service");
            assert!(
                matches!(
                    evaluate(&action, &cfg, &profile, &caps),
                    SafetyDecision::Block { .. }
                ),
                "{kind:?} was not blocked for non-root"
            );
        }
    }

    // --- Gate 7: dry_run ---

    #[test]
    fn dry_run_converts_permitted_action_to_require_dry_run() {
        let action = make_action(
            ActionKind::AdjustNice,
            ActionRisk::Moderate,
            process_target("myapp"),
        );
        let mut cfg = open_config_with_service("any.service");
        cfg.global.dry_run = true; // master switch
        let profile = permissive_profile();
        let caps = root_caps();
        assert_eq!(
            evaluate(&action, &cfg, &profile, &caps),
            SafetyDecision::RequireDryRun
        );
    }

    #[test]
    fn dry_run_applies_to_all_state_changing_kinds() {
        let state_changing = [
            (
                ActionKind::AdjustNice,
                ActionRisk::Moderate,
                process_target("myapp"),
            ),
            (
                ActionKind::SetCpuWeight,
                ActionRisk::Moderate,
                service_target("myapp.service"),
            ),
            (
                ActionKind::SetMemoryHigh,
                ActionRisk::Moderate,
                service_target("myapp.service"),
            ),
            (
                ActionKind::ApplyZram,
                ActionRisk::Aggressive,
                ActionTarget::System,
            ),
        ];
        let mut cfg = open_config_with_service("myapp.service");
        cfg.global.dry_run = true;
        let profile = permissive_profile();
        let caps = root_caps();

        for (kind, risk, target) in state_changing {
            let action = make_action(kind.clone(), risk, target);
            assert_eq!(
                evaluate(&action, &cfg, &profile, &caps),
                SafetyDecision::RequireDryRun,
                "{kind:?} should be RequireDryRun, not something else"
            );
        }
    }

    // --- Gate 8 (final): Allow ---

    #[test]
    fn all_gates_passed_returns_allow() {
        let action = make_action(
            ActionKind::AdjustNice,
            ActionRisk::Moderate,
            process_target("myapp"),
        );
        let cfg = open_config_with_service("any.service"); // dry_run=false
        let profile = permissive_profile();
        let caps = root_caps();
        assert_eq!(
            evaluate(&action, &cfg, &profile, &caps),
            SafetyDecision::Allow
        );
    }

    // --- Safe actions are never blocked by root/dry_run gates ---

    #[test]
    fn safe_observe_actions_always_allow() {
        let safe_kinds = [
            ActionKind::Observe,
            ActionKind::Log,
            ActionKind::Report,
            ActionKind::Recommend,
        ];
        // Even as non-root with dry_run=true.
        let cfg = AppConfig::default(); // dry_run=true
        let profile = permissive_profile();
        let caps = nonroot_caps();

        for kind in safe_kinds {
            let action = make_action(kind.clone(), ActionRisk::Safe, ActionTarget::System);
            assert_eq!(
                evaluate(&action, &cfg, &profile, &caps),
                SafetyDecision::Allow,
                "{kind:?} should always Allow regardless of dry_run or root"
            );
        }
    }

    // --- Fail-closed defaults ---

    #[test]
    fn default_config_blocks_all_state_changing_actions() {
        // With default config (dry_run=true, all flags false), every state-changing action
        // must be blocked before even reaching the dry_run gate.
        let cfg = AppConfig::default();
        let mut profile = permissive_profile();
        profile.max_allowed_risk = ActionRisk::Safe; // conservative
        let caps = root_caps();

        let actions = [
            make_action(
                ActionKind::AdjustNice,
                ActionRisk::Moderate,
                process_target("myapp"),
            ),
            make_action(
                ActionKind::SetCpuWeight,
                ActionRisk::Moderate,
                service_target("myapp.service"),
            ),
        ];
        for action in &actions {
            assert!(
                matches!(
                    evaluate(action, &cfg, &profile, &caps),
                    SafetyDecision::Block { .. }
                ),
                "{:?} should be blocked by default conservative config",
                action.kind
            );
        }
    }

    #[test]
    fn block_reason_is_not_empty() {
        let action = make_action(
            ActionKind::AdjustNice,
            ActionRisk::Prohibited,
            process_target("myapp"),
        );
        let cfg = AppConfig::default();
        let profile = permissive_profile();
        let caps = root_caps();
        if let SafetyDecision::Block { reason } = evaluate(&action, &cfg, &profile, &caps) {
            assert!(!reason.is_empty(), "block reason must not be empty");
        } else {
            panic!("expected Block");
        }
    }
}
