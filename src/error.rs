//! Shared typed error enum (`SyswardenError`) for all modules (planning.md §4, §6).
#![allow(dead_code)]

use thiserror::Error;

/// Typed errors returned by syswarden library functions.
///
/// Library functions return `Result<T, SyswardenError>`. The daemon wraps these
/// with `anyhow` context at call sites. No `unwrap`/`expect` on runtime paths.
#[derive(Debug, Error)]
pub enum SyswardenError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("parse error: {0}")]
    Parse(String),

    #[error("permission denied: {0}")]
    Permission(String),

    #[error("systemd error: {0}")]
    Systemd(String),

    #[error("capability unavailable: {0}")]
    Capability(String),

    #[error("action error: {0}")]
    Action(String),

    #[error("rollback error: {0}")]
    Rollback(String),
}
