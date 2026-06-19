//! Process enumeration and anomaly flagging; never acts on processes (architecture.md §5.6).
#![allow(dead_code)]

use crate::config::AppConfig;
use crate::error::SyswardenError;

/// User-space clock ticks per second. Virtually universal on Linux; avoids `libc::sysconf`.
const USER_HZ: u64 = 100;

/// Why a process was flagged (architecture.md §5.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessFlag {
    HighCpu,
    HighMemory,
    RuleViolation,
}

/// Per-process snapshot for one collection tick (architecture.md §15).
///
/// `cpu_pct` is a lifetime average (since process start). `io_read_rate` and
/// `io_write_rate` are always 0.0 in v0.1: computing rates requires two samples
/// and root access to `/proc/{pid}/io`.
#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub comm: String,
    pub cmdline: String,
    pub cpu_pct: f64,
    pub rss_kb: u64,
    pub io_read_rate: f64,
    pub io_write_rate: f64,
    pub nice: i32,
    pub is_protected: bool,
    pub flags: Vec<ProcessFlag>,
}

/// Parse `/proc/{pid}/stat` content → `(total_cpu_ticks, nice)`.
///
/// Uses `rfind(')')` to handle process names that contain spaces or parentheses.
pub fn parse_pid_stat(content: &str) -> Result<(u64, i32), SyswardenError> {
    let after_comm = content
        .rfind(')')
        .ok_or_else(|| SyswardenError::Parse("missing ')' in /proc/pid/stat".into()))?;
    let rest = content[after_comm + 1..].trim_start();
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // After ')': state(0) ppid(1) pgrp(2) sess(3) tty(4) tpgid(5) flags(6)
    //   minflt(7) cminflt(8) majflt(9) cmajflt(10) utime(11) stime(12) cutime(13)
    //   cstime(14) priority(15) nice(16) …
    if fields.len() < 17 {
        return Err(SyswardenError::Parse(format!(
            "/proc/pid/stat: {} fields after comm, need ≥17",
            fields.len()
        )));
    }
    let utime: u64 = fields[11]
        .parse()
        .map_err(|_| SyswardenError::Parse("bad utime in /proc/pid/stat".into()))?;
    let stime: u64 = fields[12]
        .parse()
        .map_err(|_| SyswardenError::Parse("bad stime in /proc/pid/stat".into()))?;
    let nice: i32 = fields[16]
        .parse()
        .map_err(|_| SyswardenError::Parse("bad nice in /proc/pid/stat".into()))?;
    Ok((utime.saturating_add(stime), nice))
}

/// Extract `VmRSS` (in kB) from `/proc/{pid}/status` content.
pub fn parse_rss_kb(status_content: &str) -> Result<u64, SyswardenError> {
    status_content
        .lines()
        .find(|l| l.starts_with("VmRSS:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .ok_or_else(|| SyswardenError::Parse("missing VmRSS in /proc/pid/status".into()))?
        .parse::<u64>()
        .map_err(|_| SyswardenError::Parse("bad VmRSS value in /proc/pid/status".into()))
}

/// Parse uptime in seconds from `/proc/uptime` content.
pub fn parse_uptime(content: &str) -> Result<f64, SyswardenError> {
    content
        .split_whitespace()
        .next()
        .ok_or_else(|| SyswardenError::Parse("empty /proc/uptime".into()))?
        .parse::<f64>()
        .map_err(|_| SyswardenError::Parse("bad uptime value in /proc/uptime".into()))
}

/// Compute lifetime-average CPU% from cumulative tick count and system uptime.
#[allow(clippy::cast_precision_loss)]
pub fn cpu_pct_from_ticks(total_ticks: u64, uptime_secs: f64) -> f64 {
    if uptime_secs < 0.001 {
        return 0.0;
    }
    (total_ticks as f64) / (USER_HZ as f64) / uptime_secs * 100.0
}

fn is_process_protected(comm: &str, config: &AppConfig) -> bool {
    config.protected.processes.iter().any(|p| p == comm)
}

fn compute_flags(cpu: f64, rss_kb: u64, comm: &str, config: &AppConfig) -> Vec<ProcessFlag> {
    let mut high_cpu = false;
    let mut high_mem = false;

    for rule in &config.process_rules {
        if !comm.contains(rule.name_match.as_str()) {
            continue;
        }
        if cpu > rule.max_cpu_pct {
            high_cpu = true;
        }
        if rss_kb / 1024 > rule.max_rss_mb {
            high_mem = true;
        }
    }

    let mut flags = Vec::new();
    if high_cpu {
        flags.push(ProcessFlag::HighCpu);
    }
    if high_mem {
        flags.push(ProcessFlag::HighMemory);
    }
    if high_cpu || high_mem {
        flags.push(ProcessFlag::RuleViolation);
    }
    flags
}

#[allow(clippy::similar_names)]
fn read_process_entry(pid: u32, config: &AppConfig, uptime_secs: f64) -> Option<ProcessInfo> {
    let proc_dir = format!("/proc/{pid}");

    let comm = std::fs::read_to_string(format!("{proc_dir}/comm"))
        .ok()?
        .trim()
        .to_string();

    let cmdline = {
        let raw = std::fs::read_to_string(format!("{proc_dir}/cmdline")).unwrap_or_default();
        raw.trim_matches('\0').replace('\0', " ").trim().to_string()
    };

    let stat_text = std::fs::read_to_string(format!("{proc_dir}/stat")).ok()?;
    let (total_ticks, nice) = parse_pid_stat(&stat_text).ok()?;

    let status_text = std::fs::read_to_string(format!("{proc_dir}/status")).ok()?;
    let rss_kb = parse_rss_kb(&status_text).unwrap_or(0);

    let cpu = cpu_pct_from_ticks(total_ticks, uptime_secs);
    let protected = is_process_protected(&comm, config);
    let flags = compute_flags(cpu, rss_kb, &comm, config);

    Some(ProcessInfo {
        pid,
        comm,
        cmdline,
        cpu_pct: cpu,
        rss_kb,
        io_read_rate: 0.0,
        io_write_rate: 0.0,
        nice,
        is_protected: protected,
        flags,
    })
}

fn list_pids() -> Vec<u32> {
    std::fs::read_dir("/proc")
        .map(|dir| {
            dir.filter_map(|entry| {
                let name = entry.ok()?.file_name();
                name.to_str()?.parse::<u32>().ok()
            })
            .collect()
        })
        .unwrap_or_default()
}

/// Enumerate all processes from `/proc` and flag anomalous ones per config rules.
///
/// Processes whose `/proc` entry disappears mid-scan are silently skipped —
/// this is normal for short-lived processes and is not an error.
/// Never signals, kills, re-nices, or otherwise modifies any process.
pub fn analyze(config: &AppConfig) -> Vec<ProcessInfo> {
    let uptime_secs = std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|c| parse_uptime(&c).ok())
        .unwrap_or(1.0);

    list_pids()
        .into_iter()
        .filter_map(|pid| read_process_entry(pid, config, uptime_secs))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppConfig, OnViolation, ProcessRule};

    const PID_STAT_FIXTURE: &str = include_str!("../../examples/fixtures/proc_pid_stat.sample");
    const PID_STATUS_FIXTURE: &str = include_str!("../../examples/fixtures/proc_pid_status.sample");

    #[test]
    fn parse_pid_stat_fixture() {
        let (ticks, nice) = parse_pid_stat(PID_STAT_FIXTURE).expect("fixture should parse");
        assert_eq!(ticks, 3000); // utime=2000 + stime=1000
        assert_eq!(nice, 0);
    }

    #[test]
    fn parse_pid_stat_handles_comm_with_spaces() {
        let content = "100 (my proc name) S 1 100 100 0 -1 0 0 0 0 0 500 250 0 0 20 0 1 0 0 0 0";
        let (ticks, nice) = parse_pid_stat(content).expect("comm with spaces should parse");
        assert_eq!(ticks, 750); // 500 + 250
        assert_eq!(nice, 0);
    }

    #[test]
    fn parse_pid_stat_error_on_missing_paren() {
        let err = parse_pid_stat("1234 broken S 1").unwrap_err();
        assert!(matches!(err, SyswardenError::Parse(_)));
    }

    #[test]
    fn parse_pid_stat_error_on_too_few_fields() {
        let err = parse_pid_stat("1234 (x) S 1 2 3").unwrap_err();
        assert!(matches!(err, SyswardenError::Parse(_)));
    }

    #[test]
    fn parse_rss_kb_fixture() {
        let rss = parse_rss_kb(PID_STATUS_FIXTURE).expect("fixture should parse");
        assert_eq!(rss, 204_800);
    }

    #[test]
    fn parse_rss_kb_error_on_missing_field() {
        let err = parse_rss_kb("Name:\ttest\nState:\tS\n").unwrap_err();
        assert!(matches!(err, SyswardenError::Parse(_)));
    }

    #[test]
    fn parse_uptime_returns_first_field() {
        let uptime = "12345.67 5678.90\n";
        let got = parse_uptime(uptime).expect("should parse");
        assert!((got - 12_345.67).abs() < 0.01, "got {got}");
    }

    #[test]
    fn cpu_pct_from_ticks_correct() {
        // 3000 ticks / 100 Hz / 300s * 100 = 10%
        let pct = cpu_pct_from_ticks(3000, 300.0);
        assert!((pct - 10.0).abs() < 0.01, "got {pct}");
    }

    #[test]
    fn cpu_pct_from_ticks_zero_on_tiny_uptime() {
        let pct = cpu_pct_from_ticks(1000, 0.0);
        assert!(pct < f64::EPSILON);
    }

    #[test]
    fn is_protected_matches_default_set() {
        let cfg = AppConfig::default();
        assert!(is_process_protected("syswarden", &cfg));
        assert!(is_process_protected("systemd", &cfg));
        assert!(!is_process_protected("firefox", &cfg));
    }

    #[test]
    fn compute_flags_high_cpu_rule_violation() {
        let mut cfg = AppConfig::default();
        cfg.process_rules.push(ProcessRule {
            name_match: "stress".to_string(),
            max_cpu_pct: 50.0,
            max_rss_mb: 4096,
            sustained_secs: 30,
            on_violation: OnViolation::FlagOnly,
        });
        let flags = compute_flags(75.0, 512 * 1024, "stress", &cfg);
        assert!(flags.contains(&ProcessFlag::HighCpu));
        assert!(flags.contains(&ProcessFlag::RuleViolation));
        assert!(!flags.contains(&ProcessFlag::HighMemory));
    }

    #[test]
    fn compute_flags_high_memory_rule_violation() {
        let mut cfg = AppConfig::default();
        cfg.process_rules.push(ProcessRule {
            name_match: "chromium".to_string(),
            max_cpu_pct: 80.0,
            max_rss_mb: 2048,
            sustained_secs: 60,
            on_violation: OnViolation::RecommendNice,
        });
        // rss_kb = 3_000_000 → rss_mb = 2929 > 2048
        let flags = compute_flags(5.0, 3_000_000, "chromium", &cfg);
        assert!(flags.contains(&ProcessFlag::HighMemory));
        assert!(flags.contains(&ProcessFlag::RuleViolation));
        assert!(!flags.contains(&ProcessFlag::HighCpu));
    }

    #[test]
    fn compute_flags_empty_with_no_rules() {
        let cfg = AppConfig::default(); // process_rules is empty by default
        let flags = compute_flags(99.0, 8_000_000, "anything", &cfg);
        assert!(flags.is_empty());
    }

    #[test]
    fn compute_flags_no_violation_within_limits() {
        let mut cfg = AppConfig::default();
        cfg.process_rules.push(ProcessRule {
            name_match: "my_app".to_string(),
            max_cpu_pct: 80.0,
            max_rss_mb: 4096,
            sustained_secs: 30,
            on_violation: OnViolation::FlagOnly,
        });
        let flags = compute_flags(50.0, 1_000_000, "my_app", &cfg);
        assert!(flags.is_empty());
    }

    #[test]
    #[ignore = "reads live /proc files; requires Linux"]
    fn analyze_live_returns_processes() {
        let cfg = AppConfig::default();
        let procs = analyze(&cfg);
        assert!(!procs.is_empty(), "should find at least one process");
        if let Some(sd) = procs.iter().find(|p| p.comm == "systemd") {
            assert!(sd.is_protected, "systemd must be protected");
            assert!(sd.rss_kb > 0, "systemd must have non-zero RSS");
        }
    }
}
