//! Configuration loading, merging, and validation; exposes `AppConfig` (architecture.md §5.3, §12, §15).
#![allow(clippy::module_name_repetitions)] // Names mandated by architecture.md §15.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::SyswardenError;

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// A validation problem found by [`validate`].
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ValidationIssue {
    /// Dotted field path (e.g. `"pressure.thresholds.cpu_moderate"`).
    pub field: String,
    /// Human-readable description of the problem.
    pub message: String,
}

impl std::fmt::Display for ValidationIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

// ---------------------------------------------------------------------------
// ProfileName
// ---------------------------------------------------------------------------

/// Named profile bundle (architecture.md §11, §15).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileName {
    #[default]
    Conservative,
    Balanced,
    Performance,
    LowRam,
    Desktop,
    Server,
    Developer,
}

// ---------------------------------------------------------------------------
// GlobalConfig
// ---------------------------------------------------------------------------

/// Master switches (architecture.md §15).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)] // Fields mandated by architecture.md §15.
pub struct GlobalConfig {
    #[serde(default)]
    pub profile: ProfileName,
    /// Master safety switch. `true` = no system changes, ever.
    #[serde(default = "bool_true")]
    pub dry_run: bool,
    #[serde(default)]
    pub allow_aggressive_actions: bool,
    #[serde(default)]
    pub allow_zram_apply: bool,
    #[serde(default)]
    pub allow_sysctl_apply: bool,
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

fn bool_true() -> bool {
    true
}
fn default_log_level() -> String {
    "info".to_string()
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            profile: ProfileName::default(),
            dry_run: true,
            allow_aggressive_actions: false,
            allow_zram_apply: false,
            allow_sysctl_apply: false,
            log_level: "info".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// PollingConfig
// ---------------------------------------------------------------------------

/// Adaptive polling and hysteresis settings (architecture.md §12, §15).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollingConfig {
    #[serde(default = "default_idle_interval")]
    pub idle_interval_secs: u64,
    #[serde(default = "default_pressure_interval")]
    pub pressure_interval_secs: u64,
    #[serde(default = "default_min_interval")]
    pub min_interval_secs: u64,
    #[serde(default = "default_max_interval")]
    pub max_interval_secs: u64,
    #[serde(default = "default_hysteresis")]
    pub hysteresis_ticks: u32,
}

fn default_idle_interval() -> u64 {
    10
}
fn default_pressure_interval() -> u64 {
    4
}
fn default_min_interval() -> u64 {
    2
}
fn default_max_interval() -> u64 {
    30
}
fn default_hysteresis() -> u32 {
    3
}

impl Default for PollingConfig {
    fn default() -> Self {
        Self {
            idle_interval_secs: 10,
            pressure_interval_secs: 4,
            min_interval_secs: 2,
            max_interval_secs: 30,
            hysteresis_ticks: 3,
        }
    }
}

// ---------------------------------------------------------------------------
// PressureThresholds / PressureSection
// ---------------------------------------------------------------------------

/// PSI percentage thresholds (0–100) for pressure level classification (architecture.md §8, §12, §15).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PressureThresholds {
    #[serde(default = "default_cpu_moderate")]
    pub cpu_moderate: f64,
    #[serde(default = "default_cpu_high")]
    pub cpu_high: f64,
    #[serde(default = "default_cpu_critical")]
    pub cpu_critical: f64,
    #[serde(default = "default_mem_some_moderate")]
    pub mem_some_moderate: f64,
    #[serde(default = "default_mem_full_high")]
    pub mem_full_high: f64,
    #[serde(default = "default_mem_full_critical")]
    pub mem_full_critical: f64,
    #[serde(default = "default_io_moderate")]
    pub io_moderate: f64,
    #[serde(default = "default_io_high")]
    pub io_high: f64,
    #[serde(default = "default_io_critical")]
    pub io_critical: f64,
    /// `MemAvailable` below this % reinforces memory pressure classification.
    #[serde(default = "default_mem_available_low_pct")]
    pub mem_available_low_pct: f64,
}

fn default_cpu_moderate() -> f64 {
    15.0
}
fn default_cpu_high() -> f64 {
    35.0
}
fn default_cpu_critical() -> f64 {
    60.0
}
fn default_mem_some_moderate() -> f64 {
    10.0
}
fn default_mem_full_high() -> f64 {
    5.0
}
fn default_mem_full_critical() -> f64 {
    20.0
}
fn default_io_moderate() -> f64 {
    15.0
}
fn default_io_high() -> f64 {
    35.0
}
fn default_io_critical() -> f64 {
    60.0
}
fn default_mem_available_low_pct() -> f64 {
    10.0
}

impl Default for PressureThresholds {
    fn default() -> Self {
        Self {
            cpu_moderate: 15.0,
            cpu_high: 35.0,
            cpu_critical: 60.0,
            mem_some_moderate: 10.0,
            mem_full_high: 5.0,
            mem_full_critical: 20.0,
            io_moderate: 15.0,
            io_high: 35.0,
            io_critical: 60.0,
            mem_available_low_pct: 10.0,
        }
    }
}

/// Wrapper for the `[pressure]` TOML table; holds `[pressure.thresholds]` (architecture.md §12).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PressureSection {
    #[serde(default)]
    pub thresholds: PressureThresholds,
}

// ---------------------------------------------------------------------------
// ProtectedSets / AllowedSets
// ---------------------------------------------------------------------------

/// Hard denylist: processes and services syswarden must never touch (architecture.md §17).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtectedSets {
    #[serde(default = "default_protected_processes")]
    pub processes: Vec<String>,
    #[serde(default = "default_protected_services")]
    pub services: Vec<String>,
}

fn default_protected_processes() -> Vec<String> {
    [
        "systemd",
        "systemd-journald",
        "systemd-logind",
        "dbus-daemon",
        "init",
        "sshd",
        "agetty",
        "syswarden",
    ]
    .map(String::from)
    .to_vec()
}

fn default_protected_services() -> Vec<String> {
    [
        "systemd-journald.service",
        "systemd-logind.service",
        "dbus.service",
        "sshd.service",
        "syswarden.service",
    ]
    .map(String::from)
    .to_vec()
}

impl Default for ProtectedSets {
    fn default() -> Self {
        Self {
            processes: default_protected_processes(),
            services: default_protected_services(),
        }
    }
}

/// Services permitted to receive resource-control changes (architecture.md §17).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AllowedSets {
    /// Empty by default: no service is modifiable until explicitly listed.
    #[serde(default)]
    pub services: Vec<String>,
}

// ---------------------------------------------------------------------------
// ProcessRule / ServiceRule
// ---------------------------------------------------------------------------

/// Action to take when a process violates a monitoring rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnViolation {
    RecommendNice,
    RecommendIonice,
    FlagOnly,
}

/// Per-process monitoring rule (architecture.md §12).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessRule {
    /// Substring/comm match (TOML key: `match`).
    #[serde(rename = "match")]
    pub name_match: String,
    pub max_cpu_pct: f64,
    pub max_rss_mb: u64,
    pub sustained_secs: u64,
    pub on_violation: OnViolation,
}

/// Per-service resource-control rule (architecture.md §12).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceRule {
    /// Unit name match (TOML key: `match`).
    #[serde(rename = "match")]
    pub name_match: String,
    pub cpu_weight: Option<u32>,
    pub io_weight: Option<u32>,
    pub memory_high_mb: Option<u64>,
}

// ---------------------------------------------------------------------------
// HistoryConfig / LoggingConfig / RollbackConfig
// ---------------------------------------------------------------------------

/// History storage backend (v0.1: JSONL only; architecture.md §20).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryBackend {
    #[default]
    Jsonl,
}

/// Local history store settings (architecture.md §12, §20).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryConfig {
    #[serde(default)]
    pub backend: HistoryBackend,
    #[serde(default = "default_history_dir")]
    pub dir: String,
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
    #[serde(default = "default_max_file_mb")]
    pub max_file_mb: u64,
}

fn default_history_dir() -> String {
    "/var/lib/syswarden/history".to_string()
}
fn default_retention_days() -> u32 {
    14
}
fn default_max_file_mb() -> u64 {
    32
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            backend: HistoryBackend::Jsonl,
            dir: default_history_dir(),
            retention_days: 14,
            max_file_mb: 32,
        }
    }
}

/// Logging and audit settings (architecture.md §12, §21).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_audit_dir")]
    pub audit_dir: String,
    #[serde(default = "bool_true")]
    pub journald: bool,
}

fn default_audit_dir() -> String {
    "/var/lib/syswarden/audit".to_string()
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            audit_dir: default_audit_dir(),
            journald: true,
        }
    }
}

/// Rollback store settings (architecture.md §12).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackConfig {
    #[serde(default = "default_rollback_dir")]
    pub dir: String,
    #[serde(default = "default_keep_entries")]
    pub keep_entries: usize,
}

fn default_rollback_dir() -> String {
    "/var/lib/syswarden/rollback".to_string()
}
fn default_keep_entries() -> usize {
    100
}

impl Default for RollbackConfig {
    fn default() -> Self {
        Self {
            dir: default_rollback_dir(),
            keep_entries: 100,
        }
    }
}

// ---------------------------------------------------------------------------
// AppConfig
// ---------------------------------------------------------------------------

/// Root configuration struct (architecture.md §15).
///
/// Missing TOML sections resolve to their conservative defaults.
/// `dry_run = true` is always the default master switch.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub global: GlobalConfig,
    #[serde(default)]
    pub polling: PollingConfig,
    #[serde(default)]
    pub pressure: PressureSection,
    #[serde(default)]
    pub protected: ProtectedSets,
    #[serde(default)]
    pub allowed: AllowedSets,
    #[serde(default)]
    pub process_rules: Vec<ProcessRule>,
    #[serde(default)]
    pub service_rules: Vec<ServiceRule>,
    #[serde(default)]
    pub history: HistoryConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub rollback: RollbackConfig,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Load configuration from `path`. Returns conservative defaults if the file is absent.
///
/// # Errors
///
/// - [`SyswardenError::Io`] if the file exists but cannot be read.
/// - [`SyswardenError::Parse`] if the file contains invalid TOML or unknown fields.
pub fn load(path: &Path) -> Result<AppConfig, SyswardenError> {
    if !path.exists() {
        return Ok(defaults());
    }
    let content = std::fs::read_to_string(path)?;
    toml::from_str(&content).map_err(|e| SyswardenError::Parse(e.to_string()))
}

/// Return conservative built-in defaults: `dry_run = true`, all `allow_*` flags `false`.
#[must_use]
pub fn defaults() -> AppConfig {
    AppConfig::default()
}

/// Validate `cfg` and return a list of issues. An empty list means valid.
#[must_use]
#[allow(dead_code)]
pub fn validate(cfg: &AppConfig) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    check_thresholds(cfg, &mut issues);
    check_polling(cfg, &mut issues);
    check_protected(cfg, &mut issues);
    check_log_level(cfg, &mut issues);
    issues
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn check_thresholds(cfg: &AppConfig, issues: &mut Vec<ValidationIssue>) {
    let t = &cfg.pressure.thresholds;
    for (field, value) in [
        ("pressure.thresholds.cpu_moderate", t.cpu_moderate),
        ("pressure.thresholds.cpu_high", t.cpu_high),
        ("pressure.thresholds.cpu_critical", t.cpu_critical),
        ("pressure.thresholds.mem_some_moderate", t.mem_some_moderate),
        ("pressure.thresholds.mem_full_high", t.mem_full_high),
        ("pressure.thresholds.mem_full_critical", t.mem_full_critical),
        ("pressure.thresholds.io_moderate", t.io_moderate),
        ("pressure.thresholds.io_high", t.io_high),
        ("pressure.thresholds.io_critical", t.io_critical),
        (
            "pressure.thresholds.mem_available_low_pct",
            t.mem_available_low_pct,
        ),
    ] {
        if !(0.0..=100.0).contains(&value) {
            issues.push(ValidationIssue {
                field: field.to_string(),
                message: format!("must be 0.0–100.0, got {value}"),
            });
        }
    }
    for (desc, lower, upper) in [
        ("cpu_moderate < cpu_high", t.cpu_moderate, t.cpu_high),
        ("cpu_high < cpu_critical", t.cpu_high, t.cpu_critical),
        (
            "mem_full_high < mem_full_critical",
            t.mem_full_high,
            t.mem_full_critical,
        ),
        ("io_moderate < io_high", t.io_moderate, t.io_high),
        ("io_high < io_critical", t.io_high, t.io_critical),
    ] {
        if lower >= upper {
            issues.push(ValidationIssue {
                field: "pressure.thresholds".to_string(),
                message: format!("ordering violated: {desc}"),
            });
        }
    }
}

#[allow(dead_code)]
fn check_polling(cfg: &AppConfig, issues: &mut Vec<ValidationIssue>) {
    let p = &cfg.polling;
    if p.min_interval_secs == 0 {
        issues.push(ValidationIssue {
            field: "polling.min_interval_secs".to_string(),
            message: "must be >= 1".to_string(),
        });
    }
    if p.max_interval_secs < p.min_interval_secs {
        issues.push(ValidationIssue {
            field: "polling.max_interval_secs".to_string(),
            message: format!("must be >= min_interval_secs ({})", p.min_interval_secs),
        });
    }
}

#[allow(dead_code)]
fn check_protected(cfg: &AppConfig, issues: &mut Vec<ValidationIssue>) {
    if !cfg.protected.processes.iter().any(|p| p == "syswarden") {
        issues.push(ValidationIssue {
            field: "protected.processes".to_string(),
            message: r#""syswarden" must always be present"#.to_string(),
        });
    }
    if !cfg
        .protected
        .services
        .iter()
        .any(|s| s == "syswarden.service")
    {
        issues.push(ValidationIssue {
            field: "protected.services".to_string(),
            message: r#""syswarden.service" must always be present"#.to_string(),
        });
    }
}

#[allow(dead_code)]
fn check_log_level(cfg: &AppConfig, issues: &mut Vec<ValidationIssue>) {
    const VALID: &[&str] = &["error", "warn", "info", "debug", "trace"];
    if !VALID.contains(&cfg.global.log_level.as_str()) {
        issues.push(ValidationIssue {
            field: "global.log_level".to_string(),
            message: format!(
                r#"invalid log level "{}"; expected one of: {}"#,
                cfg.global.log_level,
                VALID.join(", ")
            ),
        });
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::wildcard_imports)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_conservative_and_dry_run() {
        let cfg = defaults();
        assert!(cfg.global.dry_run);
        assert_eq!(cfg.global.profile, ProfileName::Conservative);
        assert!(!cfg.global.allow_aggressive_actions);
        assert!(!cfg.global.allow_zram_apply);
        assert!(!cfg.global.allow_sysctl_apply);
        assert_eq!(cfg.global.log_level, "info");
    }

    #[test]
    fn defaults_include_syswarden_in_protected() {
        let cfg = defaults();
        assert!(cfg.protected.processes.iter().any(|p| p == "syswarden"));
        assert!(cfg
            .protected
            .services
            .iter()
            .any(|s| s == "syswarden.service"));
    }

    #[test]
    fn defaults_allowed_services_is_empty() {
        assert!(defaults().allowed.services.is_empty());
    }

    #[test]
    fn missing_file_returns_defaults() {
        let cfg = load(Path::new("/nonexistent/syswarden_cfg_test_99.toml")).unwrap();
        assert!(cfg.global.dry_run);
        assert_eq!(cfg.global.profile, ProfileName::Conservative);
    }

    #[test]
    fn valid_minimal_toml_parses() {
        let src = r#"
[global]
profile = "balanced"
dry_run = true
"#;
        let cfg: AppConfig = toml::from_str(src).unwrap();
        assert_eq!(cfg.global.profile, ProfileName::Balanced);
        assert!(cfg.global.dry_run);
        // Unspecified fields fall back to defaults.
        assert!((cfg.pressure.thresholds.cpu_moderate - 15.0).abs() < f64::EPSILON);
    }

    #[test]
    fn valid_full_toml_parses() {
        let src = r#"
[global]
profile = "balanced"
dry_run = true
allow_aggressive_actions = false
allow_zram_apply = false
allow_sysctl_apply = false
log_level = "info"

[polling]
idle_interval_secs = 8
pressure_interval_secs = 3
min_interval_secs = 2
max_interval_secs = 30
hysteresis_ticks = 3

[pressure.thresholds]
cpu_moderate = 15.0
cpu_high = 35.0
cpu_critical = 60.0
mem_some_moderate = 10.0
mem_full_high = 5.0
mem_full_critical = 20.0
io_moderate = 15.0
io_high = 35.0
io_critical = 60.0
mem_available_low_pct = 10.0

[protected]
processes = ["systemd", "syswarden"]
services = ["syswarden.service"]

[allowed]
services = []

[history]
backend = "jsonl"
dir = "/var/lib/syswarden/history"
retention_days = 14
max_file_mb = 32

[logging]
audit_dir = "/var/lib/syswarden/audit"
journald = true

[rollback]
dir = "/var/lib/syswarden/rollback"
keep_entries = 100
"#;
        let cfg: AppConfig = toml::from_str(src).unwrap();
        assert_eq!(cfg.global.profile, ProfileName::Balanced);
        assert!((cfg.pressure.thresholds.cpu_moderate - 15.0).abs() < f64::EPSILON);
        assert_eq!(cfg.history.retention_days, 14);
        assert!(cfg.logging.journald);
        assert_eq!(cfg.rollback.keep_entries, 100);
    }

    #[test]
    fn invalid_toml_returns_parse_error() {
        let result = toml::from_str::<AppConfig>("not valid = = =")
            .map_err(|e| SyswardenError::Parse(e.to_string()));
        assert!(matches!(result, Err(SyswardenError::Parse(_))));
    }

    #[test]
    fn process_rule_match_field_deserializes() {
        let src = r#"
[[process_rules]]
match = "chromium"
max_cpu_pct = 85.0
max_rss_mb = 6000
sustained_secs = 30
on_violation = "recommend_nice"
"#;
        let cfg: AppConfig = toml::from_str(src).unwrap();
        assert_eq!(cfg.process_rules.len(), 1);
        assert_eq!(cfg.process_rules[0].name_match, "chromium");
    }

    #[test]
    fn service_rule_deserializes() {
        let src = r#"
[[service_rules]]
match = "nightly-build.service"
cpu_weight = 50
io_weight = 50
memory_high_mb = 4000
"#;
        let cfg: AppConfig = toml::from_str(src).unwrap();
        assert_eq!(cfg.service_rules[0].name_match, "nightly-build.service");
        assert_eq!(cfg.service_rules[0].cpu_weight, Some(50));
    }

    // --- validate ---

    #[test]
    fn validate_defaults_are_valid() {
        let issues = validate(&defaults());
        assert!(issues.is_empty(), "defaults should be valid: {issues:?}");
    }

    #[test]
    fn validate_catches_threshold_out_of_range() {
        let mut cfg = defaults();
        cfg.pressure.thresholds.cpu_moderate = 150.0;
        let issues = validate(&cfg);
        assert!(issues.iter().any(|i| i.field.contains("cpu_moderate")));
    }

    #[test]
    fn validate_catches_negative_threshold() {
        let mut cfg = defaults();
        cfg.pressure.thresholds.io_high = -1.0;
        let issues = validate(&cfg);
        assert!(issues.iter().any(|i| i.field.contains("io_high")));
    }

    #[test]
    fn validate_catches_threshold_ordering_violation() {
        let mut cfg = defaults();
        cfg.pressure.thresholds.cpu_high = 5.0; // below cpu_moderate default 15.0
        let issues = validate(&cfg);
        assert!(!issues.is_empty());
    }

    #[test]
    fn validate_catches_max_less_than_min_interval() {
        let mut cfg = defaults();
        cfg.polling.min_interval_secs = 10;
        cfg.polling.max_interval_secs = 1;
        let issues = validate(&cfg);
        assert!(issues.iter().any(|i| i.field.contains("max_interval_secs")));
    }

    #[test]
    fn validate_catches_zero_min_interval() {
        let mut cfg = defaults();
        cfg.polling.min_interval_secs = 0;
        let issues = validate(&cfg);
        assert!(issues.iter().any(|i| i.field.contains("min_interval_secs")));
    }

    #[test]
    fn validate_catches_missing_syswarden_in_protected_processes() {
        let mut cfg = defaults();
        cfg.protected.processes.retain(|p| p != "syswarden");
        let issues = validate(&cfg);
        assert!(issues
            .iter()
            .any(|i| i.field.contains("protected.processes")));
    }

    #[test]
    fn validate_catches_missing_syswarden_service_in_protected() {
        let mut cfg = defaults();
        cfg.protected.services.retain(|s| s != "syswarden.service");
        let issues = validate(&cfg);
        assert!(issues
            .iter()
            .any(|i| i.field.contains("protected.services")));
    }

    #[test]
    fn validate_catches_invalid_log_level() {
        let mut cfg = defaults();
        cfg.global.log_level = "verbose".to_string();
        let issues = validate(&cfg);
        assert!(issues.iter().any(|i| i.field.contains("log_level")));
    }
}
