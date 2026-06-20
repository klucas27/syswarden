#!/usr/bin/env bash
# Activate profile: conservative
#
# Safe-only — observe and log, zero system changes (dry_run=true).
# Use this when you want syswarden running but touching nothing.
#
# Risk level : Safe
# Actions    : none (observe/log/report only)
#
# Usage: sudo ./scripts/profiles/activate-conservative.sh

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
# syswarden — conservative profile
# Safe-only: observe and log, zero system changes.
# dry_run=true is the master switch — nothing is modified.

[global]
profile                  = "conservative"
dry_run                  = true
allow_aggressive_actions = false
allow_zram_apply         = false
allow_sysctl_apply       = false
log_level                = "info"

[polling]
idle_interval_secs     = 10
pressure_interval_secs = 4
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

[allowed]
# Empty — conservative never acts on any service.
services = []

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
restart_or_enable "conservative"
first_cleanup "conservative"
profile_summary "conservative"
