#!/usr/bin/env bash
# Activate profile: low_ram
#
# Moderate risk — tuned for low-memory machines.
# Enables zram compressed swap + memory_high + nice/ionice for process fairness.
#
# Risk level : Moderate
# Actions    : AdjustNice, AdjustIonice, SetMemoryHigh, ApplyZram
#
# NOTE: ApplyZram sets up a compressed swap device. Review zram docs before
#       enabling on systems that already have a swap partition.
# NOTE: zram apply ships in v0.4 — allow_zram_apply is set ahead of time here
#       but is INERT in v0.3 (the executor does not exist yet).
#
# Usage: sudo ./scripts/profiles/activate-low_ram.sh

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
# syswarden — low_ram profile
# Moderate risk: nice + ionice + memory_high + zram compressed swap.
# Add your services to [allowed].services to enable governance.

[global]
profile                  = "low_ram"
dry_run                  = false
allow_aggressive_actions = false
allow_zram_apply         = true
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
restart_or_enable "low_ram"
first_cleanup "low_ram"
profile_summary "low_ram"
