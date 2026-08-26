use clap::Parser;
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = lpt_lib::cli::Cli::parse();
    match lpt_lib::cli::run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err:#}");
            ExitCode::FAILURE
        }
    }
}
