//! Integration tests for configuration loading and validation (planning.md §7).

use std::path::Path;

use syswarden::config::{self, AppConfig};

// ---------------------------------------------------------------------------
// load
// ---------------------------------------------------------------------------

#[test]
fn load_missing_file_returns_defaults() {
    let cfg = config::load(Path::new("/nonexistent/syswarden.toml"))
        .expect("missing file should return defaults");
    assert!(cfg.global.dry_run, "dry_run must default to true");
}

#[test]
fn load_valid_toml_overrides_field() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        r#"
[global]
dry_run = true
profile = "balanced"
log_level = "info"
"#,
    )
    .unwrap();
    let cfg = config::load(&path).expect("valid TOML");
    assert_eq!(
        format!("{:?}", cfg.global.profile),
        "Balanced",
        "profile should be Balanced"
    );
}

#[test]
fn load_malformed_toml_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.toml");
    std::fs::write(&path, "[[[[not valid toml").unwrap();
    assert!(
        config::load(&path).is_err(),
        "malformed TOML should return Err"
    );
}

// ---------------------------------------------------------------------------
// defaults
// ---------------------------------------------------------------------------

#[test]
fn defaults_dry_run_is_true() {
    let cfg = config::defaults();
    assert!(cfg.global.dry_run);
}

#[test]
fn defaults_allow_flags_are_false() {
    let cfg = config::defaults();
    assert!(!cfg.global.allow_aggressive_actions);
    assert!(!cfg.global.allow_zram_apply);
    assert!(!cfg.global.allow_sysctl_apply);
}

#[test]
fn defaults_protected_includes_syswarden() {
    let cfg = config::defaults();
    assert!(
        cfg.protected.processes.iter().any(|p| p == "syswarden"),
        "syswarden must always be in protected.processes"
    );
    assert!(
        cfg.protected
            .services
            .iter()
            .any(|s| s.contains("syswarden")),
        "syswarden.service must always be in protected.services"
    );
}

// ---------------------------------------------------------------------------
// validate
// ---------------------------------------------------------------------------

#[test]
fn validate_defaults_returns_no_issues() {
    let cfg = config::defaults();
    let issues = config::validate(&cfg);
    assert!(
        issues.is_empty(),
        "default config must be valid; issues: {issues:?}"
    );
}

#[test]
fn validate_bad_threshold_returns_issue() {
    let mut cfg = config::defaults();
    cfg.pressure.thresholds.cpu_moderate = 150.0; // out of [0, 100]
    let issues = config::validate(&cfg);
    assert!(
        !issues.is_empty(),
        "out-of-range threshold must produce an issue"
    );
    assert!(
        issues.iter().any(|i| i.field.contains("cpu_moderate")),
        "issue must name the offending field; issues: {issues:?}"
    );
}

#[test]
fn validate_min_interval_greater_than_max_returns_issue() {
    let mut cfg = config::defaults();
    cfg.polling.min_interval_secs = 60;
    cfg.polling.max_interval_secs = 5; // min > max
    let issues = config::validate(&cfg);
    assert!(
        !issues.is_empty(),
        "min_interval > max_interval must produce an issue"
    );
}

// ---------------------------------------------------------------------------
// AppConfig round-trip
// ---------------------------------------------------------------------------

#[test]
fn appconfig_serializes_and_deserializes() {
    let cfg = config::defaults();
    let toml_str = toml::to_string(&cfg).expect("serialize to TOML");
    let back: AppConfig = toml::from_str(&toml_str).expect("deserialize from TOML");
    assert_eq!(back.global.dry_run, cfg.global.dry_run);
    assert_eq!(
        back.polling.min_interval_secs,
        cfg.polling.min_interval_secs
    );
}
