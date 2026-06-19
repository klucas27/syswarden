//! syswarden — system supervision daemon entry point.

use std::path::Path;
use std::process::ExitCode;

mod actions;
mod cgroups;
mod cli;
mod config;
mod daemon;
mod error;
mod explain;
mod history;
mod logging;
mod metrics;
mod policy;
mod pressure;
mod processes;
mod profiles;
mod reports;
mod rollback;
mod safety;
mod services;
mod systemd;
mod zram;

fn main() -> ExitCode {
    let parsed = cli::parse();

    let config_path = parsed
        .config
        .as_deref()
        .unwrap_or_else(|| Path::new("/etc/syswarden/config.toml"));

    let config = match config::load(config_path) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(cli::exit_codes::RUNTIME_ERROR);
        }
    };

    logging::init(&config.global.log_level, parsed.verbose);

    cli::dispatch(&parsed, &config)
}
