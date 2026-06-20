//! Read-only systemd service analysis and flagging (architecture.md §5.7, Phase 32).
//!
//! Phase 32 adds `evaluate_resource_rules` — a deterministic, pure function that matches
//! `service_rules` against a service and produces flags + matched rules without touching the
//! system. Protected and non-allowlisted services never receive resource-control flags.
#![allow(dead_code)]

use crate::config::{AppConfig, ServiceRule};
use crate::error::SyswardenError;

/// Why a service was flagged (architecture.md §5.7, Phase 32).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceFlag {
    /// `active_state == "failed"`.
    Failing,
    /// `sub_state == "auto-restart"` — service is restart-looping.
    Restarting,
    /// `memory_current` exceeds a matching service rule's `memory_high_mb`.
    HighMemory,
    /// Any rule-based threshold (`HighMemory`) was exceeded.
    RuleViolation,
    /// A service rule specifies a `cpu_weight` for this service (Phase 32).
    CpuWeightRule,
    /// A service rule specifies an `io_weight` for this service (Phase 32).
    IoWeightRule,
}

/// Per-service snapshot for one collection tick (architecture.md §15).
///
/// `cpu_usage` is `CPUUsageNSec` (nanoseconds of CPU time since service start).
/// `memory_current` is `MemoryCurrent` in bytes; systemd returns `u64::MAX` when
/// memory accounting is disabled for the unit — callers must treat that as "unknown".
/// `restarts` is `NRestarts` from the `org.freedesktop.systemd1.Service` interface.
/// `matched_rules` carries the rules that matched this service (Phase 32), forwarded to the
/// planner so it can build `params["cpu_weight"]` / `params["persistent"]` correctly.
#[derive(Debug, Clone)]
pub struct ServiceInfo {
    pub unit: String,
    pub active_state: String,
    pub sub_state: String,
    pub is_protected: bool,
    pub is_allowed: bool,
    pub cpu_usage: u64,
    pub memory_current: u64,
    pub restarts: u32,
    pub flags: Vec<ServiceFlag>,
    /// Service rules that matched this unit (empty when protected or not in allowed list).
    pub matched_rules: Vec<ServiceRule>,
}

fn is_service_protected(unit: &str, config: &AppConfig) -> bool {
    config.protected.services.iter().any(|s| s == unit)
}

fn is_service_allowed(unit: &str, config: &AppConfig) -> bool {
    config.allowed.services.iter().any(|s| s == unit)
}

/// Evaluate `rules` against a service and return `(flags, matched_rules)`.
///
/// This is the **deterministic core** of the Phase 32 rule engine (architecture.md §5.7).
/// It never touches the system — all inputs are plain values, making it trivially testable.
///
/// Invariants (architecture.md §17 / Phase 32 acceptance):
/// - Protected services (`is_protected = true`) receive no resource-control flags.
/// - Non-allowlisted services (`is_allowed = false`) receive no resource-control flags.
/// - `Failing` and `Restarting` are state flags set by `compute_flags`, not here.
/// - `u64::MAX` for `memory_current` means accounting is disabled → treated as 0.
#[must_use]
pub fn evaluate_resource_rules(
    unit: &str,
    is_protected: bool,
    is_allowed: bool,
    memory_current: u64,
    rules: &[ServiceRule],
) -> (Vec<ServiceFlag>, Vec<ServiceRule>) {
    // Safety gate: never flag protected or non-allowlisted services for resource control.
    if is_protected || !is_allowed {
        return (Vec::new(), Vec::new());
    }

    // u64::MAX = memory accounting disabled; avoid false positives.
    let safe_mem_bytes = if memory_current == u64::MAX {
        0u64
    } else {
        memory_current
    };

    let mut flags: Vec<ServiceFlag> = Vec::new();
    let mut matched: Vec<ServiceRule> = Vec::new();

    for rule in rules {
        if !unit.contains(rule.name_match.as_str()) {
            continue;
        }
        matched.push(rule.clone());

        if rule.cpu_weight.is_some() && !flags.contains(&ServiceFlag::CpuWeightRule) {
            flags.push(ServiceFlag::CpuWeightRule);
        }
        if rule.io_weight.is_some() && !flags.contains(&ServiceFlag::IoWeightRule) {
            flags.push(ServiceFlag::IoWeightRule);
        }
        if let Some(limit_mb) = rule.memory_high_mb {
            let current_mb = safe_mem_bytes / (1024 * 1024);
            if current_mb > limit_mb && !flags.contains(&ServiceFlag::HighMemory) {
                flags.push(ServiceFlag::HighMemory);
                flags.push(ServiceFlag::RuleViolation);
            }
        }
    }

    (flags, matched)
}

fn compute_flags(info: &ServiceInfo, config: &AppConfig) -> (Vec<ServiceFlag>, Vec<ServiceRule>) {
    let mut flags = Vec::new();
    if info.active_state == "failed" {
        flags.push(ServiceFlag::Failing);
    }
    if info.sub_state == "auto-restart" {
        flags.push(ServiceFlag::Restarting);
    }
    let (resource_flags, matched) = evaluate_resource_rules(
        &info.unit,
        info.is_protected,
        info.is_allowed,
        info.memory_current,
        &config.service_rules,
    );
    flags.extend(resource_flags);
    (flags, matched)
}

// D-Bus tuple from org.freedesktop.systemd1.Manager.ListUnits:
//   (name, description, load_state, active_state, sub_state, followed,
//    unit_path, job_id, job_type, job_path)
type UnitTuple = (
    String,
    String,
    String,
    String,
    String,
    String,
    zbus::zvariant::OwnedObjectPath,
    u32,
    String,
    zbus::zvariant::OwnedObjectPath,
);

/// Read per-unit resource properties from D-Bus.
///
/// Returns `(cpu_usage_ns, memory_current_bytes, n_restarts)`.
/// Any property that cannot be read defaults to 0 (or `u64::MAX` for memory).
fn read_unit_props(conn: &zbus::blocking::Connection, path: &str) -> (u64, u64, u32) {
    let Ok(unit_proxy) = zbus::blocking::Proxy::new(
        conn,
        "org.freedesktop.systemd1",
        path,
        "org.freedesktop.systemd1.Unit",
    ) else {
        return (0, u64::MAX, 0);
    };

    let cpu: u64 = unit_proxy.get_property("CPUUsageNSec").unwrap_or(0u64);
    // systemd returns u64::MAX when MemoryAccounting is off for the unit.
    let mem: u64 = unit_proxy.get_property("MemoryCurrent").unwrap_or(u64::MAX);

    let restarts: u32 = zbus::blocking::Proxy::new(
        conn,
        "org.freedesktop.systemd1",
        path,
        "org.freedesktop.systemd1.Service",
    )
    .ok()
    .and_then(|p| p.get_property::<u32>("NRestarts").ok())
    .unwrap_or(0);

    (cpu, mem, restarts)
}

/// Connect to the system D-Bus, call `ListUnits`, and build `ServiceInfo` for every
/// `.service` unit. Returns `Err(SyswardenError::Systemd(_))` if D-Bus is unavailable.
fn query_systemd(config: &AppConfig) -> Result<Vec<ServiceInfo>, SyswardenError> {
    let conn =
        zbus::blocking::Connection::system().map_err(|e| SyswardenError::Systemd(e.to_string()))?;

    let units: Vec<UnitTuple> = {
        let manager = zbus::blocking::Proxy::new(
            &conn,
            "org.freedesktop.systemd1",
            "/org/freedesktop/systemd1",
            "org.freedesktop.systemd1.Manager",
        )
        .map_err(|e| SyswardenError::Systemd(e.to_string()))?;

        manager
            .call("ListUnits", &())
            .map_err(|e| SyswardenError::Systemd(e.to_string()))?
    };

    let mut result = Vec::new();
    for (
        name,
        _desc,
        _load,
        active_state,
        sub_state,
        _followed,
        unit_path,
        _job_id,
        _job_type,
        _job_path,
    ) in units
    {
        if !name.ends_with(".service") {
            continue;
        }

        let (cpu_usage, memory_current, restarts) = read_unit_props(&conn, unit_path.as_str());
        let is_protected = is_service_protected(&name, config);
        let is_allowed = is_service_allowed(&name, config);

        let mut info = ServiceInfo {
            unit: name,
            active_state,
            sub_state,
            is_protected,
            is_allowed,
            cpu_usage,
            memory_current,
            restarts,
            flags: Vec::new(),
            matched_rules: Vec::new(),
        };
        let (flags, matched) = compute_flags(&info, config);
        info.flags = flags;
        info.matched_rules = matched;
        result.push(info);
    }

    Ok(result)
}

/// Enumerate all systemd services and flag anomalous ones per config rules.
///
/// Degrades gracefully — returns an empty list when systemd or D-Bus is unavailable
/// (e.g., no systemd on host, non-root, D-Bus socket absent). Never modifies any
/// service state; read-only.
pub fn analyze(config: &AppConfig) -> Vec<ServiceInfo> {
    match query_systemd(config) {
        Ok(services) => services,
        Err(e) => {
            tracing::warn!("service analysis degraded (no D-Bus or systemd): {e}");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppConfig, ServiceRule};

    // ---- helpers ----

    fn make_info(unit: &str, active: &str, sub: &str, mem_bytes: u64) -> ServiceInfo {
        ServiceInfo {
            unit: unit.to_string(),
            active_state: active.to_string(),
            sub_state: sub.to_string(),
            is_protected: false,
            is_allowed: true, // allowed by default in tests so resource flags can fire
            cpu_usage: 0,
            memory_current: mem_bytes,
            restarts: 0,
            flags: Vec::new(),
            matched_rules: Vec::new(),
        }
    }

    fn mem_rule(name: &str, mb: u64) -> ServiceRule {
        ServiceRule {
            name_match: name.to_string(),
            cpu_weight: None,
            io_weight: None,
            memory_high_mb: Some(mb),
        }
    }

    fn full_rule(name: &str, cpu: u32, io: u32, mem_mb: u64) -> ServiceRule {
        ServiceRule {
            name_match: name.to_string(),
            cpu_weight: Some(cpu),
            io_weight: Some(io),
            memory_high_mb: Some(mem_mb),
        }
    }

    // ---- compute_flags: state flags (failing/restarting) ----

    #[test]
    fn compute_flags_failing_service() {
        let cfg = AppConfig::default();
        let info = make_info("foo.service", "failed", "failed", 0);
        let (flags, _) = compute_flags(&info, &cfg);
        assert!(flags.contains(&ServiceFlag::Failing));
        assert!(!flags.contains(&ServiceFlag::RuleViolation));
    }

    #[test]
    fn compute_flags_restarting_service() {
        let cfg = AppConfig::default();
        let info = make_info("foo.service", "activating", "auto-restart", 0);
        let (flags, _) = compute_flags(&info, &cfg);
        assert!(flags.contains(&ServiceFlag::Restarting));
        assert!(!flags.contains(&ServiceFlag::Failing));
    }

    // ---- Phase 32: evaluate_resource_rules decision table ----

    #[test]
    fn resource_rules_high_memory_violation() {
        let rules = [mem_rule("nightly-build.service", 4096)];
        // 5 GiB > 4096 MiB
        let (flags, matched) = evaluate_resource_rules(
            "nightly-build.service",
            false,
            true,
            5 * 1024 * 1024 * 1024,
            &rules,
        );
        assert!(flags.contains(&ServiceFlag::HighMemory));
        assert!(flags.contains(&ServiceFlag::RuleViolation));
        assert!(!flags.contains(&ServiceFlag::Failing));
        assert_eq!(matched.len(), 1);
    }

    #[test]
    fn resource_rules_within_memory_limit_no_flags() {
        let rules = [mem_rule("myapp.service", 4096)];
        let (flags, _) = evaluate_resource_rules(
            "myapp.service",
            false,
            true,
            1024 * 1024 * 1024, // 1 GiB < 4096 MiB
            &rules,
        );
        assert!(flags.is_empty());
    }

    #[test]
    fn resource_rules_memory_accounting_disabled_not_flagged() {
        let rules = [mem_rule("myapp.service", 100)];
        let (flags, _) = evaluate_resource_rules("myapp.service", false, true, u64::MAX, &rules);
        assert!(!flags.contains(&ServiceFlag::HighMemory));
    }

    #[test]
    fn resource_rules_cpu_weight_rule_sets_flag() {
        let rules = [ServiceRule {
            name_match: "heavy.service".to_string(),
            cpu_weight: Some(50),
            io_weight: None,
            memory_high_mb: None,
        }];
        let (flags, matched) = evaluate_resource_rules("heavy.service", false, true, 0, &rules);
        assert!(flags.contains(&ServiceFlag::CpuWeightRule));
        assert!(!flags.contains(&ServiceFlag::IoWeightRule));
        assert_eq!(matched[0].cpu_weight, Some(50));
    }

    #[test]
    fn resource_rules_io_weight_rule_sets_flag() {
        let rules = [ServiceRule {
            name_match: "iobound.service".to_string(),
            cpu_weight: None,
            io_weight: Some(25),
            memory_high_mb: None,
        }];
        let (flags, _) = evaluate_resource_rules("iobound.service", false, true, 0, &rules);
        assert!(flags.contains(&ServiceFlag::IoWeightRule));
        assert!(!flags.contains(&ServiceFlag::CpuWeightRule));
    }

    #[test]
    fn resource_rules_all_three_flags_from_full_rule() {
        let rules = [full_rule("nightly.service", 50, 25, 2048)];
        let (flags, matched) = evaluate_resource_rules(
            "nightly.service",
            false,
            true,
            3 * 1024 * 1024 * 1024, // 3 GiB > 2048 MiB
            &rules,
        );
        assert!(flags.contains(&ServiceFlag::CpuWeightRule));
        assert!(flags.contains(&ServiceFlag::IoWeightRule));
        assert!(flags.contains(&ServiceFlag::HighMemory));
        assert!(flags.contains(&ServiceFlag::RuleViolation));
        assert_eq!(matched.len(), 1);
    }

    // ---- Phase 32 invariant: protected untouched ----

    #[test]
    fn resource_rules_protected_service_gets_no_resource_flags() {
        let rules = [full_rule("protected.service", 50, 25, 100)];
        let (flags, matched) = evaluate_resource_rules(
            "protected.service",
            true, // is_protected
            true,
            200 * 1024 * 1024, // over 100 MiB limit
            &rules,
        );
        assert!(
            flags.is_empty(),
            "protected service must not get resource flags"
        );
        assert!(matched.is_empty());
    }

    // ---- Phase 32 invariant: non-allowlisted untouched ----

    #[test]
    fn resource_rules_non_allowed_service_gets_no_resource_flags() {
        let rules = [full_rule("stranger.service", 50, 25, 100)];
        let (flags, matched) = evaluate_resource_rules(
            "stranger.service",
            false,
            false, // is_allowed = false
            200 * 1024 * 1024,
            &rules,
        );
        assert!(
            flags.is_empty(),
            "non-allowed service must not get resource flags"
        );
        assert!(matched.is_empty());
    }

    // ---- Phase 32: substring matching ----

    #[test]
    fn resource_rules_no_match_gives_empty() {
        let rules = [mem_rule("specific.service", 100)];
        let (flags, matched) =
            evaluate_resource_rules("other.service", false, true, 200 * 1024 * 1024, &rules);
        assert!(flags.is_empty());
        assert!(matched.is_empty());
    }

    #[test]
    fn resource_rules_multiple_rules_matched_returns_all() {
        let rules = [
            mem_rule("myapp.service", 1024),
            ServiceRule {
                name_match: "myapp".to_string(), // substring match
                cpu_weight: Some(50),
                io_weight: None,
                memory_high_mb: None,
            },
        ];
        let (flags, matched) = evaluate_resource_rules("myapp.service", false, true, 0, &rules);
        assert!(flags.contains(&ServiceFlag::CpuWeightRule));
        assert_eq!(matched.len(), 2);
    }

    // ---- pre-existing: protected/allowed helpers ----

    #[test]
    fn is_protected_matches_default_set() {
        let cfg = AppConfig::default();
        assert!(is_service_protected("syswarden.service", &cfg));
        assert!(is_service_protected("dbus.service", &cfg));
        assert!(!is_service_protected("firefox.service", &cfg));
    }

    #[test]
    fn is_allowed_matches_config() {
        let mut cfg = AppConfig::default();
        cfg.allowed
            .services
            .push("nightly-build.service".to_string());
        assert!(is_service_allowed("nightly-build.service", &cfg));
        assert!(!is_service_allowed("other.service", &cfg));
    }

    #[test]
    fn is_allowed_empty_by_default() {
        let cfg = AppConfig::default();
        assert!(!is_service_allowed("anything.service", &cfg));
    }

    #[test]
    fn analyze_does_not_panic_without_systemd() {
        let cfg = AppConfig::default();
        let _services = analyze(&cfg);
    }

    #[test]
    #[ignore = "queries live systemd D-Bus; requires a running systemd instance"]
    fn analyze_live_returns_services() {
        let cfg = AppConfig::default();
        let services = analyze(&cfg);
        assert!(
            !services.is_empty(),
            "should find at least one service on a live system"
        );
        if let Some(journald) = services
            .iter()
            .find(|s| s.unit == "systemd-journald.service")
        {
            assert!(
                journald.is_protected,
                "systemd-journald.service must be protected"
            );
        }
    }
}
