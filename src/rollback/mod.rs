//! Prior-state capture, rollback entry listing, and revert scaffolding (architecture.md §5.15).
#![allow(dead_code)]

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

    /// Attempt to revert the action with `id`.
    ///
    /// In v0.1, all recorded entries are simulation scaffolding with no
    /// captured prior state — revert is always refused with a clear message.
    /// Real revert is implemented in v0.2+ when actual execution paths exist.
    ///
    /// # Errors
    /// Returns `Err` if: the `id` is not found; the entry is irreversible;
    /// or no real prior state was captured (v0.1 scaffolding).
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

        // An empty object indicates no real prior state was captured (v0.1 scaffolding).
        if entry.prior_state == serde_json::Value::Object(serde_json::Map::default()) {
            bail!(
                "rollback: entry id={id} ({}) has no captured prior state; \
                 real revert requires v0.2+ execution paths",
                entry.action_kind
            );
        }

        bail!("rollback: revert of id={id} is not yet implemented (v0.2+)");
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
}
