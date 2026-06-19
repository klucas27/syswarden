//! Read-only systemd service analysis and flagging (architecture.md §5.7).
#![allow(dead_code)]

use crate::config::AppConfig;
use crate::error::SyswardenError;

/// Why a service was flagged (architecture.md §5.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceFlag {
    /// `active_state == "failed"`.
    Failing,
    /// `sub_state == "auto-restart"` — service is restart-looping.
    Restarting,
    /// `memory_current` exceeds a matching service rule's `memory_high_mb`.
    HighMemory,
    /// Any rule-based threshold (currently `HighMemory`) was exceeded.
    RuleViolation,
}

/// Per-service snapshot for one collection tick (architecture.md §15).
///
/// `cpu_usage` is `CPUUsageNSec` (nanoseconds of CPU time since service start).
/// `memory_current` is `MemoryCurrent` in bytes; systemd returns `u64::MAX` when
/// memory accounting is disabled for the unit — callers must treat that as "unknown".
/// `restarts` is `NRestarts` from the `org.freedesktop.systemd1.Service` interface.
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
}

fn is_service_protected(unit: &str, config: &AppConfig) -> bool {
    config.protected.services.iter().any(|s| s == unit)
}

fn is_service_allowed(unit: &str, config: &AppConfig) -> bool {
    config.allowed.services.iter().any(|s| s == unit)
}

fn compute_flags(info: &ServiceInfo, config: &AppConfig) -> Vec<ServiceFlag> {
    let mut high_mem = false;
    // u64::MAX means accounting disabled — treat as 0 to avoid false positives.
    let safe_mem = if info.memory_current == u64::MAX {
        0
    } else {
        info.memory_current
    };

    for rule in &config.service_rules {
        if !info.unit.contains(rule.name_match.as_str()) {
            continue;
        }
        if let Some(limit_mb) = rule.memory_high_mb {
            if safe_mem / (1024 * 1024) > limit_mb {
                high_mem = true;
            }
        }
    }

    let mut flags = Vec::new();
    if info.active_state == "failed" {
        flags.push(ServiceFlag::Failing);
    }
    if info.sub_state == "auto-restart" {
        flags.push(ServiceFlag::Restarting);
    }
    if high_mem {
        flags.push(ServiceFlag::HighMemory);
        flags.push(ServiceFlag::RuleViolation);
    }
    flags
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
        };
        info.flags = compute_flags(&info, config);
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

    fn make_info(unit: &str, active: &str, sub: &str, mem_bytes: u64) -> ServiceInfo {
        ServiceInfo {
            unit: unit.to_string(),
            active_state: active.to_string(),
            sub_state: sub.to_string(),
            is_protected: false,
            is_allowed: false,
            cpu_usage: 0,
            memory_current: mem_bytes,
            restarts: 0,
            flags: Vec::new(),
        }
    }

    #[test]
    fn compute_flags_failing_service() {
        let cfg = AppConfig::default();
        let info = make_info("foo.service", "failed", "failed", 0);
        let flags = compute_flags(&info, &cfg);
        assert!(flags.contains(&ServiceFlag::Failing));
        assert!(!flags.contains(&ServiceFlag::RuleViolation));
    }

    #[test]
    fn compute_flags_restarting_service() {
        let cfg = AppConfig::default();
        let info = make_info("foo.service", "activating", "auto-restart", 0);
        let flags = compute_flags(&info, &cfg);
        assert!(flags.contains(&ServiceFlag::Restarting));
        assert!(!flags.contains(&ServiceFlag::Failing));
    }

    #[test]
    fn compute_flags_high_memory_rule_violation() {
        let mut cfg = AppConfig::default();
        cfg.service_rules.push(ServiceRule {
            name_match: "nightly-build.service".to_string(),
            cpu_weight: None,
            io_weight: None,
            memory_high_mb: Some(4096),
        });
        // 5 GiB > 4096 MiB
        let info = make_info(
            "nightly-build.service",
            "active",
            "running",
            5 * 1024 * 1024 * 1024,
        );
        let flags = compute_flags(&info, &cfg);
        assert!(flags.contains(&ServiceFlag::HighMemory));
        assert!(flags.contains(&ServiceFlag::RuleViolation));
        assert!(!flags.contains(&ServiceFlag::Failing));
    }

    #[test]
    fn compute_flags_empty_within_limits() {
        let mut cfg = AppConfig::default();
        cfg.service_rules.push(ServiceRule {
            name_match: "myapp.service".to_string(),
            cpu_weight: None,
            io_weight: None,
            memory_high_mb: Some(4096),
        });
        // 1 GiB < 4096 MiB — within limits
        let info = make_info("myapp.service", "active", "running", 1024 * 1024 * 1024);
        let flags = compute_flags(&info, &cfg);
        assert!(flags.is_empty());
    }

    #[test]
    fn compute_flags_memory_max_unavailable_not_flagged() {
        let mut cfg = AppConfig::default();
        cfg.service_rules.push(ServiceRule {
            name_match: "myapp.service".to_string(),
            cpu_weight: None,
            io_weight: None,
            memory_high_mb: Some(100),
        });
        // u64::MAX = accounting disabled — must NOT trigger HighMemory
        let info = make_info("myapp.service", "active", "running", u64::MAX);
        let flags = compute_flags(&info, &cfg);
        assert!(!flags.contains(&ServiceFlag::HighMemory));
    }

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
        // Verifies the degradation contract: analyze must never panic regardless of
        // whether D-Bus / systemd is available in the test environment.
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
