# ADR-009 — Persistent drop-ins and aggressive-action gating

**Status:** Accepted  
**Date:** 2026-06-20  
**Deciders:** Kresley (owner/architect)

---

## Context

v0.3 adds three categories of actions that go beyond transient cgroup tweaks:

1. **Persistent systemd drop-ins** — write `[Service]` overrides to
   `/etc/systemd/system/<unit>.d/50-syswarden.conf` and issue `daemon-reload`.
   These survive a reboot, unlike `SetUnitProperties(runtime=true)`.

2. **Aggressive resource limits** — `MemoryMax` (hard cgroup cap), service
   restart, and service stop. These can make a service unavailable or be hard
   to diagnose without looking at audit logs.

3. **sysctl apply** — writes `/proc/sys/<key>` directly. Kernel tunables affect
   the entire system, not just one service.

All three require a deliberate opt-in model that preserves the core invariant:
**the default config makes zero system changes**.

---

## Decision

### Layered gating (§18 of architecture.md)

Every aggressive action requires **all** of the following to be true
simultaneously:

| Gate | Why |
|---|---|
| `dry_run = false` | Master switch; default is `true` |
| `allow_aggressive_actions = true` | Explicit acknowledgement |
| Profile `max_allowed_risk = Aggressive` | `conservative`/`balanced` never reach this |
| Target in `allowed.services` | No implicit targeting |
| Target not in `protected.services` | Defence-in-depth re-check at execute time |

`ApplySysctl` adds a fifth gate: `allow_sysctl_apply = true`.

### MemoryMax escalation gate

`SetMemoryMax` is blocked unless `memory.high` is already set on the target
cgroup at the moment of dispatch. This prevents jumping straight to a hard cap
without a prior soft cap — a common cause of OOM kills that are difficult to
attribute. The check reads the live cgroup file, not the rollback log.

### Persistent drop-in backup policy

Before writing a drop-in:

1. Read the current file content (or record absence).
2. Store `{path, prior_content, written_content}` in `RollbackEntry.prior_state`.
3. Write the new file.
4. Issue `daemon-reload` via D-Bus.

On rollback:

1. Re-read the current file; compare to `written_content`.
   If they differ → **refuse** (external modification detected).
2. If `prior_content` is `None` → remove the file.
3. If `prior_content` is `Some(original)` → restore it.
4. Issue `daemon-reload`.

The integrity check (step 1) prevents silently overwriting an admin's
manual edits with a stale backup.

### Rollback prior-state tagging

All service-property rollback entries carry a `"backend"` discriminant:

- `"backend": "transient"` — revert via `SetUnitProperties(runtime=true)`
- `"backend": "persistent"` — revert via drop-in file restore + `daemon-reload`

Entries without a `backend` tag (written before v0.3) are treated as
`"transient"` for backward compatibility.

### sysctl backup policy

Before writing a key:

1. Read the current value.
2. Store `{key, prior_value, applied_value}` in `RollbackEntry.prior_state`.
3. Write `value\n` to `/proc/sys/<key>` via `std::fs::write` (no shell).
4. Re-read and verify the kernel accepted the value; fail if not.

On rollback: call `sysctl::apply(key, prior_value)` — same write path with
its own verify step.

---

## Alternatives considered

### Single `allow_aggressive_actions` flag (no profile gate)

Rejected: profile gating ensures that even a misconfigured config file cannot
enable aggressive actions under a conservative profile. The two-layer check
(flag + profile) is redundant by design.

### Shell-based `sysctl -w`

Rejected unconditionally: architecture.md §17 prohibits shell invocation
and external binaries on hot paths. Direct `/proc/sys` writes are simpler,
auditable, and avoid injection vectors.

### Write drop-ins only — no transient path

Rejected: transient runtime changes are preferable for ephemeral tuning
(auto-cleared on reboot). Persistent drop-ins are opt-in and reserved for
cases where the operator explicitly wants the change to survive reboots
(`params["persistent"] = "true"`).

---

## Consequences

- Default config is unchanged: `dry_run = true`, all aggressive flags `false`.
- Aggressive actions are fully audited in the JSONL rollback log.
- Every aggressive write is reversible via `syswarden rollback apply <id>`,
  except `RestartService` (the service is running either way).
- The `ProtectKernelTunables=yes` unit hardening must be relaxed to
  `ProtectKernelTunables=no` when `allow_sysctl_apply = true` (noted in the
  systemd unit template; operator responsibility to apply).
