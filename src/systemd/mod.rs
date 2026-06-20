//! systemd D-Bus integration: unit property reads and transient resource-control writes
//! (architecture.md §5.8, §18).
//!
//! ## Invariants
//! - All D-Bus calls use explicit typed arguments — no shell, no string interpolation
//!   (architecture.md §17).
//! - Writes are transient (`runtime = true`) per architecture.md §18:
//!   "prefer transient runtime properties for temporary pressure response."
//! - `set_unit_properties` captures prior state and returns it to the caller so it can be
//!   stored in `RollbackEntry.prior_state` (architecture.md §5.15).
//! - No safety decisions are made here; the caller is responsible for all gating.

#![allow(dead_code)]

use std::collections::HashMap;

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
const PROPS_IFACE: &str = "org.freedesktop.DBus.Properties";

const PROP_CPU_WEIGHT: &str = "CPUWeight";
const PROP_IO_WEIGHT: &str = "IOWeight";
const PROP_MEMORY_HIGH: &str = "MemoryHigh";

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
/// `MemoryHigh = u64::MAX` is systemd's representation of "unlimited". We preserve it
/// as-is so rollback can restore the unlimited state accurately, unlike the `cgroups`
/// module which normalises it to `None` for display purposes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitProps {
    /// `CPUWeight` (1–10000; systemd default 100). `None` = not set / skip on write.
    pub cpu_weight: Option<u64>,
    /// `IOWeight` (1–10000; systemd default 100). `None` = not set / skip on write.
    pub io_weight: Option<u64>,
    /// `MemoryHigh` in bytes. `u64::MAX` = unlimited (systemd's sentinel). `None` = skip.
    pub memory_high: Option<u64>,
}

impl UnitProps {
    /// `true` when all fields are `None` — nothing to write to systemd.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cpu_weight.is_none() && self.io_weight.is_none() && self.memory_high.is_none()
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

// ---------------------------------------------------------------------------
// Private helpers (also used by unit tests below)
// ---------------------------------------------------------------------------

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
        };
        let dbus = build_dbus_props(&props).expect("build");
        assert_eq!(dbus.len(), 3);
        let names: Vec<&str> = dbus.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&PROP_CPU_WEIGHT));
        assert!(names.contains(&PROP_IO_WEIGHT));
        assert!(names.contains(&PROP_MEMORY_HIGH));
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
}
