# syswarden — Implementation Plan

> **Status:** Authoritative implementation contract for future models (Claude Sonnet, GPT-5.5) and human contributors.
> **Companion:** `architecture.md` (the immutable architecture source of truth).
> **Form:** Imperative. Do exactly what this document says, in order, without changing the architecture.

---

## 1. Implementer Contract

You must:

- **Read `architecture.md` first**, in full, before writing any code.
- **Do not change the architecture.** Names, modules, data contracts, and gates are fixed.
- **Do not rename modules**, files, structs, enums, or CLI commands.
- **Do not change public data contracts** (§15 of `architecture.md`). Add private fields only if needed, never alter documented meanings.
- **Do not add unsafe behavior.** No new destructive or networked capability.
- **Do not skip safety.** Every action goes through the safety layer.
- **Do not add network access.** No sockets, no HTTP, no outbound connections, no networking crates.
- **Do not add paid APIs** or any external account dependency.
- **Do not implement an AI runtime** or background model.
- **Do not implement aggressive actions before safety and rollback exist and pass tests.**
- **Implement only the assigned phase.** Do not jump ahead.
- **Report files changed** at the end of each phase.
- **Run checks when possible:** `cargo fmt`, `cargo clippy`, `cargo test`, `cargo build`.

If a requirement seems to conflict with `architecture.md`, stop and ask the human owner. Never resolve a conflict by changing the architecture yourself.

---

## 2. Implementation Order

Implement strictly in this order:

- ✅ **Phase 0 — Repository skeleton:** directory tree, empty module files with `//!` docs, placeholder `main.rs`.
- ✅ **Phase 1 — Cargo setup:** `Cargo.toml`, dependencies (only those marked Required for the current phase), workspace settings, lints.
- ✅ **Phase 2 — Config model:** `config`, data contracts for config, TOML parse + validation, defaults.
- ✅ **Phase 3 — CLI:** `clap` command tree, dispatch stubs, global flags, exit codes.
- ✅ **Phase 4 — Logging:** `tracing` setup, `AuditEvent`, audit JSONL writer.
- ✅ **Phase 5 — Metrics collection:** `metrics` (`memory.rs`, `cpu.rs`, `io.rs`), `MetricsSnapshot`.
- ✅ **Phase 6 — PSI parsing:** `pressure` PSI parser, `PsiMetrics`.
- ✅ **Phase 7 — Process analysis:** `processes`, `ProcessInfo`, flagging.
- ✅ **Phase 8 — Service analysis:** `services`, `ServiceInfo` (read-only systemd).
- ✅ **Phase 9 — Pressure model:** classification → `PressureSnapshot`, `PressureLevel`, system-state classification.
- ✅ **Phase 10 — Profiles:** built-in `ProfileConfig`s, resolution.
- ✅ **Phase 11 — Policy engine:** `policy`, decision tables → `PolicyDecision`.
- ✅ **Phase 12 — Safety layer:** `safety`, gates, `SafetyDecision` (fail-closed).
- ✅ **Phase 13 — Dry-run actions:** `actions` planner + simulator, `PlannedAction`/`ActionResult`. **No real execution yet.**
- ✅ **Phase 14 — Daemon loop:** `daemon`, full loop with dry-run only, signals, adaptive sleep.
- ✅ **Phase 15 — Local history:** `history` JSONL, `HistoryRecord`, retention.
- ✅ **Phase 16 — Rollback:** `rollback` metadata capture/list (revert scaffolding; real revert lands with real actions in later versions).
- ✅ **Phase 17 — systemd integration:** `packaging/systemd/syswarden.service`, hardened, manual install docs.
- ✅ **Phase 18 — Tests:** complete the test suite per §7.
- ✅ **Phase 19 — Packaging:** AUR `PKGBUILD`, examples.
- ✅ **Phase 20 — Documentation:** `README.md`, `docs/usage.md`, finalize ADRs.

**Real state-changing action execution is NOT part of v0.1.** It begins in v0.2 only after Phases 12 (safety) and 16 (rollback) are complete and tested.

---

## 3. Milestones

- **M0:** Documentation complete (`architecture.md`, `planning.md`). *(This milestone is already met.)*
- **M1:** Repository skeleton compiles (`cargo build` succeeds with stubs).
- **M2:** CLI works (subcommands parse, `--help`, `version`).
- **M3:** Config loads and validates (`config show`, `config validate`).
- **M4:** Metrics snapshot works (`status` shows real memory/CPU/IO).
- **M5:** PSI parser works (`pressure` shows real PSI).
- **M6:** Pressure classification works (`analyze` shows `PressureLevel` + contributors).
- **M7:** Policy decisions work (`actions dry-run` shows intended decisions).
- **M8:** Safety layer blocks unsafe actions (tests prove prohibited/protected blocking).
- **M9:** Dry-run action planner works (plans produced, nothing executed).
- **M10:** Daemon loop runs safely (foreground, dry-run, clean shutdown).
- **M11:** History and audit logs work (records appended, `report` works).
- **M12:** Rollback metadata works (entries captured/listed).
- **M13:** systemd service installed manually (runs under systemd, journald logs).
- **M14:** v0.1 release candidate (all above + tests green + docs).

---

## 4. File-by-File Implementation Checklist

For each file: **purpose / required structs / required enums / required functions / dependencies / tests / acceptance criteria.**

### `Cargo.toml`
- Purpose: crate metadata, deps, lints.
- Deps: add per-phase (see §16 of architecture). Lints: `unsafe_code = "warn"` (allow only in `actions` with justification), clippy pedantic where reasonable.
- Acceptance: builds; no unused deps.

### `src/main.rs`
- Purpose: entry point; build config, dispatch CLI or daemon.
- Functions: `fn main() -> ExitCode`.
- Deps: cli, config, logging, anyhow.
- Tests: none (thin).
- Acceptance: returns correct exit codes.

### `src/error.rs`
- Purpose: typed error enums (`thiserror`) shared across modules.
- Enums: `SyswardenError` with variants for Config, Io, Parse, Permission, Systemd, Capability, Action, Rollback.
- Acceptance: all modules use these; no stringly-typed errors on library paths.

### `src/cli.rs`
- Purpose: define `clap` command tree (§13) and dispatch.
- Structs/enums: `Cli`, `Command` enum mirroring every subcommand; global flags.
- Functions: `parse()`, `dispatch(cli, config)`.
- Tests: parse each subcommand; exit-code mapping; `--json` flag presence.
- Acceptance: every §13 command parses and routes; `--help` lists all.

### `src/daemon.rs`
- Purpose: main supervision loop (§6 pseudocode).
- Functions: `run(config)`, `single_tick(...)` (for testing), `adaptive_interval(...)`, signal install.
- Deps: every analysis/decision/action module, tokio, tracing.
- Tests: single-tick determinism with mocked collectors; shutdown signal ends loop.
- Acceptance: runs in foreground dry-run, logs each tick, exits 0 on SIGTERM.

### `src/config/mod.rs`
- Purpose: load/merge/validate config; expose `AppConfig`.
- Structs: `AppConfig`, `GlobalConfig`, `PollingConfig`, `PressureThresholds`, `ProtectedSets`, `AllowedSets`, `ProcessRule`, `ServiceRule`, `HistoryConfig`, `LoggingConfig`, `RollbackConfig`.
- Enums: `ProfileName`.
- Functions: `load(path) -> Result<AppConfig>`, `defaults() -> AppConfig`, `validate(&AppConfig) -> Vec<ConfigIssue>`.
- Deps: serde, toml, profiles.
- Tests: parse valid/invalid TOML; default fallback when file missing; validation catches bad thresholds, unknown profile, prohibited requests.
- Acceptance: missing file → conservative defaults with `dry_run = true`; `config validate` returns all issues.

### `src/metrics/mod.rs` + `memory.rs` + `cpu.rs` + `io.rs`
- Purpose: collect `MetricsSnapshot`.
- Structs: `MetricsSnapshot`, `MemoryMetrics`, `CpuMetrics`, `IoMetrics`.
- Functions: `collect(&Capabilities, &mut CpuSample) -> Result<MetricsSnapshot>`; per-file parsers.
- Deps: procfs (+ raw fallback).
- Tests: parse fixture files; CPU utilization from two samples; `MemAvailable` used (not `MemFree`).
- Acceptance: real values on a live system; robust to missing optional fields.

### `src/pressure/mod.rs`
- Purpose: parse PSI; classify `PressureLevel`; system state.
- Structs: `PsiMetrics`, `PressureSnapshot`.
- Enums: `PressureLevel`, `SystemState`.
- Functions: `parse_psi(path) -> Result<PsiMetrics>`, `compute(&Capabilities, &MetricsSnapshot, trend) -> PressureSnapshot`, `classify_state(...) -> SystemState`.
- Tests: PSI fixture parsing; threshold→level mapping; PSI-absent degradation → `degraded`; hysteresis.
- Acceptance: correct level for known inputs; never raises memory pressure on healthy cache use.

### `src/processes/mod.rs`
- Purpose: enumerate + flag processes.
- Structs: `ProcessInfo`; enum `ProcessFlag`.
- Functions: `analyze(&AppConfig, &ProfileConfig) -> Vec<ProcessInfo>`.
- Deps: procfs, config, safety (protected check).
- Tests: fixture `/proc` tree; protected exclusion; rule thresholds.
- Acceptance: flags only; never signals/changes a process.

### `src/services/mod.rs`
- Purpose: read + flag services.
- Structs: `ServiceInfo`; enum `ServiceFlag`.
- Functions: `analyze(&Capabilities, &AppConfig, &ProfileConfig) -> Vec<ServiceInfo>`.
- Deps: systemd (read), zbus, config.
- Tests: mocked systemd responses; allow/protected handling; no-systemd degradation.
- Acceptance: read-only; flags only.

### `src/systemd/mod.rs`
- Purpose: read unit props; render drop-ins; (later) apply via D-Bus with rollback capture.
- Functions (v0.1): `read_unit(unit)`, `render_dropin(action) -> String` (preview only).
- Deps: zbus.
- Tests: drop-in rendering; mocked reads.
- Acceptance: v0.1 performs no writes; render + read only.

### `src/cgroups/mod.rs`
- Purpose: detect v2; read cgroup usage/limits.
- Functions: `detect() -> CgroupMode`, `read(unit_path)`.
- Tests: fixture cgroup tree; v2 detection.
- Acceptance: read-only; never writes (writes go through systemd).

### `src/zram/mod.rs`
- Purpose: detect zram/zswap/swap; recommend; (later) gated apply.
- Functions: `detect() -> ZramReport`, `recommend(&MetricsSnapshot, &ProfileConfig) -> ZramRecommendation`; `apply(...)` exists but returns `Blocked` unless all gates pass (v0.2+).
- Tests: fixture sysfs parsing; recommendation math; conflict detection.
- Acceptance: v0.1 detection + recommend only; apply blocked without flags.

### `src/policy/mod.rs`
- Purpose: pure decision function.
- Structs: `PolicyDecision`; enum `DecisionIntent`.
- Functions: `decide(&SystemState, &ProfileConfig, &[ProcessInfo], &[ServiceInfo]) -> PolicyDecision`.
- Tests: decision-table coverage per state/profile.
- Acceptance: total, deterministic, no side effects.

### `src/actions/mod.rs`
- Purpose: plan + simulate (v0.1) / execute (v0.2+) actions.
- Structs: `PlannedAction`, `ActionResult`.
- Enums: `ActionKind`, `ActionRisk`, `ActionStatus`.
- Functions: `plan(&PolicyDecision, &ProfileConfig) -> Vec<PlannedAction>`, `simulate(&PlannedAction) -> ActionResult`, `execute(...)` (v0.2+, gated, always after `safety::evaluate`).
- Deps: safety (mandatory), systemd, cgroups, zram, nix.
- Tests: plan per decision; simulate produces no side effects.
- Acceptance: every executable path calls `safety::evaluate` first; v0.1 only simulates.

### `src/safety/mod.rs`
- Purpose: mandatory fail-closed gate.
- Structs/enums: `SafetyDecision`.
- Functions: `evaluate(&PlannedAction, &AppConfig, &ProfileConfig, &Capabilities) -> SafetyDecision`.
- Tests: prohibited → block; protected proc/service → block; missing flag → block; non-root → block state changes; dry_run → RequireDryRun.
- Acceptance: defaults to block on any uncertainty; prohibited actions never allowed.

### `src/profiles/mod.rs`
- Purpose: built-in profiles + resolution.
- Structs: `ProfileConfig`.
- Functions: `resolve(ProfileName, &AppConfig) -> ProfileConfig`, `all() -> [...]`.
- Tests: each built-in resolves; override merge; unknown → error.
- Acceptance: matches §11 table.

### `src/history/mod.rs`
- Purpose: JSONL append/query + retention.
- Structs: `HistoryRecord`.
- Functions: `open(&HistoryConfig)`, `append(record)`, `recent_trend()`, `prune()`.
- Tests: append+read round-trip; pruning; corrupt-line tolerance.
- Acceptance: append-only; survives malformed lines.

### `src/rollback/mod.rs`
- Purpose: capture/list/revert metadata.
- Structs: `RollbackEntry`.
- Functions: `record(entry)`, `list()`, `apply(id)` (revert real changes in v0.2+).
- Tests: capture/list round-trip; revert of reversible kinds (v0.2+).
- Acceptance: v0.1 records + lists; refuses revert without valid prior state.

### `src/logging/mod.rs`
- Purpose: tracing init + audit writer.
- Structs: `AuditEvent`.
- Functions: `init(&LoggingConfig)`, `audit(event)`.
- Tests: audit serialization; level filtering.
- Acceptance: journald-friendly; never panics on unwritable audit path.

### `src/explain/mod.rs`
- Purpose: build `Explanation`s.
- Structs: `Explanation`.
- Functions: `explain(state, pressure, decision, results) -> Explanation`.
- Tests: deterministic text for known inputs.
- Acceptance: every decision explainable.

### `src/reports/mod.rs`
- Purpose: aggregate history.
- Functions: `report(window) -> Report`.
- Tests: aggregation math over fixtures.
- Acceptance: empty history → empty report (not error).

### `tests/*`
- Integration tests per §7. Acceptance: all green in CI/local.

### `packaging/systemd/syswarden.service`
- Hardened unit per §18. Acceptance: starts under systemd; logs to journald.

### `packaging/aur/PKGBUILD`
- Builds release binary, installs unit + config example. Acceptance: `makepkg` produces a package (later phase).

### `README.md`, `docs/usage.md`, `docs/adr/`
- User docs + ADRs mirroring §24. Acceptance: a new user can install, run `doctor`, and understand safety defaults.

---

## 5. Data Contract Implementation Details

For every struct/enum in §15 of `architecture.md`:

- **Fields/types:** Use the fields listed in §15. Prefer `u64` for byte/kB counters, `f64` for PSI/percentages, `String` for names, `chrono::DateTime<Utc>` for timestamps, `Option<T>` for capability-dependent fields.
- **Validation rules:** `config` validates ranges (thresholds 0–100, intervals ≥ `min_interval_secs`, `max ≥ min`), enum membership (`ProfileName`), and that no field requests a prohibited action. Protected sets must always include `syswarden` itself.
- **Serialization:** All persisted types (`AppConfig`, `HistoryRecord`, `AuditEvent`, `RollbackEntry`) derive `Serialize`/`Deserialize`. Persisted JSONL records include `schema_version`.
- **Default values:** Defaults are the `conservative` profile, `dry_run = true`, all `allow_*` flags `false`, empty `allowed.services`, protected sets pre-populated with system-critical units.
- **Test cases:** Round-trip serialization; default construction; validation accept/reject pairs; enum exhaustiveness in `match`.

Enums (`PressureLevel`, `SystemState`, `ActionKind`, `ActionRisk`, `ActionStatus`, `DecisionIntent`, `SafetyDecision`) must be matched exhaustively (no catch-all `_` that hides new variants in safety-critical code).

---

## 6. Error Handling Plan

- **Error enum structure:** `SyswardenError` (`thiserror`) in `error.rs`; library functions return `Result<T, SyswardenError>`. Binaries/daemon use `anyhow::Result` with `.context(...)`.
- **Context-rich errors:** Every error carries what was attempted and the path/unit involved.
- **Permission-denied behavior:** Treat as expected; downgrade to observe-only for that action; log at `warn`; never crash.
- **Missing `/proc` file behavior:** Skip that metric, set `Option` to `None`, record a capability gap, continue.
- **Missing PSI behavior:** Enter `degraded`; classify from available metrics; clearly explain reduced capability.
- **Non-systemd behavior:** Disable service analysis and systemd actions; `degraded`; continue with process/memory features.
- **Missing zram behavior:** zram features become recommend-with-caveat or unavailable; never error the daemon.
- **Non-root behavior:** Auto-block all state-changing actions with a clear reason; analysis continues.
- **Invalid config behavior:** `config validate`/load reports all issues; daemon refuses to act (enters `protected_mode`) but may still observe with safe defaults; never silently "fix" by acting.
- **Failed action behavior:** Record `ActionResult::Failed` with message; do not retry blindly; do not crash; surface in audit + explain.
- **Logging requirements:** Every error path logs once at the appropriate level with structured fields; no silent swallow.

No `unwrap()`/`expect()`/`panic!` on runtime paths. They are permitted only in tests and in truly-impossible-state assertions with a justifying comment.

---

## 7. Testing Plan

- **Unit tests:** Per module (parsing, classification, decision tables, gate logic, recommendation math).
- **Integration tests (`tests/`):** End-to-end through public APIs with fixtures.
- **Mock metric snapshots:** Construct `MetricsSnapshot` directly to drive pressure/policy tests.
- **Fake PSI files:** Use `examples/fixtures/pressure_*.sample`; parse and assert.
- **Config parsing tests:** Valid, invalid, partial, missing-file → defaults.
- **Config validation tests:** Each rule’s accept/reject case.
- **Policy decision tests:** Cover every `(SystemState, ProfileName)` combination against the decision tables.
- **Safety blocking tests:** Prohibited actions blocked; protected proc/service untouched; flag-gating; non-root blocking; fail-closed on unknown.
- **Dry-run tests:** Planner produces actions; simulator and dry-run path cause **zero** side effects (assert no external calls via mocks).
- **Rollback tests:** Capture/list round-trip; (v0.2+) revert restores prior state.
- **CLI output tests:** Snapshot human + `--json` output for `status`, `analyze`, `pressure`, `config show`.
- **Daemon loop smoke test:** Run `single_tick` with mocked collectors; assert classification, decision, dry-run result, history append, clean shutdown.

Target: deterministic tests, no reliance on host state in unit tests (use fixtures), live-system behavior covered only in clearly-marked, ignorable integration tests.

---

## 8. Acceptance Criteria

**Per phase** — the phase’s files exist, compile, are formatted, pass clippy with no new warnings, and have the tests listed in §4/§7 passing.

**Per milestone:**
- M1: `cargo build` green with stubs.
- M2: all subcommands parse; `version`/`--help` correct.
- M3: `config show`/`config validate` correct; missing-file defaults safe.
- M4: `status` shows accurate live metrics.
- M5: `pressure` shows accurate live PSI.
- M6: `analyze` shows correct `PressureLevel` + contributors on crafted inputs.
- M7: `actions dry-run` shows correct decisions per state/profile.
- M8: safety tests prove prohibited/protected/non-root blocking; fail-closed default.
- M9: planner output correct; zero side effects under dry-run.
- M10: daemon runs foreground in dry-run for an extended period, low overhead, clean SIGTERM exit.
- M11: history + audit JSONL written and queryable; `report` correct.
- M12: rollback entries captured and listed.
- M13: hardened systemd unit runs; `journalctl -u syswarden` shows logs.
- M14: all milestones met; tests green; docs complete; default config makes zero system changes.

---

## 9. Development Commands

```bash
cargo check                      # fast type-check
cargo fmt --all                  # format
cargo clippy --all-targets -- -D warnings   # lint, deny warnings
cargo test                       # run all tests
cargo build --release            # optimized build

# Local dry-run execution (no system changes; default dry_run=true):
./target/release/syswarden analyze
./target/release/syswarden actions dry-run

# Local daemon foreground execution (dry-run):
./target/release/syswarden daemon

# Log inspection:
./target/release/syswarden logs --since 1h
journalctl -u syswarden -f       # when running under systemd
```

---

## 10. Implementation Restrictions

- Do not run destructive commands.
- Do not modify `/etc` during tests (use temp dirs / fixtures).
- Do not require root for basic analysis.
- Do not kill processes.
- Do not remove packages.
- Do not edit the bootloader.
- Do not edit `fstab`.
- Do not use the network.
- Do not call external APIs.
- Do not add telemetry.
- Do not create busy loops (always sleep between ticks; honor `min_interval_secs`).
- Do not poll aggressively (respect adaptive interval bounds).
- Do not add architecture not present in `architecture.md`.

---

## 11. Commit Strategy

Use small, ordered, single-purpose commits. Suggested sequence (one or a few per phase):

1. `chore: repository skeleton and module stubs`
2. `build: Cargo.toml with phase-1 dependencies`
3. `feat(config): AppConfig model, TOML load, validation`
4. `feat(cli): clap command tree and dispatch`
5. `feat(logging): tracing setup and audit JSONL`
6. `feat(metrics): memory/cpu/io collection`
7. `feat(pressure): PSI parser and PressureLevel classification`
8. `feat(processes): process analysis and flagging`
9. `feat(services): read-only systemd service analysis`
10. `feat(pressure): system-state classification`
11. `feat(profiles): built-in profiles and resolution`
12. `feat(policy): decision engine and tables`
13. `feat(safety): mandatory fail-closed safety gate`
14. `feat(actions): dry-run planner and simulator`
15. `feat(daemon): main supervision loop (dry-run)`
16. `feat(history): JSONL history store and retention`
17. `feat(rollback): rollback metadata capture and list`
18. `feat(packaging): hardened systemd unit`
19. `test: complete unit and integration suite`
20. `docs: README, usage, ADRs`

Each commit must compile and keep tests green.

---

## 12. Prompts for Future Implementation Models

Each prompt below is ready to paste. Every prompt implicitly requires: **follow `architecture.md` and `planning.md`; do not change architecture; implement only this phase; report changed files; run checks if possible.**

**Skeleton (Phase 0):**
> Read `architecture.md` and `planning.md`. Implement Phase 0 only: create the exact repository tree from `architecture.md` §14 with empty module files containing `//!` module docs and a minimal `main.rs` that compiles. Do not change architecture. Report changed files and run `cargo build`.

**Cargo.toml (Phase 1):**
> Read both documents. Implement Phase 1 only: write `Cargo.toml` with the crates marked Required in `architecture.md` §16, minimal `tokio` features, and strict lints. Do not add deferred crates. Report changed files and run `cargo build`.

**Config (Phase 2):**
> Implement Phase 2 only: the `config` module and config data contracts from §15, TOML loading, defaults (conservative + dry_run=true), and validation. Do not change architecture. Add config tests. Report changed files and run `cargo test`.

**CLI (Phase 3):**
> Implement Phase 3 only: the `clap` command tree for every command in §13 with dispatch stubs and exit codes. Do not implement business logic beyond routing. Report changed files and run `cargo test`.

**Logging (Phase 4):**
> Implement Phase 4 only: `tracing` setup and the `AuditEvent` JSONL writer per §17/§21. Never panic on unwritable audit paths. Report changed files and run `cargo test`.

**Memory metrics (Phase 5a):**
> Implement Phase 5 (memory) only: `metrics/memory.rs` and `MemoryMetrics`, parsing `/proc/meminfo` and swap state, using `MemAvailable`. Add fixture tests. Report changed files and run `cargo test`.

**CPU metrics (Phase 5b):**
> Implement Phase 5 (CPU) only: `metrics/cpu.rs` and `CpuMetrics`, computing utilization from two `/proc/stat` samples plus loadavg. Add fixture tests. Report changed files and run `cargo test`.

**IO + snapshot (Phase 5c):**
> Implement Phase 5 (IO + snapshot) only: `metrics/io.rs`, `IoMetrics`, and `MetricsSnapshot::collect`. Add tests. Report changed files and run `cargo test`.

**PSI parser (Phase 6):**
> Implement Phase 6 only: PSI parsing in `pressure` into `PsiMetrics`, handling absent PSI (degraded). Add fixture tests using `examples/fixtures/pressure_*.sample`. Report changed files and run `cargo test`.

**Process analyzer (Phase 7):**
> Implement Phase 7 only: `processes` producing `ProcessInfo` and flags; never act on processes; honor protected lists. Add fixture tests. Report changed files and run `cargo test`.

**Service analyzer (Phase 8):**
> Implement Phase 8 only: read-only `services` producing `ServiceInfo` via systemd D-Bus, with graceful no-systemd degradation. Add mocked tests. Report changed files and run `cargo test`.

**Pressure model (Phase 9):**
> Implement Phase 9 only: `compute` (PressureSnapshot + PressureLevel with cross-checks and hysteresis) and `classify_state` (SystemState). Healthy cache must not raise memory pressure. Add tests. Report changed files and run `cargo test`.

**Profiles (Phase 10):**
> Implement Phase 10 only: built-in `ProfileConfig`s matching §11 and `resolve`. Add tests for each profile and unknown-name error. Report changed files and run `cargo test`.

**Policy engine (Phase 11):**
> Implement Phase 11 only: pure `decide` mapping state+profile+findings to `PolicyDecision`, matching the §9 tables. Add exhaustive decision-table tests. Report changed files and run `cargo test`.

**Safety layer (Phase 12):**
> Implement Phase 12 only: the mandatory fail-closed `safety::evaluate` enforcing prohibited/protected/allowlist/flag/non-root/dry-run gates per §10/§17. Add blocking tests. Report changed files and run `cargo test`.

**Dry-run actions (Phase 13):**
> Implement Phase 13 only: `actions` planner and simulator producing `PlannedAction`/`ActionResult` with zero side effects. Every executable path must call `safety::evaluate` first; do not implement real execution. Add tests. Report changed files and run `cargo test`.

**Daemon loop (Phase 14):**
> Implement Phase 14 only: the `daemon` main loop per §6 pseudocode, dry-run only, adaptive sleep, signal-based clean shutdown, per-tick audit + history. Add a single-tick smoke test. Report changed files and run `cargo test`.

**History store (Phase 15):**
> Implement Phase 15 only: JSONL `history` with `HistoryRecord`, append/query/retention, corrupt-line tolerance, `schema_version`. Add tests. Report changed files and run `cargo test`.

**Rollback metadata (Phase 16):**
> Implement Phase 16 only: `rollback` capture/list of `RollbackEntry` and revert scaffolding that refuses without valid prior state. Add round-trip tests. Report changed files and run `cargo test`.

**systemd packaging (Phase 17):**
> Implement Phase 17 only: a hardened `packaging/systemd/syswarden.service` per §18 plus install/uninstall docs. Do not modify host systemd. Report changed files.

**Tests (Phase 18):**
> Implement Phase 18 only: complete the unit + integration suite per §7. Do not change production behavior. Report changed files and run `cargo test`.

---

## 13. Definition of Done

- **MVP done:** Phases 0–14 complete; daemon runs in dry-run; analysis/classification/decision/safety/dry-run all working and tested; zero system changes by default.
- **v0.1 done:** Phases 0–20 complete; M1–M14 met; tests green; docs complete; hardened systemd unit; default config makes no system changes.
- **Safe for local testing:** Yes — analysis and dry-run require no root and change nothing.
- **Safe for systemd foreground testing:** Yes — run `syswarden daemon` (dry-run) under systemd; observe logs.
- **Not yet safe for aggressive automatic optimization:** Correct — real state-changing execution (v0.2+) requires completed, tested safety + rollback and explicit user opt-in flags. Do not enable it in v0.1.

---

## 14. Risk Register

| Risk | Mitigation |
|---|---|
| Over-optimization | Conservative defaults; dry-run default; hysteresis; recovery state relaxes limits. |
| Unsafe process control | Protected lists; never kill; nice/ionice only on allowed non-protected procs behind flags. |
| Incorrect PSI interpretation | Cross-check PSI with `MemAvailable`/swap-in; extensive fixture tests; explain contributors. |
| Service disruption | Allowlist-only changes; protected services hard-blocked; restart/stop are aggressive + gated. |
| zram misconfiguration | Detection-first; recommend-only by default; gated apply with rollback; conflict detection. |
| Excessive daemon overhead | Adaptive interval; cheap parsing; self-overhead monitoring with warnings. |
| Bad config | Strict validation; safe defaults on missing file; protected_mode on invalid config. |
| Insufficient permissions | Non-root → observe-only; expected, logged, never crashes. |
| Arch version differences | Capability detection each run; degrade gracefully for missing PSI/systemd/cgroups. |
| cgroup inconsistencies | Detect v2; read-only cgroup access; writes only via systemd. |
| User misunderstanding | Clear explanations, `doctor`, README emphasizing that Linux cache use is normal and dry-run is default. |

---

## 15. Forbidden Implementation Order

Never violate this order:

- Do not implement real process killing before the safety layer — and process killing is **not** implemented at all (prohibited by default).
- Do not implement sysctl changes before backup and rollback exist and are tested.
- Do not implement zram apply before zram detection and dry-run exist.
- Do not implement `MemoryMax` before `MemoryHigh` is implemented and proven.
- Do not implement the `performance`/aggressive profiles' acting behavior before `conservative` and `balanced` exist and pass tests.
- Do not implement any AI integration before v1.0 (and only if explicitly requested by the human owner).
- Do not implement network features (never).
- Do not implement package removal (never).
- Do not implement destructive cleanup (never).

---

## 16. Final Handoff Instructions

- **Always start by reading both documents** (`architecture.md`, then `planning.md`) before writing code.
- **Treat `architecture.md` as immutable** unless the human owner explicitly requests a revision.
- **Treat `planning.md` as the execution checklist** — implement phases in order, one at a time.
- **Ask for confirmation only when a phase requires real system changes** (v0.2+ actions, systemd writes, zram apply). For v0.1 (observe + dry-run), proceed without changing the host.
- **Preserve safety defaults** — `dry_run = true`, all `allow_*` flags `false`, protected sets intact, safety layer mandatory and fail-closed.
- **Keep all artifacts in English** — code, comments, docs, commits, logs, tests.

Hand off after each phase with: a list of changed files, the checks run and their results, and the next phase to implement.
