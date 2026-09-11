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
pub mod repology;

// Re-export key types for ergonomic `crate::index::Registry` access.
pub use registry::Registry;

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use std::path::PathBuf;

/// One search hit from any source.
///
/// The `newest`/`repos`/`outdated`/`vulnerable`/`host_*` fields are
/// populated by metadata-only sources (repology). Recipe/prebuilt sources
/// (lx-community, aur) leave them at their defaults.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct IndexHit {
    pub name: String,
    pub description: String,
    pub source: String,
    pub installed: bool,
    pub available: Vec<String>,
    // --- repology metadata (optional) ---
    /// Newest known version across all repos.
    pub newest: Option<String>,
    /// Total number of repos (families) carrying this project.
    pub repos: usize,
    /// How many repos have an outdated version.
    pub outdated: usize,
    /// How many repos have a vulnerable version.
    pub vulnerable: usize,
    /// The version in the host's own distro, if tracked.
    pub host_version: Option<String>,
    /// Status of the host distro's version (newest/outdated/vulnerable/…).
    pub host_status: Option<String>,
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
    /// Show packages where the host distro lags behind upstream
    Outdated(OutdatedOpts),
    /// Show the host distro's repology identity and tracked repo count
    Status,
    /// Compare repology projects against the recipe index + latest-debs org
    /// to find popular packages LX does not yet cover
    Coverage(CoverageOpts),
}

#[derive(Debug, Clone, Args)]
pub struct SearchOpts {
    pub pattern: Option<String>,
    #[arg(long)]
    pub raw: bool,
    /// Include distro metadata (repology) in results
    #[arg(long)]
    pub distro: bool,
    /// Output as JSON (machine-readable)
    #[arg(long)]
    pub json: bool,
    /// Only search this source (matches the index name from `lx index list`)
    #[arg(long)]
    pub repo: Option<String>,
    /// Show which indexes were searched and how many hits each returned
    #[arg(long)]
    pub verbose: bool,
}

#[derive(Debug, Clone, Args)]
pub struct OutdatedOpts {
    /// Limit output to N results
    #[arg(long, default_value = "50")]
    pub limit: usize,
}

#[derive(Debug, Clone, Args)]
pub struct CoverageOpts {
    /// Only count repology projects with at least this many repos
    #[arg(long, default_value = "5")]
    pub min_repos: usize,
    /// Show the top N gap-fillers
    #[arg(long, default_value = "30")]
    pub limit: usize,
    /// Only show the gap list (skip the covered/totals breakdown)
    #[arg(long)]
    pub gaps_only: bool,
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
    /// Output as JSON (machine-readable)
    #[arg(long)]
    pub json: bool,
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
        IndexCommands::Outdated(o) => run_outdated(o),
        IndexCommands::Status => run_status(),
        IndexCommands::Coverage(o) => run_coverage(o),
    }
}

fn run_search(opts: SearchOpts) -> Result<()> {
    let reg = registry::Registry::ensure_exists()?;
    let mut sources = registry::active_sources(&reg);
    // --repo filter: keep only the named source.
    if let Some(ref filter) = opts.repo {
        sources.retain(|s| s.name() == filter.as_str());
        if sources.is_empty() {
            bail!(
                "no enabled index named '{}'. Run `lx index list` to see available indexes.",
                filter
            );
        }
    }
    let mut all = Vec::new();
    let mut per_source_counts: Vec<(&str, usize)> = Vec::new();
    for src in &sources {
        match src.search(opts.pattern.as_deref()) {
            Ok(hits) => {
                let count = hits.len();
                all.extend(hits);
                if opts.verbose {
                    per_source_counts.push((src.name(), count));
                }
            }
            Err(e) => eprintln!("⚠ {}: {e:#}", src.name()),
        }
    }
    if opts.verbose {
        eprintln!(
            "searched {} index(es): {}",
            per_source_counts.len(),
            per_source_counts
                .iter()
                .map(|(n, c)| format!("{n}:{c}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    all.sort_by(|a, b| a.name.cmp(&b.name));
    all.dedup_by(|a, b| a.name == b.name && a.source == b.source);
    if all.is_empty() {
        println!("no packages matched");
        return Ok(());
    }
    if opts.json {
        println!("{}", serde_json::to_string_pretty(&all)?);
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
        // Repology hits carry richer metadata — render an extra distro line.
        let distro_info = if h.repos > 0 {
            let host = match &h.host_status {
                Some(s) if s == "outdated" || s == "legacy" => {
                    let newest = h.newest.as_deref().unwrap_or("?");
                    let hv = h.host_version.as_deref().unwrap_or("?");
                    format!(" ◆ distro: {hv} (newest {newest})")
                }
                Some(s) if s == "vulnerable" => {
                    let hv = h.host_version.as_deref().unwrap_or("?");
                    let vuln = h.vulnerable;
                    format!(" ◆ distro: {hv} ({vuln} vulnerable)")
                }
                Some(s) => {
                    let hv = h.host_version.as_deref().unwrap_or("?");
                    format!(" ◆ distro: {hv} ({s})")
                }
                None => String::new(),
            };
            let outdated_info = if h.outdated > 0 {
                format!(" [{}/{} repos outdated]", h.outdated, h.repos)
            } else {
                format!(" [{} repos]", h.repos)
            };
            format!("{host}{outdated_info}")
        } else {
            String::new()
        };
        if h.description.is_empty() {
            println!("{:<pad$}  {}{tag}{distro_info}", h.name, h.source);
        } else {
            println!(
                "{:<pad$}  {} [{source}]{avail}{tag}{distro_info}",
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
    // Collect all hits for JSON output.
    let mut all_hits: Vec<(&str, IndexHit)> = Vec::new();
    for src in &sources {
        match src.info(&opts.package) {
            Ok(Some(h)) => {
                found = true;
                if opts.json {
                    all_hits.push((src.name(), h));
                } else {
                    println!("[{}]", src.name());
                    print_info_hit(&h);
                }
            }
            Ok(None) => {}
            Err(e) => eprintln!("⚠ {}: {e:#}", src.name()),
        }
    }
    if opts.json {
        if all_hits.is_empty() {
            println!("[]");
        } else {
            let objects: Vec<serde_json::Value> = all_hits
                .iter()
                .map(|(src, h)| {
                    let mut v = serde_json::to_value(h).unwrap_or_default();
                    if let serde_json::Value::Object(ref mut m) = v {
                        m.insert("index".into(), serde_json::Value::String((*src).into()));
                    }
                    v
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&objects)?);
        }
        return Ok(());
    }
    if !found {
        println!("'{}' not found in any enabled index", opts.package);
    }
    Ok(())
}

/// Print a single info hit's human-readable fields.
fn print_info_hit(h: &IndexHit) {
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
    if h.repos > 0 {
        println!("  distros: {} repos", h.repos);
        if let Some(v) = &h.newest {
            println!("  newest: {v}");
        }
        if let Some(v) = &h.host_version {
            let status = h.host_status.as_deref().unwrap_or("unknown");
            println!("  host distro: {v} ({status})");
        }
        if h.outdated > 0 {
            println!("  outdated in: {} repos", h.outdated);
        }
        if h.vulnerable > 0 {
            println!("  vulnerable in: {} repos", h.vulnerable);
        }
    }
}

/// Return the set of locally installed package names by probing the host's
/// package manager (dpkg, rpm, or pacman). Used by `lx index coverage` to
/// avoid flagging already-installed packages as gaps.
///
/// Probes each available manager and returns the first one that yields
/// results — a manager that succeeds but returns nothing (e.g. dpkg-query
/// installed on an Arch host) is treated as "not the real manager" and we
/// fall through to the next.
fn installed_packages() -> Result<Vec<String>> {
    // dpkg (Debian/Ubuntu)
    if let Ok(out) = std::process::Command::new("dpkg-query")
        .args(["-W", "-f=${Package}\n"])
        .output()
    {
        if out.status.success() {
            let pkgs: Vec<String> = String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect();
            if !pkgs.is_empty() {
                return Ok(pkgs);
            }
        }
    }
    // rpm (Fedora/RHEL/openSUSE)
    if let Ok(out) = std::process::Command::new("rpm")
        .args(["-qa", "--queryformat", "%{NAME}\n"])
        .output()
    {
        if out.status.success() {
            let pkgs: Vec<String> = String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect();
            if !pkgs.is_empty() {
                return Ok(pkgs);
            }
        }
    }
    // pacman (Arch)
    if let Ok(out) = std::process::Command::new("pacman").arg("-Qq").output() {
        if out.status.success() {
            let pkgs: Vec<String> = String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect();
            if !pkgs.is_empty() {
                return Ok(pkgs);
            }
        }
    }
    Ok(Vec::new())
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
            registry::SourceKind::Repology => "repology".to_string(),
            registry::SourceKind::Custom { url } => format!("custom ({url})"),
        };
        println!("{:<16} {:<14} {}", s.name, kind, state);
    }
    Ok(())
}

fn run_outdated(opts: OutdatedOpts) -> Result<()> {
    let reg = registry::Registry::ensure_exists()?;
    // Find the repology source.
    let rep = reg
        .sources
        .iter()
        .find(|s| matches!(s.kind, registry::SourceKind::Repology) && s.enabled)
        .context("repology index not enabled; run `lx index update` first")?;
    let src = repology::RepologySource::new(&rep.name);
    let cache = src.load_cache()?;
    if cache.is_empty() {
        println!("no repology data cached. Run `lx index update` first.");
        return Ok(());
    }
    let outdated = repology::RepologySource::count_outdated(&cache);
    if outdated.is_empty() {
        println!("no outdated packages detected on this host distro.");
        return Ok(());
    }
    println!(
        "packages where host distro lags behind newest upstream (showing {} of {}):",
        opts.limit.min(outdated.len()),
        outdated.len()
    );
    let pad = outdated.iter().map(|(n, _, _)| n.len()).max().unwrap_or(0);
    for (name, host_ver, newest_ver) in outdated.iter().take(opts.limit) {
        println!(
            "  {:<pad$}  host: {host_ver}  →  newest: {newest_ver}",
            name
        );
    }
    Ok(())
}

fn run_status() -> Result<()> {
    let reg = registry::Registry::ensure_exists()?;
    println!("host distro information (repology):");
    if let Some(repo) = repology::RepologySource::host_repo_name() {
        println!("  repology repo: {repo}");
    } else {
        println!("  repology repo: unknown (could not detect host distro)");
    }
    // Count cached projects.
    if let Some(rep) = reg
        .sources
        .iter()
        .find(|s| matches!(s.kind, registry::SourceKind::Repology))
    {
        let src = repology::RepologySource::new(&rep.name);
        let cache = src.load_cache()?;
        println!("  cached projects: {}", cache.len());
        let stale = src.stale();
        println!(
            "  cache state: {}",
            if stale {
                "stale (run `lx index update`)"
            } else {
                "fresh"
            }
        );
    }
    Ok(())
}

fn run_coverage(opts: CoverageOpts) -> Result<()> {
    let reg = registry::Registry::ensure_exists()?;
    // 1. Load repology cache.
    let rep = reg
        .sources
        .iter()
        .find(|s| matches!(s.kind, registry::SourceKind::Repology) && s.enabled)
        .context("repology index not enabled; run `lx index update` first")?;
    let rep_src = repology::RepologySource::new(&rep.name);
    let cache = rep_src.load_cache()?;
    if cache.is_empty() {
        println!("no repology data cached. Run `lx index update` first.");
        return Ok(());
    }

    // 2. Build the set of names LX already covers.
    let mut covered = std::collections::HashSet::new();

    // Embedded templates (e.g. "rust/eza" → "eza").
    for (name, _) in crate::wizard::EMBEDDED_TEMPLATES {
        let short = name.rsplit('/').next().unwrap_or(name);
        covered.insert(short.to_ascii_lowercase());
    }

    // Community recipe index.
    let comm = lx_community::LxCommunitySource::new("lx-community");
    if let Ok(recipes) = comm.recipes() {
        for name in recipes.keys() {
            covered.insert(name.to_ascii_lowercase());
        }
    }

    // latest-debs org repos (strip "-debian" suffix).
    if let Ok(repos) = crate::search::org_repos(crate::debs::LATEST_DEBS_ORG, None) {
        for repo in &repos {
            if let Some(pkg) = repo.name.strip_suffix("-debian") {
                covered.insert(pkg.to_ascii_lowercase());
            }
        }
    }

    // Locally installed packages (dpkg/rpm/pacman) — already present on this
    // host, so they don't need a recipe even if the distro version is stale.
    if let Ok(installed) = installed_packages() {
        for pkg in installed {
            covered.insert(pkg.to_ascii_lowercase());
        }
    }

    // 3. Partition repology projects into covered vs gap.
    let mut gap: Vec<(String, usize)> = Vec::new();
    let mut covered_count = 0;
    let mut total_projects = 0;
    for (name, project) in &cache {
        let (newest, repos, _outdated, _vulnerable) =
            repology::RepologySource::distro_summary(project);
        if repos < opts.min_repos {
            continue;
        }
        // Skip rolling/unique-only projects (e.g. homebrew HEAD) with no
        // meaningful newest version.
        if newest.is_none() {
            continue;
        }
        total_projects += 1;
        if covered.contains(&name.to_ascii_lowercase()) {
            covered_count += 1;
        } else {
            gap.push((name.clone(), repos));
        }
    }

    // Sort gap by repo count descending (most popular un-packed first).
    gap.sort_by(|a, b| b.1.cmp(&a.1));

    // 4. Print.
    if !opts.gaps_only {
        println!("coverage: repology vs. LX (recipe index + latest-debs org)");
        let pct = if total_projects > 0 {
            (covered_count * 100) / total_projects
        } else {
            0
        };
        println!(
            "  repology projects (≥{} repos): {}",
            opts.min_repos, total_projects
        );
        println!("  covered by LX:                 {covered_count} ({pct}%)");
        println!("  gap (not yet packaged):        {}", gap.len());
        println!(
            "  coverage sources: {} embedded templates, {} community recipes, latest-debs org",
            crate::wizard::EMBEDDED_TEMPLATES.len(),
            comm.recipes().map(|r| r.len()).unwrap_or(0)
        );
        println!();
    }

    if gap.is_empty() {
        if !opts.gaps_only {
            println!("all popular repology projects are covered by LX.");
        }
        return Ok(());
    }

    if !opts.gaps_only {
        println!(
            "top gap-fillers by repo count (showing {} of {}):",
            opts.limit.min(gap.len()),
            gap.len()
        );
    }
    let pad = gap
        .iter()
        .take(opts.limit)
        .map(|(n, _)| n.len())
        .max()
        .unwrap_or(0);
    for (name, repos) in gap.iter().take(opts.limit) {
        println!("  {:<pad$}  {} repos", name, repos);
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
    fn registry_defaults_include_lx_community_aur_and_repology() {
        let reg = Registry::with_defaults();
        assert_eq!(reg.sources.len(), 3);
        assert!(reg.sources.iter().all(|s| s.enabled));
    }

    #[test]
    fn registry_crud_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("lx");
        std::fs::create_dir_all(&cfg).unwrap();
        std::env::set_var("XDG_CONFIG_HOME", dir.path());

        // Fresh registry seeds defaults (lx-community, aur, repology).
        let reg = Registry::ensure_exists().unwrap();
        assert_eq!(reg.sources.len(), 3);

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
            assert_eq!(r.sources.len(), 4);
        }
        {
            let mut r = Registry::load().unwrap();
            assert_eq!(r.sources.len(), 4);
            r.remove("custom").unwrap();
            assert_eq!(r.sources.len(), 3);
        }
        {
            let r = Registry::load().unwrap();
            assert_eq!(r.sources.len(), 3);
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


