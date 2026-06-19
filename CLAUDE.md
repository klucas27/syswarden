# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`syswarden` is a local, offline, low-overhead Rust system-supervision daemon for Arch Linux. It reads kernel pressure signals (PSI) and applies **safe, reversible, explainable** resource governance via systemd / cgroups v2. Project is pre-code: implementation is driven entirely by two docs.

## Authoritative docs — read before coding

- `@architecture.md` — **immutable source of truth.** Do not redesign, rename, simplify, or reinterpret it. Names, modules, data contracts (§15), and safety gates (§17) are fixed.
- `@planning.md` — phased implementation plan and the implementer contract (§1).

If a request conflicts with `architecture.md`, **stop and ask the owner.** Never resolve a conflict by changing the architecture.

## Implementer contract (planning.md §1)

- Implement **strictly one phase at a time, in order** (planning.md §2). Don't jump ahead. Real state-changing execution is **not** part of v0.1 — only after safety (Phase 12) and rollback (Phase 16) exist and pass tests.
- Don't rename modules, files, structs, enums, or CLI commands. Don't alter the meaning of documented data contracts (add private fields only if needed).
- Report the files you changed at the end of each phase.

## Hard invariants (never violate)

- **No network.** No sockets, no HTTP, no outbound calls, no networking crates — ever.
- **No AI runtime, no paid APIs, no telemetry.**
- **Dry-run is the default** and the master switch. Default config must make zero system changes.
- **Fail closed.** On any error/uncertainty/invalid config, block actions and degrade — never fail open.
- **Every action goes through `safety::evaluate`** before execution.
- **No destructive defaults**: never kill processes, drop caches, remove packages, edit `fstab`/bootloader, or do irreversible tuning.
- **No shell interpolation.** External tools (fallback only) are invoked with explicit arg vectors, never a shell, never with config-derived strings as code.
- **No `unwrap`/`expect` on runtime paths.** Libraries return typed errors (`thiserror`); the daemon adds context (`anyhow`) and recovers.

## Dev gate

Run in this order; each must pass:

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

`/check` runs this gate. `/implement-phase <n>` implements a single planning.md phase under the contract above.

## Testing notes (planning.md §7)

- Unit tests must be deterministic and not depend on host state — use fixtures in `examples/fixtures/`. Construct `MetricsSnapshot` directly to drive pressure/policy tests.
- Dry-run tests must assert **zero** side effects (no external calls via mocks).
- Do not modify `/etc` in tests; use temp dirs.
- Live-system behavior goes only in clearly-marked, `#[ignore]`-able integration tests.

## Conventions

- Commits: small, ordered, single-purpose, conventional style (`feat(config): ...`, `feat(safety): ...`). Each commit must compile and keep tests green. See planning.md §11.
- Keep `tokio` features minimal (`rt`, `macros`, `time`, `signal`). Use `chrono` (not `time`). v0.1 history/audit/rollback use JSONL via `serde_json` — SQLite/sled are deferred.

## Local tooling

`.brain/` and `brain.config.json` are a local semantic-index tool ("Brain Engine"); `.brain/` is gitignored. Not part of the daemon.
