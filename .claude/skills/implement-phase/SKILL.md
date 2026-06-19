---
name: implement-phase
description: Implement exactly one syswarden phase from planning.md §2 under the strict implementer contract. Use when the user asks to build/implement a numbered phase (e.g. "/implement-phase 2", "do phase 5").
disable-model-invocation: true
---

# Implement one phase

Implement **only** phase `$ARGUMENTS` from `planning.md` §2. Do not jump ahead or pull work from later phases.

## Steps

1. **Read both docs first.** Read `architecture.md` in full and the relevant `planning.md` sections (§1 contract, §2 order, §4 file checklist for this phase, §7 testing, the matching ready-to-paste prompt in §12).
2. **Plan, then wait.** Post a short plan: which files this phase creates/touches (per planning.md §4/§14) and the data contracts involved (§15). Wait for owner approval before writing code (owner prefers plan-before-coding).
3. **Implement the phase only**, honoring every invariant in `CLAUDE.md`:
   - Don't rename modules/files/structs/enums/CLI commands; don't change documented data-contract meanings.
   - No network / AI / paid APIs / telemetry. Dry-run default. Fail closed. Every action path calls `safety::evaluate`. No `unwrap`/`expect` on runtime paths.
   - Respect the forbidden-order rules in planning.md §15 (e.g. no real actions before safety + rollback are done and tested).
4. **Add the phase's tests** per planning.md §4/§7. Unit tests must be deterministic and fixture-driven (no host-state dependence). Dry-run tests assert zero side effects.
5. **Run the gate** (or invoke `/check`): `cargo fmt --all` → `cargo clippy --all-targets -- -D warnings` → `cargo test` → `cargo build --release`. Fix failures.
6. **Report changed files** and cite the architecture/planning sections that justify the implementation.

If anything in the phase conflicts with `architecture.md`, **stop and ask the owner** — never resolve by editing the architecture.
