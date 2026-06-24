You are Claude Opus acting as the Principal Architect, Linux Systems Engineer, and Technical to implement code now.

Your task is to think deeply, make the final architectural decisions, create the project documentation, and write two authoritative files that future implementation models such as Claude Sonnet and GPT-5.5 must follow exactly.

You are the architect.
Future models are implementers.
They must not redesign, rename, simplify, reinterpret, or override your architecture.

============================================================
MANDATORY LANGUAGE RULE
=======================

All generated project artifacts must be written entirely in English.

This includes:

* project folder name;
* file names;
* architecture.md;
* planning.md;
* headings;
* explanations;
* technical decisions;
* module names;
* CLI names;
* config examples;
* comments;
* code examples;
* prompts for future models;
* commit messages;
* logs;
* tests;
* documentation;
* implementation instructions.

Do not write any project artifact in Portuguese.
Do not mix Portuguese and English inside the generated files.
Use clear, professional, precise technical English.

============================================================
MISSION
=======

Create a complete architecture and implementation plan for a real Arch Linux optimization daemon written primarily in Rust.

The application must run continuously as a low-overhead local system daemon, monitor real system pressure, and safely optimize system responsiveness, resource usage, and stability.

The project must be technically realistic.

It must not be a fake “RAM cleaner”.
It must not promise impossible performance gains.
It must not fight Linux cache management.
It must not run a heavy AI model in the background.
It must not use paid APIs.
It must not require internet access.
It must not execute destructive system changes during this planning task.

The correct concept is:

A local Rust-based Linux resource optimization and supervision daemon for Arch Linux, using PSI, cgroups v2, systemd resource control, zram/zswap awareness, process analysis, service analysis, safety policies, dry-run execution, rollback, profiles, and explainable logs.

============================================================
PROJECT NAMING
==============

Choose a creative, professional, memorable project name.

The name should suggest protection, optimization, system guardianship, Linux control, or resource intelligence.

Examples of naming direction:

* arch-aegis;
* arch-sentinel;
* kernelwarden;
* resource-aegis;
* syswarden;
* memguard;
* loadwarden;
* pressureguard.

You must choose the final name yourself.

After choosing the name:

1. Create a folder in the current directory.
2. The folder name must be lowercase, Linux-friendly, hyphenated if needed, and without spaces.
3. Use the chosen name consistently across all documentation.

============================================================
STRICT FILE OUTPUT REQUIREMENTS
===============================

Inside the created project folder, create exactly these two files:

1. architecture.md
2. planning.md

Do not create source code files yet.
Do not create Cargo.toml yet.
Do not install packages.
Do not modify anything outside the created project folder.
Do not change system configuration.
Do not execute optimization commands.
Do not edit /etc, /usr, /boot, /var, or systemd units on the host.

This run is documentation-only.

The two files must be complete enough for future models to implement the project without making new architectural decisions.

============================================================
QUALITY MODE
============

Before finalizing the files, perform three explicit quality review passes:

Review Pass 1: Technical Correctness

* Verify that the architecture is realistic for Arch Linux.
* Verify that the design respects Linux memory management.
* Verify that PSI, cgroups, systemd, zram/zswap, and process monitoring are used correctly.
* Remove any impossible or misleading claims.

Review Pass 2: Safety and Reversibility

* Verify that destructive actions are prohibited by default.
* Verify that dry-run, backups, rollback, audit logs, protected processes, and protected services are included.
* Verify that implementation models cannot accidentally create unsafe behavior.

Review Pass 3: Implementability

* Verify that future models can implement the project step by step.
* Verify that planning.md contains exact implementation order, file responsibilities, data contracts, tests, acceptance criteria, and prompts.
* Remove ambiguity.
* Add missing details.

After each pass, improve the files if needed.
At the end, include a short final report in the terminal response summarizing the three review passes.

============================================================
TECHNICAL FOUNDATIONS
=====================

The architecture must evaluate and use these concepts where appropriate:

Core platform:

* Arch Linux;
* Linux kernel;
* systemd;
* cgroups v2;
* systemd resource control;
* journald;
* /proc;
* /sys;
* sysctl;
* zram;
* zswap;
* swap;
* nice;
* ionice;
* process priorities;
* service-level resource limits.

Metrics and pressure:

* /proc/pressure/cpu;
* /proc/pressure/memory;
* /proc/pressure/io;
* /proc/meminfo;
* /proc/stat;
* /proc/loadavg;
* /proc/[pid];
* memory available;
* buffers;
* cache;
* swap usage;
* swap in/out;
* CPU utilization;
* I/O wait;
* process CPU;
* process memory;
* process I/O;
* service state;
* historical pressure trends.

Rust ecosystem candidates:

* clap;
* serde;
* toml;
* anyhow;
* thiserror;
* tracing;
* tracing-subscriber;
* tokio;
* sysinfo;
* procfs;
* nix;
* libc;
* zbus or zbus_systemd;
* chrono or time;
* directories;
* rusqlite, sled, or JSONL for local history.

You must decide which crates should be used and which should be avoided or deferred.

============================================================
IMPORTANT TECHNICAL REALITY CHECKS
==================================

The documentation must clearly state:

1. Linux using RAM for cache is normal and usually good.
2. The project must focus on real pressure and responsiveness, not cosmetic “free RAM”.
3. A global garbage collector for Linux processes does not exist.
4. Rust does not require a traditional garbage collector.
5. Constant cache dropping is usually harmful.
6. Killing processes automatically is dangerous and must be prohibited by default.
7. zram can help under memory pressure but is not magic.
8. zswap and zram must be handled carefully and should not be blindly combined.
9. systemd/cgroups are appropriate for resource governance.
10. PSI is a better signal than raw RAM usage for pressure-aware decisions.
11. The daemon itself must remain lightweight.
12. The project should optimize for responsiveness, stability, and controlled degradation.

============================================================
architecture.md REQUIREMENTS
============================

Create architecture.md as the definitive architecture source of truth.

It must include the following sections.

# 1. Project Overview

Include:

* final project name;
* short description;
* long description;
* problem statement;
* target users;
* target systems;
* non-goals;
* project philosophy;
* success definition.

# 2. Core Principles

Define principles such as:

* safety before aggressiveness;
* pressure-aware optimization;
* explainability;
* reversibility;
* low overhead;
* offline-first;
* deterministic local intelligence;
* no paid APIs;
* no external AI dependency;
* no destructive defaults;
* no fake RAM cleaning;
* no blind automation;
* no architecture drift.

# 3. Technical Approach

Explain why the project uses:

* Rust;
* systemd;
* cgroups v2;
* PSI;
* /proc and /sys;
* zram/zswap awareness;
* profiles;
* policy engine;
* safety layer;
* dry-run;
* rollback;
* local history;
* audit logging.

Also explain why the project does not use:

* a continuously running generative AI model;
* aggressive cache clearing;
* global garbage collection;
* random process killing;
* package removal automation;
* irreversible tuning;
* opaque optimization scripts.

# 4. High-Level Architecture

Describe all major components:

* CLI;
* daemon;
* config manager;
* metrics collector;
* PSI collector;
* process analyzer;
* service analyzer;
* systemd manager;
* cgroup manager;
* zram/zswap manager;
* policy engine;
* action planner;
* action executor;
* safety layer;
* profile manager;
* rollback manager;
* local history store;
* audit logger;
* explainability engine;
* reporting layer.

Include a text-based architecture diagram.

# 5. Module-by-Module Architecture

For each module, define:

* responsibility;
* inputs;
* outputs;
* internal dependencies;
* external dependencies;
* permissions required;
* failure modes;
* what the module is not allowed to decide;
* public data contracts;
* testing strategy.

Modules must include at least:

* cli;
* daemon;
* config;
* metrics;
* pressure;
* processes;
* services;
* systemd;
* cgroups;
* zram;
* policy;
* actions;
* safety;
* profiles;
* rollback;
* history;
* logging;
* explain;
* reports.

# 6. Daemon Lifecycle

Describe:

* startup;
* configuration loading;
* privilege detection;
* environment validation;
* metric collection;
* pressure calculation;
* process analysis;
* service analysis;
* system state classification;
* policy evaluation;
* action planning;
* safety validation;
* dry-run or execution;
* audit logging;
* local history write;
* adaptive sleep interval;
* graceful shutdown;
* error recovery.

Include detailed pseudocode for the main daemon loop.

# 7. System States

Define:

* initializing;
* idle;
* healthy;
* moderate_pressure;
* high_pressure;
* critical_pressure;
* recovery;
* degraded;
* protected_mode.

For each state, define:

* entry conditions;
* exit conditions;
* allowed actions;
* forbidden actions;
* log behavior;
* user-facing explanation.

# 8. Pressure Model

Define:

* CPU pressure model;
* memory pressure model;
* I/O pressure model;
* swap pressure model;
* service health model;
* process anomaly model;
* historical pressure model.

Explain how raw metrics become a final PressureLevel.

Define PressureLevel values:

* none;
* low;
* moderate;
* high;
* critical.

# 9. Decision Model

Explain how the application decides:

* observe only;
* log only;
* recommend;
* alert;
* adjust process priority;
* apply cgroup/systemd limits;
* recommend zram;
* apply zram only when explicitly allowed;
* enter protected mode;
* block an action;
* do nothing.

Include decision tables.

# 10. Actions and Risk Classification

Classify actions into:

Safe actions:

* observe;
* log;
* report;
* explain;
* recommend;
* detect heavy process;
* detect failing service;
* calculate zram recommendation;
* adjust daemon polling interval.

Moderate actions:

* apply nice adjustment to non-protected processes;
* apply ionice adjustment to non-protected processes;
* apply CPUWeight to allowed services;
* apply IOWeight to allowed services;
* apply MemoryHigh to allowed services;
* create backups;
* create systemd drop-in previews.

Aggressive actions:

* apply MemoryMax;
* restart explicitly allowed non-critical services;
* stop explicitly allowed non-critical services;
* change sysctl values;
* apply zram configuration;
* apply zswap configuration.

Prohibited by default:

* remove packages;
* delete files;
* kill protected processes;
* alter bootloader;
* alter kernel command line;
* alter fstab automatically;
* edit critical files without backup;
* run drop_caches loops;
* hide memory leaks;
* send telemetry externally;
* execute shell snippets from untrusted config;
* use online APIs.

# 11. Profiles

Define these profiles:

* conservative;
* balanced;
* performance;
* low_ram;
* desktop;
* server;
* developer.

For each profile include:

* purpose;
* target system;
* default polling interval;
* thresholds;
* allowed action risk;
* blocked actions;
* zram behavior;
* cgroup behavior;
* priority behavior;
* recommended use case.

# 12. Configuration Design

Define the complete configuration model for:

/etc/<project-name>/config.toml

Include a full TOML example.

The configuration must include:

* global settings;
* selected profile;
* dry_run;
* allow_aggressive_actions;
* allow_zram_apply;
* allow_sysctl_apply;
* polling settings;
* protected processes;
* protected services;
* allowed services;
* process rules;
* service rules;
* pressure thresholds;
* history settings;
* logging settings;
* rollback settings.

# 13. CLI Design

Define all CLI commands, options, expected outputs, and examples.

Must include:

* <binary> status;
* <binary> analyze;
* <binary> doctor;
* <binary> daemon;
* <binary> logs;
* <binary> explain;
* <binary> pressure;
* <binary> processes;
* <binary> services;
* <binary> profile list;
* <binary> profile set <name>;
* <binary> config show;
* <binary> config validate;
* <binary> actions dry-run;
* <binary> actions apply;
* <binary> zram status;
* <binary> zram recommend;
* <binary> zram apply;
* <binary> rollback list;
* <binary> rollback apply <id>;
* <binary> report;
* <binary> version.

# 14. Repository Structure

Define the final Rust repository tree.

Include at least:

* Cargo.toml;
* README.md;
* architecture.md;
* planning.md;
* src/main.rs;
* src/cli.rs;
* src/daemon.rs;
* src/config/mod.rs;
* src/metrics/mod.rs;
* src/metrics/memory.rs;
* src/metrics/cpu.rs;
* src/metrics/io.rs;
* src/pressure/mod.rs;
* src/processes/mod.rs;
* src/services/mod.rs;
* src/systemd/mod.rs;
* src/cgroups/mod.rs;
* src/zram/mod.rs;
* src/policy/mod.rs;
* src/actions/mod.rs;
* src/safety/mod.rs;
* src/profiles/mod.rs;
* src/history/mod.rs;
* src/rollback/mod.rs;
* src/logging/mod.rs;
* src/explain/mod.rs;
* src/reports/mod.rs;
* tests/;
* docs/;
* packaging/systemd/;
* packaging/aur/;
* examples/.

# 15. Data Contracts

Define important structs and enums conceptually.

Must include:

* AppConfig;
* GlobalConfig;
* ProfileName;
* ProfileConfig;
* MetricsSnapshot;
* MemoryMetrics;
* CpuMetrics;
* IoMetrics;
* PressureSnapshot;
* PsiMetrics;
* ProcessInfo;
* ServiceInfo;
* SystemState;
* PressureLevel;
* PolicyDecision;
* PlannedAction;
* ActionKind;
* ActionRisk;
* ActionStatus;
* ActionResult;
* SafetyDecision;
* RollbackEntry;
* AuditEvent;
* HistoryRecord;
* Explanation.

For each, list fields and purpose.

# 16. Rust Crate Strategy

List recommended crates and explain:

* why to use them;
* where to use them;
* risks;
* alternatives;
* whether they are required for v0.1 or deferred.

# 17. Security Architecture

Define:

* privilege model;
* root vs non-root behavior;
* protected processes;
* protected services;
* allowlists;
* denylists;
* config validation;
* dry-run behavior;
* action gating;
* backup policy;
* rollback policy;
* audit events;
* fail-safe behavior;
* error handling philosophy;
* command execution restrictions;
* no network policy;
* no shell injection policy.

# 18. systemd Integration

Define:

* daemon service file design;
* service hardening;
* restart behavior;
* logging to journald;
* permissions;
* possible capabilities;
* installation path;
* config path;
* state path;
* cache path;
* systemd drop-in strategy;
* resource control strategy;
* how to inspect logs;
* how to disable and uninstall.

# 19. zram/zswap Strategy

Define:

* when to recommend zram;
* when not to recommend zram;
* when zswap is preferred;
* when neither should be touched;
* how to detect existing swap/zram/zswap;
* recommended sizing strategy;
* compression algorithm strategy;
* CPU tradeoffs;
* rollback;
* Arch-specific integration;
* what must require explicit user permission.

# 20. Local History Store

Decide whether to use SQLite, JSONL, sled, or another approach.

Define:

* chosen backend;
* stored records;
* retention policy;
* cleanup policy;
* schema;
* privacy;
* how history affects decisions.

# 21. Observability and Explainability

Define:

* log levels;
* audit log format;
* explain command;
* report command;
* user-readable action explanations;
* debugging mode;
* performance overhead monitoring for the daemon itself.

# 22. Limitations

State clearly:

* cannot create physical RAM;
* cannot fix all memory leaks;
* cannot safely optimize every process;
* cannot guarantee universal performance improvement;
* cannot replace hardware upgrades;
* cannot blindly disable services;
* cannot replace human review for aggressive changes.

# 23. Roadmap

Define:

* v0.1 MVP;
* v0.2 safe actions;
* v0.3 systemd/cgroup control;
* v0.4 zram/zswap management;
* v1.0 stable release;
* future ideas.

# 24. ADR Summary

Create Architecture Decision Records in concise form.

Each ADR must include:

* decision;
* status;
* context;
* options considered;
* chosen option;
* consequences.

============================================================
planning.md REQUIREMENTS
========================

Create planning.md as the implementation contract for future models.

It must be written in imperative form.

It must tell Sonnet/GPT-5.5 exactly what to implement, in what order, without changing architecture.

It must include the following sections.

# 1. Implementer Contract

State:

* read architecture.md first;
* do not change architecture;
* do not rename modules;
* do not change public data contracts;
* do not add unsafe behavior;
* do not skip safety;
* do not add network access;
* do not add paid APIs;
* do not implement AI runtime;
* do not implement aggressive actions before safety and rollback;
* implement only the assigned phase;
* report files changed;
* run checks when possible.

# 2. Implementation Order

Give exact ordered phases:

* Phase 0: repository skeleton;
* Phase 1: Cargo setup;
* Phase 2: config model;
* Phase 3: CLI;
* Phase 4: logging;
* Phase 5: metrics collection;
* Phase 6: PSI parsing;
* Phase 7: process analysis;
* Phase 8: service analysis;
* Phase 9: pressure model;
* Phase 10: profiles;
* Phase 11: policy engine;
* Phase 12: safety layer;
* Phase 13: dry-run actions;
* Phase 14: daemon loop;
* Phase 15: local history;
* Phase 16: rollback;
* Phase 17: systemd integration;
* Phase 18: tests;
* Phase 19: packaging;
* Phase 20: documentation.

# 3. Milestones

Define:

* M0: documentation complete;
* M1: repository skeleton compiles;
* M2: CLI works;
* M3: config loads and validates;
* M4: metrics snapshot works;
* M5: PSI parser works;
* M6: pressure classification works;
* M7: policy decisions work;
* M8: safety layer blocks unsafe actions;
* M9: dry-run action planner works;
* M10: daemon loop runs safely;
* M11: history and audit logs work;
* M12: rollback metadata works;
* M13: systemd service installed manually;
* M14: v0.1 release candidate.

# 4. File-by-File Implementation Checklist

For every file in the repository structure, describe:

* purpose;
* required structs;
* required enums;
* required functions;
* dependencies;
* tests;
* acceptance criteria.

# 5. Data Contract Implementation Details

For each required struct and enum from architecture.md:

* define fields;
* define types;
* define validation rules;
* define serialization requirements;
* define default values;
* define test cases.

# 6. Error Handling Plan

Define:

* error enum structure;
* context-rich errors;
* permission-denied behavior;
* missing /proc file behavior;
* missing PSI behavior;
* non-systemd behavior;
* missing zram behavior;
* non-root behavior;
* invalid config behavior;
* failed action behavior;
* logging requirements.

# 7. Testing Plan

Define:

* unit tests;
* integration tests;
* mock metric snapshots;
* fake PSI files;
* config parsing tests;
* config validation tests;
* policy decision tests;
* safety blocking tests;
* dry-run tests;
* rollback tests;
* CLI output tests;
* daemon loop smoke test.

# 8. Acceptance Criteria

Define clear acceptance criteria per phase and per milestone.

# 9. Development Commands

Include:

* cargo check;
* cargo fmt;
* cargo clippy;
* cargo test;
* cargo build --release;
* local dry-run execution;
* local daemon foreground execution;
* log inspection.

# 10. Implementation Restrictions

State:

* do not run destructive commands;
* do not modify /etc during tests;
* do not require root for basic analysis;
* do not kill processes;
* do not remove packages;
* do not edit bootloader;
* do not edit fstab;
* do not use network;
* do not call external APIs;
* do not add telemetry;
* do not create busy loops;
* do not poll aggressively;
* do not add architecture not present in architecture.md.

# 11. Commit Strategy

Suggest small commits in the correct order.

# 12. Prompts for Future Implementation Models

Create ready-to-use prompts for:

* implementing repository skeleton;
* implementing Cargo.toml;
* implementing config module;
* implementing CLI;
* implementing logging;
* implementing memory metrics;
* implementing CPU metrics;
* implementing PSI parser;
* implementing process analyzer;
* implementing service analyzer;
* implementing pressure model;
* implementing profiles;
* implementing policy engine;
* implementing safety layer;
* implementing dry-run actions;
* implementing daemon loop;
* implementing history store;
* implementing rollback metadata;
* implementing systemd packaging;
* implementing tests.

Each prompt must explicitly say:

* follow architecture.md and planning.md;
* do not change architecture;
* implement only this phase;
* report changed files;
* run checks if possible.

# 13. Definition of Done

Define:

* MVP done;
* v0.1 done;
* safe for local testing;
* safe for systemd foreground testing;
* not yet safe for aggressive automatic optimization.

# 14. Risk Register

List technical risks and mitigations:

* over-optimization;
* unsafe process control;
* incorrect PSI interpretation;
* service disruption;
* zram misconfiguration;
* excessive daemon overhead;
* bad config;
* insufficient permissions;
* Arch version differences;
* cgroup inconsistencies;
* user misunderstanding.

# 15. Forbidden Implementation Order

Explicitly state:

* do not implement real process killing before safety layer;
* do not implement sysctl changes before backup and rollback;
* do not implement zram apply before detection and dry-run;
* do not implement MemoryMax before MemoryHigh;
* do not implement aggressive profile before conservative and balanced;
* do not implement AI integration before v1.0;
* do not implement network features;
* do not implement package removal;
* do not implement destructive cleanup.

# 16. Final Handoff Instructions

Write final instructions to future implementers:

* always start by reading both documents;
* treat architecture.md as immutable unless the human owner explicitly requests a revision;
* treat planning.md as the execution checklist;
* ask for confirmation only when a phase requires real system changes;
* preserve safety defaults;
* keep artifacts in English.

============================================================
TERMINAL RESPONSE REQUIREMENTS
==============================

After creating the folder and both files, respond in the terminal with:

1. Selected project name.
2. Created folder path.
3. Created files.
4. Short summary of architecture.md.
5. Short summary of planning.md.
6. Confirmation that no code was created.
7. Confirmation that no system files were modified.
8. Summary of the three review passes.
9. Recommended next prompt to give to Sonnet/GPT-5.5 for Phase 0 implementation.

============================================================
EXECUTE NOW
===========

Execute the task now.

Do not ask clarification questions.
Make conservative, professional decisions where needed.
Create the project folder.
Create architecture.md.
Create planning.md.
Write both files in complete professional English.
Do not create code.
Do not modify anything outside the project folder.
