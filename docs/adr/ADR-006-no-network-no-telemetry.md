# ADR-006 — No network, no external AI, no telemetry

**Status:** Accepted

## Context

A system daemon has privileged access to process lists, resource usage,
service configuration, and system state. Any outbound network call from such
a daemon is a privacy and security concern. Users should be able to audit
every decision the daemon makes without trusting an external service.

Additionally, "AI-driven optimization" tools that call external APIs introduce
non-determinism, latency, cost, and a dependency on external service
availability — all unacceptable for a daemon whose job is to improve system
reliability.

## Decision

syswarden has no network capability and will never have any:

- No sockets of any kind except `AF_UNIX` (D-Bus only).
- No HTTP client, no DNS resolution.
- No calls to external APIs, paid or free.
- No telemetry, no crash reporting, no usage metrics.
- No background AI model.

This is enforced at the systemd level (`RestrictAddressFamilies=AF_UNIX`,
`IPAddressDeny=any`) and in the codebase (no networking crates, enforced by
code review and architecture invariants).

## Rationale

- Privacy: the daemon sees your process list and service state. That data
  stays on your machine.
- Determinism: decisions are reproducible from config + PSI readings. There
  is no opaque external model to blame.
- Reliability: no dependency on external service uptime.
- Auditability: every decision can be traced to a config value, a PSI reading,
  or a policy table in the source code.

## Consequences

- All intelligence is local and algorithmic — policy tables, thresholds,
  hysteresis. This is a feature, not a limitation.
- No remote config push, no automatic updates to thresholds.
- Users who want "smarter" decisions must adjust their config or profiles.
