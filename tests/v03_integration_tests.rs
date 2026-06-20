//! Live integration tests for v0.3 aggressive actions (planning.md §7, Phase 36).
//!
//! All tests are `#[ignore]` — they require root, a running systemd, and a writable
//! `/proc/sys`. Run selectively:
//!
//! ```sh
//! sudo cargo test --test v03_integration_tests -- --ignored
//! ```
//!
//! These tests exercise the real D-Bus and real kernel interfaces. They are never
//! run in CI by default and must not depend on host-specific services.

// ---------------------------------------------------------------------------
// Live drop-in: persistent drop-in write + daemon-reload + rollback
// ---------------------------------------------------------------------------

/// Write a persistent drop-in for a real (harmless) unit via `write_drop_in`,
/// then roll it back end-to-end through the `RollbackStore`.
///
/// Requires: root, running systemd, `systemd-tmpfiles-clean.service` loaded.
#[test]
#[ignore = "requires root + running systemd; writes to /etc/systemd/system/"]
fn live_persistent_drop_in_write_and_rollback() {
    use syswarden::config;
    use syswarden::rollback::{RollbackEntry, RollbackStore};
    use syswarden::systemd::{write_drop_in, UnitProps};

    let unit = "systemd-tmpfiles-clean.service";
    let props = UnitProps {
        cpu_weight: Some(50),
        ..Default::default()
    };

    // Write via the public API — captures prior state and issues daemon-reload.
    let prior = write_drop_in(unit, &props).expect("write_drop_in");
    assert!(prior.path.exists(), "drop-in file must exist after write");

    // Build a rollback entry and store it.
    let prior_state = serde_json::json!({
        "backend": "persistent",
        "path": prior.path,
        "prior_content": prior.prior_content,
        "written_content": prior.written_content,
    });
    let tmp_dir = std::env::temp_dir().join("syswarden_live_dropin_test");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let mut cfg = config::defaults();
    cfg.rollback.dir = tmp_dir.to_string_lossy().to_string();
    let mut store = RollbackStore::open(&cfg).expect("open rollback store");

    let entry = RollbackEntry::new("SetCpuWeight", &format!("unit={unit}"), true)
        .with_prior_state(prior_state);
    let id = entry.id;
    store.record(entry);

    // Apply rollback: revert_drop_in → restore_drop_in_file + daemon-reload.
    store.apply(id).expect("rollback must succeed");

    assert!(
        !prior.path.exists(),
        "drop-in must be removed after rollback (prior_content was None)"
    );

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

// ---------------------------------------------------------------------------
// Live sysctl: round-trip on a safe kernel tunable
// ---------------------------------------------------------------------------

/// Apply a known-safe sysctl change and restore the original value.
///
/// Uses `vm.swappiness` which is safe to modify transiently on any kernel.
/// Requires: root.
#[test]
#[ignore = "requires root; modifies /proc/sys/vm/swappiness transiently"]
fn live_sysctl_round_trip_on_swappiness() {
    use syswarden::sysctl;

    let key = "vm.swappiness";
    let original = sysctl::read(key).expect("read vm.swappiness");

    // Choose a value different from original to confirm the write takes effect.
    let test_value = if original == "60" { "61" } else { "60" };

    let prior = sysctl::apply(key, test_value).expect("apply new value");
    assert_eq!(prior.prior_value, original, "prior must match original");
    assert_eq!(prior.applied_value, test_value);

    // Verify the new value is live.
    let live = sysctl::read(key).expect("read after apply");
    assert_eq!(live, test_value, "kernel should reflect new value");

    // Restore.
    sysctl::apply(key, &original).expect("restore original value");
    let restored = sysctl::read(key).expect("read after restore");
    assert_eq!(restored, original, "value must be restored");
}

// ---------------------------------------------------------------------------
// Live service restart: allowlisted non-protected unit
// ---------------------------------------------------------------------------

/// Restart a real but throwaway unit and verify no error is returned.
///
/// Uses `systemd-tmpfiles-clean.service` — harmless oneshot service.
/// Requires: root, running systemd.
#[test]
#[ignore = "requires root + running systemd; restarts systemd-tmpfiles-clean.service"]
fn live_restart_allowlisted_service() {
    use syswarden::systemd::{get_active_state, restart_unit};

    let unit = "systemd-tmpfiles-clean.service";

    // The service may be inactive (oneshot). Restart it and confirm no error.
    restart_unit(unit).expect("RestartUnit should succeed");

    // After restart, active state is transitioning; just confirm no panic/error.
    let _state = get_active_state(unit).expect("get_active_state should succeed");
}
