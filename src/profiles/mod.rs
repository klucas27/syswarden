//! Built-in profile definitions and `ProfileConfig` resolution (architecture.md §5.14, §11).
#![allow(dead_code)]

use crate::config::{AppConfig, ProfileName};

// ---------------------------------------------------------------------------
// ActionRisk
// ---------------------------------------------------------------------------

/// Risk level for a planned action (architecture.md §10, §15).
///
/// Defined here because `profiles` is the first module that needs it.
/// The `actions` module (Phase 13) imports this type from `profiles`.
///
/// `Ord` uses declaration order: `Safe < Moderate < Aggressive < Prohibited`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ActionRisk {
    Safe,
    Moderate,
    Aggressive,
    Prohibited,
}

// ---------------------------------------------------------------------------
// ProfileConfig
// ---------------------------------------------------------------------------

/// Runtime behavior bundle for the active profile (architecture.md §11, §15).
///
/// Holds resolved polling intervals, the maximum permitted action risk, and
/// fine-grained permission booleans for each category of action. The safety
/// layer (Phase 12) consults these before allowing any action to execute.
///
/// Global flags in `AppConfig` act as an additional gate on top of profile
/// permissions: e.g. `allow_zram_apply=false` in config overrides even a
/// profile that sets `allow_zram_apply=true`.
#[allow(clippy::struct_excessive_bools)] // Permission fields mandated by architecture.md §11.
#[derive(Debug, Clone)]
pub struct ProfileConfig {
    /// Polling interval (seconds) when the system is idle.
    pub idle_interval_secs: u64,
    /// Polling interval (seconds) when pressure is elevated.
    pub pressure_interval_secs: u64,
    /// Highest risk level this profile permits; `Prohibited` is never permitted.
    pub max_allowed_risk: ActionRisk,
    /// May re-nice non-protected processes.
    pub allow_nice: bool,
    /// May apply `ionice` to non-protected processes.
    pub allow_ionice: bool,
    /// May set `CPUWeight` on allowed services.
    pub allow_cpu_weight: bool,
    /// May set `IOWeight` on allowed services.
    pub allow_io_weight: bool,
    /// May set `MemoryHigh` on allowed services.
    pub allow_memory_high: bool,
    /// May set `MemoryMax` on allowed services (only after `MemoryHigh` tried).
    pub allow_memory_max: bool,
    /// May restart explicitly allowed non-critical services.
    pub allow_service_restart: bool,
    /// May stop explicitly allowed non-critical services.
    pub allow_service_stop: bool,
    /// May apply zram configuration (still gated by `AppConfig.global.allow_zram_apply`).
    pub allow_zram_apply: bool,
}

// ---------------------------------------------------------------------------
// Built-in profile constructors (private)
// ---------------------------------------------------------------------------

fn conservative() -> ProfileConfig {
    ProfileConfig {
        idle_interval_secs: 10,
        pressure_interval_secs: 4,
        max_allowed_risk: ActionRisk::Safe,
        allow_nice: false,
        allow_ionice: false,
        allow_cpu_weight: false,
        allow_io_weight: false,
        allow_memory_high: false,
        allow_memory_max: false,
        allow_service_restart: false,
        allow_service_stop: false,
        allow_zram_apply: false,
    }
}

fn balanced() -> ProfileConfig {
    ProfileConfig {
        idle_interval_secs: 8,
        pressure_interval_secs: 3,
        max_allowed_risk: ActionRisk::Moderate,
        allow_nice: true,
        allow_ionice: false,
        allow_cpu_weight: true,
        allow_io_weight: false,
        allow_memory_high: true,
        allow_memory_max: false,
        allow_service_restart: false,
        allow_service_stop: false,
        allow_zram_apply: false,
    }
}

fn performance() -> ProfileConfig {
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
        allow_service_restart: false,
        allow_service_stop: false,
        allow_zram_apply: true,
    }
}

fn low_ram() -> ProfileConfig {
    ProfileConfig {
        idle_interval_secs: 6,
        pressure_interval_secs: 2,
        max_allowed_risk: ActionRisk::Moderate,
        allow_nice: true,
        allow_ionice: true,
        allow_cpu_weight: false,
        allow_io_weight: false,
        allow_memory_high: true,
        allow_memory_max: false,
        allow_service_restart: false,
        allow_service_stop: false,
        allow_zram_apply: true,
    }
}

fn desktop() -> ProfileConfig {
    ProfileConfig {
        idle_interval_secs: 8,
        pressure_interval_secs: 3,
        max_allowed_risk: ActionRisk::Moderate,
        allow_nice: true,
        allow_ionice: false,
        allow_cpu_weight: true,
        allow_io_weight: false,
        allow_memory_high: true,
        allow_memory_max: false,
        allow_service_restart: false,
        allow_service_stop: false,
        allow_zram_apply: false,
    }
}

fn server() -> ProfileConfig {
    ProfileConfig {
        idle_interval_secs: 10,
        pressure_interval_secs: 4,
        max_allowed_risk: ActionRisk::Moderate,
        allow_nice: false,
        allow_ionice: false,
        allow_cpu_weight: true,
        allow_io_weight: true,
        allow_memory_high: true,
        allow_memory_max: false,
        allow_service_restart: false,
        allow_service_stop: false,
        allow_zram_apply: false,
    }
}

fn developer() -> ProfileConfig {
    ProfileConfig {
        idle_interval_secs: 6,
        pressure_interval_secs: 2,
        max_allowed_risk: ActionRisk::Moderate,
        allow_nice: true,
        allow_ionice: true,
        allow_cpu_weight: true,
        allow_io_weight: true,
        allow_memory_high: true,
        allow_memory_max: false,
        allow_service_restart: false,
        allow_service_stop: false,
        allow_zram_apply: false,
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// All supported profile names, in declaration order.
#[must_use]
pub fn all() -> &'static [ProfileName] {
    &[
        ProfileName::Conservative,
        ProfileName::Balanced,
        ProfileName::Performance,
        ProfileName::LowRam,
        ProfileName::Desktop,
        ProfileName::Server,
        ProfileName::Developer,
    ]
}

/// Resolve the built-in [`ProfileConfig`] for `name`, then apply global config
/// overrides (architecture.md §11, §17):
///
/// - `allow_aggressive_actions=false` caps `max_allowed_risk` at `Moderate`.
/// - `allow_zram_apply=false` zeroes `allow_zram_apply` regardless of profile.
///
/// These caps ensure global safety switches always win over profile permissions.
#[must_use]
pub fn resolve(name: &ProfileName, config: &AppConfig) -> ProfileConfig {
    let mut profile = match name {
        ProfileName::Conservative => conservative(),
        ProfileName::Balanced => balanced(),
        ProfileName::Performance => performance(),
        ProfileName::LowRam => low_ram(),
        ProfileName::Desktop => desktop(),
        ProfileName::Server => server(),
        ProfileName::Developer => developer(),
    };

    // Global flag overrides — config gates always win over profile permissions.
    if !config.global.allow_aggressive_actions {
        profile.max_allowed_risk = profile.max_allowed_risk.min(ActionRisk::Moderate);
        profile.allow_memory_max = false;
    }
    if !config.global.allow_zram_apply {
        profile.allow_zram_apply = false;
    }

    profile
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_risk_ordering() {
        assert!(ActionRisk::Safe < ActionRisk::Moderate);
        assert!(ActionRisk::Moderate < ActionRisk::Aggressive);
        assert!(ActionRisk::Aggressive < ActionRisk::Prohibited);
    }

    #[test]
    fn all_profiles_listed() {
        let names = all();
        assert_eq!(names.len(), 7);
        assert!(names.contains(&ProfileName::Conservative));
        assert!(names.contains(&ProfileName::Performance));
        assert!(names.contains(&ProfileName::Developer));
    }

    #[test]
    fn all_profiles_resolve_without_panic() {
        let cfg = AppConfig::default();
        for name in all() {
            let _ = resolve(name, &cfg);
        }
    }

    #[test]
    fn conservative_allows_safe_only() {
        let cfg = AppConfig::default();
        let p = resolve(&ProfileName::Conservative, &cfg);
        assert_eq!(p.max_allowed_risk, ActionRisk::Safe);
        assert!(!p.allow_nice);
        assert!(!p.allow_memory_high);
        assert!(!p.allow_zram_apply);
    }

    #[test]
    fn balanced_allows_moderate_not_aggressive() {
        let cfg = AppConfig::default();
        let p = resolve(&ProfileName::Balanced, &cfg);
        assert_eq!(p.max_allowed_risk, ActionRisk::Moderate);
        assert!(p.allow_nice);
        assert!(p.allow_memory_high);
        assert!(!p.allow_memory_max);
        assert!(!p.allow_service_stop);
    }

    #[test]
    fn performance_allows_aggressive_but_capped_by_global_flag() {
        // Default config has allow_aggressive_actions=false → capped at Moderate.
        let cfg = AppConfig::default();
        let p = resolve(&ProfileName::Performance, &cfg);
        assert_eq!(p.max_allowed_risk, ActionRisk::Moderate);
        assert!(
            !p.allow_memory_max,
            "memory_max blocked when aggressive disabled"
        );
    }

    #[test]
    fn performance_with_aggressive_flag_enabled() {
        let mut cfg = AppConfig::default();
        cfg.global.allow_aggressive_actions = true;
        cfg.global.allow_zram_apply = true;
        let p = resolve(&ProfileName::Performance, &cfg);
        assert_eq!(p.max_allowed_risk, ActionRisk::Aggressive);
        assert!(p.allow_memory_max);
        assert!(p.allow_zram_apply);
    }

    #[test]
    fn low_ram_no_service_stop() {
        let cfg = AppConfig::default();
        let p = resolve(&ProfileName::LowRam, &cfg);
        assert!(!p.allow_service_stop);
        assert!(p.allow_ionice);
        assert!(p.allow_memory_high);
    }

    #[test]
    fn desktop_allows_nice_and_cpu_weight() {
        let cfg = AppConfig::default();
        let p = resolve(&ProfileName::Desktop, &cfg);
        assert!(p.allow_nice);
        assert!(p.allow_cpu_weight);
        assert!(!p.allow_service_stop);
        assert!(!p.allow_service_restart);
    }

    #[test]
    fn server_allows_memory_high_and_io_weight() {
        let cfg = AppConfig::default();
        let p = resolve(&ProfileName::Server, &cfg);
        assert!(p.allow_memory_high);
        assert!(p.allow_io_weight);
        assert!(!p.allow_nice, "server: minimal priority intervention");
        assert!(!p.allow_service_restart);
    }

    #[test]
    fn developer_allows_nice_ionice_and_io_cpu_weight() {
        let cfg = AppConfig::default();
        let p = resolve(&ProfileName::Developer, &cfg);
        assert!(p.allow_nice);
        assert!(p.allow_ionice);
        assert!(p.allow_cpu_weight);
        assert!(p.allow_io_weight);
        assert!(!p.allow_service_stop);
    }

    #[test]
    fn zram_apply_blocked_by_global_flag() {
        // Even if the profile permits zram, the global flag gates it.
        let mut cfg = AppConfig::default();
        cfg.global.allow_aggressive_actions = true;
        // allow_zram_apply remains false (default)
        let p = resolve(&ProfileName::Performance, &cfg);
        assert!(!p.allow_zram_apply);
    }

    #[test]
    fn polling_intervals_match_spec() {
        let cfg = AppConfig::default();
        assert_eq!(
            resolve(&ProfileName::Conservative, &cfg).idle_interval_secs,
            10
        );
        assert_eq!(resolve(&ProfileName::Balanced, &cfg).idle_interval_secs, 8);
        assert_eq!(
            resolve(&ProfileName::Performance, &cfg).pressure_interval_secs,
            2
        );
        assert_eq!(resolve(&ProfileName::Server, &cfg).idle_interval_secs, 10);
    }
}
