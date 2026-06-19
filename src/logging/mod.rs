//! `tracing` subscriber setup and structured `AuditEvent` JSONL writer (architecture.md §5.17, §21).
#![allow(dead_code)]

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Audit event classification (architecture.md §15).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditKind {
    Decision,
    Action,
    Block,
    Error,
}

/// Append-only audit record written to JSONL (architecture.md §15, §21).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub schema_version: u32,
    pub timestamp: DateTime<Utc>,
    pub kind: AuditKind,
    /// Current system state name (e.g. `"Normal"`, `"MemoryPressure"`).
    pub state: String,
    /// Current pressure level name (e.g. `"None"`, `"Moderate"`).
    pub pressure_level: String,
    pub detail: String,
    pub result: String,
}

impl AuditEvent {
    #[must_use]
    pub fn new(
        kind: AuditKind,
        state: impl Into<String>,
        pressure_level: impl Into<String>,
        detail: impl Into<String>,
        result: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: 1,
            timestamp: Utc::now(),
            kind,
            state: state.into(),
            pressure_level: pressure_level.into(),
            detail: detail.into(),
            result: result.into(),
        }
    }
}

/// Appends [`AuditEvent`]s to `{audit_dir}/audit.jsonl` (architecture.md §5.17).
///
/// Never panics; on any I/O failure, emits a `tracing::warn!` and returns.
pub struct AuditWriter {
    audit_dir: PathBuf,
}

impl AuditWriter {
    #[must_use]
    pub fn new(audit_dir: impl Into<PathBuf>) -> Self {
        Self {
            audit_dir: audit_dir.into(),
        }
    }

    /// Append one event to `audit.jsonl`. Silently degrades on I/O errors (never panics).
    pub fn append(&self, event: &AuditEvent) {
        if let Err(e) = fs::create_dir_all(&self.audit_dir) {
            tracing::warn!("audit: cannot create dir {:?}: {e}", self.audit_dir);
            return;
        }
        let json = match serde_json::to_string(event) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("audit: serialize failed: {e}");
                return;
            }
        };
        let path = self.audit_dir.join("audit.jsonl");
        if let Err(e) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut f| writeln!(f, "{json}"))
        {
            tracing::warn!("audit: write to {path:?} failed: {e}");
        }
    }
}

/// Initialize the `tracing` subscriber (stderr, journald-compatible).
///
/// `log_level`: string from `config.global.log_level` (e.g. `"info"`, `"debug"`).
/// `verbosity`: 0 = use `log_level`, 1 = `debug`, 2+ = `trace`.
/// Safe to call multiple times; subsequent calls are no-ops.
pub fn init(log_level: &str, verbosity: u8) {
    let level: &str = match verbosity {
        0 => log_level,
        1 => "debug",
        _ => "trace",
    };
    let filter = tracing_subscriber::EnvFilter::try_new(level).unwrap_or_else(|_| {
        eprintln!("syswarden: unknown log_level {level:?}; using info");
        tracing_subscriber::EnvFilter::new("info")
    });
    // Err means a subscriber is already set (common in tests). Intentionally ignored.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_event() -> AuditEvent {
        AuditEvent::new(AuditKind::Decision, "normal", "none", "test detail", "ok")
    }

    #[test]
    fn audit_event_round_trip() {
        let event = make_event();
        let json = serde_json::to_string(&event).expect("serialize");
        let back: AuditEvent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.schema_version, 1);
        assert_eq!(back.kind, AuditKind::Decision);
        assert_eq!(back.detail, "test detail");
        assert_eq!(back.result, "ok");
    }

    #[test]
    fn audit_event_kind_serializes_snake_case() {
        let event = AuditEvent::new(AuditKind::Block, "s", "l", "d", "r");
        let json = serde_json::to_string(&event).unwrap();
        assert!(
            json.contains("\"block\""),
            "AuditKind should serialize as snake_case"
        );
    }

    #[test]
    fn audit_writer_appends_jsonl_lines() {
        let dir = std::env::temp_dir().join("syswarden_test_audit_writer");
        let _ = std::fs::remove_dir_all(&dir);

        let writer = AuditWriter::new(&dir);
        writer.append(&make_event());
        writer.append(&AuditEvent::new(
            AuditKind::Action,
            "pressure",
            "moderate",
            "action detail",
            "executed",
        ));

        let path = dir.join("audit.jsonl");
        let content = std::fs::read_to_string(&path).expect("audit.jsonl should exist");
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2, "two appended events → two lines");

        let first: serde_json::Value = serde_json::from_str(lines[0]).expect("valid JSON");
        assert_eq!(first["schema_version"], 1);
        assert_eq!(first["kind"], "decision");
        assert_eq!(first["detail"], "test detail");

        let second: serde_json::Value = serde_json::from_str(lines[1]).expect("valid JSON");
        assert_eq!(second["kind"], "action");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn audit_writer_no_panic_on_unwritable_path() {
        // Make a regular file occupy the slot where a directory would be created.
        let blocker = std::env::temp_dir().join("syswarden_test_path_blocker");
        std::fs::write(&blocker, b"I am a file, not a dir").unwrap();
        // Trying to create a subdirectory inside a file must fail gracefully.
        let bad_dir = blocker.join("subdir");
        let writer = AuditWriter::new(bad_dir);
        writer.append(&make_event()); // must not panic
        let _ = std::fs::remove_file(blocker);
    }

    #[test]
    fn init_does_not_panic_with_valid_level() {
        init("info", 0);
        init("info", 1); // debug override; second call is a no-op
        init("info", 2); // trace override; third call is a no-op
    }

    #[test]
    fn init_falls_back_on_invalid_level() {
        init("nonsense_level", 0); // must not panic; falls back to info
    }
}
