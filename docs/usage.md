# syswarden Usage Guide

This document covers installation, configuration, day-to-day use, and the
safety model in detail. For the full design rationale see `architecture.md`.

---

## Table of contents

1. [Installation](#installation)
2. [First run](#first-run)
3. [Configuration reference](#configuration-reference)
4. [Profiles](#profiles)
5. [Enabling actions step by step](#enabling-actions-step-by-step)
6. [Safety model](#safety-model)
7. [Rollback](#rollback)
8. [Logs and history](#logs-and-history)
9. [Systemd service](#systemd-service)
10. [Troubleshooting](#troubleshooting)

---

## Installation

### From AUR

```sh
paru -S syswarden
# or: yay -S syswarden
```

### Manual

```sh
git clone https://github.com/kresley/syswarden
cd syswarden
cargo build --release
sudo install -Dm755 target/release/syswarden /usr/bin/syswarden
sudo install -Dm644 packaging/systemd/syswarden.service \
    /etc/systemd/system/syswarden.service
sudo systemctl daemon-reload
```

---

## First run

syswarden works without a config file — conservative built-in defaults apply:

```sh
# Analyze current system state (read-only, no changes):
syswarden analyze

# See what PSI looks like right now:
syswarden pressure

# See what actions would be taken (dry-run):
syswarden actions dry-run

# Run the daemon in the foreground (dry-run, zero changes):
syswarden daemon
```

To configure:

```sh
sudo mkdir -p /etc/syswarden
sudo cp /usr/share/doc/syswarden/config.balanced.toml /etc/syswarden/config.toml
# or use the example from the source tree:
# sudo cp examples/config.balanced.toml /etc/syswarden/config.toml
syswarden config validate   # check for issues
syswarden config show       # inspect effective config
```

---

## Configuration reference

Config file: `/etc/syswarden/config.toml` (TOML format).
Missing file → conservative defaults (`dry_run = true`, no actions).

### `[global]`

| Key | Type | Default | Description |
|---|---|---|---|
| `profile` | string | `"conservative"` | Active profile: `"conservative"`, `"balanced"`, `"performance"` |
| `dry_run` | bool | `true` | Master switch. `false` enables real state-changing actions. |
| `allow_aggressive_actions` | bool | `false` | Gate for `SetMemoryMax`, `RestartService`, `StopService` |
| `allow_zram_apply` | bool | `false` | Gate for zram configuration (v0.2+) |
| `allow_sysctl_apply` | bool | `false` | Gate for sysctl writes with backup (v0.2+) |
| `log_level` | string | `"info"` | Tracing level: `"trace"`, `"debug"`, `"info"`, `"warn"`, `"error"` |

### `[polling]`

| Key | Type | Default | Description |
|---|---|---|---|
| `idle_interval_secs` | u64 | `10` | Tick interval when idle or healthy |
| `pressure_interval_secs` | u64 | `3` | Tick interval when under pressure |
| `min_interval_secs` | u64 | `1` | Absolute minimum poll interval |
| `max_interval_secs` | u64 | `60` | Absolute maximum poll interval |
| `hysteresis_ticks` | u32 | `3` | Ticks of stable pressure before level changes |

### `[pressure.thresholds]`

All values are percentages (0–100). These are PSI `some_avg10` values except
where noted (`mem_full_*` use `full_avg10`).

| Key | Default | Description |
|---|---|---|
| `cpu_moderate` | 15.0 | CPU pressure → Moderate |
| `cpu_high` | 35.0 | CPU pressure → High |
| `cpu_critical` | 60.0 | CPU pressure → Critical |
| `mem_some_moderate` | 10.0 | Memory some-pressure → Moderate |
| `mem_full_high` | 5.0 | Memory full-pressure → High |
| `mem_full_critical` | 20.0 | Memory full-pressure → Critical |
| `io_moderate` | 15.0 | IO pressure → Moderate |
| `io_high` | 35.0 | IO pressure → High |
| `io_critical` | 60.0 | IO pressure → Critical |
| `mem_available_low_pct` | 10.0 | MemAvailable < this% of MemTotal triggers cross-check |

### `[protected]`

```toml
[protected]
processes = ["systemd", "init", "kthreadd", "syswarden", "sshd", "dbus-daemon"]
services  = ["syswarden.service", "dbus.service", "sshd.service", ...]
```

Protected targets are **never modified**, regardless of profile or flags.
`syswarden` and `syswarden.service` are always in these lists.

### `[allowed]`

```toml
[allowed]
services = []  # empty = nothing modifiable (default)
```

Only services listed here may receive resource-control changes. This is
a separate gate from `protected` — a service must be in `allowed` and
not in `protected` for any action to apply to it.

### `[history]`

| Key | Default | Description |
|---|---|---|
| `dir` | `/var/lib/syswarden/history` | JSONL history directory |
| `retention_days` | 14 | Days of history to keep |
| `max_file_mb` | 32 | Max size per daily JSONL file (MB) |

### `[logging]`

| Key | Default | Description |
|---|---|---|
| `audit_dir` | `/var/lib/syswarden/audit` | Audit JSONL directory |
| `journald` | `true` | Emit structured logs to journald |

### `[rollback]`

| Key | Default | Description |
|---|---|---|
| `dir` | `/var/lib/syswarden/rollback` | Rollback JSONL store directory |
| `keep_entries` | 100 | Maximum rollback entries to retain |

---

## Profiles

Profiles control the risk tolerance and which action categories are permitted.
They do **not** override `dry_run` or `allow_*` flags — those are independent
gates on top of the profile.

### `conservative` (default)

- Observe, log, recommend. No state-changing actions.
- Appropriate for any system, especially production.

### `balanced`

- Observe + `nice`/`ionice` on non-protected, allowed processes.
- `CPUWeight`, `IOWeight`, `MemoryHigh` on allowlisted services.
- Requires `dry_run = false` and services in `allowed.services`.

### `performance`

- All balanced actions plus `MemoryMax`, `RestartService`.
- Requires `allow_aggressive_actions = true` in addition.
- Read the safety model section before using this profile.

---

## Enabling actions step by step

Real actions are disabled by default. To gradually enable them:

### Step 1 — observe (default)

Run with all defaults. Read `syswarden analyze` and `journalctl -u syswarden`.
Understand what the daemon classifies and recommends before changing anything.

### Step 2 — allow balanced actions on specific services

```toml
[global]
profile  = "balanced"
dry_run  = false        # Enable real actions

[allowed]
services = ["my-app.service"]   # Only this service may be governed
```

syswarden will now apply `CPUWeight`/`IOWeight`/`MemoryHigh` to `my-app.service`
under pressure. Prior state is recorded for rollback.

### Step 3 — review rollback entries

```sh
syswarden rollback list      # See what was changed
```

### Step 4 (optional) — aggressive actions

```toml
[global]
allow_aggressive_actions = true
```

This enables `MemoryMax` and service restart under critical pressure. Use
only if you understand the implications for your services.

---

## Safety model

syswarden enforces a mandatory, fail-closed safety gate on every action:

1. **Prohibited** — `RestartService`/`StopService` without `allow_aggressive_actions`,
   `ApplySysctl` without `allow_sysctl_apply`, etc. → always blocked.
2. **Risk vs profile** — action risk must not exceed the profile's `max_allowed_risk`.
3. **Protected targets** — processes/services in `protected.*` → always blocked.
4. **Allowlist** — service resource-control changes → blocked unless the service
   is in `allowed.services`.
5. **Permission flags** — per-kind gates (`allow_aggressive_actions`, etc.) → blocked
   if the relevant flag is `false`.
6. **Non-root** — state-changing actions → blocked when not running as root.
7. **`dry_run`** — even if all gates pass, `dry_run = true` → simulate only.

**Fail-closed**: any error, unknown state, or missing capability → block and
degrade gracefully. syswarden never fails open.

### What syswarden will never do

- Kill or signal any process.
- Drop caches.
- Remove packages.
- Edit `/etc/fstab`, the bootloader, or `grub.cfg`.
- Apply irreversible tuning (every change must have a rollback path).
- Make outbound network connections.
- Phone home or collect telemetry.

---

## Rollback

Every real (non-simulated) action records a `RollbackEntry` in
`/var/lib/syswarden/rollback/rollback.jsonl` before making the change.

```sh
syswarden rollback list          # List entries (newest last)
syswarden rollback apply <id>    # Revert entry by id (v0.2+)
```

Revert is scaffolded in v0.1 and fully implemented in v0.2+ when real
execution paths exist.

---

## Logs and history

### Structured daemon log

```sh
journalctl -u syswarden -f              # Follow live
journalctl -u syswarden --since "1h ago"
```

### Audit log

Every decision, block, and action is written to
`/var/lib/syswarden/audit/audit-YYYY-MM-DD.jsonl`:

```sh
tail -f /var/lib/syswarden/audit/$(date +audit-%F.jsonl)
# or pretty-print:
jq . /var/lib/syswarden/audit/audit-$(date +%F).jsonl | less
```

### History

Tick summaries are written to `/var/lib/syswarden/history/history-YYYY-MM-DD.jsonl`.

---

## Systemd service

```sh
# Enable and start:
sudo systemctl enable --now syswarden

# Status:
sudo systemctl status syswarden

# Stop:
sudo systemctl stop syswarden

# Reload config (restart):
sudo systemctl restart syswarden
```

The service unit runs as root (required for cgroup writes) with extensive
hardening: `NoNewPrivileges`, `ProtectSystem=strict`, `PrivateTmp`,
`RestrictAddressFamilies=AF_UNIX`, `IPAddressDeny=any`, `MemoryDenyWriteExecute`.
See `packaging/systemd/syswarden.service` for the full unit.

---

## Troubleshooting

### `degraded: PSI unavailable`

Your kernel was built without `CONFIG_PSI`. On Arch, all official kernels since
5.2 include PSI. If you're on a custom kernel, add `CONFIG_PSI=y` and rebuild.
syswarden degrades gracefully — it continues with available metrics.

### Permission denied opening `/var/lib/syswarden/`

Run as root (via systemd) or change `history.dir`, `logging.audit_dir`, and
`rollback.dir` to a directory you own. The daemon never requires root for
read-only analysis — only for state-changing actions.

### `config validate` reports issues

Run `syswarden config validate` and fix each issue listed. The most common:

- `min_interval_secs` ≥ `max_interval_secs` — decrease min or increase max.
- Threshold out of `[0, 100]` range — correct the value.

### High daemon CPU usage

Unlikely, but if it happens: increase `idle_interval_secs` and
`min_interval_secs`. The daemon is designed to be lighter than the problems
it solves.
