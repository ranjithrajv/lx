// SPDX-License-Identifier: GPL-3.0-or-later
//! `lx index` — a unified package-index manager.
//!
//! One command, many upstream indexes. Each *source* (AUR, the LX community
//! index, a COPR, a custom apt repo, …) implements the [`IndexSource`] trait;
//! the registry in `~/.config/lx/indexes.yaml` lists the enabled ones.
//! `lx index search/install/info` fan out across every enabled source;
//! `lx index add/remove/list` manage the registry.

pub mod aur;
pub mod lx_community;
pub mod registry;

// Re-export key types for ergonomic `crate::index::Registry` access.
pub use registry::Registry;

use anyhow::{bail, Result};
use clap::{Args, Subcommand};
use std::path::PathBuf;

/// One search hit from any source.
#[derive(Debug, Clone)]
pub struct IndexHit {
    pub name: String,
    pub description: String,
    pub source: String,
    pub installed: bool,
    pub available: Vec<String>,
}

/// What any package index must implement. New backends (COPR, custom apt, …)
/// just implement this trait and add one line to
/// [`registry::active_sources`](registry::active_sources).
pub trait IndexSource: Send + Sync {
    fn name(&self) -> &str;
    fn search(&self, pattern: Option<&str>) -> Result<Vec<IndexHit>>;
    fn info(&self, package: &str) -> Result<Option<IndexHit>>;
    fn install(&self, package: &str, opts: InstallOpts) -> Result<()>;
    fn update(&self) -> Result<bool>;
}

#[derive(Debug, Clone, Default)]
pub struct InstallOpts {
    pub tag: Option<String>,
    pub build: bool,
    pub no_verify: bool,
    pub allow_unverified: bool,
    pub yes: bool,
    pub download_only: Option<PathBuf>,
}

// ---------------------------------------------------------------------------
// Host format detection
// ---------------------------------------------------------------------------

/// The package format natively consumed by this host (deb/rpm/arch). Used
/// to pick prebuilt binaries from an index and to fall back to building in
/// the right format.
#[derive(Debug, Clone, Copy, Default)]
pub enum InstallFormat {
    #[default]
    Deb,
    Rpm,
    Arch,
}

impl InstallFormat {
    /// Short display name for the format.
    pub fn name(&self) -> &'static str {
        match self {
            InstallFormat::Deb => "deb",
            InstallFormat::Rpm => "rpm",
            InstallFormat::Arch => "arch",
        }
    }

    /// File extension for this format (without dot).
    pub fn extension(&self) -> &'static str {
        match self {
            InstallFormat::Deb => "deb",
            InstallFormat::Rpm => "rpm",
            InstallFormat::Arch => "pkg.tar.zst",
        }
    }
}

/// Detect the host's native package format by probing for dpkg/rpm/pacman.
pub fn detect_host_format() -> InstallFormat {
    if std::process::Command::new("dpkg")
        .arg("--version")
        .output()
        .is_ok()
    {
        InstallFormat::Deb
    } else if std::process::Command::new("rpm")
        .arg("--version")
        .output()
        .is_ok()
    {
        InstallFormat::Rpm
    } else {
        InstallFormat::Arch
    }
}

// ---------------------------------------------------------------------------
// CLI-facing types & dispatch
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct IndexArgs {
    #[command(subcommand)]
    pub command: IndexCommands,
}

#[derive(Debug, Clone, Subcommand)]
pub enum IndexCommands {
    /// Full-text search over all enabled indexes
    Search(SearchOpts),
    /// Install a package (prebuilt first, fall back to building)
    Install(InstallOptsCli),
    /// Pull the latest recipes and prebuilt binaries for all indexes
    Update,
    /// Show a package's details from every index that has it
    Info(InfoOpts),
    /// List configured indexes
    List,
    /// Add an index to the registry
    Add(AddOpts),
    /// Remove an index from the registry
    Remove(RemoveOpts),
}

#[derive(Debug, Clone, Args)]
pub struct SearchOpts {
    pub pattern: Option<String>,
    #[arg(long)]
    pub raw: bool,
}

#[derive(Debug, Clone, Args)]
pub struct InstallOptsCli {
    pub package: String,
    #[arg(long)]
    pub tag: Option<String>,
    #[arg(long)]
    pub build: bool,
    #[arg(long)]
    pub no_verify: bool,
    #[arg(long)]
    pub allow_unverified: bool,
    #[arg(short = 'y', long)]
    pub yes: bool,
    #[arg(long)]
    pub download_only: Option<PathBuf>,
}

#[derive(Debug, Clone, Args)]
pub struct InfoOpts {
    pub package: String,
}

#[derive(Debug, Clone, Args)]
pub struct AddOpts {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Args)]
pub struct RemoveOpts {
    pub name: String,
}

pub fn run(args: IndexArgs, token: Option<&str>) -> Result<()> {
    let _ = token;
    match args.command {
        IndexCommands::Search(o) => run_search(o),
        IndexCommands::Install(o) => run_install(o),
        IndexCommands::Update => run_update(),
        IndexCommands::Info(o) => run_info(o),
        IndexCommands::List => run_list(),
        IndexCommands::Add(o) => run_add(o),
        IndexCommands::Remove(o) => run_remove(o),
    }
}

fn run_search(opts: SearchOpts) -> Result<()> {
    let reg = registry::Registry::ensure_exists()?;
    let sources = registry::active_sources(&reg);
    let mut all = Vec::new();
    for src in &sources {
        match src.search(opts.pattern.as_deref()) {
            Ok(hits) => all.extend(hits),
            Err(e) => eprintln!("⚠ {}: {e:#}", src.name()),
        }
    }
    all.sort_by(|a, b| a.name.cmp(&b.name));
    all.dedup_by(|a, b| a.name == b.name && a.source == b.source);
    if all.is_empty() {
        println!("no packages matched");
        return Ok(());
    }
    if opts.raw {
        for h in &all {
            println!("{:<20} {}", h.name, h.source);
        }
        return Ok(());
    }
    let pad = all.iter().map(|h| h.name.len()).max().unwrap_or(0);
    for h in &all {
        let tag = if h.installed { " [ installed ]" } else { "" };
        let avail = if h.available.is_empty() {
            String::new()
        } else {
            format!(" ({})", h.available.join(", "))
        };
        if h.description.is_empty() {
            println!("{:<pad$}  {}{tag}", h.name, h.source);
        } else {
            println!(
                "{:<pad$}  {} [{source}]{avail}{tag}",
                h.name,
                h.description,
                source = h.source
            );
        }
    }
    Ok(())
}

fn run_install(opts: InstallOptsCli) -> Result<()> {
    let reg = registry::Registry::ensure_exists()?;
    let sources = registry::active_sources(&reg);
    let io = InstallOpts {
        tag: opts.tag,
        build: opts.build,
        no_verify: opts.no_verify,
        allow_unverified: opts.allow_unverified,
        yes: opts.yes,
        download_only: opts.download_only,
    };
    for src in &sources {
        if src.info(&opts.package)?.is_some() {
            return src.install(&opts.package, io);
        }
    }
    bail!(
        "'{}' not found in any enabled index. Run `lx index search {}`.",
        opts.package,
        opts.package
    )
}

fn run_update() -> Result<()> {
    let reg = registry::Registry::ensure_exists()?;
    let sources = registry::active_sources(&reg);
    if sources.is_empty() {
        println!("no indexes enabled");
        return Ok(());
    }
    for src in &sources {
        match src.update() {
            Ok(true) => println!("{}: updated", src.name()),
            Ok(false) => println!("{}: up to date", src.name()),
            Err(e) => eprintln!("⚠ {}: {e:#}", src.name()),
        }
    }
    Ok(())
}

fn run_info(opts: InfoOpts) -> Result<()> {
    let reg = registry::Registry::ensure_exists()?;
    let sources = registry::active_sources(&reg);
    let mut found = false;
    for src in &sources {
        match src.info(&opts.package) {
            Ok(Some(h)) => {
                found = true;
                println!("[{}]", src.name());
                println!("  package: {}", h.name);
                if !h.description.is_empty() {
                    println!("  description: {}", h.description);
                }
                if h.installed {
                    println!("  installed: yes");
                }
                if !h.available.is_empty() {
                    println!("  available: {}", h.available.join(", "));
                }
            }
            Ok(None) => {}
            Err(e) => eprintln!("⚠ {}: {e:#}", src.name()),
        }
    }
    if !found {
        println!("'{}' not found in any enabled index", opts.package);
    }
    Ok(())
}

fn run_list() -> Result<()> {
    let reg = registry::Registry::ensure_exists()?;
    if reg.sources.is_empty() {
        println!("no indexes configured");
        return Ok(());
    }
    for s in &reg.sources {
        let state = if s.enabled { "enabled" } else { "disabled" };
        let kind = match &s.kind {
            registry::SourceKind::LxCommunity => "lx-community".to_string(),
            registry::SourceKind::Aur => "aur".to_string(),
            registry::SourceKind::Custom { url } => format!("custom ({url})"),
        };
        println!("{:<16} {:<14} {}", s.name, kind, state);
    }
    Ok(())
}

fn run_add(opts: AddOpts) -> Result<()> {
    let mut reg = registry::Registry::ensure_exists()?;
    reg.add(
        opts.name.clone(),
        registry::SourceKind::Custom { url: opts.url },
    )?;
    println!("added index '{}'", opts.name);
    Ok(())
}

fn run_remove(opts: RemoveOpts) -> Result<()> {
    let mut reg = registry::Registry::ensure_exists()?;
    reg.remove(&opts.name)?;
    println!("removed index '{}'", opts.name);
    Ok(())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::index::registry::{Registry, SourceKind};
    use std::collections::BTreeMap;

    #[test]
    fn registry_defaults_include_lx_community_and_aur() {
        let reg = Registry::with_defaults();
        assert_eq!(reg.sources.len(), 2);
        assert!(reg.sources.iter().all(|s| s.enabled));
    }

    #[test]
    fn registry_crud_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("lx");
        std::fs::create_dir_all(&cfg).unwrap();
        std::env::set_var("XDG_CONFIG_HOME", dir.path());

        // Fresh registry seeds defaults.
        let reg = Registry::ensure_exists().unwrap();
        assert_eq!(reg.sources.len(), 2);

        // Direct load/save round-trip.
        let path = Registry::config_path().unwrap();
        assert!(path.exists());

        // Add + remove a custom source via a fresh load each time.
        {
            let mut r = Registry::load().unwrap();
            r.add(
                "custom".into(),
                SourceKind::Custom {
                    url: "https://x/y.git".into(),
                },
            )
            .unwrap();
            assert_eq!(r.sources.len(), 3);
        }
        {
            let mut r = Registry::load().unwrap();
            assert_eq!(r.sources.len(), 3);
            r.remove("custom").unwrap();
            assert_eq!(r.sources.len(), 2);
        }
        {
            let r = Registry::load().unwrap();
            assert_eq!(r.sources.len(), 2);
        }

        // Duplicate add errors.
        {
            let mut r = Registry::load().unwrap();
            assert!(r
                .add(
                    "aur".into(),
                    SourceKind::Custom {
                        url: "https://x/y.git".into()
                    }
                )
                .is_err());
        }
        // Remove missing errors.
        {
            let mut r = Registry::load().unwrap();
            assert!(r.remove("nonexistent").is_err());
        }

        std::env::remove_var("XDG_CONFIG_HOME");
    }

    #[test]
    fn parsed_prebuilt_name_deb_with_dist() {
        let p = crate::index::lx_community::parse_prebuilt_name("eza_0.20.0-1+bookworm_amd64.deb")
            .unwrap();
        assert_eq!(p.arch, "amd64");
        assert_eq!(p.dist, "bookworm");
    }

    #[test]
    fn parsed_prebuilt_name_deb_without_dist() {
        let p = crate::index::lx_community::parse_prebuilt_name("foo_1.0-1_amd64.deb").unwrap();
        assert_eq!(p.arch, "amd64");
        assert_eq!(p.dist, "");
    }

    #[test]
    fn parsed_prebuilt_name_unknown_ext_falls_back() {
        // Unknown extension: take everything before the last '.' as stem.
        let p = crate::index::lx_community::parse_prebuilt_name("foo_1.0-1+bookworm_amd64.xyz")
            .unwrap();
        assert_eq!(p.arch, "amd64");
        assert_eq!(p.dist, "bookworm");
    }

    #[allow(dead_code)]
    fn _quiet() {
        let _: BTreeMap<String, String> = BTreeMap::new();
    }
}
