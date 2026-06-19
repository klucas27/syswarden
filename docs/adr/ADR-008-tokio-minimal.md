# ADR-008 — Async runtime: tokio (minimal features)

**Status:** Accepted

## Context

The supervision loop needs:
- A timer (`tokio::time::sleep`) for adaptive sleep between ticks.
- Signal handling (`SIGTERM`, `SIGINT`) for clean shutdown.
- `select!` to race the sleep against signals.

The rest of the daemon is synchronous: `/proc` reads, PSI parsing, policy
decisions, and JSONL writes are all short, blocking operations.

Options: threads + `signal-hook` (no async), `async-std`, `tokio`.

## Decision

Use `tokio` with a minimal feature set: `rt`, `macros`, `time`, `signal`.
`current_thread` runtime (no thread pool).

```toml
tokio = { version = "1", default-features = false, features = ["rt", "macros", "time", "signal"] }
```

## Rationale

- `tokio::select!` on sleep + SIGTERM is cleaner than thread-based alternatives.
- `current_thread` runtime avoids spawning background threads for a daemon that
  does one thing per tick.
- Disabling default features (`rt-multi-thread`, `io`, `net`, `fs`, `process`,
  `sync`, `test-util`) keeps the dependency footprint small and avoids pulling
  in networking primitives that are prohibited by design (ADR-006).
- `async-std` has similar capability but smaller ecosystem; `tokio` is better
  supported by `zbus` (which uses it internally).

## Consequences

- `async fn` is limited to the daemon loop entrypoint and signal handling.
  All analysis, policy, and I/O functions are synchronous (`fn`, not `async fn`).
- Blocking operations (file reads) inside an `async fn` are fine on
  `current_thread` since there is no thread pool to starve.
- The `time` feature is the only async I/O used; adding real async I/O later
  would require enabling additional features explicitly.
