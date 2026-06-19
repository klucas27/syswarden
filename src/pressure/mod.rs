//! PSI parsing, pressure classification, and system-state derivation (architecture.md §5.5, §7, §8).
#![allow(dead_code)]

use std::path::Path;

use crate::error::SyswardenError;

/// Parsed PSI metrics for one resource (`cpu`, `memory`, or `io`) (architecture.md §15).
///
/// CPU has no `full` line — `full_*` fields are 0.0 for CPU.
/// `total_us` is from the `some` line (microseconds of any stall since boot).
#[derive(Debug, Clone, Default)]
pub struct PsiMetrics {
    pub some_avg10: f64,
    pub some_avg60: f64,
    pub some_avg300: f64,
    pub full_avg10: f64,
    pub full_avg60: f64,
    pub full_avg300: f64,
    pub total_us: u64,
}

/// Parse the text content of a `/proc/pressure/{cpu,memory,io}` file into [`PsiMetrics`].
///
/// Accepts files with just a `some` line (CPU) or both `some` and `full` (memory, io).
/// Returns `Err(SyswardenError::Parse)` if the `some` line is missing or malformed.
pub fn parse_psi(content: &str) -> Result<PsiMetrics, SyswardenError> {
    let mut metrics = PsiMetrics::default();
    let mut found_some = false;

    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("some ") {
            let kv = parse_kv(rest);
            metrics.some_avg10 = get_f64(&kv, "avg10")?;
            metrics.some_avg60 = get_f64(&kv, "avg60")?;
            metrics.some_avg300 = get_f64(&kv, "avg300")?;
            metrics.total_us = get_u64(&kv, "total")?;
            found_some = true;
        } else if let Some(rest) = line.strip_prefix("full ") {
            let kv = parse_kv(rest);
            metrics.full_avg10 = get_f64(&kv, "avg10")?;
            metrics.full_avg60 = get_f64(&kv, "avg60")?;
            metrics.full_avg300 = get_f64(&kv, "avg300")?;
        }
    }

    if !found_some {
        return Err(SyswardenError::Parse(
            "missing 'some' line in PSI file".into(),
        ));
    }

    Ok(metrics)
}

/// Read and parse a PSI file at `path`.
///
/// Returns `Err(SyswardenError::Io(_))` with `kind() == NotFound` when PSI is unavailable
/// (kernel built without `CONFIG_PSI`). Callers match on this to degrade gracefully.
pub fn read_psi(path: &Path) -> Result<PsiMetrics, SyswardenError> {
    let content = std::fs::read_to_string(path)?;
    parse_psi(&content)
}

fn parse_kv(s: &str) -> std::collections::HashMap<&str, &str> {
    s.split_whitespace()
        .filter_map(|token| token.split_once('='))
        .collect()
}

fn get_f64(kv: &std::collections::HashMap<&str, &str>, key: &str) -> Result<f64, SyswardenError> {
    kv.get(key)
        .ok_or_else(|| SyswardenError::Parse(format!("missing PSI field '{key}'")))?
        .parse::<f64>()
        .map_err(|_| SyswardenError::Parse(format!("bad f64 for PSI field '{key}'")))
}

fn get_u64(kv: &std::collections::HashMap<&str, &str>, key: &str) -> Result<u64, SyswardenError> {
    kv.get(key)
        .ok_or_else(|| SyswardenError::Parse(format!("missing PSI field '{key}'")))?
        .parse::<u64>()
        .map_err(|_| SyswardenError::Parse(format!("bad u64 for PSI field '{key}'")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CPU_FIXTURE: &str = include_str!("../../examples/fixtures/pressure_cpu.sample");
    const MEMORY_FIXTURE: &str = include_str!("../../examples/fixtures/pressure_memory.sample");
    const IO_FIXTURE: &str = include_str!("../../examples/fixtures/pressure_io.sample");

    #[test]
    fn parse_cpu_psi_some_only() {
        let m = parse_psi(CPU_FIXTURE).expect("cpu fixture should parse");
        assert!(
            (m.some_avg10 - 5.32).abs() < f64::EPSILON,
            "got {}",
            m.some_avg10
        );
        assert!(
            (m.some_avg60 - 3.17).abs() < f64::EPSILON,
            "got {}",
            m.some_avg60
        );
        assert!(
            (m.some_avg300 - 1.87).abs() < f64::EPSILON,
            "got {}",
            m.some_avg300
        );
        assert_eq!(m.total_us, 12_345_678);
        // CPU has no full line — full_* must default to 0.0
        assert!(m.full_avg10 < f64::EPSILON);
        assert!(m.full_avg60 < f64::EPSILON);
    }

    #[test]
    fn parse_memory_psi_some_and_full() {
        let m = parse_psi(MEMORY_FIXTURE).expect("memory fixture should parse");
        assert!((m.some_avg10 - 2.11).abs() < f64::EPSILON);
        assert!((m.some_avg60 - 0.94).abs() < f64::EPSILON);
        assert!((m.some_avg300 - 0.42).abs() < f64::EPSILON);
        assert_eq!(m.total_us, 5_678_901);
        assert!(
            (m.full_avg10 - 0.02).abs() < f64::EPSILON,
            "got {}",
            m.full_avg10
        );
        assert!((m.full_avg60 - 0.01).abs() < f64::EPSILON);
        assert!(m.full_avg300 < f64::EPSILON);
    }

    #[test]
    fn parse_io_psi_some_and_full() {
        let m = parse_psi(IO_FIXTURE).expect("io fixture should parse");
        assert!((m.some_avg10 - 8.75).abs() < f64::EPSILON);
        assert!((m.full_avg10 - 0.51).abs() < f64::EPSILON);
        assert_eq!(m.total_us, 9_876_543);
    }

    #[test]
    fn parse_error_on_missing_some_line() {
        let err = parse_psi("full avg10=0.00 avg60=0.00 avg300=0.00 total=0\n").unwrap_err();
        assert!(matches!(err, SyswardenError::Parse(_)));
    }

    #[test]
    fn parse_error_on_empty_content() {
        let err = parse_psi("").unwrap_err();
        assert!(matches!(err, SyswardenError::Parse(_)));
    }

    #[test]
    fn parse_error_on_bad_float() {
        let err = parse_psi("some avg10=bad avg60=0.00 avg300=0.00 total=0\n").unwrap_err();
        assert!(matches!(err, SyswardenError::Parse(_)));
    }

    #[test]
    fn read_psi_not_found_is_io_error() {
        let err = read_psi(Path::new("/proc/pressure/__nonexistent__")).unwrap_err();
        assert!(
            matches!(err, SyswardenError::Io(ref e) if e.kind() == std::io::ErrorKind::NotFound),
            "expected NotFound Io error, got {err:?}"
        );
    }

    #[test]
    #[ignore = "reads live /proc/pressure files; requires Linux with CONFIG_PSI"]
    fn read_psi_live_cpu() {
        let m = read_psi(Path::new("/proc/pressure/cpu")).expect("live PSI should be readable");
        assert!(m.some_avg10 >= 0.0);
        assert!(m.some_avg10 <= 100.0);
    }
}
