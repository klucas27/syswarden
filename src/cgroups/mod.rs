//! cgroups v2 detection and read-only usage/limit queries (architecture.md §5.9).
//!
//! Writes are never performed here — all resource-control changes go through systemd.

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

/// Resource-limit and usage readings from one cgroup directory.
#[derive(Debug, Clone, Default)]
pub struct CgroupReading {
    /// `cpu.weight` (v2 default: 100).
    pub cpu_weight: Option<u64>,
    /// `io.weight` (v2 default: 100).
    pub io_weight: Option<u64>,
    /// `memory.high` in bytes. `None` means the kernel reports "max" (unlimited).
    pub memory_high: Option<u64>,
    /// `memory.current` in bytes (instantaneous RSS + page cache for the cgroup).
    pub memory_current: Option<u64>,
    /// `usage_usec` from `cpu.stat` (total CPU time consumed by the cgroup in µs).
    pub cpu_usage_usec: Option<u64>,
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
    Ok(CgroupReading {
        cpu_weight: read_u64_file(&cgroup_path.join("cpu.weight")),
        io_weight: read_u64_file(&cgroup_path.join("io.weight")),
        memory_high: read_memory_high(&cgroup_path.join("memory.high")),
        memory_current: read_u64_file(&cgroup_path.join("memory.current")),
        cpu_usage_usec: read_cpu_usage(&cgroup_path.join("cpu.stat")),
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

fn read_memory_high(path: &Path) -> Option<u64> {
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

    // --- read ---

    #[test]
    fn reads_all_fields_from_fixture() {
        let dir = fixture_root().join("system.slice").join("test.service");
        let r = read(&dir).expect("read fixture");
        assert_eq!(r.cpu_weight, Some(100));
        assert_eq!(r.io_weight, Some(50));
        assert_eq!(r.memory_high, Some(4_294_967_296));
        assert_eq!(r.memory_current, Some(1_073_741_824));
        assert_eq!(r.cpu_usage_usec, Some(1_234_567));
    }

    #[test]
    fn memory_high_max_returns_none() {
        let tmp = TempDir::new().unwrap();
        let dir = make_service_dir(tmp.path(), "unlimited.service");
        fs::write(dir.join("memory.high"), "max\n").unwrap();
        let r = read(&dir).unwrap();
        assert_eq!(r.memory_high, None);
    }

    #[test]
    fn missing_files_produce_all_none() {
        let tmp = TempDir::new().unwrap();
        let dir = make_service_dir(tmp.path(), "empty.service");
        let r = read(&dir).unwrap();
        assert!(r.cpu_weight.is_none());
        assert!(r.io_weight.is_none());
        assert!(r.memory_high.is_none());
        assert!(r.memory_current.is_none());
        assert!(r.cpu_usage_usec.is_none());
    }

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

    #[test]
    fn reads_memory_high_bytes_from_integer_file() {
        let tmp = TempDir::new().unwrap();
        let dir = make_service_dir(tmp.path(), "limited.service");
        fs::write(dir.join("memory.high"), "2147483648\n").unwrap();
        let r = read(&dir).unwrap();
        assert_eq!(r.memory_high, Some(2_147_483_648));
    }
}
