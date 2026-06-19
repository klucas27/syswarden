//! Append-only JSONL local history store; `HistoryRecord` persistence and retention (architecture.md §5.16, §20).
#![allow(dead_code)]

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::config::AppConfig;
use crate::pressure::PressureLevel;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// In-memory ring-buffer capacity for trend queries and warm-up on restart.
const RECENT_BUF_CAP: usize = 256;

pub(crate) const SCHEMA_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// HistoryRecord
// ---------------------------------------------------------------------------

/// One persisted supervision-tick summary (architecture.md §15).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryRecord {
    pub schema_version: u32,
    pub timestamp: DateTime<Utc>,
    pub pressure_level: PressureLevel,
    /// Brief PSI averages: `"cpu=N mem=N io=N"` (`some_avg10` values).
    pub psi_summary: String,
    pub state: String,
    pub action_count: usize,
    pub simulated_count: usize,
    pub blocked_count: usize,
    /// Per-action outcome strings: `"{ActionKind:?}:{ActionStatus:?}"`.
    pub outcomes: Vec<String>,
}

// ---------------------------------------------------------------------------
// HistoryStore
// ---------------------------------------------------------------------------

/// Append-only JSONL history store (architecture.md §5.16, §20).
///
/// One file per calendar day: `{dir}/history-YYYY-MM-DD.jsonl`.
/// Corrupt lines are skipped with a `warn!`; the store never panics on I/O errors.
pub struct HistoryStore {
    dir: PathBuf,
    retention_days: u32,
    max_file_mb: u64,
    /// In-memory ring buffer: last [`RECENT_BUF_CAP`] records for trend queries.
    recent_buf: Vec<HistoryRecord>,
}

impl HistoryStore {
    /// Open (or create) the history store.
    ///
    /// Creates the store directory, loads recent records into the in-memory
    /// buffer for trend seeding, and prunes files older than
    /// `config.history.retention_days`.
    ///
    /// # Errors
    /// Returns an error if the store directory cannot be created.
    pub fn open(config: &AppConfig) -> Result<Self> {
        let dir = PathBuf::from(&config.history.dir);
        fs::create_dir_all(&dir)
            .with_context(|| format!("history: cannot create dir {}", dir.display()))?;

        let mut store = Self {
            dir,
            retention_days: config.history.retention_days,
            max_file_mb: config.history.max_file_mb,
            recent_buf: Vec::with_capacity(RECENT_BUF_CAP),
        };
        store.load_recent_from_disk();
        store.prune_old_files();
        Ok(store)
    }

    /// Append one record to today's JSONL file and update the in-memory buffer.
    ///
    /// Never panics; on I/O failure, emits a `warn!` and returns (in-memory
    /// buffer is always updated regardless of disk result).
    pub fn append(&mut self, record: HistoryRecord) {
        let path = self.today_file();

        // Enforce max_file_mb: warn and skip disk write if file exceeds limit.
        let over_limit = fs::metadata(&path)
            .map(|m| m.len() >= self.max_file_mb * 1_048_576)
            .unwrap_or(false);

        if over_limit {
            warn!(
                "history: {path:?} exceeds {}MB limit; skipping disk append",
                self.max_file_mb
            );
        } else {
            match serde_json::to_string(&record) {
                Ok(json) => {
                    if let Err(e) = OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&path)
                        .and_then(|mut f| writeln!(f, "{json}"))
                    {
                        warn!("history: write to {path:?} failed: {e}");
                    }
                }
                Err(e) => warn!("history: serialize failed: {e}"),
            }
        }

        self.push_to_buf(record);
    }

    /// Return the last `n` pressure levels (oldest first) for hysteresis trend seeding.
    #[must_use]
    pub fn recent_levels(&self, n: usize) -> Vec<PressureLevel> {
        let start = self.recent_buf.len().saturating_sub(n);
        self.recent_buf[start..]
            .iter()
            .map(|r| r.pressure_level)
            .collect()
    }

    /// Total in-memory record count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.recent_buf.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.recent_buf.is_empty()
    }

    // ---------------------------------------------------------------------------
    // Private helpers
    // ---------------------------------------------------------------------------

    fn today_file(&self) -> PathBuf {
        self.dir
            .join(format!("history-{}.jsonl", Utc::now().format("%Y-%m-%d")))
    }

    fn push_to_buf(&mut self, record: HistoryRecord) {
        self.recent_buf.push(record);
        if self.recent_buf.len() > RECENT_BUF_CAP {
            self.recent_buf.remove(0);
        }
    }

    /// Load the most recent records from disk into `recent_buf`.
    /// Corrupt lines are skipped with a `warn!`.
    fn load_recent_from_disk(&mut self) {
        let Some(path) = self.latest_file() else {
            return;
        };
        let content = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                warn!("history: cannot read {path:?}: {e}");
                return;
            }
        };
        let records: Vec<HistoryRecord> = content
            .lines()
            .filter_map(|line| {
                if line.trim().is_empty() {
                    return None;
                }
                serde_json::from_str::<HistoryRecord>(line)
                    .map_err(|e| warn!("history: corrupt line in {path:?}: {e}"))
                    .ok()
            })
            .collect();

        let start = records.len().saturating_sub(RECENT_BUF_CAP);
        self.recent_buf = records[start..].to_vec();
    }

    /// Return the path of the most recently dated history file in the store dir.
    fn latest_file(&self) -> Option<PathBuf> {
        Self::history_files(&self.dir)
            .into_iter()
            .max_by_key(|(date, _)| *date)
            .map(|(_, path)| path)
    }

    /// Delete files whose filename date is older than `retention_days` days ago.
    fn prune_old_files(&self) {
        let cutoff =
            Utc::now().date_naive() - chrono::Duration::days(i64::from(self.retention_days));
        for (date, path) in Self::history_files(&self.dir) {
            if date < cutoff {
                if let Err(e) = fs::remove_file(&path) {
                    warn!("history: cannot prune {path:?}: {e}");
                }
            }
        }
    }

    /// Parse all `history-YYYY-MM-DD.jsonl` entries in `dir` into `(NaiveDate, PathBuf)` pairs.
    fn history_files(dir: &Path) -> Vec<(NaiveDate, PathBuf)> {
        let Ok(entries) = fs::read_dir(dir) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .filter_map(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                let date_str = name
                    .strip_prefix("history-")
                    .and_then(|s| s.strip_suffix(".jsonl"))?;
                let date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d").ok()?;
                Some((date, e.path()))
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::config;
    use crate::pressure::PressureLevel;

    fn temp_store(suffix: &str) -> (HistoryStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!("syswarden_history_test_{suffix}"));
        let _ = fs::remove_dir_all(&dir);
        let mut config = config::defaults();
        config.history.dir = dir.to_string_lossy().to_string();
        let store = HistoryStore::open(&config).expect("open");
        (store, dir)
    }

    fn make_record(level: PressureLevel) -> HistoryRecord {
        HistoryRecord {
            schema_version: SCHEMA_VERSION,
            timestamp: Utc::now(),
            pressure_level: level,
            psi_summary: "cpu=0.0 mem=0.0 io=0.0".to_string(),
            state: "Idle".to_string(),
            action_count: 0,
            simulated_count: 0,
            blocked_count: 0,
            outcomes: Vec::new(),
        }
    }

    #[test]
    fn history_record_round_trip() {
        let rec = make_record(PressureLevel::Moderate);
        let json = serde_json::to_string(&rec).expect("serialize");
        let back: HistoryRecord = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.schema_version, SCHEMA_VERSION);
        assert_eq!(back.pressure_level, PressureLevel::Moderate);
        assert_eq!(back.state, "Idle");
        assert_eq!(back.psi_summary, "cpu=0.0 mem=0.0 io=0.0");
    }

    #[test]
    fn history_store_append_persists_to_disk() {
        let (mut store, dir) = temp_store("append");
        store.append(make_record(PressureLevel::None));
        store.append(make_record(PressureLevel::High));
        assert_eq!(store.len(), 2);

        // Verify disk file exists and has two lines.
        let today = Utc::now().format("%Y-%m-%d");
        let path = dir.join(format!("history-{today}.jsonl"));
        let content = fs::read_to_string(&path).expect("jsonl exists");
        assert_eq!(content.lines().count(), 2);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_store_loads_from_disk_on_open() {
        let (mut store, dir) = temp_store("reload");
        store.append(make_record(PressureLevel::Low));
        store.append(make_record(PressureLevel::High));

        // Re-open and verify recent_buf is seeded from disk.
        let mut config = config::defaults();
        config.history.dir = dir.to_string_lossy().to_string();
        let store2 = HistoryStore::open(&config).expect("reopen");
        assert_eq!(store2.len(), 2);
        let levels = store2.recent_levels(2);
        assert_eq!(levels, vec![PressureLevel::Low, PressureLevel::High]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_store_corrupt_line_tolerance() {
        let (_, dir) = temp_store("corrupt");
        let today = Utc::now().format("%Y-%m-%d");
        let path = dir.join(format!("history-{today}.jsonl"));

        // Write one valid + one corrupt line manually.
        let valid = make_record(PressureLevel::Critical);
        let valid_json = serde_json::to_string(&valid).unwrap();
        fs::write(&path, format!("{valid_json}\n{{not valid json}}\n")).unwrap();

        let mut config = config::defaults();
        config.history.dir = dir.to_string_lossy().to_string();
        let store = HistoryStore::open(&config).expect("open despite corrupt line");

        // Corrupt line is skipped; only the valid record loads.
        assert_eq!(store.len(), 1);
        assert_eq!(store.recent_levels(1), vec![PressureLevel::Critical]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_store_retention_prune() {
        let (_, dir) = temp_store("retention");
        // Write two old files (outside retention window).
        let old1 = dir.join("history-2000-01-01.jsonl");
        let old2 = dir.join("history-2000-06-15.jsonl");
        // Write one file that is within retention (today).
        let today = Utc::now().format("%Y-%m-%d");
        let current = dir.join(format!("history-{today}.jsonl"));

        fs::write(&old1, b"").unwrap();
        fs::write(&old2, b"").unwrap();
        fs::write(&current, b"").unwrap();

        let mut config = config::defaults();
        config.history.dir = dir.to_string_lossy().to_string();
        config.history.retention_days = 14;
        let _ = HistoryStore::open(&config).expect("open triggers prune");

        assert!(!old1.exists(), "old file 1 should be pruned");
        assert!(!old2.exists(), "old file 2 should be pruned");
        assert!(current.exists(), "current file should survive");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_store_recent_levels_oldest_first() {
        let (mut store, dir) = temp_store("levels");
        let levels = [
            PressureLevel::None,
            PressureLevel::Low,
            PressureLevel::Moderate,
            PressureLevel::High,
            PressureLevel::Critical,
        ];
        for &l in &levels {
            store.append(make_record(l));
        }
        // recent_levels(3) → last 3: Moderate, High, Critical
        let recent = store.recent_levels(3);
        assert_eq!(
            recent,
            vec![
                PressureLevel::Moderate,
                PressureLevel::High,
                PressureLevel::Critical
            ]
        );
        // recent_levels(0) → empty
        assert!(store.recent_levels(0).is_empty());
        // recent_levels(10) → all 5 (clamped by saturating_sub)
        assert_eq!(store.recent_levels(10).len(), 5);

        let _ = fs::remove_dir_all(&dir);
    }
}
