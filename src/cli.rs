//! CLI argument parsing and subcommand dispatch (architecture.md §5.1, §13).

#![allow(clippy::struct_excessive_bools)] // Cli global flags mandated by architecture.md §13.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::actions;
use crate::config::{self, AppConfig};
use crate::metrics::{self, CpuSample};
use crate::policy;
use crate::pressure;
use crate::processes;
use crate::profiles;
use crate::rollback::RollbackStore;
use crate::safety::Capabilities;
use crate::services;

// ---------------------------------------------------------------------------
// Exit codes
// ---------------------------------------------------------------------------

/// Well-known exit codes returned by syswarden (architecture.md §5.1).
pub mod exit_codes {
    /// Unexpected runtime error.
    pub const RUNTIME_ERROR: u8 = 1;
    /// Config validation found issues (`config validate` with errors).
    #[allow(dead_code)]
    pub const VALIDATION_FAILED: u8 = 2;
}

// ---------------------------------------------------------------------------
// Top-level CLI
// ---------------------------------------------------------------------------

/// syswarden — local PSI-driven system supervision daemon.
#[derive(Debug, Parser)]
#[command(
    name = "syswarden",
    version,
    about = "PSI-driven system supervision daemon",
    long_about = None
)]
pub struct Cli {
    /// Path to config file (default: /etc/syswarden/config.toml).
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    /// Emit output as JSON where supported.
    #[arg(long, global = true)]
    pub json: bool,

    /// Override active profile (`conservative|balanced|performance|low_ram|desktop|server|developer`).
    #[arg(long, global = true)]
    pub profile: Option<String>,

    /// Increase log verbosity (-v = debug, -vv = trace).
    #[arg(short = 'v', action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Force dry-run mode; overrides config (no system changes).
    #[arg(long, global = true, overrides_with = "no_dry_run")]
    pub dry_run: bool,

    /// Disable dry-run override (still subject to all safety gates and allowlists).
    #[arg(long = "no-dry-run", global = true, overrides_with = "dry_run")]
    pub no_dry_run: bool,

    #[command(subcommand)]
    pub command: Command,
}

// ---------------------------------------------------------------------------
// Subcommands (architecture.md §13)
// ---------------------------------------------------------------------------

/// All syswarden subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// One-shot system health and pressure summary.
    Status,
    /// Full one-shot analysis without acting.
    Analyze,
    /// Environment and capability check (PSI, cgroup v2, systemd, root, zram).
    Doctor,
    /// Run the supervision loop in the foreground.
    Daemon,
    /// Show recent audit log entries.
    Logs {
        /// Show entries since this duration ago (e.g. "1h", "30m").
        #[arg(long)]
        since: Option<String>,
    },
    /// Explain the latest (or most recent) decision.
    Explain,
    /// Show PSI pressure breakdown (CPU / memory / I/O).
    Pressure,
    /// List and flag heavy or anomalous processes.
    Processes {
        /// Show only the top N processes by resource usage.
        #[arg(long)]
        top: Option<usize>,
    },
    /// List and flag systemd services.
    Services {
        /// Show only failed or degraded services.
        #[arg(long)]
        failed: bool,
    },
    /// Profile management.
    Profile {
        #[command(subcommand)]
        cmd: ProfileCommand,
    },
    /// Configuration management.
    Config {
        #[command(subcommand)]
        cmd: ConfigCommand,
    },
    /// Action planning and application.
    Actions {
        #[command(subcommand)]
        cmd: ActionsCommand,
    },
    /// zram / zswap / swap management.
    Zram {
        #[command(subcommand)]
        cmd: ZramCommand,
    },
    /// Rollback management.
    Rollback {
        #[command(subcommand)]
        cmd: RollbackCommand,
    },
    /// Aggregated pressure and action report.
    Report {
        /// Report window in days.
        #[arg(long, default_value_t = 7)]
        days: u32,
    },
    /// Print version and build information.
    Version,
}

/// `syswarden profile` subcommands.
#[derive(Debug, Subcommand)]
pub enum ProfileCommand {
    /// List all built-in profiles with summaries.
    List,
    /// Persist a profile selection to the config file.
    Set {
        /// Profile name (`conservative|balanced|performance|low_ram|desktop|server|developer`).
        name: String,
    },
}

/// `syswarden config` subcommands.
#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Print the fully-resolved effective configuration.
    Show,
    /// Validate the config file and report all issues.
    Validate,
}

/// `syswarden actions` subcommands.
#[derive(Debug, Subcommand)]
pub enum ActionsCommand {
    /// Plan actions for the current state without applying them.
    DryRun,
    /// Apply currently-planned safe and permitted actions.
    Apply,
}

/// `syswarden zram` subcommands.
#[derive(Debug, Subcommand)]
pub enum ZramCommand {
    /// Show current zram / zswap / swap detection report.
    Status,
    /// Compute a zram sizing recommendation.
    Recommend,
    /// Apply zram configuration (requires `allow_zram_apply` = true).
    Apply,
}

/// `syswarden rollback` subcommands.
#[derive(Debug, Subcommand)]
pub enum RollbackCommand {
    /// List recorded rollback entries.
    List,
    /// Revert a recorded action by ID.
    Apply {
        /// Rollback entry ID to revert.
        id: u64,
    },
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse CLI arguments from `std::env::args_os`.
#[must_use]
pub fn parse() -> Cli {
    Cli::parse()
}

/// Dispatch the parsed CLI to the appropriate handler.
///
/// Applies `--dry-run` / `--no-dry-run` overrides before routing.
/// Returns an [`ExitCode`] for `main` to forward to the OS.
#[must_use]
pub fn dispatch(cli: &Cli, config: &AppConfig) -> ExitCode {
    let mut cfg = config.clone();
    if cli.dry_run {
        cfg.global.dry_run = true;
    }
    if cli.no_dry_run {
        cfg.global.dry_run = false;
    }
    let config = &cfg;

    match &cli.command {
        Command::Status => dispatch_status(cli, config),
        Command::Analyze => dispatch_analyze(cli, config),
        Command::Doctor => dispatch_doctor(cli, config),
        Command::Daemon => match crate::daemon::run(config.clone()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                tracing::error!("{e:#}");
                ExitCode::from(exit_codes::RUNTIME_ERROR)
            }
        },
        Command::Logs { .. } => run_stub("logs"),
        Command::Explain => run_stub("explain"),
        Command::Pressure => dispatch_pressure(cli, config),
        Command::Processes { top } => dispatch_processes(cli, config, *top),
        Command::Services { failed } => dispatch_services(cli, config, *failed),
        Command::Profile { cmd } => dispatch_profile(cmd),
        Command::Config { cmd } => dispatch_config(cmd, config, cli.json),
        Command::Actions { cmd } => dispatch_actions(cmd, cli, config),
        Command::Zram { cmd } => dispatch_zram(cmd),
        Command::Rollback { cmd } => dispatch_rollback(cmd, config),
        Command::Report { .. } => run_stub("report"),
        Command::Version => {
            println!("syswarden {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
    }
}

// ---------------------------------------------------------------------------
// Private dispatch helpers
// ---------------------------------------------------------------------------

/// Convert safety capabilities to the metrics module's capability type (same fields, separate types).
fn to_metrics_caps(caps: &Capabilities) -> metrics::Capabilities {
    metrics::Capabilities {
        has_psi: caps.has_psi,
        has_cgroup_v2: caps.has_cgroup_v2,
        has_systemd: caps.has_systemd,
        is_root: caps.is_root,
        has_zram: caps.has_zram,
    }
}

/// Collect metrics or print error and return early.
macro_rules! collect_metrics {
    ($mcaps:expr) => {{
        let mut prev = CpuSample::default();
        match metrics::collect($mcaps, &mut prev) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("error: metrics collection failed: {e:#}");
                return ExitCode::from(exit_codes::RUNTIME_ERROR);
            }
        }
    }};
}

fn yn(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

#[allow(clippy::cast_precision_loss)]
fn mb(kb: u64) -> f64 {
    kb as f64 / 1024.0
}

// ---------------------------------------------------------------------------
// analyze
// ---------------------------------------------------------------------------

#[must_use]
#[allow(clippy::too_many_lines)]
fn dispatch_analyze(cli: &Cli, config: &AppConfig) -> ExitCode {
    let caps = Capabilities::detect();
    let mcaps = to_metrics_caps(&caps);
    let metrics_snap = collect_metrics!(&mcaps);

    let profile_name = &config.global.profile;
    let profile = profiles::resolve(profile_name, config);
    let pressure_snap = pressure::compute(&mcaps, &metrics_snap, config, &[]);
    let procs = processes::analyze(config);
    let svcs = services::analyze(config);
    let state = pressure::classify_state(&pressure_snap, &procs, &svcs, &mcaps);
    let decision = policy::decide(state, &profile, &procs, &svcs);
    let planned = actions::plan(&decision, &profile, &procs);
    let results: Vec<_> = planned
        .iter()
        .map(|a| actions::simulate(a, config, &profile, &caps))
        .collect();

    if cli.json {
        let json = serde_json::json!({
            "capabilities": {
                "root": caps.is_root,
                "psi": caps.has_psi,
                "cgroup_v2": caps.has_cgroup_v2,
                "systemd": caps.has_systemd,
                "zram": caps.has_zram,
            },
            "pressure": {
                "level": format!("{:?}", pressure_snap.level),
                "contributors": pressure_snap.contributors,
                "cpu": { "some_avg10": pressure_snap.cpu.some_avg10,
                          "some_avg60": pressure_snap.cpu.some_avg60,
                          "some_avg300": pressure_snap.cpu.some_avg300 },
                "memory": { "some_avg10": pressure_snap.memory.some_avg10,
                             "full_avg10": pressure_snap.memory.full_avg10,
                             "some_avg300": pressure_snap.memory.some_avg300 },
                "io": { "some_avg10": pressure_snap.io.some_avg10,
                         "some_avg60": pressure_snap.io.some_avg60,
                         "some_avg300": pressure_snap.io.some_avg300 },
            },
            "memory": {
                "total_mb": mb(metrics_snap.memory.total_kb),
                "available_mb": mb(metrics_snap.memory.available_kb),
                "swap_in_rate": metrics_snap.memory.swap_in_rate,
            },
            "cpu": {
                "load_avg_1m": metrics_snap.cpu.load1,
                "pct": metrics_snap.cpu.utilization_pct,
            },
            "state": format!("{state:?}"),
            "profile": format!("{profile_name:?}"),
            "policy": {
                "intent": format!("{:?}", decision.intent),
                "rationale": decision.rationale,
            },
            "actions": results.iter().map(|r| serde_json::json!({
                "id": r.action_id,
                "status": format!("{:?}", r.status),
                "message": r.message,
            })).collect::<Vec<_>>(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&json).unwrap_or_default()
        );
        return ExitCode::SUCCESS;
    }

    println!("Capabilities");
    println!(
        "  root:      {:<4}  {}",
        yn(caps.is_root),
        if caps.is_root {
            ""
        } else {
            "(dry-run enforced; real actions require root)"
        }
    );
    println!("  PSI:       {}", yn(caps.has_psi));
    println!("  cgroup v2: {}", yn(caps.has_cgroup_v2));
    println!("  systemd:   {}", yn(caps.has_systemd));
    println!("  zram:      {}", yn(caps.has_zram));

    println!();
    println!("Pressure  [{:?}]", pressure_snap.level);
    println!(
        "  cpu     some10={:.2}  some60={:.2}  some300={:.2}",
        pressure_snap.cpu.some_avg10, pressure_snap.cpu.some_avg60, pressure_snap.cpu.some_avg300
    );
    println!(
        "  memory  some10={:.2}  full10={:.2}  some300={:.2}",
        pressure_snap.memory.some_avg10,
        pressure_snap.memory.full_avg10,
        pressure_snap.memory.some_avg300
    );
    println!(
        "  io      some10={:.2}  some60={:.2}  some300={:.2}",
        pressure_snap.io.some_avg10, pressure_snap.io.some_avg60, pressure_snap.io.some_avg300
    );
    if !pressure_snap.contributors.is_empty() {
        println!("  contributors: {}", pressure_snap.contributors.join(", "));
    }

    let avail_pct = if metrics_snap.memory.total_kb > 0 {
        #[allow(clippy::cast_precision_loss)]
        let pct =
            metrics_snap.memory.available_kb as f64 / metrics_snap.memory.total_kb as f64 * 100.0;
        format!("{pct:.1}% available")
    } else {
        "unknown".to_string()
    };
    println!();
    println!(
        "Memory   {:.0} MB total  {:.0} MB available  ({avail_pct})  swap_in={:.1} pg/s",
        mb(metrics_snap.memory.total_kb),
        mb(metrics_snap.memory.available_kb),
        metrics_snap.memory.swap_in_rate
    );
    println!(
        "CPU      load={:.2}  pct={:.1}%",
        metrics_snap.cpu.load1, metrics_snap.cpu.utilization_pct
    );

    println!();
    println!("State:   {state:?}");
    println!("Profile: {profile_name:?}");
    println!("Policy:  {:?}", decision.intent);
    println!("         \"{}\"", decision.rationale);

    println!();
    if results.is_empty() {
        println!("Actions  none");
    } else {
        println!("Actions  {} planned:", results.len());
        for r in &results {
            println!(
                "  [{}]  {:?}  {}",
                r.action_id,
                planned
                    .iter()
                    .find(|a| a.id == r.action_id)
                    .map(|a| format!("{:?}  {:?}  {:?}", a.kind, a.risk, a.target))
                    .unwrap_or_default(),
                r.message
            );
        }
    }

    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// status
// ---------------------------------------------------------------------------

#[must_use]
fn dispatch_status(cli: &Cli, config: &AppConfig) -> ExitCode {
    let caps = Capabilities::detect();
    let mcaps = to_metrics_caps(&caps);
    let metrics_snap = collect_metrics!(&mcaps);

    let profile_name = &config.global.profile;
    let profile = profiles::resolve(profile_name, config);
    let pressure_snap = pressure::compute(&mcaps, &metrics_snap, config, &[]);
    // Use empty process/service lists for a fast status query — just pressure state.
    let state = pressure::classify_state(&pressure_snap, &[], &[], &mcaps);

    if cli.json {
        let avail_pct = if metrics_snap.memory.total_kb > 0 {
            #[allow(clippy::cast_precision_loss)]
            {
                metrics_snap.memory.available_kb as f64 / metrics_snap.memory.total_kb as f64
                    * 100.0
            }
        } else {
            0.0
        };
        let json = serde_json::json!({
            "state": format!("{state:?}"),
            "pressure_level": format!("{:?}", pressure_snap.level),
            "cpu_some_avg10": pressure_snap.cpu.some_avg10,
            "mem_some_avg10": pressure_snap.memory.some_avg10,
            "io_some_avg10": pressure_snap.io.some_avg10,
            "memory_total_mb": mb(metrics_snap.memory.total_kb),
            "memory_available_mb": mb(metrics_snap.memory.available_kb),
            "memory_available_pct": avail_pct,
            "cpu_load_avg_1m": metrics_snap.cpu.load1,
            "cpu_pct": metrics_snap.cpu.utilization_pct,
            "profile": format!("{profile_name:?}"),
            "dry_run": config.global.dry_run,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&json).unwrap_or_default()
        );
        return ExitCode::SUCCESS;
    }

    let avail_pct = if metrics_snap.memory.total_kb > 0 {
        #[allow(clippy::cast_precision_loss)]
        let p =
            metrics_snap.memory.available_kb as f64 / metrics_snap.memory.total_kb as f64 * 100.0;
        format!("{p:.1}%")
    } else {
        "?".to_string()
    };
    println!("State:    {state:?}");
    println!(
        "Pressure: {:?}  [cpu={:.2}% mem={:.2}% io={:.2}% some_avg10]",
        pressure_snap.level,
        pressure_snap.cpu.some_avg10,
        pressure_snap.memory.some_avg10,
        pressure_snap.io.some_avg10
    );
    println!(
        "Memory:   {:.0}/{:.0} MB  ({avail_pct} available)",
        mb(metrics_snap.memory.available_kb),
        mb(metrics_snap.memory.total_kb)
    );
    println!(
        "CPU:      load={:.2}  pct={:.1}%",
        metrics_snap.cpu.load1, metrics_snap.cpu.utilization_pct
    );
    println!(
        "Profile:  {:?}  (max_risk={:?}  dry_run={})",
        profile_name,
        profile.max_allowed_risk,
        yn(config.global.dry_run)
    );

    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// pressure
// ---------------------------------------------------------------------------

#[must_use]
fn dispatch_pressure(cli: &Cli, config: &AppConfig) -> ExitCode {
    let caps = Capabilities::detect();
    let mcaps = to_metrics_caps(&caps);
    let metrics_snap = collect_metrics!(&mcaps);
    let pressure_snap = pressure::compute(&mcaps, &metrics_snap, config, &[]);

    if cli.json {
        let json = serde_json::json!({
            "level": format!("{:?}", pressure_snap.level),
            "contributors": pressure_snap.contributors,
            "cpu": {
                "some_avg10": pressure_snap.cpu.some_avg10,
                "some_avg60": pressure_snap.cpu.some_avg60,
                "some_avg300": pressure_snap.cpu.some_avg300,
                "total_us": pressure_snap.cpu.total_us,
            },
            "memory": {
                "some_avg10": pressure_snap.memory.some_avg10,
                "some_avg60": pressure_snap.memory.some_avg60,
                "some_avg300": pressure_snap.memory.some_avg300,
                "full_avg10": pressure_snap.memory.full_avg10,
                "full_avg60": pressure_snap.memory.full_avg60,
                "full_avg300": pressure_snap.memory.full_avg300,
                "total_us": pressure_snap.memory.total_us,
            },
            "io": {
                "some_avg10": pressure_snap.io.some_avg10,
                "some_avg60": pressure_snap.io.some_avg60,
                "some_avg300": pressure_snap.io.some_avg300,
                "full_avg10": pressure_snap.io.full_avg10,
                "full_avg60": pressure_snap.io.full_avg60,
                "full_avg300": pressure_snap.io.full_avg300,
                "total_us": pressure_snap.io.total_us,
            },
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&json).unwrap_or_default()
        );
        return ExitCode::SUCCESS;
    }

    println!(
        "{:<8}  {:>8}  {:>8}  {:>8}  {:>8}  {:>8}  {:>8}",
        "resource", "some10", "some60", "some300", "full10", "full60", "full300"
    );
    println!(
        "{:<8}  {:>7.2}%  {:>7.2}%  {:>7.2}%  {:>8}  {:>8}  {:>8}",
        "cpu",
        pressure_snap.cpu.some_avg10,
        pressure_snap.cpu.some_avg60,
        pressure_snap.cpu.some_avg300,
        "—",
        "—",
        "—"
    );
    println!(
        "{:<8}  {:>7.2}%  {:>7.2}%  {:>7.2}%  {:>7.2}%  {:>7.2}%  {:>7.2}%",
        "memory",
        pressure_snap.memory.some_avg10,
        pressure_snap.memory.some_avg60,
        pressure_snap.memory.some_avg300,
        pressure_snap.memory.full_avg10,
        pressure_snap.memory.full_avg60,
        pressure_snap.memory.full_avg300
    );
    println!(
        "{:<8}  {:>7.2}%  {:>7.2}%  {:>7.2}%  {:>7.2}%  {:>7.2}%  {:>7.2}%",
        "io",
        pressure_snap.io.some_avg10,
        pressure_snap.io.some_avg60,
        pressure_snap.io.some_avg300,
        pressure_snap.io.full_avg10,
        pressure_snap.io.full_avg60,
        pressure_snap.io.full_avg300
    );
    println!();
    println!("Level: {:?}", pressure_snap.level);
    if !pressure_snap.contributors.is_empty() {
        println!("Contributors: {}", pressure_snap.contributors.join(", "));
    }

    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// doctor
// ---------------------------------------------------------------------------

#[must_use]
fn dispatch_doctor(cli: &Cli, config: &AppConfig) -> ExitCode {
    let caps = Capabilities::detect();

    if cli.json {
        let json = serde_json::json!({
            "root": caps.is_root,
            "psi": caps.has_psi,
            "cgroup_v2": caps.has_cgroup_v2,
            "systemd": caps.has_systemd,
            "zram": caps.has_zram,
            "dry_run": config.global.dry_run,
            "profile": format!("{:?}", config.global.profile),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&json).unwrap_or_default()
        );
        return ExitCode::SUCCESS;
    }

    println!(
        "  root:      {:<4}  {}",
        yn(caps.is_root),
        if caps.is_root {
            ""
        } else {
            "(dry-run enforced; real actions require root + --no-dry-run)"
        }
    );
    println!("  PSI:       {:<4}  /proc/pressure/cpu", yn(caps.has_psi));
    println!(
        "  cgroup v2: {:<4}  /sys/fs/cgroup/cgroup.controllers",
        yn(caps.has_cgroup_v2)
    );
    println!(
        "  systemd:   {:<4}  /run/systemd/private",
        yn(caps.has_systemd)
    );
    println!("  zram:      {:<4}  /sys/block/zram0", yn(caps.has_zram));
    println!();
    println!("  profile:   {:?}", config.global.profile);
    println!("  dry_run:   {}", yn(config.global.dry_run));

    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// processes
// ---------------------------------------------------------------------------

#[must_use]
fn dispatch_processes(cli: &Cli, config: &AppConfig, top: Option<usize>) -> ExitCode {
    let procs = processes::analyze(config);
    let procs: Vec<_> = if let Some(n) = top {
        let mut sorted = procs;
        sorted.sort_by(|a, b| {
            b.rss_kb.cmp(&a.rss_kb).then(
                b.cpu_pct
                    .partial_cmp(&a.cpu_pct)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
        });
        sorted.into_iter().take(n).collect()
    } else {
        procs
    };

    if cli.json {
        let json: Vec<_> = procs
            .iter()
            .map(|p| {
                serde_json::json!({
                    "pid": p.pid,
                    "comm": p.comm,
                    "cpu_pct": p.cpu_pct,
                    "rss_mb": mb(p.rss_kb),
                    "nice": p.nice,
                    "protected": p.is_protected,
                    "flags": p.flags.iter().map(|f| format!("{f:?}")).collect::<Vec<_>>(),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json).unwrap_or_default()
        );
        return ExitCode::SUCCESS;
    }

    println!(
        "{:<7}  {:<16}  {:>6}  {:>8}  FLAGS",
        "PID", "COMM", "CPU%", "RSS MB"
    );
    for p in &procs {
        let flags: Vec<_> = p.flags.iter().map(|f| format!("{f:?}")).collect();
        println!(
            "{:<7}  {:<16}  {:>5.1}%  {:>7.1}  {}",
            p.pid,
            p.comm,
            p.cpu_pct,
            mb(p.rss_kb),
            if flags.is_empty() {
                String::new()
            } else {
                flags.join(" ")
            }
        );
    }
    if procs.is_empty() {
        println!("(no processes found — may require /proc access)");
    }

    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// services
// ---------------------------------------------------------------------------

#[must_use]
fn dispatch_services(cli: &Cli, config: &AppConfig, failed_only: bool) -> ExitCode {
    let svcs = services::analyze(config);
    let svcs: Vec<_> = if failed_only {
        svcs.into_iter()
            .filter(|s| s.active_state != "active" && s.active_state != "activating")
            .collect()
    } else {
        svcs
    };

    if cli.json {
        let json: Vec<_> = svcs
            .iter()
            .map(|s| {
                serde_json::json!({
                    "unit": s.unit,
                    "active_state": s.active_state,
                    "protected": s.is_protected,
                    "allowed": s.is_allowed,
                    "flags": s.flags.iter().map(|f| format!("{f:?}")).collect::<Vec<_>>(),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json).unwrap_or_default()
        );
        return ExitCode::SUCCESS;
    }

    println!("{:<36}  {:<12}  FLAGS", "UNIT", "STATE");
    for s in &svcs {
        let flags: Vec<_> = s.flags.iter().map(|f| format!("{f:?}")).collect();
        println!(
            "{:<36}  {:<12}  {}",
            s.unit,
            s.active_state,
            if flags.is_empty() {
                String::new()
            } else {
                flags.join(" ")
            }
        );
    }
    if svcs.is_empty() {
        let reason = if failed_only {
            "no failed/degraded services found"
        } else {
            "no services found — systemd or D-Bus may be unavailable"
        };
        println!("({reason})");
    }

    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// profile / config / actions / zram (partially stubbed)
// ---------------------------------------------------------------------------

#[must_use]
fn dispatch_profile(cmd: &ProfileCommand) -> ExitCode {
    match cmd {
        ProfileCommand::List => {
            let names = [
                (
                    "conservative",
                    "Safe-only: observe and log. No system changes.",
                ),
                (
                    "balanced",
                    "Moderate: nice + cpu_weight + memory_high on allowed services.",
                ),
                (
                    "performance",
                    "Aggressive: nice + ionice + all cgroup limits + zram.",
                ),
                ("low_ram", "Tuned for ≤4 GB RAM; tight memory thresholds."),
                ("desktop", "Desktop workstation; protect UI processes."),
                ("server", "Server; conservative with service management."),
                ("developer", "Developer; wider allowlists, short intervals."),
            ];
            println!("{:<16}  DESCRIPTION", "PROFILE");
            for (name, desc) in names {
                println!("{name:<16}  {desc}");
            }
            ExitCode::SUCCESS
        }
        ProfileCommand::Set { .. } => run_stub("profile set"),
    }
}

#[must_use]
fn dispatch_config(cmd: &ConfigCommand, config: &AppConfig, json: bool) -> ExitCode {
    match cmd {
        ConfigCommand::Show => {
            if json {
                match serde_json::to_string_pretty(config) {
                    Ok(s) => println!("{s}"),
                    Err(e) => {
                        eprintln!("error: {e}");
                        return ExitCode::from(exit_codes::RUNTIME_ERROR);
                    }
                }
            } else {
                match toml::to_string_pretty(config) {
                    Ok(s) => print!("{s}"),
                    Err(e) => {
                        eprintln!("error: {e}");
                        return ExitCode::from(exit_codes::RUNTIME_ERROR);
                    }
                }
            }
            ExitCode::SUCCESS
        }
        ConfigCommand::Validate => {
            let issues = config::validate(config);
            if issues.is_empty() {
                println!("Config is valid (no issues found).");
            } else {
                println!("{} issue(s) found:", issues.len());
                for issue in &issues {
                    println!("  {issue}");
                }
                return ExitCode::from(exit_codes::VALIDATION_FAILED);
            }
            ExitCode::SUCCESS
        }
    }
}

#[must_use]
#[allow(clippy::too_many_lines)] // two exhaustive subcommand branches, each legitimately long
fn dispatch_actions(cmd: &ActionsCommand, cli: &Cli, config: &AppConfig) -> ExitCode {
    match cmd {
        ActionsCommand::DryRun => {
            let caps = Capabilities::detect();
            let mcaps = to_metrics_caps(&caps);
            let metrics_snap = collect_metrics!(&mcaps);

            let profile_name = &config.global.profile;
            let profile = profiles::resolve(profile_name, config);
            let pressure_snap = pressure::compute(&mcaps, &metrics_snap, config, &[]);
            let procs = processes::analyze(config);
            let svcs = services::analyze(config);
            let state = pressure::classify_state(&pressure_snap, &procs, &svcs, &mcaps);
            let decision = policy::decide(state, &profile, &procs, &svcs);
            let planned = actions::plan(&decision, &profile, &procs);
            let results: Vec<_> = planned
                .iter()
                .map(|a| actions::simulate(a, config, &profile, &caps))
                .collect();

            if cli.json {
                let json = serde_json::json!({
                    "state": format!("{state:?}"),
                    "intent": format!("{:?}", decision.intent),
                    "rationale": decision.rationale,
                    "actions": results.iter().zip(planned.iter()).map(|(r, a)| serde_json::json!({
                        "id": r.action_id,
                        "kind": format!("{:?}", a.kind),
                        "risk": format!("{:?}", a.risk),
                        "target": format!("{:?}", a.target),
                        "status": format!("{:?}", r.status),
                        "message": r.message,
                    })).collect::<Vec<_>>(),
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json).unwrap_or_default()
                );
                return ExitCode::SUCCESS;
            }

            println!("State:   {state:?}");
            println!("Policy:  {:?}", decision.intent);
            println!("         \"{}\"", decision.rationale);
            println!();
            if results.is_empty() {
                println!("Actions: none");
            } else {
                println!("Actions: {} planned:", results.len());
                for (r, a) in results.iter().zip(planned.iter()) {
                    println!(
                        "  [{}]  {:?}  risk={:?}  target={:?}",
                        r.action_id, a.kind, a.risk, a.target
                    );
                    println!("       Status: {:?}", r.status);
                    println!("       {}", r.message);
                }
            }

            ExitCode::SUCCESS
        }
        ActionsCommand::Apply => {
            // Phase 27: real execution path (architecture.md §6 Allow branch).
            let caps = Capabilities::detect();
            let mcaps = to_metrics_caps(&caps);
            let metrics_snap = collect_metrics!(&mcaps);

            let profile_name = &config.global.profile;
            let profile = profiles::resolve(profile_name, config);
            let pressure_snap = pressure::compute(&mcaps, &metrics_snap, config, &[]);
            let procs = processes::analyze(config);
            let svcs = services::analyze(config);
            let state = pressure::classify_state(&pressure_snap, &procs, &svcs, &mcaps);
            let decision = policy::decide(state, &profile, &procs, &svcs);
            let planned = actions::plan(&decision, &profile, &procs);

            let mut rollback = match RollbackStore::open(config) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("error: cannot open rollback store: {e:#}");
                    return ExitCode::from(exit_codes::RUNTIME_ERROR);
                }
            };

            let results: Vec<_> = planned
                .iter()
                .map(|a| actions::execute(a, config, &profile, &caps, &mut rollback))
                .collect();

            if cli.json {
                let json = serde_json::json!({
                    "state": format!("{state:?}"),
                    "intent": format!("{:?}", decision.intent),
                    "rationale": decision.rationale,
                    "actions": results.iter().zip(planned.iter()).map(|(r, a)| serde_json::json!({
                        "id": r.action_id,
                        "kind": format!("{:?}", a.kind),
                        "risk": format!("{:?}", a.risk),
                        "target": format!("{:?}", a.target),
                        "status": format!("{:?}", r.status),
                        "message": r.message,
                        "rollback_id": r.rollback_id,
                    })).collect::<Vec<_>>(),
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json).unwrap_or_default()
                );
                return ExitCode::SUCCESS;
            }

            println!("State:   {state:?}");
            println!("Policy:  {:?}", decision.intent);
            println!("         \"{}\"", decision.rationale);
            println!();
            if results.is_empty() {
                println!("Actions: none");
            } else {
                println!("Actions: {} executed:", results.len());
                for (r, a) in results.iter().zip(planned.iter()) {
                    println!(
                        "  [{}]  {:?}  risk={:?}  target={:?}",
                        r.action_id, a.kind, a.risk, a.target
                    );
                    println!("       Status: {:?}", r.status);
                    println!("       {}", r.message);
                    if let Some(rb) = r.rollback_id {
                        println!("       rollback_id={rb}");
                    }
                }
            }

            ExitCode::SUCCESS
        }
    }
}

#[must_use]
fn dispatch_zram(cmd: &ZramCommand) -> ExitCode {
    match cmd {
        ZramCommand::Status => run_stub("zram status"),
        ZramCommand::Recommend => run_stub("zram recommend"),
        ZramCommand::Apply => run_stub("zram apply"),
    }
}

#[must_use]
fn dispatch_rollback(cmd: &RollbackCommand, config: &AppConfig) -> ExitCode {
    match cmd {
        RollbackCommand::List => {
            let store = match RollbackStore::open(config) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("rollback: cannot open store: {e:#}");
                    return ExitCode::from(exit_codes::RUNTIME_ERROR);
                }
            };
            let entries = store.list();
            if entries.is_empty() {
                println!("No rollback entries.");
            } else {
                for e in entries {
                    println!(
                        "id={} ts={} kind={} target={} reversible={}",
                        e.id,
                        e.timestamp.format("%Y-%m-%dT%H:%M:%SZ"),
                        e.action_kind,
                        e.target,
                        e.reversible,
                    );
                }
            }
            ExitCode::SUCCESS
        }
        RollbackCommand::Apply { id } => {
            let store = match RollbackStore::open(config) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("rollback: cannot open store: {e:#}");
                    return ExitCode::from(exit_codes::RUNTIME_ERROR);
                }
            };
            match store.apply(*id) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("rollback apply: {e:#}");
                    ExitCode::from(exit_codes::RUNTIME_ERROR)
                }
            }
        }
    }
}

#[must_use]
fn run_stub(name: &str) -> ExitCode {
    println!("[stub] {name}: not yet implemented");
    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::wildcard_imports)]
mod tests {
    use std::path::Path;

    use clap::Parser;

    use super::*;

    fn parse_args(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).expect("args should parse")
    }

    // --- Flat subcommands ---

    #[test]
    fn parse_status() {
        assert!(matches!(
            parse_args(&["syswarden", "status"]).command,
            Command::Status
        ));
    }

    #[test]
    fn parse_analyze() {
        assert!(matches!(
            parse_args(&["syswarden", "analyze"]).command,
            Command::Analyze
        ));
    }

    #[test]
    fn parse_doctor() {
        assert!(matches!(
            parse_args(&["syswarden", "doctor"]).command,
            Command::Doctor
        ));
    }

    #[test]
    fn parse_daemon() {
        assert!(matches!(
            parse_args(&["syswarden", "daemon"]).command,
            Command::Daemon
        ));
    }

    #[test]
    fn parse_logs_no_flags() {
        assert!(matches!(
            parse_args(&["syswarden", "logs"]).command,
            Command::Logs { since: None }
        ));
    }

    #[test]
    fn parse_logs_since() {
        let cli = parse_args(&["syswarden", "logs", "--since", "1h"]);
        let Command::Logs { since } = cli.command else {
            panic!("expected Logs");
        };
        assert_eq!(since.as_deref(), Some("1h"));
    }

    #[test]
    fn parse_explain() {
        assert!(matches!(
            parse_args(&["syswarden", "explain"]).command,
            Command::Explain
        ));
    }

    #[test]
    fn parse_pressure() {
        assert!(matches!(
            parse_args(&["syswarden", "pressure"]).command,
            Command::Pressure
        ));
    }

    #[test]
    fn parse_processes_no_flags() {
        assert!(matches!(
            parse_args(&["syswarden", "processes"]).command,
            Command::Processes { top: None }
        ));
    }

    #[test]
    fn parse_processes_top() {
        let cli = parse_args(&["syswarden", "processes", "--top", "10"]);
        let Command::Processes { top } = cli.command else {
            panic!("expected Processes");
        };
        assert_eq!(top, Some(10));
    }

    #[test]
    fn parse_services_no_flags() {
        assert!(matches!(
            parse_args(&["syswarden", "services"]).command,
            Command::Services { failed: false }
        ));
    }

    #[test]
    fn parse_services_failed() {
        assert!(matches!(
            parse_args(&["syswarden", "services", "--failed"]).command,
            Command::Services { failed: true }
        ));
    }

    #[test]
    fn parse_report_default_days() {
        assert!(matches!(
            parse_args(&["syswarden", "report"]).command,
            Command::Report { days: 7 }
        ));
    }

    #[test]
    fn parse_report_custom_days() {
        assert!(matches!(
            parse_args(&["syswarden", "report", "--days", "14"]).command,
            Command::Report { days: 14 }
        ));
    }

    #[test]
    fn parse_version() {
        assert!(matches!(
            parse_args(&["syswarden", "version"]).command,
            Command::Version
        ));
    }

    // --- Nested: profile ---

    #[test]
    fn parse_profile_list() {
        assert!(matches!(
            parse_args(&["syswarden", "profile", "list"]).command,
            Command::Profile {
                cmd: ProfileCommand::List
            }
        ));
    }

    #[test]
    fn parse_profile_set() {
        let cli = parse_args(&["syswarden", "profile", "set", "low_ram"]);
        let Command::Profile {
            cmd: ProfileCommand::Set { name },
        } = cli.command
        else {
            panic!("expected Profile::Set");
        };
        assert_eq!(name, "low_ram");
    }

    // --- Nested: config ---

    #[test]
    fn parse_config_show() {
        assert!(matches!(
            parse_args(&["syswarden", "config", "show"]).command,
            Command::Config {
                cmd: ConfigCommand::Show
            }
        ));
    }

    #[test]
    fn parse_config_validate() {
        assert!(matches!(
            parse_args(&["syswarden", "config", "validate"]).command,
            Command::Config {
                cmd: ConfigCommand::Validate
            }
        ));
    }

    // --- Nested: actions ---

    #[test]
    fn parse_actions_dry_run() {
        assert!(matches!(
            parse_args(&["syswarden", "actions", "dry-run"]).command,
            Command::Actions {
                cmd: ActionsCommand::DryRun
            }
        ));
    }

    #[test]
    fn parse_actions_apply() {
        assert!(matches!(
            parse_args(&["syswarden", "actions", "apply"]).command,
            Command::Actions {
                cmd: ActionsCommand::Apply
            }
        ));
    }

    // --- Nested: zram ---

    #[test]
    fn parse_zram_status() {
        assert!(matches!(
            parse_args(&["syswarden", "zram", "status"]).command,
            Command::Zram {
                cmd: ZramCommand::Status
            }
        ));
    }

    #[test]
    fn parse_zram_recommend() {
        assert!(matches!(
            parse_args(&["syswarden", "zram", "recommend"]).command,
            Command::Zram {
                cmd: ZramCommand::Recommend
            }
        ));
    }

    #[test]
    fn parse_zram_apply() {
        assert!(matches!(
            parse_args(&["syswarden", "zram", "apply"]).command,
            Command::Zram {
                cmd: ZramCommand::Apply
            }
        ));
    }

    // --- Nested: rollback ---

    #[test]
    fn parse_rollback_list() {
        assert!(matches!(
            parse_args(&["syswarden", "rollback", "list"]).command,
            Command::Rollback {
                cmd: RollbackCommand::List
            }
        ));
    }

    #[test]
    fn parse_rollback_apply() {
        let cli = parse_args(&["syswarden", "rollback", "apply", "42"]);
        let Command::Rollback {
            cmd: RollbackCommand::Apply { id },
        } = cli.command
        else {
            panic!("expected Rollback::Apply");
        };
        assert_eq!(id, 42);
    }

    // --- Global flags ---

    #[test]
    fn global_json_flag_before_subcommand() {
        let cli = parse_args(&["syswarden", "--json", "status"]);
        assert!(cli.json);
    }

    #[test]
    fn global_dry_run_flag() {
        let cli = parse_args(&["syswarden", "--dry-run", "status"]);
        assert!(cli.dry_run);
        assert!(!cli.no_dry_run);
    }

    #[test]
    fn global_no_dry_run_flag() {
        let cli = parse_args(&["syswarden", "--no-dry-run", "status"]);
        assert!(cli.no_dry_run);
        assert!(!cli.dry_run);
    }

    #[test]
    fn global_verbose_single() {
        let cli = parse_args(&["syswarden", "-v", "status"]);
        assert_eq!(cli.verbose, 1);
    }

    #[test]
    fn global_verbose_double() {
        let cli = parse_args(&["syswarden", "-vv", "status"]);
        assert_eq!(cli.verbose, 2);
    }

    #[test]
    fn global_config_path() {
        let cli = parse_args(&["syswarden", "--config", "/tmp/test.toml", "status"]);
        assert_eq!(cli.config.as_deref(), Some(Path::new("/tmp/test.toml")));
    }

    // --- Exit code mapping ---

    #[test]
    fn version_dispatch_exits_success() {
        let cli = parse_args(&["syswarden", "version"]);
        let cfg = crate::config::defaults();
        assert_eq!(dispatch(&cli, &cfg), ExitCode::SUCCESS);
    }

    #[test]
    fn stub_commands_exit_success() {
        let mut cfg = crate::config::defaults();
        // Override dirs that require root so the test runs without privileges.
        cfg.rollback.dir = std::env::temp_dir()
            .join("syswarden_cli_stub_rollback")
            .to_string_lossy()
            .to_string();
        let cases: &[&[&str]] = &[
            &["syswarden", "status"],
            &["syswarden", "analyze"],
            &["syswarden", "doctor"],
            &["syswarden", "pressure"],
            &["syswarden", "explain"],
            &["syswarden", "profile", "list"],
            &["syswarden", "config", "show"],
            &["syswarden", "config", "validate"],
            &["syswarden", "actions", "dry-run"],
            &["syswarden", "actions", "apply"],
            &["syswarden", "zram", "status"],
            &["syswarden", "zram", "recommend"],
            &["syswarden", "rollback", "list"],
        ];
        for args in cases {
            let cli = Cli::try_parse_from(*args).expect("parse");
            assert_eq!(dispatch(&cli, &cfg), ExitCode::SUCCESS, "{args:?}");
        }
    }

    #[test]
    fn invalid_subcommand_fails_to_parse() {
        assert!(Cli::try_parse_from(["syswarden", "nonexistent"]).is_err());
    }
}
