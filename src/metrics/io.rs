//! I/O metrics; produces `IoMetrics` (architecture.md §5.4, §15).
#![allow(dead_code, clippy::module_name_repetitions)]

/// I/O state for one collection tick (architecture.md §15).
///
/// `io_wait_pct` is derived from the CPU iowait delta in [`super::collect`].
#[derive(Debug, Clone)]
pub struct IoMetrics {
    pub io_wait_pct: f64,
}
