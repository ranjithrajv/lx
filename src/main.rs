// SPDX-License-Identifier: GPL-3.0-or-later

use clap::Parser;
use colored::Colorize;
use std::process::ExitCode;

fn main() -> ExitCode {
    // Enable `did you mean` suggestions for typos; clap handles this on parse.
    let cli = lx_lib::cli::Cli::try_parse().unwrap_or_else(|e| e.exit());
    match lx_lib::cli::run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // Respect NO_COLOR / non-TTY: colored handles this via CLICOLOR env,
            // force off when stderr is not a terminal.
            let msg = format!("{err:#}");
            if std::io::IsTerminal::is_terminal(&std::io::stderr()) {
                eprintln!("{} {}", "error:".red().bold(), msg);
            } else {
                eprintln!("error: {msg}");
            }
            if let Some(hint) = hint_for(&msg) {
                eprintln!("hint: {hint}");
            }
            // Usage errors (bad flags, missing files) exit 2; runtime errors exit 1.
            if is_usage_error(&msg) {
                ExitCode::from(2)
            } else {
                ExitCode::FAILURE
            }
        }
    }
}

fn is_usage_error(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    [
        "conflicts with",
        "requires",
        "required",
        "invalid",
        "unsupported --",
        "unsupported source",
        "unsupported --format",
        "unsupported --sign-method",
        "no package.yaml",
        "does not exist",
        "not found",
        "failed to read",
        "failed to parse",
    ]
    .iter()
    .any(|s| m.contains(s))
}

fn hint_for(msg: &str) -> Option<&'static str> {
    let m = msg.to_ascii_lowercase();
    if m.contains("unverified") || m.contains("no pinned") || m.contains("checksum") {
        Some("retry with --allow-unverified, or pin with --pinned-metadata / --update-lock")
    } else if m.contains("rate limit") || m.contains("403") || m.contains("401") {
        Some("set GITHUB_TOKEN (or pass --token) to raise API rate limits")
    } else if m.contains("package.yaml")
        && (m.contains("no such") || m.contains("not found") || m.contains("failed to read"))
    {
        Some("run `lx init --from owner/repo` to scaffold a package.yaml")
    } else if m.contains("version") && m.contains("not found") {
        Some("check available tags with `lx validate <config>`")
    } else {
        None
    }
}
