#!/usr/bin/env bash
# Activate profile: developer
#
# Moderate risk — mix of interactive and batch workloads (builds, test runners,
# language servers). nice + ionice + cpu_weight + io_weight + memory_high.
#
# Risk level : Moderate
# Actions    : AdjustNice, AdjustIonice, SetCpuWeight, SetIoWeight, SetMemoryHigh
#
# Usage: sudo ./scripts/profiles/activate-developer.sh

set -euo pipefail
# shellcheck source=_bootstrap.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_bootstrap.sh"

check_root
ensure_syswarden

# ---------------------------------------------------------------------------
# Write config
# ---------------------------------------------------------------------------

mkdir -p "$CONFIG_DIR"
cat > "$CONFIG_FILE" << 'TOML'
# syswarden — developer profile
# Moderate risk: nice + ionice + cpu_weight + io_weight + memory_high.
# Suited for machines running builds, test runners, and language servers.
# Add your services to [allowed].services to enable governance.

[global]
profile                  = "developer"
dry_run                  = false
allow_aggressive_actions = false
allow_zram_apply         = false
allow_sysctl_apply       = false
log_level                = "info"

[polling]
idle_interval_secs     = 6
pressure_interval_secs = 2
min_interval_secs      = 2
max_interval_secs      = 60
hysteresis_ticks       = 3

[pressure.thresholds]
cpu_moderate           = 15.0
cpu_high               = 35.0
cpu_critical           = 60.0
mem_some_moderate      = 10.0
mem_full_high          = 5.0
mem_full_critical      = 20.0
io_moderate            = 15.0
io_high                = 35.0
io_critical            = 60.0
mem_available_low_pct  = 10.0

[protected]
processes = ["systemd", "init", "kthreadd", "syswarden", "sshd", "dbus-daemon"]
services  = [
    "syswarden.service",
    "dbus.service",
    "NetworkManager.service",
    "sshd.service",
    "systemd-journald.service",
    "systemd-logind.service",
    "systemd-udevd.service",
]

# Add the services you want syswarden to govern.
[allowed]
services = [
    # "myapp.service",
    # "docker.service",
    # "containerd.service",
]

[history]
dir            = "/var/lib/syswarden/history"
retention_days = 14
max_file_mb    = 32

[logging]
audit_dir = "/var/lib/syswarden/audit"
journald  = true

[rollback]
dir          = "/var/lib/syswarden/rollback"
keep_entries = 100
TOML

green "  Config written: $CONFIG_FILE"
restart_or_enable "developer"
first_cleanup "developer"
profile_summary "developer"
