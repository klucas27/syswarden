# ADR-004 — Default mode: dry-run, observe-only

**Status:** Accepted

## Context

A system daemon that changes resource limits is inherently risky. A bug in
policy logic, an overly aggressive threshold, or a misconfigured allowlist
could degrade a service or make a system unresponsive. Users need confidence
that installing syswarden cannot cause harm before they have reviewed its
behavior on their specific system.

## Decision

Ship with `dry_run = true` as the baked-in default. All actions are simulated;
no system state is changed. Users must explicitly set `dry_run = false` to
enable real actions.

The `dry_run` flag is the master switch and is checked as gate 7 in the
mandatory safety layer (`safety::evaluate`), after all other gates. Even
if every other gate passes, `dry_run = true` → `RequireDryRun` → simulate.

## Rationale

- The cost of a user having to set `dry_run = false` after reviewing audit
  logs is trivial. The cost of an accidental destructive action on a production
  system is not.
- Users can run the daemon for days in dry-run mode, inspecting `analyze`,
  `actions dry-run`, and the audit log before committing to real changes.
- This default is opt-out, not opt-in. It cannot be accidentally circumvented
  by a missing config file — the built-in defaults set `dry_run = true`.

## Consequences

- v0.1 is entirely observe-and-recommend. Real actions land in v0.2+ after
  safety (Phase 12) and rollback (Phase 16) are complete and tested.
- `dry_run = true` must be preserved in all example configs; removing it from
  an example is considered a documentation bug.
