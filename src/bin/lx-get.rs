//! `lx-get` — the thin consumer client (mist spirit): install, upgrade,
//! and inspect prebuilt packages without any of the build machinery.
//! Same manifest, same `latest-debs` org, subset of `lx` subcommands.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "lx-get",
    version,
    about = "Thin client for lx-managed prebuilt packages (install/upgrade/inspect only, no building)"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// GitHub token (defaults to GITHUB_TOKEN)
    #[arg(long, env = "GITHUB_TOKEN")]
    token: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Fetch and install a pre-built .deb
    Install(lx_lib::install::InstallArgs),
    /// Upgrade lx-managed packages
    Upgrade(lx_lib::upgrade::UpgradeArgs),
    /// Check installed packages against their latest release (no install)
    Update(lx_lib::update::UpdateArgs),
    /// Remove an installed package
    Remove(lx_lib::remove::RemoveArgs),
    /// Show everything known about one package
    Show(lx_lib::show::ShowArgs),
    /// Reinstall the recorded version of a package
    Reinstall(lx_lib::reinstall::ReinstallArgs),
    /// List packages installed by lx
    List(lx_lib::list::ListArgs),
    /// Search available packages (regex; --local for the offline index)
    Search(lx_lib::search::SearchArgs),
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let token = cli.token.as_deref();
    let res = match cli.command {
        Commands::Install(a) => lx_lib::install::run(a, token),
        Commands::Upgrade(a) => lx_lib::upgrade::run(a, token),
        Commands::Update(a) => lx_lib::update::run(a, token),
        Commands::Remove(a) => lx_lib::remove::run(a),
        Commands::Show(a) => lx_lib::show::run(a),
        Commands::Reinstall(a) => lx_lib::reinstall::run(a, token),
        Commands::List(a) => lx_lib::list::run(a),
        Commands::Search(a) => lx_lib::search::run(a, token),
    };
    match res {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e:#}");
            std::process::ExitCode::FAILURE
        }
    }
}
