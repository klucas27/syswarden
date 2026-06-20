#!/usr/bin/env bash
# Activate profile: performance
#
# Aggressive — full resource governance including MemoryMax and zram.
# allow_aggressive_actions=true is required for MemoryMax to fire.
#
# Risk level : Aggressive
# Actions    : AdjustNice, AdjustIonice, SetCpuWeight, SetIoWeight,
#              SetMemoryHigh, SetMemoryMax, ApplyZram
#
# NOTE: SetMemoryMax only fires AFTER SetMemoryHigh has been applied first.
# NOTE: enable allow_sysctl_apply manually if you also want sysctl tuning.
# NOTE: zram apply (ApplyZram) ships in v0.4 — allow_zram_apply is INERT in v0.3.
#
# Usage: sudo ./scripts/profiles/activate-performance.sh

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
# syswarden — performance profile
# Aggressive: full cpu/io/memory/zram governance.
# WARNING: allow_aggressive_actions=true enables MemoryMax on allowlisted services.
# Add your services to [allowed].services to enable governance.

[global]
profile                  = "performance"
dry_run                  = false
allow_aggressive_actions = true
allow_zram_apply         = true
allow_sysctl_apply       = false   # set true to also enable sysctl tuning
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
restart_or_enable "performance"
first_cleanup "performance"
profile_summary "performance"
