---
name: check
description: Run syswarden's full static dev gate (cargo fmt, clippy -D warnings, test, build --release) in order, stopping at the first failure. Use before committing or to verify a phase compiles cleanly.
---

# Static dev gate

Run these in order. Stop at the first failure, report it, and fix before continuing.

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

Notes:
- `clippy` denies all warnings (`-D warnings`) — treat any warning as a failure.
- If `Cargo.toml` doesn't exist yet (pre-Phase-1), say so and stop; there's nothing to check.
- Tests must stay deterministic and fixture-driven; don't make them depend on host PSI/systemd state (`planning.md` §7).
