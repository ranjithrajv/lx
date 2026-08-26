use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "lpt",
    version,
    about = "Latest Package Tool: build and install .deb packages repackaged from GitHub release binaries across suites and architectures",
    long_about = "lpt (Latest Package Tool) watches GitHub releases, fetches the release assets
matching each architecture, verifies their checksums against pinned metadata, and
builds .deb packages for multiple Debian suites using Docker. It also installs,
upgrades, and removes pre-built .deb packages published under the latest-debs
GitHub org, tracking what it manages in a local install manifest -- an apt-like
front end for software that only ships GitHub releases.",
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
    /// Check lpt-managed packages against their latest release (no install)
    Update(crate::update::UpdateArgs),
    /// Upgrade lpt-managed packages to their latest release
    Upgrade(crate::upgrade::UpgradeArgs),
    /// Remove an installed package
    Remove(crate::remove::RemoveArgs),
    /// List packages installed by lpt
    List(crate::list::ListArgs),
    /// Scan a release binary's ELF shared-library dependencies (helps
    /// verify/fill in package.yaml's depends:)
    ScanDeps(crate::scandeps::ScanDepsArgs),
    /// Generate JSON schema for package.yaml
    #[command(alias = "jsonschema")]
    JsonSchema(crate::schema::SchemaArgs),
}

pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Build(args) => crate::build::run(args, cli.token.as_deref()),
        Commands::Validate(args) => crate::validate::run(args, cli.token.as_deref()),
        Commands::Discover(args) => crate::discovery::run(args, cli.token.as_deref()),
        Commands::Init(args) => crate::wizard::run(args),
        Commands::Install(args) => crate::install::run(args, cli.token.as_deref()),
        Commands::Update(args) => crate::update::run(args, cli.token.as_deref()),
        Commands::Upgrade(args) => crate::upgrade::run(args, cli.token.as_deref()),
        Commands::Remove(args) => crate::remove::run(args),
        Commands::List(args) => crate::list::run(args),
        Commands::ScanDeps(args) => crate::scandeps::run(args, cli.token.as_deref()),
        Commands::JsonSchema(args) => crate::schema::run(args),
    }
}
