// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;
use clap::{Args, CommandFactory, Parser, Subcommand};
use std::path::Path;

#[derive(Parser)]
#[command(
    name = "lx",
    version,
    infer_subcommands = true,
    about = "lx: build (.deb/.rpm/Arch/.apk/.ipk) from forge releases or source, install, and distribute Linux packages — native bare-metal builds, no containers",
    long_about = "lx watches forge releases, fetches the release assets
matching each architecture, verifies their checksums against pinned metadata, and
builds .deb/.rpm/Arch/.apk/.ipk packages natively on bare metal — no containers, no emulation. It also installs,
upgrades, and removes the packages it builds — dispatching on the host's own package
manager (dpkg, rpm, pacman) — tracking what it manages in a local install manifest. The
package org is configurable with LX_INDEX_ORG (default: latest-debs).",
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
    /// Build packages from a package.yaml config
    Build(crate::build::BuildArgs),
    /// Build every requested format and generate its repository index in one
    /// run (the producer→distributor loop)
    Publish(crate::publish::PublishArgs),
    /// Convert a built package from one format to another (deb↔rpm↔arch)
    Convert(crate::convert::ConvertArgs),
    /// Validate a package.yaml config and check release availability (no build)
    Validate(crate::validate::ValidateArgs),
    /// Interactively generate a package.yaml config (or scaffold from a
    /// forge repo with `--from`, a template, an AUR PKGBUILD, or an
    /// nfpm.yaml with `--from-nfpm`)
    Init(crate::wizard::InitArgs),
    /// Fetch and install a pre-built native package (deb/rpm/arch).
    /// `--reinstall` re-installs (the recorded version for lx-managed packages)
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
    /// Shared-library dependency tooling: scan a binary's ELF needs, or
    /// resolve sonames to versioned `Depends` (`dpkg-shlibdeps` parity)
    Deps(DepsArgs),
    /// Show everything lx knows about one package (manifest + dpkg)
    Show(crate::show::ShowArgs),
    /// Detect and report the host OS and package system
    Info(crate::info::InfoArgs),
    /// Migrate to `lx`: carry legacy `lpt` state/workflows, or convert
    /// snap/flatpak/nix/`curl | sh` installs to native packages
    Migrate(MigrateRoot),
    /// Turn a directory of .debs into an apt-servable repository
    /// (Packages/Release/InRelease)
    Repo(crate::repo::RepoArgs),
    /// Community recipe index with prebuilt binaries (search/install/update)
    Index(crate::index::IndexArgs),
    /// Generate JSON schema for package.yaml
    #[command(alias = "json-schema", alias = "jsonschema")]
    Schema(crate::schema::SchemaArgs),
    /// Consumer commands for prebuilt packages (install/upgrade/inspect only, no building)
    #[command(subcommand)]
    Get(GetCommands),

    // --- hidden back-compat shims for top-level names that moved into a
    // group (`lx scan-deps` → `lx deps scan`, etc.). Kept working, not
    // advertised; see `docs/analysis/dogfooding-roadmap.md` and CHANGELOG. ---
    /// (moved) use `lx deps scan`
    #[command(hide = true)]
    ScanDeps(crate::scandeps::ScanDepsArgs),
    /// (moved) use `lx deps resolve`
    #[command(hide = true)]
    Shlibdeps(crate::shlibdeps::ShlibdepsArgs),
    /// (moved) use `lx migrate native`
    #[command(hide = true)]
    GoNative(crate::go_native::GoNativeArgs),
    /// (moved) use `lx install --reinstall`
    #[command(hide = true)]
    Reinstall(crate::reinstall::ReinstallArgs),
    /// (moved) use `lx init --from`
    #[command(hide = true)]
    Discover(crate::discovery::DiscoverArgs),
}

/// `lx deps` — shared-library dependency tooling.
#[derive(Debug, Clone, Args)]
pub struct DepsArgs {
    #[command(subcommand)]
    pub command: DepsCommands,
}

#[derive(Debug, Clone, Subcommand)]
pub enum DepsCommands {
    /// Scan a release binary's ELF shared-library dependencies (helps
    /// verify/fill in package.yaml's depends:)
    Scan(crate::scandeps::ScanDepsArgs),
    /// Resolve ELF libraries to versioned Debian `Depends` (dpkg-shlibdeps
    /// parity: reads the dpkg symbols/shlibs databases)
    Resolve(crate::shlibdeps::ShlibdepsArgs),
}

/// `lx migrate` — legacy-state and non-native migration. Bare `lx migrate`
/// (and `lx migrate --repo DIR`) keeps the historical `lpt` behavior, while
/// `lx migrate native …` is the former `lx go-native`. Named `MigrateRoot`
/// (not `MigrateArgs`) so clap's implicit argument-group id doesn't collide
/// with the flattened `migrate::MigrateArgs`.
#[derive(Debug, Clone, Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct MigrateRoot {
    #[command(subcommand)]
    pub command: Option<MigrateCommands>,

    #[command(flatten)]
    pub lpt: crate::migrate::MigrateArgs,
}

#[derive(Debug, Clone, Subcommand)]
pub enum MigrateCommands {
    /// Carry legacy `lpt` state (manifest, caches) and workflows to `lx`
    Lpt(crate::migrate::MigrateArgs),
    /// Migrate snap/flatpak/nix/curl|sh installs to native packages
    /// (plan by default, apply with --yes)
    Native(crate::go_native::GoNativeArgs),
}

#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
pub enum GetCommands {
    /// Fetch and install a pre-built native package
    Install(crate::install::InstallArgs),
    /// Upgrade lx-managed packages
    Upgrade(crate::upgrade::UpgradeArgs),
    /// Check installed packages against their latest release (no install)
    Update(crate::update::UpdateArgs),
    /// Remove an installed package
    Remove(crate::remove::RemoveArgs),
    /// Show everything known about one package
    Show(crate::show::ShowArgs),
    /// Reinstall the recorded version of a package
    Reinstall(crate::reinstall::ReinstallArgs),
    /// Reinstall a prior generation of an lx-managed package
    Rollback(crate::rollback::RollbackArgs),
    /// List packages installed by lx
    List(crate::list::ListArgs),
    /// Search available packages (regex; --local for the offline index)
    Search(crate::search::SearchArgs),
}

pub fn run(cli: Cli) -> Result<()> {
    // A command whose only input is `package.yaml` was invoked with the
    // implicit default path and no such file exists: show that command's help
    // instead of a bare "failed to read config file" error. An explicitly
    // named config that is missing still errors, so scripts/CI can't pass on a
    // typo.
    if let Some(path) = help_path_for_bare_invocation(&cli.command) {
        return print_help_for(&path);
    }

    match cli.command {
        Commands::Build(args) => run_build(args, cli.token.as_deref()),
        Commands::Publish(args) => crate::publish::run(args, cli.token.as_deref()),
        Commands::Convert(args) => crate::convert::run(args),
        Commands::Validate(args) => crate::validate::run(args, cli.token.as_deref()),
        Commands::Init(args) => crate::wizard::run(args),
        Commands::Install(args) => crate::install::run(args, cli.token.as_deref()),
        Commands::Update(args) => crate::update::run(args, cli.token.as_deref()),
        Commands::Upgrade(args) => crate::upgrade::run(args, cli.token.as_deref()),
        Commands::Remove(args) => crate::remove::run(args),
        Commands::Rollback(args) => crate::rollback::run(args, cli.token.as_deref()),
        Commands::List(args) => crate::list::run(args),
        Commands::Show(args) => crate::show::run(args),
        Commands::Info(args) => crate::info::run(args),
        Commands::Repo(args) => crate::repo::run(args),
        Commands::Migrate(args) => match args.command {
            Some(MigrateCommands::Lpt(a)) => crate::migrate::run(a),
            Some(MigrateCommands::Native(a)) => crate::go_native::run(a, cli.token.as_deref()),
            None => crate::migrate::run(args.lpt),
        },
        Commands::Search(args) => crate::search::run(args, cli.token.as_deref()),
        Commands::Index(args) => crate::index::run(args, cli.token.as_deref()),
        Commands::Deps(args) => match args.command {
            DepsCommands::Scan(a) => crate::scandeps::run(a, cli.token.as_deref()),
            DepsCommands::Resolve(a) => crate::shlibdeps::run(a),
        },
        Commands::Schema(args) => crate::schema::run(args),
        Commands::Get(cmd) => match cmd {
            GetCommands::Install(a) => crate::install::run(a, cli.token.as_deref()),
            GetCommands::Upgrade(a) => crate::upgrade::run(a, cli.token.as_deref()),
            GetCommands::Update(a) => crate::update::run(a, cli.token.as_deref()),
            GetCommands::Remove(a) => crate::remove::run(a),
            GetCommands::Show(a) => crate::show::run(a),
            GetCommands::Reinstall(a) => crate::reinstall::run(a, cli.token.as_deref()),
            GetCommands::Rollback(a) => crate::rollback::run(a, cli.token.as_deref()),
            GetCommands::List(a) => crate::list::run(a),
            GetCommands::Search(a) => crate::search::run(a, cli.token.as_deref()),
        },
        // Hidden back-compat shims (names that moved into a group).
        Commands::ScanDeps(args) => crate::scandeps::run(args, cli.token.as_deref()),
        Commands::Shlibdeps(args) => crate::shlibdeps::run(args),
        Commands::GoNative(args) => crate::go_native::run(args, cli.token.as_deref()),
        Commands::Reinstall(args) => crate::reinstall::run(args, cli.token.as_deref()),
        Commands::Discover(args) => crate::discovery::run(args, cli.token.as_deref()),
    }
}

/// Dispatch `lx build`, expanding `--format all` / `--format a,b` into one
/// build per format. A single format passes straight through, so its
/// validation and error messages are unchanged.
fn run_build(args: crate::build::BuildArgs, token: Option<&str>) -> Result<()> {
    // Mirror the bash action's `./failed-build-logs/` artifact dir so the
    // GitHub Action can upload it on failure; the error passes through
    // unchanged.
    let log = |e: &anyhow::Error| crate::build::write_failed_build_log(&format!("{e:#}"));

    let expanded = args
        .format
        .as_deref()
        .and_then(crate::plugins::expand_formats);
    let Some(formats) = expanded else {
        return crate::build::run(args, token).inspect_err(log);
    };
    for format in formats {
        let mut per = args.clone();
        per.format = Some(format);
        crate::build::run(per, token).inspect_err(log)?;
    }
    Ok(())
}

/// The clap path to the command whose help should be shown for a bare
/// invocation with no default `package.yaml`, or `None` if this invocation
/// isn't that case. Covers every command whose input defaults to
/// `package.yaml`: `build`, `publish`, `validate`, and `deps scan` (plus the
/// hidden `scan-deps` shim).
fn help_path_for_bare_invocation(command: &Commands) -> Option<Vec<&'static str>> {
    match command {
        Commands::Build(a) if is_bare_build_without_config(a) => Some(vec!["build"]),
        Commands::Publish(a) if is_implicit_missing_default(&a.config) => Some(vec!["publish"]),
        Commands::Validate(a) if is_implicit_missing_default(&a.config) => Some(vec!["validate"]),
        Commands::Deps(d) => match &d.command {
            DepsCommands::Scan(a) if is_implicit_missing_default(&a.config) => {
                Some(vec!["deps", "scan"])
            }
            _ => None,
        },
        Commands::ScanDeps(a) if is_implicit_missing_default(&a.config) => Some(vec!["scan-deps"]),
        _ => None,
    }
}

/// True for the implicit `lx build` invocation — the default `package.yaml`
/// path was used but that file is absent, and no alternative input was given
/// (`--all`, `--from-dir`/`--from-file`, or a forge URL passed in place of the
/// config, which the filename equality check also excludes).
fn is_bare_build_without_config(args: &crate::build::BuildArgs) -> bool {
    args.all.is_none()
        && args.from_dir.is_none()
        && args.from_file.is_none()
        && is_implicit_missing_default(&args.config)
}

/// True when `config` is the implicit `package.yaml` default and that file
/// does not exist. An explicitly named path never matches, so a missing file
/// the user asked for by name is still a real error.
fn is_implicit_missing_default(config: &Path) -> bool {
    config == Path::new(lx_lib::constants::DEFAULT_CONFIG_FILENAME) && !config.exists()
}

/// Print the help of the subcommand at `path` (e.g. `["deps", "scan"]`) exactly
/// as `<path> --help` would (through clap, so the usage line keeps its full
/// `lx …` prefix).
fn print_help_for(path: &[&str]) -> Result<()> {
    let mut argv = vec!["lx"];
    argv.extend_from_slice(path);
    argv.push("--help");
    let help = Cli::command()
        .try_get_matches_from(argv)
        .expect_err("`--help` always yields a clap DisplayHelp error");
    help.print()?;
    Ok(())
}
