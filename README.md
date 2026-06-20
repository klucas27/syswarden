# syswarden

A local, low-overhead Rust system supervision daemon for Arch Linux.

syswarden reads kernel PSI (Pressure Stall Information) signals and classifies
your system into a well-defined pressure state. From that state it can observe
and explain what is happening, recommend actions, or — when explicitly permitted
— apply conservative, reversible resource governance through systemd/cgroups v2.

**By default it does nothing.** `dry_run = true` is baked into every default
config. Zero system changes happen unless you opt in, step by step.

---

## Why

Most "RAM optimizer" or "system cleaner" tools on Linux misread memory state.
They see high "used RAM" and drop caches, kill processes, or apply irreversible
tuning — producing cosmetic "free RAM" while actually harming responsiveness.

syswarden uses the kernel's own PSI signals (which measure real stall time, not
byte counts), cross-checks them against `MemAvailable`, and only acts when there
is genuine pressure. It never mistakes a warm page cache for a problem.

---

## Features

- **PSI-based pressure classification** — `None → Low → Moderate → High → Critical`, with hysteresis.
- **Safe, reversible actions** — resource governance via systemd drop-ins; rollback metadata captured for every change.
- **Mandatory safety gate** — every action passes through a fail-closed gate; prohibited and protected targets can never be touched.
- **Profiles** — `conservative` (default), `balanced`, `performance`; each controls thresholds, risk tolerance, and permitted actions.
- **Dry-run default** — the daemon observes and logs; you read the audit log and decide whether to enable actions.
- **No network, no telemetry, no AI runtime** — fully offline and auditable.

---

## Requirements

- Arch Linux (or any systemd + cgroups v2 distro)
- Kernel with `CONFIG_PSI=y` (all Arch kernels since 5.2)
- Rust 1.75+ (build only)

---

## Installation

### AUR (recommended)

```sh
paru -S syswarden
# or
yay -S syswarden
```

### Manual build

```sh
git clone https://github.com/kresley/syswarden
cd syswarden
cargo build --release
sudo install -Dm755 target/release/syswarden /usr/bin/syswarden
sudo install -Dm644 packaging/systemd/syswarden.service \
    /etc/systemd/system/syswarden.service
```

### One-command profile activation

`scripts/profiles/activate-<profile>.sh` does the whole lifecycle in one go:
installs Rust if needed, builds + installs the binary and unit, writes
`/etc/syswarden/config.toml` for the profile, enables the service **at boot**,
and runs a first cleanup.

```sh
git clone https://github.com/kresley/syswarden
cd syswarden
sudo ./scripts/profiles/activate-balanced.sh
# profiles: conservative | balanced | performance | low_ram | desktop | server | developer
```

> **First cleanup** (every profile except `conservative`): after the service is
> up, the script offers a one-time, **interactively confirmed** reclaim — drop
> caches + graceful `SIGTERM` to heavy *user* processes. It is safe by
> construction and **never** touches the kernel, system/root processes
> (`uid < 1000`), `syswarden`, `sshd`, or your login/desktop session, so the
> system stays up (architecture.md §17.1). It is skipped on non-interactive
> shells unless `SYSWARDEN_ASSUME_YES=1`; tune the process threshold with
> `SYSWARDEN_CLEAN_RSS_MB` (default 300). The daemon itself never kills or drops
> caches — this is operator tooling only.

---

## Quick start

```sh
# 1. Install the binary (see above).

# 2. Copy an example config (optional — defaults are safe without a config file).
sudo mkdir -p /etc/syswarden
sudo cp examples/config.balanced.toml /etc/syswarden/config.toml

# 3. Validate the config.
syswarden config validate

# 4. Run in the foreground to observe (dry-run, no system changes).
syswarden daemon

# 5. Check what syswarden would do right now.
syswarden analyze
syswarden actions dry-run

# 6. Enable and start the systemd service (still dry-run by default).
sudo systemctl enable --now syswarden
journalctl -u syswarden -f
```

---

## Configuration

The config file lives at `/etc/syswarden/config.toml`. If the file is missing,
conservative built-in defaults are used (`dry_run = true`, no actions permitted).

See `examples/config.balanced.toml` and `examples/config.low_ram.toml` for
annotated starting points. Run `syswarden config show` to inspect the active
effective config and `syswarden config validate` to check for issues.

### Key settings

| Setting | Default | Meaning |
|---|---|---|
| `global.dry_run` | `true` | Master switch. `false` enables real actions. |
| `global.profile` | `"conservative"` | Risk tolerance and thresholds. |
| `global.allow_aggressive_actions` | `false` | Gate for `SetMemoryMax`, `RestartService`, etc. |
| `global.allow_zram_apply` | `false` | Gate for zram configuration. |
| `global.allow_sysctl_apply` | `false` | Gate for sysctl writes (with backup). |
| `protected.processes` | `["systemd", "syswarden", ...]` | Never touched, regardless of profile. |
| `protected.services` | `["syswarden.service", ...]` | Never touched. |
| `allowed.services` | `[]` | Only services listed here may be governed. |

---

## Profiles

| Profile | Risk tolerance | Typical use |
|---|---|---|
| `conservative` | Low — recommend only (`dry_run`) | Default; safe for any system |
| `balanced` | Moderate — `CPUWeight`, `IOWeight`, `MemoryHigh`, `nice`/`ionice` | Desktop / workstation |
| `desktop` | Moderate — `nice`, `CPUWeight`, `MemoryHigh` | Interactive desktop |
| `developer` | Moderate — adds `ionice`, `IOWeight` | Dev box / heavy I/O |
| `server` | Moderate — `CPUWeight`, `IOWeight`, `MemoryHigh` | Headless servers |
| `low_ram` | Moderate — `nice`/`ionice`, `MemoryHigh` (+ zram in v0.4) | Low-memory machines |
| `performance` | Aggressive — all moderate + `MemoryMax`, restart | Servers (read docs first) |

Each profile has a one-command installer under `scripts/profiles/` — see
[One-command profile activation](#one-command-profile-activation).

---

## Commands

```
syswarden daemon             # Run foreground supervision loop
syswarden analyze            # One-shot pressure + process + service analysis
syswarden status             # Quick system health summary
syswarden pressure           # Show current PSI readings
syswarden actions dry-run    # Show what would be done right now
syswarden config show        # Print effective config
syswarden config validate    # Validate config, print issues
syswarden rollback list      # List rollback entries
syswarden rollback apply <id>  # Revert a recorded action (v0.2+)
syswarden doctor             # Self-check: capabilities, config, permissions
syswarden version            # Print version
```

---

## Safety

- **Fail-closed**: any error, uncertainty, or invalid config → block all actions and degrade gracefully.
- **Protected lists**: `protected.processes` and `protected.services` are absolute — no flag or profile can override them. `syswarden` itself is always protected.
- **Allowlist-only**: service resource-control changes only apply to services explicitly listed in `allowed.services`. Empty list = nothing modifiable (default).
- **No destructive defaults**: syswarden never kills processes, drops caches, removes packages, edits `/etc/fstab`, or touches the bootloader.
- **Reversible**: every executed action records prior state in the rollback store. Real revert lands in v0.2+.
- **Audit log**: every decision, block, and simulated action is written to a JSONL audit log at `/var/lib/syswarden/audit/`.

See `docs/usage.md` for the full safety model and opt-in guide.

---

## Architecture

See `architecture.md` for the complete design — module structure, data contracts,
safety gates, PSI cross-check logic, and all ADRs.

---

## License

MIT — see `LICENSE`.
