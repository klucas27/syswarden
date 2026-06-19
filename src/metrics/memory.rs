//! Memory metrics from `/proc/meminfo`; produces `MemoryMetrics` (architecture.md §5.4, §15).
#![allow(dead_code, clippy::module_name_repetitions)]

use std::collections::HashMap;
use std::path::Path;

use crate::error::SyswardenError;

/// Memory state from one collection tick (architecture.md §15).
///
/// `available_kb` (from `MemAvailable`) drives pressure decisions — not `free_kb`.
#[derive(Debug, Clone)]
pub struct MemoryMetrics {
    pub total_kb: u64,
    pub available_kb: u64,
    pub free_kb: u64,
    pub buffers_kb: u64,
    pub cached_kb: u64,
    pub swap_total_kb: u64,
    pub swap_used_kb: u64,
    /// Pages/second swapped in; 0.0 until a previous sample exists.
    pub swap_in_rate: f64,
    /// Pages/second swapped out; 0.0 until a previous sample exists.
    pub swap_out_rate: f64,
}

/// Parse `/proc/meminfo` text content into [`MemoryMetrics`].
///
/// `swap_in_rate` and `swap_out_rate` are always 0.0 here; `collect()` fills them in
/// from the vmstat delta after two samples.
///
/// # Errors
/// Returns `Err` if required fields (`MemTotal`, `MemAvailable`) are missing.
pub fn parse(content: &str) -> Result<MemoryMetrics, SyswardenError> {
    let mut map: HashMap<&str, u64> = HashMap::new();
    for line in content.lines() {
        let mut parts = line.split_whitespace();
        if let (Some(key), Some(val)) = (parts.next(), parts.next()) {
            let key = key.trim_end_matches(':');
            if let Ok(n) = val.parse::<u64>() {
                map.insert(key, n);
            }
        }
    }

    let get = |k: &str| -> Result<u64, SyswardenError> {
        map.get(k)
            .copied()
            .ok_or_else(|| SyswardenError::Parse(format!("missing /proc/meminfo field: {k}")))
    };

    let swap_total = get("SwapTotal")?;
    let swap_free = get("SwapFree")?;

    Ok(MemoryMetrics {
        total_kb: get("MemTotal")?,
        available_kb: get("MemAvailable")?,
        free_kb: get("MemFree")?,
        buffers_kb: get("Buffers")?,
        cached_kb: get("Cached")?,
        swap_total_kb: swap_total,
        swap_used_kb: swap_total.saturating_sub(swap_free),
        swap_in_rate: 0.0,
        swap_out_rate: 0.0,
    })
}

/// Read and parse `/proc/meminfo` from `path`.
///
/// # Errors
/// Returns `Err` on I/O failure or if required fields are missing.
pub fn read(path: &Path) -> Result<MemoryMetrics, SyswardenError> {
    let content = std::fs::read_to_string(path)?;
    parse(&content)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../examples/fixtures/proc_meminfo.sample");

    #[test]
    fn parses_all_fields() {
        let m = parse(FIXTURE).expect("fixture should parse");
        assert_eq!(m.total_kb, 16_384_000);
        assert_eq!(m.available_kb, 4_096_000);
        assert_eq!(m.free_kb, 512_000);
        assert_eq!(m.buffers_kb, 256_000);
        assert_eq!(m.cached_kb, 2_048_000);
        assert_eq!(m.swap_total_kb, 8_192_000);
        assert_eq!(m.swap_used_kb, 0); // SwapFree == SwapTotal in fixture
    }

    #[test]
    fn uses_mem_available_not_mem_free() {
        let m = parse(FIXTURE).expect("fixture should parse");
        // MemAvailable (4_096_000) != MemFree (512_000) in the fixture.
        assert_ne!(
            m.available_kb, m.free_kb,
            "available_kb must come from MemAvailable, not MemFree"
        );
        assert_eq!(m.available_kb, 4_096_000);
    }

    #[test]
    fn swap_used_is_total_minus_free() {
        let content = "MemTotal: 1000 kB\nMemFree: 100 kB\nMemAvailable: 200 kB\n\
            Buffers: 50 kB\nCached: 100 kB\nSwapTotal: 2000 kB\nSwapFree: 1500 kB\n";
        let m = parse(content).expect("should parse");
        assert_eq!(m.swap_used_kb, 500);
    }

    #[test]
    fn swap_rates_default_to_zero() {
        let m = parse(FIXTURE).expect("fixture should parse");
        assert!(m.swap_in_rate < f64::EPSILON);
        assert!(m.swap_out_rate < f64::EPSILON);
    }

    #[test]
    fn parse_error_on_missing_field() {
        let err = parse("MemTotal: 1000 kB\n").unwrap_err();
        assert!(matches!(err, SyswardenError::Parse(_)));
    }
}
