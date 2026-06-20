//! cgroups v2 detection and read-only usage/limit queries (architecture.md §5.9, Phase 31).
//!
//! Writes are never performed here — all resource-control changes go through systemd.
//! Phase 31 extends `CgroupReading` with fields needed by the service rule engine
//! (Phase 32) and the `SetMemoryMax` guard (Phase 33).

use std::path::{Path, PathBuf};

use anyhow::Result;

const CGROUP_ROOT: &str = "/sys/fs/cgroup";

/// Which cgroup hierarchy is active on the running system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CgroupMode {
    /// Unified cgroup v2 hierarchy (`/sys/fs/cgroup/cgroup.controllers` present).
    V2,
    /// Hybrid: v1 controllers + v2 unified mount side-by-side.
    Hybrid,
    /// Legacy cgroup v1.
    V1,
    /// `/sys/fs/cgroup` absent; cgroups unavailable.
    Unavailable,
}

/// Resource-limit and usage readings from one cgroup directory (Phase 31, architecture.md §5.9).
///
/// All fields are `Option<u64>`; missing or unreadable kernel files yield `None`.
/// "max" sentinel values (kernel's representation of "unlimited") are normalised to `None`.
#[derive(Debug, Clone, Default)]
pub struct CgroupReading {
    // ---- resource-control limits (set by systemd) ----
    /// `cpu.weight` (v2 default: 100).
    pub cpu_weight: Option<u64>,
    /// `io.weight` (v2 default: 100).
    pub io_weight: Option<u64>,
    /// `memory.high` in bytes. `None` = kernel reports "max" (unlimited).
    pub memory_high: Option<u64>,
    /// `memory.max` in bytes. `None` = kernel reports "max" (unlimited).
    /// Used by Phase 33 to check whether `MemoryMax` has already been applied.
    pub memory_max: Option<u64>,

    // ---- live usage ----
    /// `memory.current` in bytes (instantaneous RSS + page cache for the cgroup).
    pub memory_current: Option<u64>,
    /// `memory.swap.current` in bytes (swap currently in use by the cgroup).
    pub memory_swap_current: Option<u64>,
    /// `usage_usec` from `cpu.stat` (total CPU time consumed by the cgroup in µs).
    pub cpu_usage_usec: Option<u64>,
    /// `pids.current` — number of live tasks in the cgroup.
    pub pids_current: Option<u64>,

    // ---- memory.stat breakdown (anonymous vs file-backed) ----
    /// Anonymous memory in bytes (from `memory.stat`).
    pub memory_anon: Option<u64>,
    /// File-backed memory (page cache) in bytes (from `memory.stat`).
    pub memory_file: Option<u64>,

    // ---- io.stat aggregates (summed across all block devices) ----
    /// Total bytes read across all block devices (from `io.stat`).
    pub io_rbytes: Option<u64>,
    /// Total bytes written across all block devices (from `io.stat`).
    pub io_wbytes: Option<u64>,
}

/// Detect which cgroup hierarchy is active.
#[must_use]
pub fn detect() -> CgroupMode {
    detect_from_root(Path::new(CGROUP_ROOT))
}

/// Testable inner version of [`detect`]: operates on an arbitrary `root`.
#[must_use]
pub fn detect_from_root(root: &Path) -> CgroupMode {
    if root.join("cgroup.controllers").exists() {
        CgroupMode::V2
    } else if root.join("unified").join("cgroup.controllers").exists() {
        CgroupMode::Hybrid
    } else if root.exists() {
        CgroupMode::V1
    } else {
        CgroupMode::Unavailable
    }
}

/// Read resource limits and current usage from `cgroup_path` (a cgroup directory).
///
/// Missing files are silently skipped and the corresponding field is set to `None`.
/// Returns an error only if the path cannot be accessed at all.
///
/// # Errors
///
/// Currently infallible — always returns `Ok`. The `Result` wrapper exists for
/// forward compatibility with richer failure modes in later phases.
pub fn read(cgroup_path: &Path) -> Result<CgroupReading> {
    let mem_stat = read_memory_stat(&cgroup_path.join("memory.stat"));
    let io_counters = read_io_stat(&cgroup_path.join("io.stat"));
    Ok(CgroupReading {
        cpu_weight: read_u64_file(&cgroup_path.join("cpu.weight")),
        io_weight: read_u64_file(&cgroup_path.join("io.weight")),
        memory_high: read_max_sentinel(&cgroup_path.join("memory.high")),
        memory_max: read_max_sentinel(&cgroup_path.join("memory.max")),
        memory_current: read_u64_file(&cgroup_path.join("memory.current")),
        memory_swap_current: read_u64_file(&cgroup_path.join("memory.swap.current")),
        cpu_usage_usec: read_cpu_usage(&cgroup_path.join("cpu.stat")),
        pids_current: read_u64_file(&cgroup_path.join("pids.current")),
        memory_anon: mem_stat.0,
        memory_file: mem_stat.1,
        io_rbytes: io_counters.0,
        io_wbytes: io_counters.1,
    })
}

/// Derive the conventional cgroup v2 path for a `system.slice` unit.
///
/// Most user-space services land here; the path is a best-guess used for
/// prior-state capture before applying a resource-control change.
#[must_use]
pub fn service_cgroup_path(unit_name: &str) -> PathBuf {
    PathBuf::from(CGROUP_ROOT)
        .join("system.slice")
        .join(unit_name)
}

fn read_u64_file(path: &Path) -> Option<u64> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

/// Read a cgroup file that uses `"max"` as the kernel sentinel for unlimited.
/// `"max"` → `None`; a numeric value → `Some(n)`.
fn read_max_sentinel(path: &Path) -> Option<u64> {
    let content = std::fs::read_to_string(path).ok()?;
    let s = content.trim();
    if s == "max" {
        None
    } else {
        s.parse().ok()
    }
}

fn read_cpu_usage(path: &Path) -> Option<u64> {
    let content = std::fs::read_to_string(path).ok()?;
    content.lines().find_map(|line| {
        line.strip_prefix("usage_usec ")
            .and_then(|v| v.trim().parse().ok())
    })
}

/// Parse `memory.stat` → `(anon, file)` in bytes.
///
/// Returns `(None, None)` if the file is absent or unreadable.
fn read_memory_stat(path: &Path) -> (Option<u64>, Option<u64>) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return (None, None);
    };
    let mut anon: Option<u64> = None;
    let mut file: Option<u64> = None;
    for line in content.lines() {
        if let Some(v) = line.strip_prefix("anon ") {
            anon = v.trim().parse().ok();
        } else if let Some(v) = line.strip_prefix("file ") {
            file = v.trim().parse().ok();
        }
        if anon.is_some() && file.is_some() {
            break;
        }
    }
    (anon, file)
}

/// Parse `io.stat` → `(total_rbytes, total_wbytes)` summed across all devices.
///
/// Format per line: `MAJ:MIN rbytes=N wbytes=N rios=N wios=N dbytes=N dios=N`
/// Returns `(None, None)` if the file is absent or no lines parse.
fn read_io_stat(path: &Path) -> (Option<u64>, Option<u64>) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return (None, None);
    };
    let mut sum_read: u64 = 0;
    let mut sum_write: u64 = 0;
    let mut found = false;
    for line in content.lines() {
        // Skip the device major:minor prefix, then scan key=value pairs.
        let fields: Vec<&str> = line.split_whitespace().collect();
        for field in &fields[1..] {
            if let Some(v) = field.strip_prefix("rbytes=") {
                if let Ok(n) = v.parse::<u64>() {
                    sum_read = sum_read.saturating_add(n);
                    found = true;
                }
            } else if let Some(v) = field.strip_prefix("wbytes=") {
                if let Ok(n) = v.parse::<u64>() {
                    sum_write = sum_write.saturating_add(n);
                }
            }
        }
    }
    if found {
        (Some(sum_read), Some(sum_write))
    } else {
        (None, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/fixtures/cgroup_v2")
    }

    fn make_service_dir(root: &Path, unit: &str) -> PathBuf {
        let dir = root.join("system.slice").join(unit);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    // --- detect_from_root ---

    #[test]
    fn detects_v2_when_controllers_file_present() {
        assert_eq!(detect_from_root(&fixture_root()), CgroupMode::V2);
    }

    #[test]
    fn detects_unavailable_for_nonexistent_root() {
        assert_eq!(
            detect_from_root(Path::new("/nonexistent/cgroup/xyz123")),
            CgroupMode::Unavailable
        );
    }

    #[test]
    fn detects_hybrid_when_only_unified_subdir_has_controllers() {
        let tmp = TempDir::new().unwrap();
        let unified = tmp.path().join("unified");
        fs::create_dir_all(&unified).unwrap();
        fs::write(unified.join("cgroup.controllers"), "cpu memory\n").unwrap();
        assert_eq!(detect_from_root(tmp.path()), CgroupMode::Hybrid);
    }

    #[test]
    fn detects_v1_when_root_exists_without_controllers() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(detect_from_root(tmp.path()), CgroupMode::V1);
    }

    // --- read: fixture round-trip (all original fields) ---

    #[test]
    fn reads_original_fields_from_fixture() {
        let dir = fixture_root().join("system.slice").join("test.service");
        let r = read(&dir).expect("read fixture");
        assert_eq!(r.cpu_weight, Some(100));
        assert_eq!(r.io_weight, Some(50));
        assert_eq!(r.memory_high, Some(4_294_967_296));
        assert_eq!(r.memory_current, Some(1_073_741_824));
        assert_eq!(r.cpu_usage_usec, Some(1_234_567));
    }

    // --- Phase 31: new fixture fields ---

    #[test]
    fn reads_memory_max_from_fixture() {
        let dir = fixture_root().join("system.slice").join("test.service");
        let r = read(&dir).expect("read fixture");
        assert_eq!(r.memory_max, Some(2_147_483_648));
    }

    #[test]
    fn reads_memory_swap_current_from_fixture() {
        let dir = fixture_root().join("system.slice").join("test.service");
        let r = read(&dir).expect("read fixture");
        assert_eq!(r.memory_swap_current, Some(67_108_864));
    }

    #[test]
    fn reads_pids_current_from_fixture() {
        let dir = fixture_root().join("system.slice").join("test.service");
        let r = read(&dir).expect("read fixture");
        assert_eq!(r.pids_current, Some(12));
    }

    #[test]
    fn reads_memory_anon_and_file_from_fixture() {
        let dir = fixture_root().join("system.slice").join("test.service");
        let r = read(&dir).expect("read fixture");
        assert_eq!(r.memory_anon, Some(524_288_000));
        assert_eq!(r.memory_file, Some(549_453_824));
    }

    #[test]
    fn reads_io_rbytes_and_wbytes_from_fixture() {
        let dir = fixture_root().join("system.slice").join("test.service");
        let r = read(&dir).expect("read fixture");
        assert_eq!(r.io_rbytes, Some(104_857_600));
        assert_eq!(r.io_wbytes, Some(52_428_800));
    }

    // --- Phase 31: max sentinel normalization ---

    #[test]
    fn memory_high_max_returns_none() {
        let tmp = TempDir::new().unwrap();
        let dir = make_service_dir(tmp.path(), "unlimited.service");
        fs::write(dir.join("memory.high"), "max\n").unwrap();
        let r = read(&dir).unwrap();
        assert_eq!(r.memory_high, None);
    }

    #[test]
    fn memory_max_sentinel_returns_none() {
        let tmp = TempDir::new().unwrap();
        let dir = make_service_dir(tmp.path(), "unlimitedmax.service");
        fs::write(dir.join("memory.max"), "max\n").unwrap();
        let r = read(&dir).unwrap();
        assert_eq!(r.memory_max, None);
    }

    // --- Phase 31: missing files → all None ---

    #[test]
    fn missing_files_produce_all_none() {
        let tmp = TempDir::new().unwrap();
        let dir = make_service_dir(tmp.path(), "empty.service");
        let r = read(&dir).unwrap();
        assert!(r.cpu_weight.is_none());
        assert!(r.io_weight.is_none());
        assert!(r.memory_high.is_none());
        assert!(r.memory_max.is_none());
        assert!(r.memory_current.is_none());
        assert!(r.memory_swap_current.is_none());
        assert!(r.cpu_usage_usec.is_none());
        assert!(r.pids_current.is_none());
        assert!(r.memory_anon.is_none());
        assert!(r.memory_file.is_none());
        assert!(r.io_rbytes.is_none());
        assert!(r.io_wbytes.is_none());
    }

    // --- Phase 31: cpu.stat ---

    #[test]
    fn reads_cpu_stat_usage_usec_from_multiline_file() {
        let tmp = TempDir::new().unwrap();
        let dir = make_service_dir(tmp.path(), "cpu_test.service");
        fs::write(
            dir.join("cpu.stat"),
            "usage_usec 9876543\nuser_usec 7000000\nsystem_usec 2876543\n",
        )
        .unwrap();
        let r = read(&dir).unwrap();
        assert_eq!(r.cpu_usage_usec, Some(9_876_543));
    }

    // --- Phase 31: memory.stat ---

    #[test]
    fn reads_memory_stat_anon_and_file() {
        let tmp = TempDir::new().unwrap();
        let dir = make_service_dir(tmp.path(), "memstat.service");
        fs::write(
            dir.join("memory.stat"),
            "anon 123456\nfile 789012\nkernel 1024\n",
        )
        .unwrap();
        let r = read(&dir).unwrap();
        assert_eq!(r.memory_anon, Some(123_456));
        assert_eq!(r.memory_file, Some(789_012));
    }

    #[test]
    fn memory_stat_missing_gives_none() {
        let tmp = TempDir::new().unwrap();
        let dir = make_service_dir(tmp.path(), "nomemstat.service");
        let r = read(&dir).unwrap();
        assert!(r.memory_anon.is_none());
        assert!(r.memory_file.is_none());
    }

    // --- Phase 31: io.stat ---

    #[test]
    fn reads_io_stat_sums_across_devices() {
        let tmp = TempDir::new().unwrap();
        let dir = make_service_dir(tmp.path(), "iost.service");
        fs::write(
            dir.join("io.stat"),
            "8:0 rbytes=1000 wbytes=2000 rios=10 wios=20 dbytes=0 dios=0\n\
             8:16 rbytes=3000 wbytes=4000 rios=30 wios=40 dbytes=0 dios=0\n",
        )
        .unwrap();
        let r = read(&dir).unwrap();
        assert_eq!(r.io_rbytes, Some(4_000));
        assert_eq!(r.io_wbytes, Some(6_000));
    }

    #[test]
    fn io_stat_missing_gives_none() {
        let tmp = TempDir::new().unwrap();
        let dir = make_service_dir(tmp.path(), "noio.service");
        let r = read(&dir).unwrap();
        assert!(r.io_rbytes.is_none());
        assert!(r.io_wbytes.is_none());
    }

    // --- Phase 31: memory.high numeric ---

    #[test]
    fn reads_memory_high_bytes_from_integer_file() {
        let tmp = TempDir::new().unwrap();
        let dir = make_service_dir(tmp.path(), "limited.service");
        fs::write(dir.join("memory.high"), "2147483648\n").unwrap();
        let r = read(&dir).unwrap();
        assert_eq!(r.memory_high, Some(2_147_483_648));
    }
}
