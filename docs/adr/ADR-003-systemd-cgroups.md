# ADR-003 — Resource governance via systemd/cgroups v2

**Status:** Accepted

## Context

When syswarden decides to apply a resource limit (e.g. `MemoryHigh` on a
service that is swamping memory), it needs a mechanism that is:
- Reversible (prior state can be captured and restored).
- Non-destructive (does not kill the process).
- Integrated with the OS service manager.
- Auditable.

Options: direct cgroup writes via `/sys/fs/cgroup/`, systemd D-Bus API (drop-ins
or transient properties), shell out to `systemctl set-property`.

## Decision

Govern resources through systemd's resource-control API (D-Bus `SetUnitProperties`
for transient changes, drop-ins for persistent changes). Read cgroup state
directly; never write to cgroup files directly.

## Rationale

- systemd is the standard service manager on Arch; its cgroup integration is
  stable, documented, and supported.
- `SetUnitProperties` / drop-ins are transient or persistent, reversible
  (drop the drop-in → `daemon-reload`), and survive process restarts.
- Prior state (existing `CPUWeight`, `MemoryHigh`, etc.) can be read before
  writing, enabling accurate rollback metadata.
- Direct cgroup writes bypass systemd's bookkeeping and can be clobbered by
  a `daemon-reload`. Writing through systemd avoids this.

## Consequences

- Requires systemd; degrades gracefully (observe-only) on non-systemd systems.
- D-Bus adds a dependency on `zbus`; a `systemctl` shell-out is the audited
  fallback when D-Bus is unavailable.
- Resource control changes only apply to services in `allowed.services`;
  empty allowlist = nothing modifiable (default).
