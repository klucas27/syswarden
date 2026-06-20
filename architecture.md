# syswarden — Architecture

> **Status:** Authoritative architecture source of truth.
> **Audience:** Implementation models (Claude Sonnet, GPT-5.5) and human maintainers.
> **Rule:** This document is immutable unless the human owner explicitly requests a revision. Implementers must not redesign, rename, simplify, reinterpret, or override this architecture.

---

## 1. Project Overview

### Final project name
**syswarden**

The binary, daemon, configuration directory, and systemd unit all use the name `syswarden`.

### Short description
A local, low-overhead Rust system supervision and resource-optimization daemon for Arch Linux that observes real system pressure (PSI) and applies safe, reversible, explainable optimizations.

### Long description
syswarden is a continuously running local daemon written primarily in Rust. It monitors real kernel pressure signals (PSI for CPU, memory, and I/O), memory and swap state, process and systemd service behavior, and historical trends. From these signals it classifies the system into a well-defined state machine and, depending on the active profile and explicit user permissions, it observes, recommends, or applies conservative resource-governance actions through systemd resource control, cgroups v2, process priorities (`nice`/`ionice`), and zram/zswap awareness.

syswarden is built around three non-negotiable ideas: **safety first**, **explainability**, and **reversibility**. By default it runs in dry-run mode and only observes and recommends. Any action that changes system state is gated behind explicit configuration flags, an allowlist, a safety layer, and a rollback record.

### Problem statement
Linux users — especially on rolling-release distributions like Arch — frequently reach for "RAM cleaner" or "optimizer" tools that misunderstand Linux memory management. These tools drop caches, kill processes, or apply irreversible tuning, producing cosmetic "free RAM" while harming real responsiveness and stability. There is a gap for a tool that uses the kernel's own pressure signals (PSI) to make conservative, explainable, reversible decisions, and that defaults to doing nothing harmful.

### Target users
- Arch Linux desktop users who want a safe responsiveness supervisor.
- Developers running heavy workloads (compilers, containers, browsers, IDEs) who want graceful degradation under load.
- Self-hosters and homelab operators running small Arch-based servers.
- Power users who want explainable, auditable resource governance instead of opaque scripts.

### Target systems
- Arch Linux (and Arch-derived distributions) with a modern Linux kernel.
- systemd as init system.
- cgroups v2 unified hierarchy.
- Kernel with PSI enabled (`CONFIG_PSI=y`, typically exposed under `/proc/pressure/`).
- x86_64 primary target; aarch64 best-effort.

### Non-goals
- Not a "RAM cleaner" and not a cache dropper.
- Not a generative-AI background agent.
- Not a benchmark-chasing overclocking tool.
- Not a package manager, file cleaner, or bootloader/kernel tuner.
- Not a network service, telemetry collector, or cloud product.
- Not a replacement for human judgement on aggressive changes.

### Project philosophy
Optimize for **responsiveness, stability, and controlled degradation**, never for cosmetic metrics. Prefer observing and explaining over acting. Make every action reversible and auditable. Keep the daemon itself lighter than the problems it solves.

### Success definition
syswarden is successful when it can run for weeks as a near-zero-overhead daemon, correctly classify system pressure from PSI, produce clear human-readable explanations and recommendations, and — only when explicitly permitted — apply conservative, reversible resource-governance actions that measurably improve responsiveness under load without ever destabilizing the system.

---

## 2. Core Principles

1. **Safety before aggressiveness.** The default behavior is observe-only dry-run. Nothing destructive can happen by default.
2. **Pressure-aware optimization.** Decisions are driven by PSI and real pressure, not raw "used RAM".
3. **Explainability.** Every decision and action carries a human-readable explanation and the metrics that justified it.
4. **Reversibility.** Any state-changing action records enough metadata to be rolled back.
5. **Low overhead.** The daemon must be lightweight: adaptive polling, cheap parsing, bounded memory.
6. **Offline-first.** No network access, ever. All intelligence is local and deterministic.
7. **Deterministic local intelligence.** Decisions come from explicit rules, thresholds, and tables — not from a model that could behave unpredictably.
8. **No paid APIs.** No external dependency requiring payment or accounts.
9. **No external AI dependency.** No background generative model.
10. **No destructive defaults.** Destructive actions are prohibited by default and require multiple explicit opt-ins.
11. **No fake RAM cleaning.** syswarden never drops caches to inflate "free memory".
12. **No blind automation.** Aggressive actions require explicit allowlists and per-action permission.
13. **No architecture drift.** Implementers follow this document exactly.

---

## 3. Technical Approach

### Why syswarden uses these technologies

- **Rust** — Memory safety without a garbage collector, predictable low overhead, strong type system to encode safety invariants (e.g., action risk levels) at compile time, and excellent crates for `/proc`, `nix`, and systemd integration.
- **systemd** — The native service manager on Arch. It already provides robust resource control (`CPUWeight`, `IOWeight`, `MemoryHigh`, `MemoryMax`) and a transactional drop-in mechanism. syswarden governs resources through systemd rather than reinventing it.
- **cgroups v2** — The unified hierarchy is the correct, supported mechanism for resource governance. syswarden reads cgroup state and applies limits through systemd's cgroup integration.
- **PSI (Pressure Stall Information)** — PSI directly measures the time tasks stall waiting for CPU, memory, or I/O. This is a far better signal of real pressure than raw RAM usage and is the primary input to syswarden's decision model.
- **/proc and /sys** — Authoritative, dependency-free kernel interfaces for metrics (`/proc/meminfo`, `/proc/stat`, `/proc/loadavg`, `/proc/pressure/*`, `/proc/[pid]/*`, `/sys/block/zram*`).
- **zram/zswap awareness** — Under genuine memory pressure, compressed swap can help. syswarden detects existing configuration and only *recommends* sizing; it applies changes only behind explicit permission.
- **Profiles** — Different systems (desktop, server, low-RAM, developer) need different thresholds and permissions. Profiles encapsulate these as named, auditable bundles.
- **Policy engine** — A deterministic rule layer mapping system state + profile to a `PolicyDecision`. Keeps decision logic centralized and testable.
- **Safety layer** — A mandatory gate every action passes through before execution. It enforces allowlists, protected processes/services, dry-run, and permission flags.
- **Dry-run** — The default execution mode. The action planner produces a full plan that is logged and explained but not applied.
- **Rollback** — Each applied action stores prior state so it can be reverted.
- **Local history** — A small append-only store of pressure and actions to enable trend analysis and the `report` command.
- **Audit logging** — Structured, append-only record of every decision and action for accountability.

### Why syswarden deliberately does NOT use

- **A continuously running generative AI model** — Non-deterministic, heavy, and contrary to the low-overhead, offline, explainable goals.
- **Aggressive cache clearing (`drop_caches` loops)** — Page cache is beneficial; clearing it harms performance and is a classic "fake optimization".
- **Global garbage collection** — No such concept exists for Linux processes, and Rust needs no GC. syswarden never claims to "garbage collect" the system.
- **Random process killing** — Dangerous and destabilizing; prohibited by default.
- **Package removal automation** — Out of scope and destructive.
- **Irreversible tuning** — Every change must be reversible; one-way tuning is prohibited.
- **Opaque optimization scripts** — Every action is named, typed, risk-classified, and explained.

---

## 4. High-Level Architecture

### Major components
- **CLI** — User entry point; subcommands for status, analysis, diagnostics, profiles, config, actions, zram, rollback, reports.
- **Daemon** — Long-running supervisor executing the main loop.
- **Config manager** — Loads, validates, and exposes `AppConfig`.
- **Metrics collector** — Reads `/proc` and `/sys` into `MetricsSnapshot`.
- **PSI collector** — Parses `/proc/pressure/*` into `PsiMetrics`/`PressureSnapshot`.
- **Process analyzer** — Builds `ProcessInfo` lists, flags heavy/anomalous processes.
- **Service analyzer** — Queries systemd for `ServiceInfo`, flags failing/heavy services.
- **systemd manager** — Reads service state and applies drop-in resource control.
- **cgroup manager** — Reads cgroup v2 state and resolves service cgroups.
- **zram/zswap manager** — Detects and (when permitted) configures compressed swap.
- **Policy engine** — Maps `SystemState` + `ProfileConfig` to `PolicyDecision`.
- **Action planner** — Translates decisions into `PlannedAction`s.
- **Action executor** — Executes planned actions (or simulates them in dry-run).
- **Safety layer** — Mandatory gate validating every action before execution.
- **Profile manager** — Loads and resolves the active `ProfileConfig`.
- **Rollback manager** — Records prior state and reverts actions.
- **Local history store** — Append-only persistence of pressure/action records.
- **Audit logger** — Structured append-only audit trail.
- **Explainability engine** — Produces human-readable `Explanation`s.
- **Reporting layer** — Aggregates history into reports.

### Text-based architecture diagram

```
                         +-----------------------------+
                         |            CLI              |
                         | status/analyze/doctor/...   |
                         +--------------+--------------+
                                        |
                                        v
+----------------+   loads   +----------------------+
|  Config files  |---------->|    Config Manager    |
| config.toml    |           |    (AppConfig)       |
+----------------+           +----------+-----------+
                                        |
                                        v
                         +------------------------------+
                         |           DAEMON             |
                         |        (main loop)           |
                         +--+----------+----------+-----+
                            |          |          |
              +-------------+   +------+-----+   +-------------+
              v                 v            v                 v
     +----------------+  +-------------+ +-----------+  +----------------+
     | Metrics Coll.  |  | PSI Coll.   | | Process   |  | Service Coll.  |
     | /proc /sys     |  | /proc/press | | Analyzer  |  | systemd (zbus) |
     +-------+--------+  +------+------+ +-----+-----+  +-------+--------+
             |                  |              |                |
             +---------+--------+------+-------+--------+-------+
                       v                       v
                 +-----------+           +-----------+
                 | Pressure  |           |  System   |
                 |  Model    |---------->|  State    |
                 +-----------+           +-----+-----+
                                               |
                       +-----------------+     v     +------------------+
                       | Profile Manager |--> POLICY ENGINE             |
                       +-----------------+     |     +------------------+
                                               v
                                        +--------------+
                                        | Action       |
                                        | Planner      |
                                        +------+-------+
                                               v
                                        +--------------+   blocks/allows
                                        | SAFETY LAYER |<--- allowlists,
                                        +------+-------+     protected sets,
                                               |             dry_run, flags
                                  dry-run? -----+----- execute
                                     |                    |
                                     v                    v
                              +-----------+        +--------------+
                              | (simulate)|        | Action       |
                              +-----+-----+        | Executor     |
                                    |              | systemd/cgrp |
                                    |              | nice/ionice  |
                                    |              | zram         |
                                    |              +------+-------+
                                    |                     |
                                    |             records prior state
                                    |                     v
                                    |              +--------------+
                                    |              | Rollback Mgr |
                                    |              +------+-------+
                                    +----------+----------+
                                               v
                  +-----------------+   +-------------+   +----------------+
                  | History Store   |<--| Audit Logger|-->| Explainability |
                  | (JSONL)         |   | (JSONL)     |   | Engine         |
                  +--------+--------+   +-------------+   +-------+--------+
                           |                                     |
                           v                                     v
                     +-----------+                        +-----------+
                     | Reporting |                        |  CLI out  |
                     +-----------+                        +-----------+
```

---

## 5. Module-by-Module Architecture

Conventions for every module below: **Responsibility / Inputs / Outputs / Internal deps / External deps / Permissions / Failure modes / Not allowed to decide / Public data contracts / Testing strategy.**

### 5.1 `cli`
- **Responsibility:** Parse arguments, dispatch subcommands, render human/JSON output.
- **Inputs:** Process args, `AppConfig`, read-only views from other modules.
- **Outputs:** Terminal output, process exit codes.
- **Internal deps:** config, metrics, pressure, processes, services, policy, actions, zram, rollback, reports, explain, logging.
- **External deps:** `clap`.
- **Permissions:** Inherits user privileges; never escalates.
- **Failure modes:** Invalid args, missing config, insufficient permissions for a subcommand.
- **Not allowed to decide:** Whether an action is safe (delegates to safety layer); pressure classification (delegates to pressure model).
- **Public data contracts:** Command enums, exit-code mapping.
- **Testing:** Argument parsing tests, output snapshot tests, exit-code tests.

### 5.2 `daemon`
- **Responsibility:** Own the main supervision loop and lifecycle.
- **Inputs:** `AppConfig`, signals, time.
- **Outputs:** Side effects via executor (gated), history, audit logs.
- **Internal deps:** every analysis/decision/action module.
- **External deps:** `tokio` (runtime + signals), `tracing`.
- **Permissions:** Runs as a systemd service; root only for state-changing actions.
- **Failure modes:** Collector errors, executor errors, shutdown signals.
- **Not allowed to decide:** Action safety (safety layer) or pressure thresholds (config/profile).
- **Public data contracts:** Loop tick result, `SystemState` transitions.
- **Testing:** Loop smoke test with mocked collectors; single-tick determinism test.

### 5.3 `config`
- **Responsibility:** Load, merge, validate configuration; expose `AppConfig`.
- **Inputs:** `/etc/syswarden/config.toml`, defaults, env override for config path.
- **Outputs:** Validated `AppConfig`, validation report.
- **Internal deps:** profiles (for profile resolution), logging.
- **External deps:** `serde`, `toml`.
- **Permissions:** Read access to config path.
- **Failure modes:** Missing file (use defaults), parse error, validation error.
- **Not allowed to decide:** Runtime actions; it only provides configuration.
- **Public data contracts:** `AppConfig`, `GlobalConfig`, `ProfileName`, `ProfileConfig`, process/service rule structs.
- **Testing:** Parse valid/invalid TOML, default fallback, validation rule coverage.

### 5.4 `metrics`
- **Responsibility:** Collect memory/CPU/IO metrics into `MetricsSnapshot`.
- **Inputs:** `/proc/meminfo`, `/proc/stat`, `/proc/loadavg`, `/sys/block/zram*`, `/proc/swaps`.
- **Outputs:** `MetricsSnapshot` (`MemoryMetrics`, `CpuMetrics`, `IoMetrics`).
- **Internal deps:** logging.
- **External deps:** `procfs` (preferred) with raw-parse fallback.
- **Permissions:** Read-only `/proc`, `/sys`.
- **Failure modes:** Missing files, parse errors, transient read errors.
- **Not allowed to decide:** Pressure level or actions.
- **Public data contracts:** `MetricsSnapshot`, `MemoryMetrics`, `CpuMetrics`, `IoMetrics`.
- **Testing:** Parse fixed fixture files; delta computation for CPU utilization across two samples.

### 5.5 `pressure`
- **Responsibility:** Parse PSI and compute `PressureSnapshot` and final `PressureLevel`.
- **Inputs:** `/proc/pressure/{cpu,memory,io}`, `MetricsSnapshot`, history (trend), thresholds.
- **Outputs:** `PressureSnapshot`, `PressureLevel`, contributing factors.
- **Internal deps:** metrics, history, config.
- **External deps:** none beyond std (raw PSI parsing).
- **Permissions:** Read-only `/proc/pressure/*`.
- **Failure modes:** PSI absent (kernel without `CONFIG_PSI`) — degrade to metrics-only mode.
- **Not allowed to decide:** Actions; only classifies pressure.
- **Public data contracts:** `PsiMetrics`, `PressureSnapshot`, `PressureLevel`.
- **Testing:** PSI fixture parsing; threshold-to-level mapping; PSI-absent degradation.

### 5.6 `processes`
- **Responsibility:** Enumerate processes and flag heavy/anomalous ones.
- **Inputs:** `/proc/[pid]/*`, protected-process list, process rules.
- **Outputs:** `Vec<ProcessInfo>`, anomaly flags.
- **Internal deps:** config, safety (protected lists), logging.
- **External deps:** `procfs`.
- **Permissions:** Read-only `/proc`; full visibility requires root for some fields.
- **Failure modes:** Race conditions (pid disappears), permission-limited fields.
- **Not allowed to decide:** To kill or re-nice — only to *flag* candidates.
- **Public data contracts:** `ProcessInfo`.
- **Testing:** Fixture `/proc` tree parsing; protected-process exclusion; anomaly thresholds.

### 5.7 `services`
- **Responsibility:** Query systemd units, flag failing/heavy services.
- **Inputs:** systemd D-Bus, protected/allowed service lists, service rules.
- **Outputs:** `Vec<ServiceInfo>`.
- **Internal deps:** systemd module, config, logging.
- **External deps:** `zbus`.
- **Permissions:** Read-only D-Bus calls (user/system bus).
- **Failure modes:** No systemd / no D-Bus — degrade gracefully.
- **Not allowed to decide:** To stop/restart — only to flag.
- **Public data contracts:** `ServiceInfo`.
- **Testing:** Mocked systemd responses; allowlist/denylist handling.

### 5.8 `systemd`
- **Responsibility:** Read unit properties; write transient/persistent drop-ins for resource control.
- **Inputs:** Unit names, resource-control values, `PlannedAction`.
- **Outputs:** Applied drop-ins, prior-state capture for rollback.
- **Internal deps:** safety, rollback, logging.
- **External deps:** `zbus` (and `systemctl`/`systemd-run` only as an explicit, audited fallback).
- **Permissions:** Root required for system-unit changes.
- **Failure modes:** Permission denied, unit not found, D-Bus errors.
- **Not allowed to decide:** Whether a change is permitted (safety layer decides).
- **Public data contracts:** Drop-in descriptors, property maps.
- **Testing:** Dry-run rendering of drop-ins; rollback metadata capture; mocked apply.

### 5.9 `cgroups`
- **Responsibility:** Resolve and read cgroup v2 state for services/processes.
- **Inputs:** `/sys/fs/cgroup/**`, unit cgroup paths.
- **Outputs:** Cgroup usage/limit readings.
- **Internal deps:** systemd module, logging.
- **External deps:** std file IO.
- **Permissions:** Read-only `/sys/fs/cgroup`; writes are performed via systemd, not by direct cgroup edits.
- **Failure modes:** cgroups v1 hybrid systems — detect and degrade.
- **Not allowed to decide:** To write limits directly (must go through systemd manager).
- **Public data contracts:** Cgroup reading structs.
- **Testing:** Fixture cgroup tree parsing; v2 detection.

### 5.10 `zram`
- **Responsibility:** Detect zram/zswap/swap state; compute recommendations; apply only when permitted.
- **Inputs:** `/sys/block/zram*`, `/proc/swaps`, `/sys/module/zswap/*`, `/proc/meminfo`.
- **Outputs:** Detection report, sizing recommendation, (gated) applied config + rollback metadata.
- **Internal deps:** metrics, safety, rollback, logging.
- **External deps:** std file IO; `zramctl`/`swapon` only as explicit, audited fallback.
- **Permissions:** Read-only for detection; root for apply.
- **Failure modes:** Module absent, conflicting zswap+zram, apply failure.
- **Not allowed to decide:** To apply without `allow_zram_apply = true` and an explicit action.
- **Public data contracts:** zram detection + recommendation structs.
- **Testing:** Fixture sysfs parsing; recommendation math; conflict detection.

### 5.11 `policy`
- **Responsibility:** Map `SystemState` + `ProfileConfig` (+ analyzers' findings) to a `PolicyDecision`.
- **Inputs:** `SystemState`, `PressureSnapshot`, process/service findings, active profile.
- **Outputs:** `PolicyDecision` (the *intent*: observe/log/recommend/alert/adjust/limit/...).
- **Internal deps:** pressure, processes, services, profiles.
- **External deps:** none.
- **Permissions:** None (pure logic).
- **Failure modes:** None expected; total function over inputs.
- **Not allowed to decide:** Whether an action is *safe to execute* (safety layer) — it only decides intent.
- **Public data contracts:** `PolicyDecision`.
- **Testing:** Exhaustive decision-table tests per state/profile.

### 5.12 `actions`
- **Responsibility:** Plan `PlannedAction`s from `PolicyDecision`; execute or simulate.
- **Inputs:** `PolicyDecision`, profile permissions, dry-run flag.
- **Outputs:** `Vec<PlannedAction>`, `ActionResult`s.
- **Internal deps:** safety (mandatory), systemd, cgroups, zram, processes, rollback, history, audit.
- **External deps:** `nix`/`libc` for nice/ionice.
- **Permissions:** Depends on action; planning needs none, execution may need root.
- **Failure modes:** Execution errors → recorded as `ActionResult::Failed` (never panics the daemon).
- **Not allowed to decide:** To bypass the safety layer (every action passes through it).
- **Public data contracts:** `PlannedAction`, `ActionKind`, `ActionRisk`, `ActionStatus`, `ActionResult`.
- **Testing:** Plan generation per decision; dry-run produces no side effects; executor mocked.

### 5.13 `safety`
- **Responsibility:** The mandatory gate. Validate each `PlannedAction` against protected sets, allowlists, permission flags, and risk policy.
- **Inputs:** `PlannedAction`, `AppConfig`, active profile, privilege level.
- **Outputs:** `SafetyDecision` (`Allow` / `Block { reason }` / `RequireDryRun`).
- **Internal deps:** config, profiles.
- **External deps:** none.
- **Permissions:** None (pure logic) plus privilege detection.
- **Failure modes:** Fail-closed — on any doubt, block.
- **Not allowed to decide:** To allow a prohibited-by-default action under any circumstance.
- **Public data contracts:** `SafetyDecision`.
- **Testing:** Block prohibited actions; protected-process/service enforcement; flag-gating; fail-closed defaults.

### 5.14 `profiles`
- **Responsibility:** Provide built-in profiles and resolve the active `ProfileConfig`.
- **Inputs:** Profile name, config overrides.
- **Outputs:** `ProfileConfig`.
- **Internal deps:** config.
- **External deps:** none.
- **Permissions:** None.
- **Failure modes:** Unknown profile → error with valid list.
- **Not allowed to decide:** Runtime actions; only supplies thresholds/permissions.
- **Public data contracts:** `ProfileName`, `ProfileConfig`.
- **Testing:** Each built-in profile resolves; override merging; unknown-name error.

### 5.15 `rollback`
- **Responsibility:** Capture prior state before an action; provide listing and revert.
- **Inputs:** `PlannedAction`, captured prior state.
- **Outputs:** `RollbackEntry` records; revert results.
- **Internal deps:** systemd, zram, history, audit.
- **External deps:** `serde`.
- **Permissions:** Root for revert that changes state.
- **Failure modes:** Missing/incompatible entry → refuse and explain.
- **Not allowed to decide:** To revert without a valid recorded prior state.
- **Public data contracts:** `RollbackEntry`.
- **Testing:** Capture/restore round-trip; revert of each reversible action kind.

### 5.16 `history`
- **Responsibility:** Append-only local store of pressure and action records; retention.
- **Inputs:** `HistoryRecord`s.
- **Outputs:** Query results for reports and trend analysis.
- **Internal deps:** logging.
- **External deps:** `serde` (JSONL).
- **Permissions:** Read/write to state directory.
- **Failure modes:** Disk full, corrupt line → skip line, log warning.
- **Not allowed to decide:** Actions; it only persists/queries.
- **Public data contracts:** `HistoryRecord`.
- **Testing:** Append+read round-trip; retention pruning; corrupt-line tolerance.

### 5.17 `logging`
- **Responsibility:** Configure `tracing` subscribers; emit structured audit events.
- **Inputs:** Log level config, `AuditEvent`s.
- **Outputs:** journald/stderr logs; audit JSONL.
- **Internal deps:** config.
- **External deps:** `tracing`, `tracing-subscriber`.
- **Permissions:** Write to log/state path; journald via stderr.
- **Failure modes:** Unwritable audit path → warn and continue (never crash).
- **Not allowed to decide:** Anything functional.
- **Public data contracts:** `AuditEvent`.
- **Testing:** Audit serialization; level filtering.

### 5.18 `explain`
- **Responsibility:** Turn decisions/actions/metrics into human-readable `Explanation`s.
- **Inputs:** `PressureSnapshot`, `PolicyDecision`, `PlannedAction`, `SafetyDecision`.
- **Outputs:** `Explanation` (summary + reasons + evidence).
- **Internal deps:** pressure, policy, actions, safety.
- **External deps:** none.
- **Permissions:** None.
- **Failure modes:** None expected.
- **Not allowed to decide:** Anything functional; it only describes.
- **Public data contracts:** `Explanation`.
- **Testing:** Deterministic text for known inputs.

### 5.19 `reports`
- **Responsibility:** Aggregate history into summaries for `report`.
- **Inputs:** `HistoryRecord`s, time window.
- **Outputs:** Report structures + rendered text/JSON.
- **Internal deps:** history, explain.
- **External deps:** `chrono` (or `time`).
- **Permissions:** Read state directory.
- **Failure modes:** Empty history → empty report (not an error).
- **Not allowed to decide:** Actions.
- **Public data contracts:** Report structs.
- **Testing:** Aggregation math over fixture history.

### 5.20 `sysctl`
- **Responsibility:** Read current kernel tunables; (gated, v0.3+) apply `sysctl` changes with prior-state backup for rollback. Mirrors the `zram`/`cgroups` split: read-first, apply behind gates.
- **Inputs:** `PlannedAction` (`ApplySysctl`), key/value params, `Capabilities`.
- **Outputs:** Applied tunable + backup recorded in the rollback store, or `Blocked`/`Failed`.
- **Internal deps:** safety (mandatory), rollback, logging.
- **External deps:** none — reads/writes `/proc/sys/<key>` via explicit paths only; never the `sysctl` binary, never a shell (§17 "No shell interpolation").
- **Permissions:** Root + writable `/proc/sys`; the unit must relax `ProtectKernelTunables` only when sysctl apply is enabled (§18).
- **Failure modes:** Missing/unwritable key → `Failed`, never crash; non-root → blocked.
- **Not allowed to decide:** Which keys to change (config/policy supplies them) or action safety (safety layer).
- **Public data contracts:** Reuses `PlannedAction`/`ActionResult`/`ActionKind::{ApplySysctl, CreateBackup}` (§15) — no new contract.
- **Testing:** Fixture `/proc/sys` read; backup capture round-trip; mocked apply; dry-run ⇒ zero writes.

---

## 6. Daemon Lifecycle

### Phases
1. **Startup** — Initialize logging, parse `daemon` args.
2. **Configuration loading** — Load and validate `AppConfig`; resolve active `ProfileConfig`.
3. **Privilege detection** — Detect root vs non-root; record capability level. Non-root runs in observe-only.
4. **Environment validation** — Check PSI availability, cgroups v2, systemd presence, zram/zswap modules. Record a capability map; degrade features that are unavailable.
5. **Metric collection** — Read `/proc` and `/sys` into `MetricsSnapshot`.
6. **Pressure calculation** — Parse PSI; compute `PressureSnapshot` and `PressureLevel`.
7. **Process analysis** — Build `ProcessInfo`; flag candidates (no action).
8. **Service analysis** — Build `ServiceInfo`; flag failing/heavy services.
9. **System state classification** — Compute `SystemState`.
10. **Policy evaluation** — Compute `PolicyDecision` from state + profile.
11. **Action planning** — Produce `PlannedAction`s for the decision.
12. **Safety validation** — Each action through the safety layer → `SafetyDecision`.
13. **Dry-run or execution** — If `dry_run` (default) or blocked: simulate + explain. Else execute via executor with rollback capture.
14. **Audit logging** — Append `AuditEvent`s for decisions and actions.
15. **Local history write** — Append `HistoryRecord` (pressure + outcomes).
16. **Adaptive sleep interval** — Sleep longer when healthy, shorter under pressure (bounded by profile).
17. **Graceful shutdown** — On SIGTERM/SIGINT: finish current tick, flush logs/history, exit 0.
18. **Error recovery** — Any tick error is logged and the loop continues; repeated fatal errors trigger backoff, never a crash loop.

### Main loop pseudocode

```text
fn run_daemon(config: AppConfig) -> Result<()> {
    init_logging(&config);
    let caps = detect_capabilities();          // psi?, cgroup_v2?, systemd?, root?, zram?
    let mut profile = resolve_profile(&config);
    let mut history = HistoryStore::open(&config)?;
    let mut prev_cpu = read_cpu_sample();       // for delta-based CPU utilization
    let shutdown = install_signal_handlers();   // SIGTERM, SIGINT

    loop {
        if shutdown.is_triggered() { break; }

        let tick_start = now();

        // 1. Collect
        let metrics = match collect_metrics(&caps, &mut prev_cpu) {
            Ok(m) => m,
            Err(e) => { warn!("metrics error: {e}"); sleep(profile.min_interval); continue; }
        };
        let pressure = compute_pressure(&caps, &metrics, history.recent_trend());
        let processes = analyze_processes(&config, &profile);   // flag only
        let services  = analyze_services(&caps, &config, &profile);

        // 2. Classify
        let state = classify_state(&pressure, &processes, &services, &caps);

        // 3. Decide
        let decision = policy_engine(&state, &profile, &processes, &services);

        // 4. Plan
        let planned = plan_actions(&decision, &profile);

        // 5. Gate + execute (or simulate)
        let mut results = Vec::new();
        for action in planned {
            let safety = safety_layer.evaluate(&action, &config, &profile, &caps);
            match safety {
                SafetyDecision::Block { reason } => {
                    audit.block(&action, &reason);
                    results.push(ActionResult::blocked(&action, &reason));
                }
                SafetyDecision::RequireDryRun | _ if config.global.dry_run => {
                    let sim = simulate(&action);
                    audit.dry_run(&action, &sim);
                    results.push(ActionResult::simulated(&action, sim));
                }
                SafetyDecision::Allow => {
                    let prior = capture_prior_state(&action);
                    let res = executor.execute(&action);
                    match &res {
                        Ok(_)  => rollback.record(RollbackEntry::new(&action, prior)),
                        Err(e) => warn!("action failed: {e}"),
                    }
                    audit.executed(&action, &res);
                    results.push(ActionResult::from(res));
                }
            }
        }

        // 6. Persist + explain
        let explanation = explain(&state, &pressure, &decision, &results);
        history.append(HistoryRecord::new(tick_start, &pressure, &state, &results));
        trace_explanation(&explanation);

        // 7. Adaptive sleep
        let interval = adaptive_interval(&state, &profile);   // bounded [min, max]
        sleep_until_or_signal(tick_start + interval, &shutdown);
    }

    flush_all();   // logs, history
    Ok(())
}
```

---

## 7. System States

State machine values and their semantics. Transitions are evaluated every tick from the current `PressureLevel`, analyzer findings, and capability map. Hysteresis (require N consecutive ticks before escalating/de-escalating) prevents flapping.

| State | Entry conditions | Exit conditions | Allowed actions | Forbidden actions | Log behavior | User-facing explanation |
|---|---|---|---|---|---|---|
| `initializing` | Daemon start until first valid snapshot | First successful classification | observe, log | all state-changing | info | "Starting up and validating environment." |
| `idle` | Very low pressure, low load, low I/O | Pressure rises to low+ | observe, log, report | all moderate/aggressive | debug | "System is idle; nothing to do." |
| `healthy` | `PressureLevel::none`/`low`, no anomalies | Pressure → moderate, or anomaly found | observe, log, recommend | moderate+ unless explicitly enabled | info (sparse) | "System healthy; observing only." |
| `moderate_pressure` | `PressureLevel::moderate` sustained | Drops to low (recovery) or rises to high | observe, recommend, **safe** + (if allowed) moderate actions | aggressive | info | "Moderate pressure; recommending conservative adjustments." |
| `high_pressure` | `PressureLevel::high` sustained | Drops to moderate or rises to critical | recommend, moderate actions (if allowed), prepare aggressive (gated) | aggressive unless explicitly allowed | warn | "High pressure; applying conservative limits where permitted." |
| `critical_pressure` | `PressureLevel::critical` | Drops to high | strongest *allowed* moderate actions; aggressive only if all gates pass | killing protected procs, any prohibited action | warn/error | "Critical pressure; acting within strict safety limits." |
| `recovery` | Pressure falling for N ticks after high/critical | Stabilizes to healthy or re-escalates | observe, gently relax temporary limits, report | new aggressive actions | info | "Pressure decreasing; relaxing temporary measures." |
| `degraded` | Missing PSI/systemd/cgroups, or repeated collector errors | Capabilities restored | observe with available signals, log limitations | any action depending on missing capability | warn | "Running with reduced capabilities; observe-only for missing features." |
| `protected_mode` | Safety invariant tripped, untrusted/invalid config, or user-forced | User resolves cause / restarts with valid config | observe, log, explain only | **all** state-changing actions | warn | "Protected mode: observing only until the flagged condition is resolved." |

---

## 8. Pressure Model

PSI is the primary signal. Each PSI file (`/proc/pressure/{cpu,memory,io}`) provides `some` and `full` lines with `avg10`, `avg60`, `avg300`, and a `total` counter. syswarden primarily uses `avg10` and `avg60`.

### Sub-models
- **CPU pressure model** — From `/proc/pressure/cpu` `some avg10/avg60`, cross-checked with `/proc/loadavg` and CPU utilization deltas from `/proc/stat`. High CPU PSI = tasks stalling for CPU time.
- **Memory pressure model** — From `/proc/pressure/memory` `some`/`full` avg10/avg60. `full` memory pressure (all non-idle tasks stalled) is the strongest reclaim-distress signal. Cross-checked with `MemAvailable`, swap activity.
- **I/O pressure model** — From `/proc/pressure/io` `some`/`full`, cross-checked with I/O wait.
- **Swap pressure model** — Derived from swap usage trend and swap-in/out rate (from `/proc/vmstat` `pswpin`/`pswpout` deltas). Rising swap-in under memory PSI indicates genuine memory pressure.
- **Service health model** — Count and severity of failed/restart-looping services from analyzer.
- **Process anomaly model** — Processes exceeding rule thresholds (CPU%, RSS, I/O) sustained over time; flagged, never auto-acted-upon.
- **Historical pressure model** — Short trend (last N records) used for hysteresis and to distinguish a transient spike from sustained pressure.

### From raw metrics to a final `PressureLevel`

1. Compute a sub-level for CPU, memory, and I/O by mapping `avg10`/`avg60` against profile thresholds:
   - `none`: avg10 ≈ 0.
   - `low`: small but non-zero `some` pressure.
   - `moderate`: sustained `some` pressure crosses moderate threshold.
   - `high`: `some` high, or `full` starts rising.
   - `critical`: `full` pressure sustained above critical threshold.
2. Apply cross-checks (e.g., memory sub-level escalates only if `MemAvailable` low and/or swap-in rising — avoids treating healthy cache use as pressure).
3. The final `PressureLevel` is the **maximum** sub-level across CPU/memory/I/O, adjusted by hysteresis from the historical model.
4. Record contributing factors so `explain` can show *why*.

```text
PressureLevel ∈ { none, low, moderate, high, critical }
final = hysteresis( max(cpu_level, mem_level, io_level), recent_history )
```

**Reality check encoded here:** high `MemUsed`/low `MemFree` alone never raises memory pressure; only PSI + low `MemAvailable` + swap-in activity do. Linux using RAM for cache is normal and good.

---

## 9. Decision Model

The policy engine is a pure function: `(SystemState, ProfileConfig, findings) -> PolicyDecision`. A `PolicyDecision` expresses *intent*; the safety layer later decides whether the resulting actions may execute.

### Possible decisions
`ObserveOnly`, `LogOnly`, `Recommend`, `Alert`, `AdjustProcessPriority`, `ApplyCgroupSystemdLimit`, `RecommendZram`, `ApplyZram` (gated), `EnterProtectedMode`, `BlockAction`, `DoNothing`.

### Decision table — by state (with default `balanced` profile)

| State | Default decision | If `allow_aggressive_actions` + permitted target |
|---|---|---|
| initializing | ObserveOnly | ObserveOnly |
| idle | DoNothing | DoNothing |
| healthy | ObserveOnly / Recommend (if anomaly) | same |
| moderate_pressure | Recommend (+ AdjustProcessPriority on allowed targets if `dry_run=false`) | + ApplyCgroupSystemdLimit (MemoryHigh/CPUWeight) on allowed services |
| high_pressure | Recommend + Alert + AdjustProcessPriority (allowed) | + ApplyCgroupSystemdLimit; RecommendZram |
| critical_pressure | Alert + strongest *allowed moderate* actions | + ApplyZram (only if `allow_zram_apply`), MemoryMax only after MemoryHigh tried |
| recovery | Recommend (relax temp limits) | relax limits |
| degraded | LogOnly + Recommend | LogOnly |
| protected_mode | LogOnly | LogOnly |

### Decision table — process priority (when permitted)

| Condition | Decision |
|---|---|
| Process in protected list | BlockAction (never touch) |
| Non-protected, sustained heavy CPU, profile allows nice | AdjustProcessPriority (raise niceness, lower priority) |
| Non-protected, heavy I/O, profile allows ionice | AdjustProcessPriority (ionice best-effort lower) |
| dry_run = true | Recommend only |

### Decision table — zram

| Condition | Decision |
|---|---|
| No memory pressure | DoNothing |
| Memory pressure + no swap configured | RecommendZram |
| Memory pressure + zram exists + undersized | RecommendZram (resize suggestion) |
| `allow_zram_apply=true` + explicit action + critical pressure | ApplyZram |
| zswap active and conflicting | RecommendZram=false; Alert about conflict |

---

## 10. Actions and Risk Classification

Every action carries an `ActionRisk`. The safety layer enforces that an action may execute only if the active profile permits its risk level **and** all relevant flags/allowlists pass.

### Safe actions (always allowed, no system change)
- observe; log; report; explain; recommend.
- detect heavy process; detect failing service.
- calculate zram recommendation.
- adjust daemon polling interval.

### Moderate actions (allowed only when profile permits and `dry_run=false`)
- apply `nice` adjustment to non-protected processes.
- apply `ionice` adjustment to non-protected processes.
- apply `CPUWeight` to allowed services.
- apply `IOWeight` to allowed services.
- apply `MemoryHigh` to allowed services.
- create backups.
- create systemd drop-in previews.

### Aggressive actions (require `allow_aggressive_actions=true` + allowlist + specific flags)
- apply `MemoryMax` (only after `MemoryHigh` has been tried).
- restart explicitly allowed non-critical services.
- stop explicitly allowed non-critical services.
- change `sysctl` values (requires `allow_sysctl_apply=true` + backup + rollback).
- apply zram configuration (requires `allow_zram_apply=true`).
- apply zswap configuration (requires explicit flag; never combined blindly with zram).

### Prohibited by default (no flag enables these in v0.1; they are simply not implemented as executable)
- remove packages; delete files.
- kill protected processes (or any process automatically).
- alter bootloader; alter kernel command line.
- alter `fstab` automatically.
- edit critical files without backup.
- run `drop_caches` loops.
- hide memory leaks (mask symptoms).
- send telemetry externally.
- execute shell snippets from untrusted config.
- use online APIs.

---

## 11. Profiles

Each profile is a named bundle of thresholds, permissions, and behavior. All ship with conservative defaults; only explicit profiles raise permitted risk.

| Field | conservative | balanced | performance | low_ram | desktop | server | developer |
|---|---|---|---|---|---|---|---|
| **Purpose** | Maximum safety, observe-first | Sensible default | Responsiveness under load | Small-RAM survival | Interactive desktop | Headless stability | Heavy build workloads |
| **Target system** | Any/unsure | Most systems | Powerful desktop | ≤4–8 GB RAM | Workstation | Server/homelab | Dev machine |
| **Default polling interval** | 10s idle / 4s pressure | 8s / 3s | 6s / 2s | 6s / 2s | 8s / 3s | 10s / 4s | 6s / 2s |
| **Thresholds** | High (act late) | Medium | Medium-low (act earlier) | Low (act early on memory) | Medium | Medium-high | Medium-low |
| **Allowed action risk** | Safe only | Safe + Moderate (dry-run default) | Safe + Moderate + (opt-in) Aggressive | Safe + Moderate + zram recommend | Safe + Moderate | Safe + Moderate | Safe + Moderate |
| **Blocked actions** | All moderate+ unless overridden | Aggressive | none extra (still gated by flags) | service stop | service stop/restart | desktop-specific niceness | service stop |
| **zram behavior** | Recommend only | Recommend only | Recommend; apply if flag | Recommend strongly; apply if flag | Recommend | Recommend | Recommend |
| **cgroup behavior** | None | MemoryHigh/CPUWeight on allowed | + MemoryMax (gated) | MemoryHigh aggressive on allowed | CPUWeight for responsiveness | MemoryHigh/IOWeight | IOWeight/CPUWeight for builds |
| **Priority behavior** | None | nice on allowed heavy procs | nice + ionice | nice + ionice | nice foreground priority | minimal | nice/ionice for build tools |
| **Recommended use case** | First install / cautious users | Default for everyone | Gamers/power users opting in | Netbooks, VMs, old laptops | Daily-driver desktops | Always-on servers | CI-like local builds |

> Even `performance` and `server` keep destructive actions prohibited and aggressive actions behind explicit flags. Profiles raise *permitted* risk; they never remove safety gates.

---

## 12. Configuration Design

Path: `/etc/syswarden/config.toml` (override with `SYSWARDEN_CONFIG` env or `--config`). Missing file → built-in `conservative` defaults with `dry_run = true`.

### Full TOML example

```toml
# /etc/syswarden/config.toml

[global]
profile = "balanced"            # conservative | balanced | performance | low_ram | desktop | server | developer
dry_run = true                  # MASTER SAFETY SWITCH. true = never change system state.
allow_aggressive_actions = false
allow_zram_apply = false
allow_sysctl_apply = false
log_level = "info"              # error | warn | info | debug | trace

[polling]
idle_interval_secs = 8
pressure_interval_secs = 3
min_interval_secs = 2
max_interval_secs = 30
hysteresis_ticks = 3            # consecutive ticks required to change state

[pressure.thresholds]
# PSI avg10/avg60 percentage thresholds per sub-level (0-100).
cpu_moderate = 15.0
cpu_high = 35.0
cpu_critical = 60.0
mem_some_moderate = 10.0
mem_full_high = 5.0
mem_full_critical = 20.0
io_moderate = 15.0
io_high = 35.0
io_critical = 60.0
mem_available_low_pct = 10.0    # MemAvailable below this % reinforces memory pressure

[protected]
# These are NEVER touched, regardless of state or flags.
processes = [
  "systemd", "systemd-journald", "systemd-logind", "dbus-daemon",
  "init", "sshd", "agetty", "syswarden",
]
services = [
  "systemd-journald.service", "systemd-logind.service",
  "dbus.service", "sshd.service", "syswarden.service",
]

[allowed]
# Only these services may receive resource-control changes.
services = []                   # e.g. ["myapp.service", "nightly-build.service"]

[[process_rules]]
match = "chromium"              # substring/comm match
max_cpu_pct = 85.0
max_rss_mb = 6000
sustained_secs = 30
on_violation = "recommend_nice" # recommend_nice | recommend_ionice | flag_only

[[service_rules]]
match = "nightly-build.service"
cpu_weight = 50                 # applied only if allowed + not dry_run
io_weight = 50
memory_high_mb = 4000

[history]
backend = "jsonl"               # v0.1 fixed to jsonl
dir = "/var/lib/syswarden/history"
retention_days = 14
max_file_mb = 32

[logging]
audit_dir = "/var/lib/syswarden/audit"
journald = true

[rollback]
dir = "/var/lib/syswarden/rollback"
keep_entries = 100
```

### Required configuration coverage
global settings; selected profile; `dry_run`; `allow_aggressive_actions`; `allow_zram_apply`; `allow_sysctl_apply`; polling settings; protected processes; protected services; allowed services; process rules; service rules; pressure thresholds; history settings; logging settings; rollback settings.

---

## 13. CLI Design

Binary: `syswarden`. Global options: `--config <path>`, `--json`, `--profile <name>` (override), `-v/-vv` (verbosity), `--dry-run/--no-dry-run` (override; `--no-dry-run` still subject to all gates).

| Command | Purpose | Key output | Example |
|---|---|---|---|
| `syswarden status` | One-shot health + pressure summary | State, PressureLevel, top contributors | `syswarden status` |
| `syswarden analyze` | Full one-shot analysis without acting | Metrics, pressure, flagged procs/services, recommendations | `syswarden analyze --json` |
| `syswarden doctor` | Environment/capability check | PSI?, cgroup v2?, systemd?, root?, zram? + advice | `syswarden doctor` |
| `syswarden daemon` | Run the supervision loop (foreground) | Streaming logs | `syswarden daemon` |
| `syswarden logs` | Show recent audit/log entries | Recent `AuditEvent`s | `syswarden logs --since 1h` |
| `syswarden explain` | Explain the latest/given decision | `Explanation` text | `syswarden explain` |
| `syswarden pressure` | Show PSI breakdown | CPU/mem/io some/full avg | `syswarden pressure` |
| `syswarden processes` | List/flag heavy processes | `ProcessInfo` table | `syswarden processes --top 10` |
| `syswarden services` | List/flag services | `ServiceInfo` table | `syswarden services --failed` |
| `syswarden profile list` | List built-in profiles | Names + summaries | `syswarden profile list` |
| `syswarden profile set <name>` | Persist selected profile to config | Confirmation | `syswarden profile set low_ram` |
| `syswarden config show` | Print effective config | Resolved `AppConfig` | `syswarden config show --json` |
| `syswarden config validate` | Validate config file | OK / list of errors | `syswarden config validate` |
| `syswarden actions dry-run` | Plan actions for current state, no apply | `PlannedAction`s + safety verdicts | `syswarden actions dry-run` |
| `syswarden actions apply` | Apply currently-planned safe/permitted actions | `ActionResult`s | `syswarden actions apply` |
| `syswarden zram status` | Show zram/zswap/swap state | Detection report | `syswarden zram status` |
| `syswarden zram recommend` | Compute zram recommendation | Suggested size/algorithm | `syswarden zram recommend` |
| `syswarden zram apply` | Apply zram config (gated) | Result + rollback id | `syswarden zram apply` |
| `syswarden rollback list` | List rollback entries | `RollbackEntry` table | `syswarden rollback list` |
| `syswarden rollback apply <id>` | Revert a recorded action | Result | `syswarden rollback apply 42` |
| `syswarden report` | Aggregated report over a window | Pressure/action summary | `syswarden report --days 7` |
| `syswarden version` | Print version + build info | Version string | `syswarden version` |

All apply-type commands respect `dry_run` and the safety layer. `--no-dry-run` never bypasses protected sets, allowlists, or prohibited-action rules.

---

## 14. Repository Structure

```
syswarden/
├── Cargo.toml
├── README.md
├── architecture.md
├── planning.md
├── LICENSE
├── src/
│   ├── main.rs
│   ├── cli.rs
│   ├── daemon.rs
│   ├── error.rs
│   ├── config/
│   │   └── mod.rs
│   ├── metrics/
│   │   ├── mod.rs
│   │   ├── memory.rs
│   │   ├── cpu.rs
│   │   └── io.rs
│   ├── pressure/
│   │   └── mod.rs
│   ├── processes/
│   │   └── mod.rs
│   ├── services/
│   │   └── mod.rs
│   ├── systemd/
│   │   └── mod.rs
│   ├── cgroups/
│   │   └── mod.rs
│   ├── zram/
│   │   └── mod.rs
│   ├── policy/
│   │   └── mod.rs
│   ├── actions/
│   │   └── mod.rs
│   ├── safety/
│   │   └── mod.rs
│   ├── profiles/
│   │   └── mod.rs
│   ├── history/
│   │   └── mod.rs
│   ├── rollback/
│   │   └── mod.rs
│   ├── logging/
│   │   └── mod.rs
│   ├── explain/
│   │   └── mod.rs
│   ├── reports/
│   │   └── mod.rs
│   └── sysctl/
│       └── mod.rs
├── tests/
│   ├── config_tests.rs
│   ├── pressure_tests.rs
│   ├── policy_tests.rs
│   ├── safety_tests.rs
│   ├── actions_dryrun_tests.rs
│   ├── rollback_tests.rs
│   └── daemon_smoke_tests.rs
├── docs/
│   ├── usage.md
│   └── adr/
├── packaging/
│   ├── systemd/
│   │   └── syswarden.service
│   └── aur/
│       └── PKGBUILD
└── examples/
    ├── config.balanced.toml
    ├── config.low_ram.toml
    └── fixtures/
        ├── proc_meminfo.sample
        ├── proc_stat.sample
        ├── pressure_cpu.sample
        ├── pressure_memory.sample
        └── pressure_io.sample
```

---

## 15. Data Contracts

Conceptual definitions. Implementers must keep these names and field meanings. Types are Rust-oriented; exact numeric widths may be chosen sensibly (prefer `u64` for counters, `f64` for PSI percentages).

- **AppConfig** — Root config. Fields: `global: GlobalConfig`, `polling: PollingConfig`, `thresholds: PressureThresholds`, `protected: ProtectedSets`, `allowed: AllowedSets`, `process_rules: Vec<ProcessRule>`, `service_rules: Vec<ServiceRule>`, `history: HistoryConfig`, `logging: LoggingConfig`, `rollback: RollbackConfig`. Purpose: full effective configuration.
- **GlobalConfig** — `profile: ProfileName`, `dry_run: bool`, `allow_aggressive_actions: bool`, `allow_zram_apply: bool`, `allow_sysctl_apply: bool`, `log_level: String`. Purpose: master switches.
- **ProfileName** — Enum: `Conservative | Balanced | Performance | LowRam | Desktop | Server | Developer`. Purpose: select a profile bundle.
- **ProfileConfig** — Resolved thresholds, permitted `ActionRisk`, polling, zram/cgroup/priority behavior. Purpose: runtime behavior bundle.
- **MetricsSnapshot** — `timestamp`, `memory: MemoryMetrics`, `cpu: CpuMetrics`, `io: IoMetrics`. Purpose: one collection tick.
- **MemoryMetrics** — `total_kb`, `available_kb`, `free_kb`, `buffers_kb`, `cached_kb`, `swap_total_kb`, `swap_used_kb`, `swap_in_rate`, `swap_out_rate`. Purpose: real memory state (note: `available`, not `free`, drives decisions).
- **CpuMetrics** — `utilization_pct`, `load1`, `load5`, `load15`, `num_cpus`. Purpose: CPU load context.
- **IoMetrics** — `io_wait_pct`, optional per-device pressure. Purpose: I/O context.
- **PressureSnapshot** — `timestamp`, `cpu: PsiMetrics`, `memory: PsiMetrics`, `io: PsiMetrics`, `level: PressureLevel`, `contributors: Vec<String>`. Purpose: classified pressure with rationale.
- **PsiMetrics** — `some_avg10`, `some_avg60`, `some_avg300`, `full_avg10`, `full_avg60`, `full_avg300`, `total_us`. Purpose: parsed PSI for one resource.
- **ProcessInfo** — `pid`, `comm`, `cmdline`, `cpu_pct`, `rss_kb`, `io_read_rate`, `io_write_rate`, `nice`, `is_protected`, `flags: Vec<ProcessFlag>`. Purpose: per-process analysis.
- **ServiceInfo** — `unit`, `active_state`, `sub_state`, `is_protected`, `is_allowed`, `cpu_usage`, `memory_current`, `restarts`, `flags: Vec<ServiceFlag>`. Purpose: per-service analysis.
- **SystemState** — Enum matching §7. Purpose: current supervision state.
- **PressureLevel** — Enum: `None | Low | Moderate | High | Critical`. Purpose: scalar pressure classification.
- **PolicyDecision** — `intent: DecisionIntent`, `targets: Vec<Target>`, `rationale: String`. `DecisionIntent` enumerates the §9 decisions. Purpose: decision intent.
- **PlannedAction** — `id`, `kind: ActionKind`, `risk: ActionRisk`, `target`, `params`, `explanation: String`. Purpose: a concrete intended change.
- **ActionKind** — Enum: `Observe | Log | Report | Recommend | AdjustNice | AdjustIonice | SetCpuWeight | SetIoWeight | SetMemoryHigh | SetMemoryMax | RestartService | StopService | ApplyZram | ApplySysctl | CreateBackup`. Purpose: action type.
- **ActionRisk** — Enum: `Safe | Moderate | Aggressive | Prohibited`. Purpose: gating classification.
- **ActionStatus** — Enum: `Planned | Simulated | Blocked | Executed | Failed | RolledBack`. Purpose: lifecycle state.
- **ActionResult** — `action_id`, `status: ActionStatus`, `message`, `rollback_id: Option<...>`. Purpose: outcome.
- **SafetyDecision** — Enum: `Allow | Block { reason: String } | RequireDryRun`. Purpose: gate verdict.
- **RollbackEntry** — `id`, `timestamp`, `action_kind`, `target`, `prior_state`, `reversible: bool`. Purpose: revert metadata.
- **AuditEvent** — `timestamp`, `kind` (decision/action/block/error), `state`, `pressure_level`, `detail`, `result`. Purpose: append-only accountability record.
- **HistoryRecord** — `timestamp`, `pressure_level`, `psi_summary`, `state`, `action_count`, `outcomes`. Purpose: trend + reporting.
- **Explanation** — `summary: String`, `reasons: Vec<String>`, `evidence: Vec<String>`. Purpose: human-readable rationale.

---

## 16. Rust Crate Strategy

| Crate | Why | Where | Risks | Alternatives | v0.1? |
|---|---|---|---|---|---|
| `clap` (derive) | Robust CLI parsing/help | cli | API churn across majors | hand-rolled parser | Required |
| `serde` + `serde_json` | Config + JSONL (de)serialization | config, history, rollback, audit | none significant | manual | Required |
| `toml` | Parse `config.toml` | config | none significant | `toml_edit` | Required |
| `anyhow` | Ergonomic app-level errors w/ context | binaries, cli, daemon | hides types if overused | `eyre` | Required |
| `thiserror` | Typed library error enums | error.rs, modules | none | manual `impl` | Required |
| `tracing` + `tracing-subscriber` | Structured logging + journald-friendly | logging | config complexity | `log`+`env_logger` | Required |
| `tokio` (rt, time, signal, macros) | Async runtime, timers, signal handling | daemon | overhead if misused; keep features minimal | `async-std`, threads + `signal-hook` | Required |
| `procfs` | Safe `/proc` parsing | metrics, processes | parsing edge cases | raw parse | Recommended (fallback raw) |
| `nix` | `nice`/`ionice`/priorities/signals | actions | unsafe surface | `libc` directly | Required for actions phase |
| `libc` | Low-level syscalls when needed | actions, daemon | unsafe | `nix` | As needed |
| `zbus` | systemd D-Bus (read + drop-ins) | services, systemd | D-Bus complexity | shell out to `systemctl` (audited fallback) | Deferred to service/systemd phases |
| `chrono` | Timestamps, windows, retention | history, reports, logging | tz pitfalls | `time` | Required (pick one: `chrono`) |
| `directories` | XDG/state path resolution | config, history | minor | hardcoded paths | Optional |
| `rusqlite` / `sled` | History backend | history | added weight/complexity | **JSONL (chosen)** | Deferred (JSONL for v0.1) |

**Decisions:** Use JSONL (via `serde_json`) for history/audit/rollback in v0.1; defer SQLite/sled. Prefer `procfs` with a raw-parse fallback. Prefer `zbus` over shelling out, but allow an explicit, audited `systemctl`/`systemd-run`/`zramctl` fallback documented as such. Choose `chrono` over `time` for consistency. Keep `tokio` feature set minimal (`rt`, `macros`, `time`, `signal`).

---

## 17. Security Architecture

- **Privilege model** — Runs with the least privilege needed. Analysis works as non-root. State-changing actions require root (typically as a root systemd service). The daemon detects and records its privilege level each tick.
- **Root vs non-root behavior** — Non-root → observe/recommend only; all state-changing actions auto-blocked with a clear reason. Root → actions still fully gated by config flags, profile, allowlists, and the safety layer.
- **Protected processes** — Never re-niced, ioniced, signaled, or stopped. Defaults include init, journald, logind, dbus, sshd, and `syswarden` itself.
- **Protected services** — Never limited/stopped/restarted; defaults mirror protected processes.
- **Allowlists** — Resource-control changes apply only to services in `allowed.services`. Empty allowlist ⇒ no service is modifiable.
- **Denylists** — Protected sets act as hard denylists overriding any rule.
- **Config validation** — Strict parsing; unknown destructive directives rejected. No field can request a prohibited action. `config validate` reports all problems.
- **Dry-run behavior** — `dry_run = true` is the default and master switch. Under dry-run, no executor side effects occur; only simulation, explanation, audit, and history.
- **Action gating** — Order of gates: (1) prohibited? → block; (2) risk permitted by profile? (3) required flags set? (4) target in allowlist / not protected? (5) dry-run? → simulate. All must pass to execute.
- **Backup policy** — Any aggressive change that edits persistent state (sysctl, zram unit) first writes a backup recorded in the rollback store.
- **Rollback policy** — Each executed reversible action records prior state; `rollback apply <id>` restores it. Irreversible actions are not implemented in v0.1.
- **Audit events** — Every decision, block, execution, failure, and rollback is appended to the audit log.
- **Fail-safe behavior** — On any uncertainty, error, or invalid config: fail closed (block actions, enter `protected_mode`/`degraded`), never fail open.
- **Error handling philosophy** — Libraries return typed errors (`thiserror`); the daemon converts them to logged, recoverable outcomes (`anyhow` context) and continues. No `unwrap`/`expect` on runtime paths.
- **Command execution restrictions** — No shell interpolation. External tools (if ever used as fallback) are invoked with explicit argument vectors, never through a shell, never with config-derived strings as code.
- **No network policy** — syswarden opens no network sockets and makes no outbound connections. This is an invariant; the daemon links no networking crates. The invariant targets *networking* only. Two local `AF_UNIX` uses are explicitly permitted and are **not** "network": (1) systemd D-Bus for read/write of unit properties, and (2) the systemd readiness/watchdog protocol via `$NOTIFY_SOCKET` (`sd_notify`: `READY=1`, `WATCHDOG=1`). The watchdog datagram is sent with `std` (`UnixDatagram`) only — no `libsystemd`, no networking crate. The unit's `RestrictAddressFamilies=AF_UNIX` and `IPAddressDeny=any` enforce this boundary at the kernel level.
- **No shell injection policy** — Config values are data, never executed. There is no "run this command" config field.

---

## 18. systemd Integration

- **Daemon service file design** — `packaging/systemd/syswarden.service`, `Type=notify`, `ExecStart=/usr/bin/syswarden daemon`, `Restart=on-failure`, `RestartSec=5`.
- **Liveness / watchdog** — `Type=notify`: the daemon sends `READY=1` once initialization completes (capabilities detected, stores opened, signal handlers installed). With `WatchdogSec` set, the daemon pings `WATCHDOG=1` from inside the supervision loop after each completed tick (never from a side task — a hung loop must stop pinging). `WatchdogSec` is kept comfortably above the maximum adaptive poll interval to avoid false positives on long healthy sleeps; `Restart=on-watchdog` restarts a wedged daemon. Implemented with `std` `UnixDatagram` on `$NOTIFY_SOCKET` only (§17 "No network policy" exception); missing socket ⇒ silent no-op (foreground/non-systemd runs).
- **Service hardening** — `NoNewPrivileges=yes`, `ProtectSystem=strict`, `ProtectHome=yes`, `PrivateTmp=yes`, `ProtectKernelTunables=yes` (relaxed only if/when sysctl apply is explicitly enabled), `ProtectControlGroups=no` (needs cgroup access to govern), `RestrictAddressFamilies=AF_UNIX` (D-Bus only, no network), `IPAddressDeny=any`, `SystemCallFilter=@system-service`, `MemoryDenyWriteExecute=yes`, `ReadWritePaths=/var/lib/syswarden`.
- **Restart behavior** — Restart on failure with backoff; do not restart-loop aggressively (`StartLimitIntervalSec`/`StartLimitBurst`).
- **Logging to journald** — Log to stderr; journald captures it. `journald = true` keeps audit JSONL in addition for structured history.
- **Permissions** — Runs as root for actions, but hardened. A future non-acting "observe" unit may run as a dedicated unprivileged user.
- **Possible capabilities** — Minimize; prefer running as root with sandboxing over broad capability grants. If narrowed later: `CAP_SYS_NICE` (priorities), `CAP_SYS_RESOURCE`. Document before granting.
- **Installation path** — Binary `/usr/bin/syswarden`; unit `/usr/lib/systemd/system/syswarden.service`.
- **Config path** — `/etc/syswarden/config.toml`.
- **State path** — `/var/lib/syswarden/` (history, audit, rollback).
- **Cache path** — `/var/cache/syswarden/` (optional, transient).
- **systemd drop-in strategy** — Resource control applied as drop-ins under `/etc/systemd/system/<unit>.d/syswarden.conf` (or transient via D-Bus `SetUnitProperties`), captured for rollback before writing, then `daemon-reload`.
- **Resource control strategy** — Prefer transient runtime properties for temporary pressure response; persistent drop-ins only for explicitly configured service rules.
- **Inspect logs** — `journalctl -u syswarden -f`; `syswarden logs`.
- **Disable and uninstall** — `systemctl disable --now syswarden`; remove unit, binary, `/etc/syswarden`, and (optionally) `/var/lib/syswarden`; remove any syswarden drop-ins under `/etc/systemd/system/*.d/`.

---

## 19. zram/zswap Strategy

- **When to recommend zram** — Genuine, sustained memory pressure (memory PSI + low `MemAvailable` + rising swap-in) with no or undersized swap, especially on low-RAM systems.
- **When not to recommend zram** — No memory pressure; pressure is purely CPU/I/O; ample free + available memory; cache use mistaken for pressure.
- **When zswap is preferred** — When a real backing swap device/file already exists and the goal is to reduce I/O to it via a compressed cache. zswap is a front-cache for existing swap; zram is its own compressed block device.
- **When neither should be touched** — Unknown/atypical setups, hibernation configured on swap, or when the user has not granted apply permission.
- **Detect existing swap/zram/zswap** — Read `/proc/swaps`, `/sys/block/zram*`, `/sys/module/zswap/parameters/*`. Report current state before any recommendation.
- **Recommended sizing strategy** — Conservative default: zram size ≈ 25–50% of RAM (cap on low-RAM to avoid over-commit), only as a recommendation. Never auto-grow.
- **Compression algorithm strategy** — Prefer `zstd` (good ratio/speed) where available, else `lz4`. Recommend, don't force.
- **CPU tradeoffs** — Compression costs CPU; under simultaneous high CPU pressure, weigh the tradeoff and explain it. Do not recommend large zram on CPU-starved systems.
- **Rollback** — Applying zram records prior swap/zram state; rollback can `swapoff` the syswarden-created device and restore previous configuration.
- **Arch-specific integration** — Recommend the `zram-generator` approach for persistence; syswarden can generate a suggested `zram-generator` config but applies it only with explicit permission.
- **What must require explicit user permission** — Any `zram apply`/`zswap apply` requires `allow_zram_apply = true` (and a dedicated flag for zswap), is never combined blindly, and always records rollback metadata.

---

## 20. Local History Store

- **Chosen backend** — **JSONL** (newline-delimited JSON) for v0.1. Simple, append-only, human-inspectable, no native deps. SQLite/sled deferred to a later version if query needs grow.
- **Stored records** — `HistoryRecord` (pressure/state/action summaries per tick or per significant change), `AuditEvent` (separate audit JSONL), `RollbackEntry` (separate rollback JSONL).
- **Retention policy** — Time-based (`retention_days`) plus size cap (`max_file_mb`); files rotate by date.
- **Cleanup policy** — On startup and periodically, prune records older than retention and rotate oversized files; keep at most `keep_entries` rollback entries.
- **Schema** — One JSON object per line with a `schema_version` field for forward compatibility.
- **Privacy** — Local only; stores metrics and unit/process names, never file contents or network data; never transmitted anywhere.
- **How history affects decisions** — Provides the short trend used by the historical pressure model and hysteresis (distinguishing transient spikes from sustained pressure) and powers `report`. History never directly triggers an action; it only informs classification.

---

## 21. Observability and Explainability

- **Log levels** — `error`, `warn`, `info`, `debug`, `trace` via `tracing`; configurable in `[global].log_level` and `-v` flags.
- **Audit log format** — JSONL `AuditEvent`s in `/var/lib/syswarden/audit/`, one event per line, with timestamp, kind, state, pressure level, detail, and result.
- **`explain` command** — Renders the latest (or specified) decision's `Explanation`: summary, reasons, and the evidence metrics behind it.
- **`report` command** — Aggregates history over a window: pressure distribution, top contributors, actions taken/simulated/blocked.
- **User-readable action explanations** — Every `PlannedAction` carries an `explanation` string stating what, why (which metric/threshold), and its risk and reversibility.
- **Debugging mode** — `-vv` / `log_level = "trace"` exposes raw PSI values, parsing details, and per-gate safety decisions.
- **Performance overhead monitoring for the daemon itself** — The daemon samples its own CPU/RSS each tick and logs a warning if it exceeds a small configured budget, ensuring it stays lightweight.

---

## 22. Limitations

syswarden explicitly cannot:
- create physical RAM;
- fix all memory leaks (it can flag suspects, not repair them);
- safely optimize every process (many are protected or unknowable);
- guarantee universal performance improvement;
- replace hardware upgrades;
- blindly disable services;
- replace human review for aggressive changes.

---

## 23. Roadmap

- **v0.1 (MVP)** — Observe-only intelligence: config, CLI, metrics, PSI, pressure classification, process/service analysis, profiles, policy engine, safety layer, dry-run action planning, daemon loop, JSONL history + audit, rollback metadata scaffolding, systemd unit (manual install). No real state changes by default.
- **v0.2 (safe actions)** — Enable moderate actions behind flags: `nice`/`ionice` on allowed non-protected processes; `CPUWeight`/`IOWeight`/`MemoryHigh` on allowlisted services; full rollback for these.
- **v0.3 (systemd/cgroup control + gated aggressive actions)** — Mature transient + persistent drop-in management; richer cgroup reads; service rule engine; and the gated *aggressive* actions: `MemoryMax` (only after `MemoryHigh` has been tried), allowlisted service restart/stop, and `sysctl` apply (with backup + rollback). All stay off by default and behind `allow_aggressive_actions` / `allow_sysctl_apply`; "battle-tested" hardening of the aggressive set remains the v1.0 bar.
- **v0.4 (zram/zswap management)** — Detection-first zram/zswap recommendations and gated apply with rollback; `zram-generator` integration.
- **v1.0 (stable)** — Hardened, documented, AUR-packaged; aggressive actions fully gated and battle-tested; comprehensive tests.
- **Future ideas** — Optional local-only trend learning (still deterministic and explainable), per-cgroup adaptive weights, TUI dashboard, additional distro support — none involving networked or generative AI.

---

## 24. ADR Summary

**ADR-001 — Language: Rust**
- *Decision:* Implement primarily in Rust. *Status:* Accepted. *Context:* Need low overhead, memory safety, predictable performance. *Options:* Rust, Go, C, Python. *Chosen:* Rust. *Consequences:* No GC, safety invariants in types; steeper learning curve, longer build times.

**ADR-002 — Primary signal: PSI over raw RAM**
- *Decision:* Use PSI as the primary pressure signal. *Status:* Accepted. *Context:* Raw "used RAM" misleads due to caching. *Options:* RAM thresholds, load average, PSI. *Chosen:* PSI (cross-checked). *Consequences:* Requires `CONFIG_PSI`; degrade gracefully when absent.

**ADR-003 — Resource governance via systemd/cgroups v2**
- *Decision:* Govern resources through systemd resource control. *Status:* Accepted. *Context:* Native, reversible, supported. *Options:* Direct cgroup writes, systemd, `ulimit`. *Chosen:* systemd (drop-ins/transient). *Consequences:* Depends on systemd; integrates cleanly on Arch.

**ADR-004 — Default mode: dry-run, observe-only**
- *Decision:* Ship with `dry_run = true`. *Status:* Accepted. *Context:* Safety first. *Options:* Act by default, recommend by default, observe by default. *Chosen:* Observe/dry-run. *Consequences:* Zero risk out of the box; users must opt in to actions.

**ADR-005 — History backend: JSONL for v0.1**
- *Decision:* Use JSONL for history/audit/rollback. *Status:* Accepted. *Context:* Simplicity, no native deps, human-readable. *Options:* SQLite, sled, JSONL. *Chosen:* JSONL. *Consequences:* Limited query power; revisit if needed.

**ADR-006 — No network, no external AI, no telemetry**
- *Decision:* Fully offline and deterministic. *Status:* Accepted. *Context:* Privacy, predictability, low overhead. *Options:* Cloud features, local model, none. *Chosen:* None. *Consequences:* No remote features; all logic auditable.

**ADR-007 — Mandatory safety layer for every action**
- *Decision:* Route all actions through one fail-closed safety gate. *Status:* Accepted. *Context:* Prevent accidental unsafe behavior. *Options:* Per-module checks, central gate. *Chosen:* Central gate. *Consequences:* Single enforcement point; all actions must integrate with it.

**ADR-008 — Async runtime: tokio (minimal features)**
- *Decision:* Use `tokio` with a minimal feature set. *Status:* Accepted. *Context:* Need timers + signals + clean shutdown. *Options:* threads + `signal-hook`, async-std, tokio. *Chosen:* tokio (rt, macros, time, signal). *Consequences:* Async dependency; kept minimal to preserve low overhead.
