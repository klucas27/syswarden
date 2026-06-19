//! syswarden — system supervision daemon entry point.

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

fn main() {}
