//! systemd D-Bus integration: unit property reads, transient and persistent resource-control
//! writes (architecture.md §5.8, §18, Phase 29).
//!
//! ## Invariants
//! - All D-Bus calls use explicit typed arguments — no shell, no string interpolation
//!   (architecture.md §17).
//! - Persistent drop-ins are written only under `/etc/systemd/system/<unit>.d/`; path
//!   components are validated to prevent traversal (architecture.md §17).
//! - `set_unit_properties` and `write_drop_in` capture prior state and return it so the caller
//!   can record it in `RollbackEntry.prior_state` (architecture.md §5.15).
//! - No safety decisions are made here; the caller is responsible for all gating.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

// ---------------------------------------------------------------------------
// D-Bus well-known constants
// ---------------------------------------------------------------------------

const SYSTEMD_BUS: &str = "org.freedesktop.systemd1";
const MANAGER_PATH: &str = "/org/freedesktop/systemd1";
const MANAGER_IFACE: &str = "org.freedesktop.systemd1.Manager";
const SERVICE_IFACE: &str = "org.freedesktop.systemd1.Service";
const UNIT_IFACE: &str = "org.freedesktop.systemd1.Unit";
const PROPS_IFACE: &str = "org.freedesktop.DBus.Properties";

const PROP_ACTIVE_STATE: &str = "ActiveState";

const PROP_CPU_WEIGHT: &str = "CPUWeight";
const PROP_IO_WEIGHT: &str = "IOWeight";
const PROP_MEMORY_HIGH: &str = "MemoryHigh";
const PROP_MEMORY_MAX: &str = "MemoryMax";

// Drop-in file constants (Phase 29, architecture.md §5.8).
const DROPIN_DIR_BASE: &str = "/etc/systemd/system";
const DROPIN_FILENAME: &str = "50-syswarden.conf";
const KNOWN_UNIT_SUFFIXES: &[&str] = &[
    ".service", ".mount", ".socket", ".target", ".timer", ".scope", ".slice",
];

// ---------------------------------------------------------------------------
// DropInPriorState (Phase 29)
// ---------------------------------------------------------------------------

/// Prior state of the persistent drop-in file before syswarden wrote it.
///
/// Stored in `RollbackEntry.prior_state` (architecture.md §5.15).
/// `prior_content = None` means the file did not exist before we created it.
/// `written_content` is the exact bytes we wrote, used by rollback to detect
/// external modifications before reverting (Phase 30).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DropInPriorState {
    /// Absolute path of the drop-in we wrote.
    pub path: PathBuf,
    /// Content that was in the file before we wrote it; `None` = did not exist.
    pub prior_content: Option<String>,
    /// Exact content syswarden wrote — used by rollback to detect tampering.
    pub written_content: String,
}

// ---------------------------------------------------------------------------
// UnitProps
// ---------------------------------------------------------------------------

/// Resource-control properties for one systemd service unit (architecture.md §5.8).
///
/// **Dual-purpose type:**
/// - When *writing*: `Some(v)` applies that value to the unit; `None` skips the field
///   (the property is left unchanged in systemd).
/// - When *reading* (prior-state capture): `Some(v)` is the live D-Bus value; `None`
///   means the property was absent from the response.
///
/// `MemoryHigh`/`MemoryMax = u64::MAX` is systemd's representation of "unlimited". We preserve
/// it as-is so rollback can restore the unlimited state accurately, unlike the `cgroups`
/// module which normalises it to `None` for display purposes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitProps {
    /// `CPUWeight` (1–10000; systemd default 100). `None` = not set / skip on write.
    pub cpu_weight: Option<u64>,
    /// `IOWeight` (1–10000; systemd default 100). `None` = not set / skip on write.
    pub io_weight: Option<u64>,
    /// `MemoryHigh` in bytes. `u64::MAX` = unlimited (systemd's sentinel). `None` = skip.
    pub memory_high: Option<u64>,
    /// `MemoryMax` in bytes. `u64::MAX` = unlimited. `None` = skip (Phase 33).
    pub memory_max: Option<u64>,
}

impl UnitProps {
    /// `true` when all fields are `None` — nothing to write to systemd.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cpu_weight.is_none()
            && self.io_weight.is_none()
            && self.memory_high.is_none()
            && self.memory_max.is_none()
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Read the current resource-control properties for `unit` via system D-Bus.
///
/// Resolves the unit's object path with `Manager.GetUnit`, then reads `CPUWeight`,
/// `IOWeight`, and `MemoryHigh` from the `org.freedesktop.systemd1.Service` interface.
///
/// # Errors
/// System D-Bus unavailable; unit not loaded; property parse failure.
pub fn read_unit_props(unit: &str) -> Result<UnitProps> {
    let conn = Connection::system().context("systemd: connect to system D-Bus")?;
    let path = resolve_unit_path(&conn, unit)?;
    props_from_dbus(&conn, path.as_str())
}

/// Apply `new_props` to `unit` as **transient** runtime properties (architecture.md §18).
///
/// Steps:
/// 1. Capture prior state via `GetAll` — returned to caller for rollback recording.
/// 2. Build an explicit typed `a(sv)` array (no shell, no string interpolation).
/// 3. Call `Manager.SetUnitProperties(unit, runtime, props)`.
///
/// Only `Some(_)` fields are applied; `None` fields are skipped so unrelated properties
/// are not inadvertently cleared. `runtime = true` → changes auto-clear on reboot.
///
/// # Errors
/// System D-Bus unavailable; unit not loaded; `SetUnitProperties` D-Bus error.
pub fn set_unit_properties(unit: &str, new_props: &UnitProps, runtime: bool) -> Result<UnitProps> {
    let conn = Connection::system().context("systemd: connect to system D-Bus")?;
    let path = resolve_unit_path(&conn, unit)?;

    // Capture prior state BEFORE writing — returned to caller for rollback.
    let prior = props_from_dbus(&conn, path.as_str())?;

    let dbus_props = build_dbus_props(new_props)?;
    if dbus_props.is_empty() {
        return Ok(prior);
    }

    // SetUnitProperties(name: s, runtime: b, properties: a(sv)) → void
    let manager = zbus::blocking::Proxy::new(&conn, SYSTEMD_BUS, MANAGER_PATH, MANAGER_IFACE)
        .context("systemd: create manager proxy")?;
    manager
        .call::<_, _, ()>("SetUnitProperties", &(unit, runtime, dbus_props))
        .context("systemd: SetUnitProperties failed")?;

    Ok(prior)
}

/// Write a persistent drop-in for `unit` at
/// `/etc/systemd/system/<unit>.d/50-syswarden.conf` and issue `daemon-reload`
/// via D-Bus (architecture.md §5.8, Phase 29).
///
/// Steps:
/// 1. Validate `unit` name (no `/`, no `..`, known suffix).
/// 2. Capture prior state (file content or absence).
/// 3. Render `[Service]` INI section from `props`.
/// 4. Write the file (creates the `.d` dir if needed).
/// 5. Issue `Manager.Reload` via D-Bus.
/// 6. Return `DropInPriorState` for the caller to record in rollback.
///
/// # Errors
/// Invalid unit name; path traversal detected; file write fails; D-Bus unavailable.
///
/// # Panics
/// Never in practice — `resolve_drop_in_path` always returns a path with a parent component.
pub fn write_drop_in(unit: &str, props: &UnitProps) -> Result<DropInPriorState> {
    let path = resolve_drop_in_path(unit)?;
    let prior_content = read_drop_in_content(&path);
    let written_content = render_drop_in(props);

    let dir = path.parent().expect("drop-in path always has a parent dir");
    std::fs::create_dir_all(dir)
        .with_context(|| format!("systemd: create drop-in dir {d}", d = dir.display()))?;

    std::fs::write(&path, &written_content)
        .with_context(|| format!("systemd: write drop-in {p}", p = path.display()))?;

    daemon_reload()?;

    Ok(DropInPriorState {
        path,
        prior_content,
        written_content,
    })
}

/// Remove the syswarden drop-in for `unit` and issue `daemon-reload`.
///
/// No-op if the file does not exist. Used by Phase 30 rollback when the
/// prior state was `None` (syswarden created the file from scratch).
///
/// # Errors
/// Invalid unit name; file removal fails; D-Bus unavailable.
pub fn remove_drop_in(unit: &str) -> Result<()> {
    let path = resolve_drop_in_path(unit)?;
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("systemd: remove drop-in {}", path.display()))?;
    }
    daemon_reload()
}

/// Issue `org.freedesktop.systemd1.Manager.Reload` via the system D-Bus (no shell).
///
/// # Errors
/// System D-Bus unavailable; Reload call rejected by systemd.
pub fn daemon_reload() -> Result<()> {
    let conn = Connection::system().context("systemd: connect to system D-Bus")?;
    let manager = zbus::blocking::Proxy::new(&conn, SYSTEMD_BUS, MANAGER_PATH, MANAGER_IFACE)
        .context("systemd: create manager proxy")?;
    manager
        .call::<_, _, ()>("Reload", &())
        .context("systemd: Manager.Reload failed")?;
    Ok(())
}

/// Read the `ActiveState` property for `unit` from the systemd D-Bus (Phase 34).
///
/// Returns the raw string value (e.g. `"active"`, `"inactive"`, `"failed"`).
/// Used to capture prior state before `RestartUnit` / `StopUnit`.
///
/// # Errors
/// System D-Bus unavailable; unit not loaded; `ActiveState` property absent.
pub fn get_active_state(unit: &str) -> Result<String> {
    let conn = Connection::system().context("systemd: connect to system D-Bus")?;
    let path = resolve_unit_path(&conn, unit)?;
    let proxy = zbus::blocking::Proxy::new(&conn, SYSTEMD_BUS, path.as_str(), PROPS_IFACE)
        .context("systemd: create properties proxy")?;
    let all: HashMap<String, OwnedValue> = proxy
        .call("GetAll", &(UNIT_IFACE,))
        .context("systemd: GetAll(Unit) failed")?;
    all.get(PROP_ACTIVE_STATE)
        .and_then(|v| {
            if let Value::Str(s) = &**v {
                Some(s.to_string())
            } else {
                None
            }
        })
        .ok_or_else(|| anyhow::anyhow!("systemd: ActiveState not found for {unit}"))
}

/// Issue `Manager.RestartUnit(unit, "replace")` via the system D-Bus (Phase 34).
///
/// Uses `"replace"` mode: if a pending job exists it is replaced rather than
/// queued in addition. No shell, no arg interpolation.
///
/// # Errors
/// System D-Bus unavailable; `RestartUnit` call rejected by systemd.
pub fn restart_unit(unit: &str) -> Result<()> {
    let conn = Connection::system().context("systemd: connect to system D-Bus")?;
    let manager = zbus::blocking::Proxy::new(&conn, SYSTEMD_BUS, MANAGER_PATH, MANAGER_IFACE)
        .context("systemd: create manager proxy")?;
    let _job: OwnedObjectPath = manager
        .call("RestartUnit", &(unit, "replace"))
        .context("systemd: RestartUnit failed")?;
    Ok(())
}

/// Issue `Manager.StopUnit(unit, "replace")` via the system D-Bus (Phase 34).
///
/// # Errors
/// System D-Bus unavailable; `StopUnit` call rejected by systemd.
pub fn stop_unit(unit: &str) -> Result<()> {
    let conn = Connection::system().context("systemd: connect to system D-Bus")?;
    let manager = zbus::blocking::Proxy::new(&conn, SYSTEMD_BUS, MANAGER_PATH, MANAGER_IFACE)
        .context("systemd: create manager proxy")?;
    let _job: OwnedObjectPath = manager
        .call("StopUnit", &(unit, "replace"))
        .context("systemd: StopUnit failed")?;
    Ok(())
}

/// Render a `[Service]` INI drop-in from `props` (Phase 29).
///
/// `MemoryHigh = u64::MAX` is rendered as `infinity` (systemd's keyword for
/// unlimited). Fields set to `None` are omitted entirely.
///
/// Exposed for unit tests (no D-Bus required).
#[must_use]
pub fn render_drop_in(props: &UnitProps) -> String {
    let mut lines = vec!["[Service]".to_string()];
    if let Some(w) = props.cpu_weight {
        lines.push(format!("CPUWeight={w}"));
    }
    if let Some(w) = props.io_weight {
        lines.push(format!("IOWeight={w}"));
    }
    if let Some(h) = props.memory_high {
        if h == u64::MAX {
            lines.push("MemoryHigh=infinity".to_string());
        } else {
            lines.push(format!("MemoryHigh={h}"));
        }
    }
    if let Some(m) = props.memory_max {
        if m == u64::MAX {
            lines.push("MemoryMax=infinity".to_string());
        } else {
            lines.push(format!("MemoryMax={m}"));
        }
    }
    lines.push(String::new()); // trailing newline
    lines.join("\n")
}

/// Resolve and validate the drop-in path for `unit`.
///
/// Rejects empty names, names containing `/` or `..`, and names without a
/// recognised systemd unit suffix. Then confirms the canonicalised result is
/// strictly under `DROPIN_DIR_BASE` (defence against unexpected path joining).
///
/// Exposed for unit tests.
///
/// # Errors
/// Invalid unit name or resulting path escapes `DROPIN_DIR_BASE`.
pub fn resolve_drop_in_path(unit: &str) -> Result<PathBuf> {
    if unit.is_empty() || unit.contains('/') || unit.contains("..") {
        anyhow::bail!(
            "systemd: invalid unit name {unit:?} — empty, contains '/', or contains '..'"
        );
    }
    if !KNOWN_UNIT_SUFFIXES.iter().any(|s| unit.ends_with(s)) {
        anyhow::bail!("systemd: unit {unit:?} lacks a known suffix ({KNOWN_UNIT_SUFFIXES:?})");
    }
    let path = PathBuf::from(DROPIN_DIR_BASE)
        .join(format!("{unit}.d"))
        .join(DROPIN_FILENAME);
    // Double-check: resulting path must still be under DROPIN_DIR_BASE.
    if !path.starts_with(DROPIN_DIR_BASE) {
        anyhow::bail!(
            "systemd: resolved drop-in path {p} escapes base dir {DROPIN_DIR_BASE}",
            p = path.display()
        );
    }
    Ok(path)
}

// ---------------------------------------------------------------------------
// Private helpers (also used by unit tests below)
// ---------------------------------------------------------------------------

/// Read the current content of a drop-in file, or `None` if it does not exist.
///
/// Other I/O errors are treated conservatively as "absent" to avoid exposing
/// unrelated errors to callers (the write step will surface them if needed).
fn read_drop_in_content(path: &Path) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(s) => Some(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => None,
    }
}

fn resolve_unit_path(conn: &Connection, unit: &str) -> Result<OwnedObjectPath> {
    let manager = zbus::blocking::Proxy::new(conn, SYSTEMD_BUS, MANAGER_PATH, MANAGER_IFACE)
        .context("systemd: create manager proxy")?;
    manager
        .call("GetUnit", &(unit,))
        .with_context(|| format!("systemd: GetUnit({unit}) — is the unit loaded?"))
}

fn props_from_dbus(conn: &Connection, unit_path: &str) -> Result<UnitProps> {
    let proxy = zbus::blocking::Proxy::new(conn, SYSTEMD_BUS, unit_path, PROPS_IFACE)
        .context("systemd: create properties proxy")?;
    let all: HashMap<String, OwnedValue> = proxy
        .call("GetAll", &(SERVICE_IFACE,))
        .context("systemd: Properties.GetAll failed")?;
    Ok(unit_props_from_map(&all))
}

/// Extract `UnitProps` from a raw D-Bus property dict (`a{sv}`).
///
/// Kept as a named function so tests can exercise it with a constructed map,
/// without needing a live D-Bus connection.
fn unit_props_from_map(map: &HashMap<String, OwnedValue>) -> UnitProps {
    UnitProps {
        cpu_weight: extract_u64(map, PROP_CPU_WEIGHT),
        io_weight: extract_u64(map, PROP_IO_WEIGHT),
        memory_high: extract_u64(map, PROP_MEMORY_HIGH),
        memory_max: extract_u64(map, PROP_MEMORY_MAX),
    }
}

fn extract_u64(map: &HashMap<String, OwnedValue>, key: &str) -> Option<u64> {
    map.get(key).and_then(|v| u64::try_from(v).ok())
}

/// Build the `a(sv)` array for `SetUnitProperties` from a `UnitProps`.
///
/// Returns `Err` only if `OwnedValue` encoding fails — infallible for `u64` values.
fn build_dbus_props(props: &UnitProps) -> Result<Vec<(String, OwnedValue)>> {
    let mut out: Vec<(String, OwnedValue)> = Vec::new();
    if let Some(w) = props.cpu_weight {
        out.push((
            PROP_CPU_WEIGHT.to_string(),
            OwnedValue::try_from(Value::from(w)).context("encode CPUWeight")?,
        ));
    }
    if let Some(w) = props.io_weight {
        out.push((
            PROP_IO_WEIGHT.to_string(),
            OwnedValue::try_from(Value::from(w)).context("encode IOWeight")?,
        ));
    }
    if let Some(h) = props.memory_high {
        out.push((
            PROP_MEMORY_HIGH.to_string(),
            OwnedValue::try_from(Value::from(h)).context("encode MemoryHigh")?,
        ));
    }
    if let Some(m) = props.memory_max {
        out.push((
            PROP_MEMORY_MAX.to_string(),
            OwnedValue::try_from(Value::from(m)).context("encode MemoryMax")?,
        ));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn owned_u64(v: u64) -> OwnedValue {
        OwnedValue::try_from(Value::from(v)).expect("OwnedValue from u64")
    }

    fn make_dbus_map(cpu: u64, io: u64, mem: u64) -> HashMap<String, OwnedValue> {
        let mut m = HashMap::new();
        m.insert(PROP_CPU_WEIGHT.to_string(), owned_u64(cpu));
        m.insert(PROP_IO_WEIGHT.to_string(), owned_u64(io));
        m.insert(PROP_MEMORY_HIGH.to_string(), owned_u64(mem));
        m
    }

    // --- UnitProps serde (for rollback JSON round-trip) ---

    #[test]
    fn unit_props_serde_roundtrip_all_some() {
        let props = UnitProps {
            cpu_weight: Some(50),
            io_weight: Some(75),
            memory_high: Some(4 * 1024 * 1024 * 1024),
            memory_max: Some(8 * 1024 * 1024 * 1024),
        };
        let json = serde_json::to_string(&props).expect("serialize");
        let back: UnitProps = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(props, back);
    }

    #[test]
    fn unit_props_serde_roundtrip_with_none_fields() {
        let props = UnitProps {
            cpu_weight: None,
            io_weight: Some(50),
            memory_high: None,
            memory_max: None,
        };
        let json = serde_json::to_string(&props).expect("serialize");
        let back: UnitProps = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(props, back);
    }

    #[test]
    fn unit_props_serde_roundtrip_all_none() {
        let props = UnitProps::default();
        let json = serde_json::to_string(&props).expect("serialize");
        let back: UnitProps = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(props, back);
    }

    // --- unit_props_from_map (mock D-Bus extraction) ---

    #[test]
    fn extract_all_fields_from_dbus_map() {
        let map = make_dbus_map(50, 75, 4 * 1024 * 1024 * 1024);
        let props = unit_props_from_map(&map);
        assert_eq!(props.cpu_weight, Some(50));
        assert_eq!(props.io_weight, Some(75));
        assert_eq!(props.memory_high, Some(4 * 1024 * 1024 * 1024));
    }

    #[test]
    fn extract_empty_map_gives_all_none() {
        let map = HashMap::new();
        let props = unit_props_from_map(&map);
        assert!(props.cpu_weight.is_none());
        assert!(props.io_weight.is_none());
        assert!(props.memory_high.is_none());
    }

    #[test]
    fn memory_high_u64_max_preserved_for_rollback() {
        // systemd uses u64::MAX for "unlimited"; we must not lose it.
        let map = make_dbus_map(100, 100, u64::MAX);
        let props = unit_props_from_map(&map);
        assert_eq!(props.memory_high, Some(u64::MAX));
    }

    #[test]
    fn extract_partial_map_only_present_fields_are_some() {
        let mut map = HashMap::new();
        map.insert(PROP_CPU_WEIGHT.to_string(), owned_u64(200));
        // IO_WEIGHT and MEMORY_HIGH absent
        let props = unit_props_from_map(&map);
        assert_eq!(props.cpu_weight, Some(200));
        assert!(props.io_weight.is_none());
        assert!(props.memory_high.is_none());
    }

    // --- build_dbus_props (field inclusion logic) ---

    #[test]
    fn build_dbus_props_skips_none_fields() {
        let props = UnitProps {
            cpu_weight: Some(50),
            io_weight: None,
            memory_high: None,
            memory_max: None,
        };
        let dbus = build_dbus_props(&props).expect("build");
        assert_eq!(dbus.len(), 1);
        assert_eq!(dbus[0].0, PROP_CPU_WEIGHT);
    }

    #[test]
    fn build_dbus_props_includes_all_some_fields() {
        let props = UnitProps {
            cpu_weight: Some(50),
            io_weight: Some(75),
            memory_high: Some(4 * 1024 * 1024 * 1024),
            memory_max: Some(8 * 1024 * 1024 * 1024),
        };
        let dbus = build_dbus_props(&props).expect("build");
        assert_eq!(dbus.len(), 4);
        let names: Vec<&str> = dbus.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&PROP_CPU_WEIGHT));
        assert!(names.contains(&PROP_IO_WEIGHT));
        assert!(names.contains(&PROP_MEMORY_HIGH));
        assert!(names.contains(&PROP_MEMORY_MAX));
    }

    #[test]
    fn build_dbus_props_empty_when_all_none() {
        let dbus = build_dbus_props(&UnitProps::default()).expect("build");
        assert!(dbus.is_empty());
    }

    #[test]
    fn build_dbus_props_memory_high_u64_max_is_encoded() {
        let props = UnitProps {
            memory_high: Some(u64::MAX),
            ..Default::default()
        };
        let dbus = build_dbus_props(&props).expect("build");
        assert_eq!(dbus.len(), 1);
        assert_eq!(dbus[0].0, PROP_MEMORY_HIGH);
        // Value should round-trip to u64::MAX
        let val: u64 = u64::try_from(&dbus[0].1).expect("decode");
        assert_eq!(val, u64::MAX);
    }

    // --- UnitProps helpers ---

    #[test]
    fn unit_props_is_empty_when_all_none() {
        assert!(UnitProps::default().is_empty());
    }

    #[test]
    fn unit_props_not_empty_when_one_field_set() {
        assert!(!UnitProps {
            cpu_weight: Some(50),
            ..Default::default()
        }
        .is_empty());
    }

    // --- Live D-Bus test (requires running systemd) ---

    #[test]
    #[ignore = "requires a running systemd with systemd-journald.service loaded"]
    fn live_read_journald_unit_props() {
        let props = read_unit_props("systemd-journald.service").expect("read_unit_props");
        // Journald is always running; we just verify we got a valid response.
        println!("journald UnitProps: {props:?}");
    }

    // --- Phase 29: resolve_drop_in_path ---

    #[test]
    fn resolve_path_valid_service() {
        let p = resolve_drop_in_path("foo.service").expect("valid");
        assert_eq!(
            p,
            PathBuf::from("/etc/systemd/system/foo.service.d/50-syswarden.conf")
        );
    }

    #[test]
    fn resolve_path_valid_other_suffixes() {
        for suffix in KNOWN_UNIT_SUFFIXES {
            let unit = format!("test{suffix}");
            assert!(
                resolve_drop_in_path(&unit).is_ok(),
                "suffix {suffix} rejected"
            );
        }
    }

    #[test]
    fn resolve_path_rejects_empty() {
        assert!(resolve_drop_in_path("").is_err());
    }

    #[test]
    fn resolve_path_rejects_slash() {
        assert!(resolve_drop_in_path("foo/bar.service").is_err());
    }

    #[test]
    fn resolve_path_rejects_dotdot() {
        assert!(resolve_drop_in_path("../etc/shadow.service").is_err());
    }

    #[test]
    fn resolve_path_rejects_unknown_suffix() {
        assert!(resolve_drop_in_path("foo.exe").is_err());
        assert!(resolve_drop_in_path("noext").is_err());
    }

    // --- Phase 29: render_drop_in ---

    #[test]
    fn render_all_fields() {
        let props = UnitProps {
            cpu_weight: Some(50),
            io_weight: Some(75),
            memory_high: Some(1024 * 1024 * 1024),
            memory_max: Some(2 * 1024 * 1024 * 1024),
        };
        let s = render_drop_in(&props);
        assert!(s.starts_with("[Service]\n"), "must start with [Service]");
        assert!(s.contains("CPUWeight=50\n"));
        assert!(s.contains("IOWeight=75\n"));
        assert!(s.contains("MemoryHigh=1073741824\n"));
        assert!(s.contains("MemoryMax=2147483648\n"));
        assert!(s.ends_with('\n'), "must end with newline");
    }

    #[test]
    fn render_memory_high_u64_max_is_infinity() {
        let props = UnitProps {
            memory_high: Some(u64::MAX),
            ..Default::default()
        };
        let s = render_drop_in(&props);
        assert!(s.contains("MemoryHigh=infinity\n"));
    }

    #[test]
    fn render_skips_none_fields() {
        let props = UnitProps {
            cpu_weight: Some(50),
            io_weight: None,
            memory_high: None,
            memory_max: None,
        };
        let s = render_drop_in(&props);
        assert!(!s.contains("IOWeight"), "IOWeight should be absent");
        assert!(!s.contains("MemoryHigh"), "MemoryHigh should be absent");
        assert!(s.contains("CPUWeight=50"));
    }

    #[test]
    fn render_all_none_is_just_service_header() {
        let s = render_drop_in(&UnitProps::default());
        assert_eq!(s, "[Service]\n");
    }

    // --- Phase 29: DropInPriorState serde ---

    #[test]
    fn drop_in_prior_state_serde_roundtrip() {
        let state = DropInPriorState {
            path: PathBuf::from("/etc/systemd/system/foo.service.d/50-syswarden.conf"),
            prior_content: Some("[Service]\nCPUWeight=100\n".to_string()),
            written_content: "[Service]\nCPUWeight=50\n".to_string(),
        };
        let json = serde_json::to_string(&state).expect("serialize");
        let back: DropInPriorState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(state, back);
    }

    #[test]
    fn drop_in_prior_state_serde_none_prior() {
        let state = DropInPriorState {
            path: PathBuf::from("/etc/systemd/system/bar.service.d/50-syswarden.conf"),
            prior_content: None,
            written_content: "[Service]\nMemoryHigh=infinity\n".to_string(),
        };
        let json = serde_json::to_string(&state).expect("serialize");
        let back: DropInPriorState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.prior_content, None);
    }

    // --- Phase 29: live write/remove test ---

    #[test]
    #[ignore = "requires root + running systemd; writes to /etc/systemd/system/"]
    fn live_write_and_remove_drop_in() {
        let unit = "systemd-journald.service";
        let props = UnitProps {
            cpu_weight: Some(50),
            ..Default::default()
        };
        let prior = write_drop_in(unit, &props).expect("write_drop_in");
        println!("written to {}", prior.path.display());
        assert!(prior.path.exists(), "drop-in should exist after write");

        // Restore: remove it if it was newly created.
        if prior.prior_content.is_none() {
            remove_drop_in(unit).expect("remove_drop_in");
            assert!(!prior.path.exists(), "drop-in should be gone after remove");
        } else {
            // Rewrite the old content.
            std::fs::write(&prior.path, prior.prior_content.as_deref().unwrap_or(""))
                .expect("restore prior content");
            daemon_reload().expect("daemon_reload");
        }
    }
}
