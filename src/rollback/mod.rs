//! Prior-state capture, rollback entry listing, and revert (architecture.md §5.15).
#![allow(dead_code)]
// Process-priority revert uses setpriority(2) and ioprio_set(2) directly (nix 0.29 gap).
#![allow(unsafe_code)]

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::config::AppConfig;

// ---------------------------------------------------------------------------
// RollbackEntry
// ---------------------------------------------------------------------------

/// Prior-state record for one applied action (architecture.md §15).
///
/// In v0.1 all actions are simulated; entries are never written during normal
/// daemon operation. This struct is scaffolding for v0.2+ real execution paths.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackEntry {
    pub id: u64,
    pub timestamp: DateTime<Utc>,
    /// Debug-formatted `ActionKind` (e.g. `"AdjustNice"`).
    pub action_kind: String,
    /// Human-readable target (e.g. `"pid=1234 comm=firefox"` or `"unit=foo.service"`).
    pub target: String,
    /// Serialized prior state. Empty JSON object in v0.1 scaffolding entries.
    pub prior_state: serde_json::Value,
    pub reversible: bool,
}

impl RollbackEntry {
    /// Build a new entry with a millisecond-precision epoch `id`.
    #[must_use]
    pub fn new(action_kind: &str, target: &str, reversible: bool) -> Self {
        let timestamp = Utc::now();
        let id = timestamp.timestamp_millis().cast_unsigned();
        Self {
            id,
            timestamp,
            action_kind: action_kind.to_string(),
            target: target.to_string(),
            prior_state: serde_json::Value::Object(serde_json::Map::new()),
            reversible,
        }
    }

    /// Attach serialized prior state.
    #[must_use]
    pub fn with_prior_state(mut self, state: serde_json::Value) -> Self {
        self.prior_state = state;
        self
    }
}

// ---------------------------------------------------------------------------
// RollbackStore
// ---------------------------------------------------------------------------

const ROLLBACK_FILE: &str = "rollback.jsonl";

/// Append-only JSONL rollback store (architecture.md §5.15).
///
/// Stores at most `keep_entries` entries; excess oldest entries are pruned
/// on `open` and the file is rewritten. Corrupt lines are skipped with a
/// `warn!`; the store never panics on I/O errors.
pub struct RollbackStore {
    file_path: PathBuf,
    keep_entries: usize,
    /// In-memory cache; oldest first.
    entries: Vec<RollbackEntry>,
}

impl RollbackStore {
    /// Open (or create) the rollback store.
    ///
    /// Creates the directory, loads all entries from disk, and prunes to
    /// `config.rollback.keep_entries`.
    ///
    /// # Errors
    /// Returns an error if the store directory cannot be created.
    pub fn open(config: &AppConfig) -> Result<Self> {
        let dir = PathBuf::from(&config.rollback.dir);
        fs::create_dir_all(&dir)
            .with_context(|| format!("rollback: cannot create dir {}", dir.display()))?;

        let file_path = dir.join(ROLLBACK_FILE);
        let keep_entries = config.rollback.keep_entries;
        let mut store = Self {
            file_path,
            keep_entries,
            entries: Vec::new(),
        };
        store.load_from_disk();
        store.prune_to_limit();
        Ok(store)
    }

    /// Append one entry to the JSONL file and update the in-memory cache.
    ///
    /// Never panics; on I/O failure emits a `warn!` and returns (in-memory
    /// cache is always updated regardless of disk result).
    pub fn record(&mut self, entry: RollbackEntry) {
        match serde_json::to_string(&entry) {
            Ok(json) => {
                if let Err(e) = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.file_path)
                    .and_then(|mut f| writeln!(f, "{json}"))
                {
                    warn!("rollback: write to {:?} failed: {e}", self.file_path);
                }
            }
            Err(e) => warn!("rollback: serialize failed: {e}"),
        }
        self.entries.push(entry);
        if self.entries.len() > self.keep_entries {
            self.entries.remove(0);
        }
    }

    /// Return all entries, oldest first.
    #[must_use]
    pub fn list(&self) -> &[RollbackEntry] {
        &self.entries
    }

    /// Revert the action recorded under `id` using its captured prior state.
    ///
    /// Dispatches based on `action_kind`:
    /// - `AdjustNice`   → restore nice via `setpriority(2)`
    /// - `AdjustIonice` → restore ioprio via `ioprio_set(2)`
    /// - `SetCpuWeight | SetIoWeight | SetMemoryHigh` (backend=transient)
    ///   → restore via `systemd::set_unit_properties` (architecture.md §5.15)
    /// - `SetCpuWeight | SetIoWeight | SetMemoryHigh` (backend=persistent, Phase 30)
    ///   → rewrite or remove the drop-in file, then `daemon-reload`
    ///
    /// # Errors
    /// `id` not found; entry irreversible; empty prior state; revert syscall/D-Bus failure;
    /// drop-in file changed since backup (Phase 30 integrity check).
    pub fn apply(&self, id: u64) -> Result<()> {
        let entry = self
            .entries
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| anyhow::anyhow!("rollback: no entry with id={id}"))?;

        if !entry.reversible {
            bail!(
                "rollback: entry id={id} ({}) is marked irreversible",
                entry.action_kind
            );
        }

        // An empty object means no real prior state was captured (v0.1 simulation entries).
        if entry.prior_state == serde_json::Value::Object(serde_json::Map::default()) {
            bail!(
                "rollback: entry id={id} ({}) has no captured prior state",
                entry.action_kind
            );
        }

        match entry.action_kind.as_str() {
            "AdjustNice" => revert_nice(&entry.prior_state),
            "AdjustIonice" => revert_ionice(&entry.prior_state),
            "SetCpuWeight" | "SetIoWeight" | "SetMemoryHigh" => {
                if entry.prior_state.get("backend").and_then(|v| v.as_str()) == Some("persistent") {
                    revert_drop_in(&entry.prior_state)
                } else {
                    // "transient" tag or missing "backend" (backward compat with old format).
                    let unit = parse_unit_from_target(&entry.target).ok_or_else(|| {
                        anyhow::anyhow!(
                            "rollback: cannot parse unit from target '{}'",
                            entry.target
                        )
                    })?;
                    revert_service_props(&unit, &entry.prior_state)
                }
            }
            kind => bail!("rollback: no revert handler for action_kind '{kind}'"),
        }
    }

    /// Total entry count (in-memory).
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    // ---------------------------------------------------------------------------
    // Private helpers
    // ---------------------------------------------------------------------------

    fn load_from_disk(&mut self) {
        let content = match fs::read_to_string(&self.file_path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                warn!("rollback: cannot read {:?}: {e}", self.file_path);
                return;
            }
        };
        self.entries = content
            .lines()
            .filter_map(|line| {
                if line.trim().is_empty() {
                    return None;
                }
                serde_json::from_str::<RollbackEntry>(line)
                    .map_err(|e| warn!("rollback: corrupt line in {:?}: {e}", self.file_path))
                    .ok()
            })
            .collect();
    }

    /// Prune oldest entries to stay within `keep_entries`, then rewrite disk.
    fn prune_to_limit(&mut self) {
        if self.entries.len() <= self.keep_entries {
            return;
        }
        let excess = self.entries.len() - self.keep_entries;
        self.entries.drain(0..excess);
        self.rewrite_disk();
    }

    /// Rewrite the JSONL file from the in-memory cache (used after pruning).
    fn rewrite_disk(&self) {
        let mut content = String::new();
        for entry in &self.entries {
            match serde_json::to_string(entry) {
                Ok(json) => {
                    content.push_str(&json);
                    content.push('\n');
                }
                Err(e) => warn!("rollback: serialize during rewrite: {e}"),
            }
        }
        if let Err(e) = fs::write(&self.file_path, &content) {
            warn!("rollback: cannot rewrite {:?}: {e}", self.file_path);
        }
    }
}

// ---------------------------------------------------------------------------
// Private revert helpers (Phase 26)
// ---------------------------------------------------------------------------

/// Extract `"unit=foo.service"` → `"foo.service"` from a rollback target string.
fn parse_unit_from_target(target: &str) -> Option<String> {
    target.strip_prefix("unit=").map(String::from)
}

/// Restore the nice value for a process from `prior_state["pid"]` and `prior_state["nice"]`.
fn revert_nice(prior_state: &serde_json::Value) -> Result<()> {
    let pid = u32::try_from(
        prior_state["pid"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("revert_nice: missing 'pid' in prior_state"))?,
    )
    .context("revert_nice: pid overflows u32")?;
    #[allow(clippy::cast_possible_truncation)] // nice is always in [-20, 19]
    let Some(nice) = prior_state["nice"].as_i64().map(|n| n as i32) else {
        return Ok(()); // no prior nice captured → nothing to restore
    };
    // SAFETY: setpriority is a standard POSIX syscall; pid and nice are validated above.
    let ret = unsafe { nix::libc::setpriority(nix::libc::PRIO_PROCESS, pid, nice) };
    if ret != 0 {
        let e = std::io::Error::last_os_error();
        bail!("revert_nice: setpriority(pid={pid}, nice={nice}) failed: {e}");
    }
    Ok(())
}

/// Restore the ioprio for a process from `prior_state["pid"]` and `prior_state["ioprio"]`.
fn revert_ionice(prior_state: &serde_json::Value) -> Result<()> {
    let pid = u32::try_from(
        prior_state["pid"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("revert_ionice: missing 'pid' in prior_state"))?,
    )
    .context("revert_ionice: pid overflows u32")?;
    let Some(ioprio) = prior_state["ioprio"].as_i64() else {
        return Ok(()); // no prior ioprio captured → nothing to restore
    };
    // IOPRIO_WHO_PROCESS = 1; re-set the raw previously-captured value.
    // SAFETY: ioprio_set is a standard Linux syscall; ioprio value came from ioprio_get.
    let ret =
        unsafe { nix::libc::syscall(nix::libc::SYS_ioprio_set, 1_i64, i64::from(pid), ioprio) };
    if ret < 0 {
        let e = std::io::Error::last_os_error();
        bail!("revert_ionice: ioprio_set(pid={pid}) failed: {e}");
    }
    Ok(())
}

/// Restore systemd unit properties from a tagged `prior_state` (Phase 26 / Phase 29).
///
/// Accepts both the legacy untagged format (`{cpu_weight, io_weight, memory_high}`) and
/// the new tagged format (`{backend: "transient", cpu_weight, ...}`).
fn revert_service_props(unit: &str, prior_state: &serde_json::Value) -> Result<()> {
    // Extract the UnitProps fields regardless of whether the "backend" tag is present.
    let prior = crate::systemd::UnitProps {
        cpu_weight: prior_state["cpu_weight"].as_u64(),
        io_weight: prior_state["io_weight"].as_u64(),
        memory_high: prior_state["memory_high"].as_u64(),
    };
    if prior.is_empty() {
        return Ok(()); // nothing was set before → nothing to restore
    }
    crate::systemd::set_unit_properties(unit, &prior, true)
        .with_context(|| format!("rollback: restore properties for {unit}"))?;
    Ok(())
}

/// Revert a persistent drop-in (Phase 30, architecture.md §5.15 / §17 backup policy).
///
/// Reads the current file content and compares it to `written_content`. If the file has
/// been modified externally since we wrote it, refuse to revert (integrity check).
///
/// - If `prior_content` is `None` (file did not exist before): removes the file.
/// - If `prior_content` is `Some(content)`: writes the original content back.
/// - In both cases issues `daemon-reload` via D-Bus.
///
/// # Errors
/// Missing or corrupt `prior_state` fields; file changed since backup;
/// file I/O error; D-Bus unavailable.
fn revert_drop_in(prior_state: &serde_json::Value) -> Result<()> {
    let path_str = prior_state["path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("rollback: persistent prior_state missing 'path'"))?;
    let written_content = prior_state["written_content"].as_str().ok_or_else(|| {
        anyhow::anyhow!("rollback: persistent prior_state missing 'written_content'")
    })?;
    let prior_content = prior_state["prior_content"].as_str(); // None = file was absent

    let path = std::path::Path::new(path_str);

    // Integrity check: refuse if the file has been changed or deleted since we wrote it.
    restore_drop_in_file(path, prior_content, written_content)?;

    crate::systemd::daemon_reload().context("rollback: daemon-reload after drop-in revert")
}

/// Restore the drop-in file on disk (no D-Bus; separated for unit-test coverage).
///
/// Returns `Err` if the current on-disk content differs from `written_content` (external
/// modification) or if the file is missing when `prior_content.is_some()`.
pub(crate) fn restore_drop_in_file(
    path: &std::path::Path,
    prior_content: Option<&str>,
    written_content: &str,
) -> Result<()> {
    match fs::read_to_string(path) {
        Ok(current) if current == written_content => {
            // File is intact — proceed with restoration.
            if let Some(original) = prior_content {
                // Restore original content.
                fs::write(path, original)
                    .with_context(|| format!("rollback: restore drop-in {}", path.display()))?;
            } else {
                // File was created by syswarden — remove it.
                fs::remove_file(path)
                    .with_context(|| format!("rollback: remove drop-in {}", path.display()))?;
            }
            Ok(())
        }
        Ok(_current) => bail!(
            "rollback: drop-in {} has been modified since backup — refusing to revert",
            path.display()
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // File is gone — treat as "changed" (someone deleted it).
            bail!(
                "rollback: drop-in {} is missing (deleted since backup) — refusing to revert",
                path.display()
            )
        }
        Err(e) => Err(e).with_context(|| format!("rollback: read drop-in {}", path.display())),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    fn temp_store(suffix: &str) -> (RollbackStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!("syswarden_rollback_test_{suffix}"));
        let _ = fs::remove_dir_all(&dir);
        let mut config = config::defaults();
        config.rollback.dir = dir.to_string_lossy().to_string();
        let store = RollbackStore::open(&config).expect("open");
        (store, dir)
    }

    fn make_entry(kind: &str, target: &str) -> RollbackEntry {
        RollbackEntry::new(kind, target, true)
    }

    #[test]
    fn rollback_entry_round_trip() {
        let entry = make_entry("AdjustNice", "pid=1234 comm=firefox");
        let json = serde_json::to_string(&entry).expect("serialize");
        let back: RollbackEntry = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.action_kind, "AdjustNice");
        assert_eq!(back.target, "pid=1234 comm=firefox");
        assert!(back.reversible);
        assert_eq!(
            back.prior_state,
            serde_json::Value::Object(serde_json::Map::new())
        );
    }

    #[test]
    fn rollback_store_record_and_list() {
        let (mut store, dir) = temp_store("record");
        store.record(make_entry("AdjustNice", "pid=1"));
        store.record(make_entry("SetCpuWeight", "unit=foo.service"));
        assert_eq!(store.len(), 2);

        // Re-open and verify persistence.
        let mut cfg = config::defaults();
        cfg.rollback.dir = dir.to_string_lossy().to_string();
        let store2 = RollbackStore::open(&cfg).expect("reopen");
        assert_eq!(store2.len(), 2);
        assert_eq!(store2.list()[0].action_kind, "AdjustNice");
        assert_eq!(store2.list()[1].action_kind, "SetCpuWeight");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rollback_store_prune_to_keep_entries() {
        let (_, dir) = temp_store("prune");

        // Write 5 entries to the file manually.
        let file = dir.join(ROLLBACK_FILE);
        let mut content = String::new();
        for i in 0u64..5 {
            let e = RollbackEntry {
                id: i,
                timestamp: Utc::now(),
                action_kind: format!("Kind{i}"),
                target: "system".to_string(),
                prior_state: serde_json::Value::Object(serde_json::Map::new()),
                reversible: true,
            };
            content.push_str(&serde_json::to_string(&e).unwrap());
            content.push('\n');
        }
        fs::write(&file, &content).unwrap();

        // Open with keep_entries = 3 → oldest 2 pruned.
        let mut cfg = config::defaults();
        cfg.rollback.dir = dir.to_string_lossy().to_string();
        cfg.rollback.keep_entries = 3;
        let store = RollbackStore::open(&cfg).expect("open");

        assert_eq!(store.len(), 3);
        // Should retain entries 2, 3, 4 (ids 2..4).
        assert_eq!(store.list()[0].id, 2);
        assert_eq!(store.list()[2].id, 4);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rollback_apply_refuses_without_real_prior_state() {
        let (mut store, dir) = temp_store("apply");
        let entry = make_entry("AdjustNice", "pid=99 comm=stress");
        let id = entry.id;
        store.record(entry);

        // apply must fail — v0.1 scaffolding has no real prior state.
        let err = store.apply(id).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("no captured prior state"),
            "expected prior-state refusal, got: {msg}"
        );

        // Non-existent id also fails.
        let err2 = store.apply(u64::MAX).unwrap_err();
        assert!(err2.to_string().contains("no entry with id="));

        let _ = fs::remove_dir_all(&dir);
    }

    // --- Phase 30: restore_drop_in_file (file-ops only, no D-Bus) ---

    #[test]
    fn restore_drop_in_file_removes_when_prior_is_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("50-syswarden.conf");
        let written = "[Service]\nCPUWeight=50\n";
        fs::write(&path, written).expect("write");

        restore_drop_in_file(&path, None, written).expect("restore");
        assert!(!path.exists(), "file should be removed");
    }

    #[test]
    fn restore_drop_in_file_restores_prior_content() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("50-syswarden.conf");
        let written = "[Service]\nCPUWeight=50\n";
        let original = "[Service]\nCPUWeight=100\n";
        fs::write(&path, written).expect("write");

        restore_drop_in_file(&path, Some(original), written).expect("restore");
        let current = fs::read_to_string(&path).expect("read");
        assert_eq!(current, original, "prior content should be restored");
    }

    #[test]
    fn restore_drop_in_file_refuses_when_file_changed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("50-syswarden.conf");
        let written = "[Service]\nCPUWeight=50\n";
        // Simulate external modification.
        fs::write(&path, "[Service]\nCPUWeight=999\n").expect("write");

        let err = restore_drop_in_file(&path, None, written).unwrap_err();
        assert!(
            err.to_string().contains("modified since backup"),
            "expected tamper refusal, got: {err}"
        );
    }

    #[test]
    fn restore_drop_in_file_refuses_when_file_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("50-syswarden.conf");
        // File was never written — simulate it being deleted after backup.
        let err = restore_drop_in_file(&path, Some("old content"), "[Service]\nCPUWeight=50\n")
            .unwrap_err();
        assert!(
            err.to_string().contains("missing"),
            "expected missing-file refusal, got: {err}"
        );
    }

    #[test]
    fn rollback_apply_routes_persistent_to_revert_drop_in() {
        let (mut store, dir) = temp_store("persistent_route");
        let tmp = tempfile::tempdir().expect("tempdir");
        let drop_in_path = tmp.path().join("50-syswarden.conf");
        let written = "[Service]\nCPUWeight=50\n";
        fs::write(&drop_in_path, written).expect("write");

        // Build a rollback entry with persistent prior state pointing to our temp file.
        let prior_state = serde_json::json!({
            "backend": "persistent",
            "path": drop_in_path.to_str().unwrap(),
            "prior_content": null,
            "written_content": written,
        });
        let entry = RollbackEntry::new("SetCpuWeight", "unit=foo.service", true)
            .with_prior_state(prior_state);
        let id = entry.id;
        store.record(entry);

        // Calling apply will call revert_drop_in → restore_drop_in_file + daemon_reload.
        // daemon_reload will fail (no D-Bus in tests), but the file should be gone first.
        // We verify file removal happened before the daemon_reload error.
        let result = store.apply(id);
        // File should have been removed (prior_content=null) before daemon_reload attempt.
        assert!(
            !drop_in_path.exists(),
            "file should be removed even if daemon_reload fails"
        );
        // Error from daemon_reload is expected in test env.
        drop(result);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rollback_apply_routes_transient_by_backend_tag() {
        // Verify the "backend":"transient" tag is handled without panicking.
        // (Actual D-Bus call would fail; we test the routing logic stops at the right error.)
        let (mut store, dir) = temp_store("transient_route");
        let prior_state = serde_json::json!({
            "backend": "transient",
            "cpu_weight": 100u64,
            "io_weight": null,
            "memory_high": null,
        });
        let entry = RollbackEntry::new("SetCpuWeight", "unit=foo.service", true)
            .with_prior_state(prior_state);
        let id = entry.id;
        store.record(entry);

        // Will fail at D-Bus, but must not fail at routing/parse.
        let err = store.apply(id).unwrap_err();
        let msg = err.to_string();
        // Error must come from D-Bus / systemd, not from "no revert handler".
        assert!(
            !msg.contains("no revert handler"),
            "unexpected routing error: {msg}"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
