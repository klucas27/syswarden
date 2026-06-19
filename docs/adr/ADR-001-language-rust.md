# ADR-001 — Language: Rust

**Status:** Accepted

## Context

syswarden needs to run as a continuously-present daemon with sub-millisecond
tick overhead, parse `/proc` and `/sys` files in tight loops, and enforce
memory-safety invariants (no buffer overflows, no use-after-free in the
pressure parsing or audit paths). It must never become the thing it is trying
to prevent.

Options considered: Rust, Go, C, Python.

## Decision

Implement in Rust.

## Rationale

- No GC pauses — predictable latency in the supervision loop.
- Memory safety by default — no dangling pointers in PSI or config parsing.
- `thiserror` + `anyhow` give typed errors on library paths and rich context
  on the daemon path without boilerplate.
- `tokio` (minimal feature set) provides signals and timers without a full
  async framework.
- Strong type system encodes safety invariants at compile time (e.g.
  `ActionRisk::Prohibited` is a closed enum, not a string).

## Consequences

- Longer compile times than Go or Python; mitigated by incremental builds.
- Steeper learning curve for contributors unfamiliar with Rust.
- No runtime reflection — exhaustive `match` is enforced by the compiler,
  which is the desired property for safety-critical code.
