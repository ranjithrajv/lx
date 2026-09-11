// SPDX-License-Identifier: GPL-3.0-or-later

use clap::Parser;
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = lx_lib::cli::Cli::parse();
    match lx_lib::cli::run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err:#}");
            ExitCode::FAILURE
        }
    }
}
