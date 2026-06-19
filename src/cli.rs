//! CLI argument parsing and subcommand dispatch (architecture.md §5.1, §13).

#![allow(clippy::struct_excessive_bools)] // Cli global flags mandated by architecture.md §13.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::config::AppConfig;
use crate::rollback::RollbackStore;

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
/// Phase 3: all handlers are stubs; business logic is not yet implemented.
/// Returns an [`ExitCode`] for `main` to forward to the OS.
#[must_use]
pub fn dispatch(cli: &Cli, config: &AppConfig) -> ExitCode {
    match &cli.command {
        Command::Status => run_stub("status"),
        Command::Analyze => run_stub("analyze"),
        Command::Doctor => run_stub("doctor"),
        Command::Daemon => match crate::daemon::run(config.clone()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                tracing::error!("{e:#}");
                ExitCode::from(exit_codes::RUNTIME_ERROR)
            }
        },
        Command::Logs { .. } => run_stub("logs"),
        Command::Explain => run_stub("explain"),
        Command::Pressure => run_stub("pressure"),
        Command::Processes { .. } => run_stub("processes"),
        Command::Services { .. } => run_stub("services"),
        Command::Profile { cmd } => dispatch_profile(cmd),
        Command::Config { cmd } => dispatch_config(cmd),
        Command::Actions { cmd } => dispatch_actions(cmd),
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

#[must_use]
fn dispatch_profile(cmd: &ProfileCommand) -> ExitCode {
    match cmd {
        ProfileCommand::List => run_stub("profile list"),
        ProfileCommand::Set { .. } => run_stub("profile set"),
    }
}

#[must_use]
fn dispatch_config(cmd: &ConfigCommand) -> ExitCode {
    match cmd {
        ConfigCommand::Show => run_stub("config show"),
        ConfigCommand::Validate => run_stub("config validate"),
    }
}

#[must_use]
fn dispatch_actions(cmd: &ActionsCommand) -> ExitCode {
    match cmd {
        ActionsCommand::DryRun => run_stub("actions dry-run"),
        ActionsCommand::Apply => run_stub("actions apply"),
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
