mod build;
mod cli;
mod config;
mod debs;
mod discovery;
mod install;
mod list;
mod manifest;
mod remove;
mod scandeps;
mod source;
mod summary;
mod update;
mod upgrade;
mod validate;
mod wizard;

use clap::Parser;
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = cli::Cli::parse();
    match cli::run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err:#}");
            ExitCode::FAILURE
        }
    }
}
