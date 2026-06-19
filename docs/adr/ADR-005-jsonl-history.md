# ADR-005 — History backend: JSONL for v0.1

**Status:** Accepted

## Context

syswarden needs to persist: (a) per-tick pressure and action summaries
(`HistoryRecord`), (b) audit events (`AuditEvent`), and (c) rollback
metadata (`RollbackEntry`). These records need to be queryable for trend
analysis and `report` commands, and robust to partial writes or corruption
(the daemon may be killed mid-tick).

Options: SQLite, sled (embedded key-value store), JSONL (newline-delimited JSON).

## Decision

Use JSONL (append-only, one record per line) for all three stores in v0.1.

History uses one file per calendar day (`history-YYYY-MM-DD.jsonl`); audit
uses the same pattern; rollback uses a single rolling file. Files are pruned
on startup by the store's `open()` method.

## Rationale

- No native dependencies — `serde_json` is already required and writes JSONL
  with a single `to_string` + `writeln`.
- Human-readable — `tail -f audit-2026-06-19.jsonl | jq .` works without
  special tooling.
- Append-only writes are crash-safe; a truncated last line is discarded on
  the next `open()` without corrupting earlier records.
- Per-line parsing means one corrupt entry (e.g. from a `SIGKILL`) does not
  invalidate the rest of the file — corrupt lines are skipped with a `warn!`.

## Consequences

- Limited query power (no SQL, no indexing). Acceptable for v0.1 where
  history is used only for trend seeding and the `report` command.
- If query complexity grows (time-range filters, aggregations), migrate to
  SQLite in a future version without changing the `HistoryStore` public API.
- File-per-day rotation keeps individual files manageable; `max_file_mb`
  provides a secondary cap.
