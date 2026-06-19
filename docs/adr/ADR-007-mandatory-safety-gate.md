# ADR-007 — Mandatory safety gate for every action

**Status:** Accepted

## Context

Multiple modules (policy engine, action planner, zram, systemd) can produce
intended changes. Without a single enforcement point, safety rules would need
to be duplicated in every caller. Duplication leads to divergence — a safety
check present in one path but missing in another is exactly the kind of bug
that causes accidental destructive actions.

## Decision

Every action — without exception — must pass through `safety::evaluate`
before execution. The function is fail-closed: any unknown state, missing
capability, or error returns `Block`, never `Allow`.

Gate order (first failure wins):
1. `ActionRisk::Prohibited` → Block unconditionally.
2. Action risk > profile `max_allowed_risk` → Block.
3. Target is in `protected.processes` or `protected.services` → Block.
4. Service resource-control action targets a service not in `allowed.services` → Block.
5. Required per-kind permission flag not set → Block.
6. State-changing action + non-root → Block.
7. State-changing action + `dry_run = true` → `RequireDryRun`.
8. All gates passed → `Allow`.

## Rationale

- Single enforcement point: every new action kind is safe by default (blocked
  at gate 1 if Prohibited, at gate 2 if risk exceeds profile).
- Composable: callers do not need to know the full rule set; they call
  `safety::evaluate` and branch on the result.
- Testable: the gate is a pure function. All 8 gates have unit tests in
  `src/safety/mod.rs` and integration tests in `tests/safety_tests.rs`.
- Exhaustive enums: `SafetyDecision`, `ActionRisk`, `ActionKind` are closed
  enums matched exhaustively; adding a new variant forces the compiler to
  revisit every `match`.

## Consequences

- Any code that bypasses `safety::evaluate` is a bug. Code review enforces this.
- The gate adds one function call per action per tick. The cost is negligible
  compared to `/proc` parsing.
- `actions::simulate` is the only v0.1 executor; it always calls `safety::evaluate`
  first. The v0.2 `execute` path will do the same.
