//! syswarden — system supervision daemon entry point.

use std::path::Path;
use std::process::ExitCode;

use syswarden::{cli, config, logging};

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
