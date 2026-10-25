//! The `crosswire` binary.

mod api_cmd;
mod browser;
mod ca_cmd;
mod cli;
mod error;
mod paths;
mod serve;
mod token_cmd;
mod ui;

use std::process::ExitCode;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::cli::{Cli, Command};
use crate::error::CliError;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match init_logging(&cli.log).and_then(|()| dispatch(cli.command)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("crosswire: error: {e}");
            ExitCode::from(e.exit_code())
        }
    }
}

/// Logs to stderr (stdout carries command output), filtered by `RUST_LOG`
/// when set, else by `--log`.
fn init_logging(level: &str) -> Result<(), CliError> {
    let filter = match EnvFilter::try_from_default_env() {
        Ok(f) => f,
        Err(_) => EnvFilter::try_new(level)
            .map_err(|e| CliError::Config(format!("invalid --log {level:?}: {e}")))?,
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
    Ok(())
}

fn dispatch(command: Command) -> Result<(), CliError> {
    match command {
        Command::Serve(args) => serve::run(args),
        Command::Ca(cmd) => ca_cmd::run(cmd),
        Command::Token(cmd) => token_cmd::run(cmd),
        Command::Browser(args) => browser::run(args),
    }
}
