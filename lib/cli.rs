use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "lx",
    version,
    about = "lx: build (.deb/.rpm/Arch) from forge releases or source, install, and distribute Linux packages — native bare-metal builds, no containers",
    long_about = "lx watches forge releases, fetches the release assets
matching each architecture, verifies their checksums against pinned metadata, and
builds .deb/.rpm/Arch packages natively on bare metal — no containers, no emulation. It also installs,
upgrades, and removes pre-built .deb packages published under the latest-debs
GitHub org, tracking what it manages in a local install manifest -- an apt-like
front end for software that only ships forge releases.",
    after_help = "Exit codes:\n  0  success\n  1  generic error\n  2  usage error (clap)"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// GitHub token (defaults to GITHUB_TOKEN or gh CLI's stored token)
    #[arg(long, env = "GITHUB_TOKEN")]
    pub token: Option<String>,

    /// Be verbose (repeat for more)
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,
}

#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
pub enum Commands {
    /// Build .deb packages from a package.yaml config
    Build(crate::build::BuildArgs),
    /// Validate a package.yaml config and check release availability (no build)
    Validate(crate::validate::ValidateArgs),
    /// Auto-discover release patterns from a GitHub repo and print a config
    Discover(crate::discovery::DiscoverArgs),
    /// Interactively generate a package.yaml config
    Init(crate::wizard::InitArgs),
    /// Fetch and install a pre-built .deb from the latest-debs GitHub org
    Install(crate::install::InstallArgs),
    /// Check lx-managed packages against their latest release (no install)
    Update(crate::update::UpdateArgs),
    /// Upgrade lx-managed packages to their latest release
    Upgrade(crate::upgrade::UpgradeArgs),
    /// Remove an installed package
    Remove(crate::remove::RemoveArgs),
    /// Reinstall a prior generation of an lx-managed package
    Rollback(crate::rollback::RollbackArgs),
    /// List packages installed by lx
    List(crate::list::ListArgs),
    /// Search packages published under the latest-debs org
    /// (deb-get `search` parity: regex over package names + descriptions,
    /// with installed state from the local manifest/dpkg)
    Search(crate::search::SearchArgs),
    /// Scan a release binary's ELF shared-library dependencies (helps
    /// verify/fill in package.yaml's depends:)
    ScanDeps(crate::scandeps::ScanDepsArgs),
    /// Show everything lx knows about one package (manifest + dpkg)
    Show(crate::show::ShowArgs),
    /// Reinstall the recorded version of an lx-managed package
    Reinstall(crate::reinstall::ReinstallArgs),
    /// Carry legacy `lpt` state (manifest, caches) and workflows to `lx`
    Migrate(crate::migrate::MigrateArgs),
    /// Turn a directory of .debs into an apt-servable repository
    /// (Packages/Release/InRelease)
    Repo(crate::repo::RepoArgs),
    /// Migrate snap/flatpak/nix/curl|sh installs to native packages
    /// (plan by default, apply with --yes)
    GoNative(crate::go_native::GoNativeArgs),
    /// Generate JSON schema for package.yaml
    #[command(alias = "jsonschema")]
    JsonSchema(crate::schema::SchemaArgs),
}

pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Build(args) => crate::build::run(args, cli.token.as_deref()).map_err(|e| {
            // Mirror the bash action's ./failed-build-logs/ artifact dir so
            // the GitHub Action can upload it on failure.
            crate::build::write_failed_build_log(&format!("{e:#}"));
            e
        }),
        Commands::Validate(args) => crate::validate::run(args, cli.token.as_deref()),
        Commands::Discover(args) => crate::discovery::run(args, cli.token.as_deref()),
        Commands::Init(args) => crate::wizard::run(args),
        Commands::Install(args) => crate::install::run(args, cli.token.as_deref()),
        Commands::Update(args) => crate::update::run(args, cli.token.as_deref()),
        Commands::Upgrade(args) => crate::upgrade::run(args, cli.token.as_deref()),
        Commands::Remove(args) => crate::remove::run(args),
        Commands::Rollback(args) => crate::rollback::run(args, cli.token.as_deref()),
        Commands::List(args) => crate::list::run(args),
        Commands::Show(args) => crate::show::run(args),
        Commands::Reinstall(args) => crate::reinstall::run(args, cli.token.as_deref()),
        Commands::Repo(args) => crate::repo::run(args),
        Commands::Migrate(args) => crate::migrate::run(args),
        Commands::Search(args) => crate::search::run(args, cli.token.as_deref()),
        Commands::GoNative(args) => crate::go_native::run(args, cli.token.as_deref()),
        Commands::ScanDeps(args) => crate::scandeps::run(args, cli.token.as_deref()),
        Commands::JsonSchema(args) => crate::schema::run(args),
    }
}
