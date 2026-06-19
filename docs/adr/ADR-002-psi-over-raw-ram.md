# ADR-002 — Primary signal: PSI over raw RAM

**Status:** Accepted

## Context

The most common mistake in "RAM optimizer" tools is treating high "used RAM"
as a problem. Linux uses free memory for the page cache; this dramatically
improves I/O performance and is entirely normal. Dropping caches produces
cosmetically lower "used RAM" while actually harming performance.

Options considered: raw `MemFree`/`MemUsed` thresholds, system load average,
kernel PSI (Pressure Stall Information).

## Decision

Use PSI as the primary pressure signal, cross-checked against `MemAvailable`.

## Rationale

- PSI measures actual CPU/memory/IO stall time as a percentage, not byte
  counts. It directly answers "are tasks waiting for resources?" rather than
  "how many bytes are allocated?".
- `MemAvailable` (not `MemFree`) correctly accounts for reclaimable cache;
  syswarden uses it as a cross-check to avoid false positives from healthy
  cache use.
- PSI has per-resource granularity (CPU, memory, IO) and two stall modes
  (`some`: at least one task stalled; `full`: all tasks stalled), enabling
  precise classification.
- Available on all Arch kernels since 5.2 (`CONFIG_PSI=y`).

## Consequences

- Requires `CONFIG_PSI=y`. syswarden degrades gracefully when absent.
- Adds a hard dependency on `/proc/pressure/{cpu,memory,io}` parsing.
- Load average and raw RAM thresholds are still collected as supplementary
  context but are not used as primary pressure drivers.
