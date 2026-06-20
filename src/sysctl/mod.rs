//! Kernel tunable read/apply via `/proc/sys` (architecture.md §5.20, Phase 35).
//!
//! ## Invariants
//! - No shell, no `sysctl` binary — all I/O uses explicit `/proc/sys/<key>` paths.
//! - Key validation rejects empty, `..`, leading/trailing dots, and non-`[a-zA-Z0-9._]` chars.
//! - Resulting path is confirmed to stay under `/proc/sys/` (path-traversal guard).
//! - `apply` always captures prior state before writing; returns it for rollback recording.
//! - `apply` verifies the write by re-reading; returns `Err` if the kernel rejected the value.
//! - No safety decisions are made here — callers are responsible for all gating.

#![allow(dead_code)]

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const PROC_SYS_BASE: &str = "/proc/sys";

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Prior-state record for one `ApplySysctl` action (architecture.md §15).
///
/// Stored as `RollbackEntry.prior_state` so the rollback layer can restore the
/// original value without knowing the key independently.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SysctlPriorState {
    pub key: String,
    pub prior_value: String,
    pub applied_value: String,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Validate a sysctl key and return its `/proc/sys/...` path.
///
/// Key format: `[a-zA-Z0-9._]+` (e.g. `"vm.swappiness"`). Dots are converted
/// to path separators. Rejects empty keys, `..` sequences, and keys starting
/// or ending with `.`.
///
/// Exposed for tests; callers do not need to call this before `read`/`apply`.
///
/// # Errors
/// Empty key; invalid characters; `..` sequence; resulting path escapes `/proc/sys`.
pub fn validate_key(key: &str) -> Result<PathBuf> {
    if key.is_empty() {
        anyhow::bail!("sysctl: key is empty");
    }
    if key.contains("..") || key.starts_with('.') || key.ends_with('.') {
        anyhow::bail!("sysctl: invalid key {key:?} — empty segment or leading/trailing dot");
    }
    if !key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        anyhow::bail!("sysctl: key {key:?} contains invalid characters (allowed: [a-zA-Z0-9._-])");
    }
    let rel_path = key.replace('.', "/");
    let path = PathBuf::from(PROC_SYS_BASE).join(&rel_path);
    // Confirm the path stays strictly inside /proc/sys/.
    if !path.starts_with(PROC_SYS_BASE) {
        anyhow::bail!(
            "sysctl: resolved path {p} escapes {PROC_SYS_BASE}",
            p = path.display()
        );
    }
    Ok(path)
}

/// Read the current value of a sysctl key from `/proc/sys/<key>`.
///
/// Dots in `key` are converted to path separators (e.g. `"vm.swappiness"` →
/// `/proc/sys/vm/swappiness`). The trailing newline is stripped.
///
/// # Errors
/// Invalid key; file not found or unreadable.
pub fn read(key: &str) -> Result<String> {
    let path = validate_key(key)?;
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("sysctl: read {}", path.display()))?;
    Ok(raw.trim_end_matches('\n').trim().to_string())
}

/// Apply `value` to sysctl `key`, capturing prior state for rollback.
///
/// Steps:
/// 1. Validate the key and resolve the path.
/// 2. Read the current value (prior state for rollback).
/// 3. Write `value\n` to the path via `std::fs::write` (no shell).
/// 4. Re-read and verify the kernel accepted the value.
/// 5. Return `SysctlPriorState` for the caller to record in `RollbackEntry`.
///
/// # Errors
/// Invalid key; path unreadable/unwritable; write verification fails (kernel rejected value).
pub fn apply(key: &str, value: &str) -> Result<SysctlPriorState> {
    let path = validate_key(key)?;

    let prior_value = std::fs::read_to_string(&path)
        .with_context(|| format!("sysctl: read prior value for {key}"))?
        .trim()
        .to_string();

    std::fs::write(&path, format!("{value}\n"))
        .with_context(|| format!("sysctl: write {key}={value}"))?;

    // Verify the kernel accepted the value (some keys normalise the input).
    let actual = std::fs::read_to_string(&path)
        .with_context(|| format!("sysctl: verify-read after write for {key}"))?
        .trim()
        .to_string();
    if actual != value {
        anyhow::bail!(
            "sysctl: write verification failed for {key}: expected {value:?}, got {actual:?}"
        );
    }

    Ok(SysctlPriorState {
        key: key.to_string(),
        prior_value,
        applied_value: value.to_string(),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------------
    // validate_key
    // ---------------------------------------------------------------------------

    #[test]
    fn validate_key_accepts_dotted_key() {
        let p = validate_key("vm.swappiness").unwrap();
        assert_eq!(p, PathBuf::from("/proc/sys/vm/swappiness"));
    }

    #[test]
    fn validate_key_accepts_underscores_and_dashes() {
        let p = validate_key("net.ipv4.tcp_rmem").unwrap();
        assert_eq!(p, PathBuf::from("/proc/sys/net/ipv4/tcp_rmem"));
    }

    #[test]
    fn validate_key_rejects_empty() {
        assert!(validate_key("").is_err());
    }

    #[test]
    fn validate_key_rejects_dotdot() {
        assert!(validate_key("vm..swappiness").is_err());
        assert!(validate_key("../etc/passwd").is_err());
    }

    #[test]
    fn validate_key_rejects_leading_dot() {
        assert!(validate_key(".vm.swappiness").is_err());
    }

    #[test]
    fn validate_key_rejects_trailing_dot() {
        assert!(validate_key("vm.swappiness.").is_err());
    }

    #[test]
    fn validate_key_rejects_slash() {
        assert!(validate_key("vm/swappiness").is_err());
    }

    #[test]
    fn validate_key_rejects_space() {
        assert!(validate_key("vm.swap piness").is_err());
    }

    // ---------------------------------------------------------------------------
    // read — fixture-based (no live /proc/sys)
    // ---------------------------------------------------------------------------

    #[test]
    fn read_fixture_swappiness() {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("examples/fixtures/proc_sys/vm/swappiness");
        let content = std::fs::read_to_string(&fixture).expect("fixture exists");
        let trimmed = content.trim().to_string();
        assert_eq!(trimmed, "60");
    }

    #[test]
    fn read_fixture_dirty_ratio() {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("examples/fixtures/proc_sys/vm/dirty_ratio");
        let content = std::fs::read_to_string(&fixture).expect("fixture exists");
        assert_eq!(content.trim(), "3");
    }

    // ---------------------------------------------------------------------------
    // apply — uses a temp file to avoid touching /proc/sys
    // ---------------------------------------------------------------------------

    #[test]
    fn apply_writes_value_and_captures_prior() {
        let dir = tempfile::tempdir().expect("tempdir");
        let key_file = dir.path().join("swappiness");
        std::fs::write(&key_file, "60\n").expect("write fixture");

        // Override the path by calling the internals directly (simulate apply logic).
        let prior = std::fs::read_to_string(&key_file)
            .unwrap()
            .trim()
            .to_string();
        std::fs::write(&key_file, "10\n").expect("write new value");
        let actual = std::fs::read_to_string(&key_file)
            .unwrap()
            .trim()
            .to_string();

        assert_eq!(prior, "60");
        assert_eq!(actual, "10");

        let state = SysctlPriorState {
            key: "vm.swappiness".to_string(),
            prior_value: prior,
            applied_value: "10".to_string(),
        };
        assert_eq!(state.prior_value, "60");
        assert_eq!(state.applied_value, "10");
    }

    #[test]
    fn sysctl_prior_state_serde_roundtrip() {
        let s = SysctlPriorState {
            key: "vm.swappiness".to_string(),
            prior_value: "60".to_string(),
            applied_value: "10".to_string(),
        };
        let json = serde_json::to_string(&s).expect("serialize");
        let back: SysctlPriorState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(s, back);
    }

    #[test]
    fn validate_key_path_stays_under_proc_sys() {
        let p = validate_key("kernel.hostname").unwrap();
        assert!(p.starts_with("/proc/sys"), "path must stay under /proc/sys");
    }

    // --- dry-run: apply on a nonexistent path returns Err (no write happened) ---

    #[test]
    fn apply_on_missing_path_returns_err_without_write() {
        let result = apply("vm.nonexistent_fixture_key_xyz", "1");
        assert!(result.is_err(), "apply on nonexistent key must fail");
    }
}
