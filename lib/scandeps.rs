// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{anyhow, bail, Context, Result};
use clap::Args;
use std::path::{Path, PathBuf};

use crate::config::PackageConfig;
use lx_lib::github::Asset;

#[derive(Debug, Clone, Args)]
pub struct ScanDepsArgs {
    /// Path to package.yaml, or a bare GitHub URL for a zero-config scan.
    #[arg(default_value = lx_lib::constants::DEFAULT_CONFIG_FILENAME)]
    pub config: PathBuf,

    /// Version of the software to scan (overrides any version in config).
    #[arg(short = 'v', long)]
    pub version: Option<String>,

    /// Restrict to specific architectures (comma-separated). Default: this
    /// machine's own architecture only (auto-detected via `uname -m`).
    #[arg(long)]
    pub architectures: Option<String>,

    /// Scan every architecture the release publishes, instead of just this
    /// machine's own.
    #[arg(long)]
    pub all_architectures: bool,

    /// Print a why-depends report: every non-essential shared-library need
    /// found, whether it's covered by package.yaml's `depends:`, and (when
    /// resolvable locally via the host's package manager) the package that
    /// owns it.
    #[arg(long)]
    pub explain: bool,

    /// Prefer musl-static release assets (e.g. `*-linux-musl*`) over glibc
    /// variants when scanning. A musl binary has no glibc dependency and
    /// runs on any Linux regardless of distro age.
    #[arg(long)]
    pub prefer_musl: bool,
}

/// Downloads a release binary and reports its `DT_NEEDED` shared-library
/// dependencies (`lx_lib::elfdeps`), to help verify or fill in
/// `package.yaml`'s `depends:` field. Read-only: unlike `build`/`install`,
/// nothing scanned here is ever installed or built into a package, so it
/// deliberately skips checksum verification -- the downloaded bytes are
/// only ever inspected locally, never trusted onto the system.
pub fn run(args: ScanDepsArgs, token: Option<&str>) -> Result<()> {
    let mut cfg = match crate::plugins::forge::parse_any_forge_url(&args.config.to_string_lossy()) {
        Some((source, repo)) => {
            println!("Zero-config scan of {source}:{repo} (no package.yaml)");
            let cfg = PackageConfig {
                package_name: repo.split('/').next_back().unwrap_or(&repo).to_string(),
                github_repo: repo.clone(),
                source: source.clone(),
                ..PackageConfig::default()
            };
            cfg.validate()?;
            cfg
        }
        None => {
            match crate::plugins::forge::github::parse_github_url(&args.config.to_string_lossy()) {
                Some(github_repo) => {
                    println!("Zero-config scan of {github_repo} (no package.yaml)");
                    let cfg = PackageConfig {
                        package_name: github_repo
                            .split('/')
                            .next_back()
                            .unwrap_or(&github_repo)
                            .to_string(),
                        github_repo,
                        ..PackageConfig::default()
                    };
                    cfg.validate()?;
                    cfg
                }
                None => PackageConfig::load(&args.config)?,
            }
        }
    };
    if let Some(v) = &args.version {
        cfg.version = v.clone();
    }

    let source_name = cfg.effective_forge_source();
    let source = crate::plugins::forge::get_forge_source(&source_name).ok_or_else(|| {
        anyhow!(
            "unsupported source '{}' (expected one of: {})",
            source_name,
            crate::plugins::forge::forge_source_names().join(", ")
        )
    })?;
    crate::plugins::forge::apply_forge_host(source.as_ref(), &cfg);
    let token_for_source = crate::plugins::forge::resolve_forge_token(source.as_ref(), token);

    let release = if !cfg.version.is_empty() {
        match source.release_by_tag(
            &cfg.github_repo,
            &cfg.version,
            token_for_source.as_deref(),
            None,
        ) {
            Ok(r) => r,
            Err(e) => {
                crate::build::suggest_versions_source(
                    source.as_ref(),
                    &cfg.github_repo,
                    &cfg.version,
                    token_for_source.as_deref(),
                    None,
                );
                return Err(e);
            }
        }
    } else {
        source.latest_release(&cfg.github_repo, token_for_source.as_deref(), None)?
    };
    crate::build::warn_if_prerelease_or_draft(&release);

    let arch_assets = if cfg.has_manual_patterns() {
        crate::build::resolve_manual(&cfg, &release)?
    } else {
        let auto = crate::discovery::config_from_release_with_musl(
            &cfg.github_repo,
            &release,
            args.prefer_musl,
        )?;
        crate::build::resolve_manual(&auto, &release)?
    };
    if arch_assets.is_empty() {
        bail!("no release assets matched any architecture");
    }

    if args.all_architectures && args.architectures.is_some() {
        bail!("--all-architectures conflicts with --architectures; pass one or the other");
    }

    let mut target_archs: Vec<String> = if args.all_architectures {
        arch_assets.keys().cloned().collect()
    } else if let Some(a) = &args.architectures {
        a.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    } else {
        let detected = crate::build::host_arch().ok_or_else(|| {
            anyhow!(
                "could not detect this machine's architecture from `uname -m`; \
                 pass --architectures or --all-architectures explicitly"
            )
        })?;
        vec![detected]
    };
    target_archs.sort();

    let tmp = tempfile::tempdir().context("failed to create temp dir")?;
    let mut any_scanned = false;
    let mut any_failed = false;
    let mut all_sonames: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    // Each architecture is isolated: a download/extract/parse failure on
    // one (e.g. an unsupported `zip` artifact_format) reports and moves on
    // to the rest, mirroring `build_jobs`'s per-architecture error handling
    // rather than aborting the whole multi-architecture scan on the first
    // failure.
    for arch in &target_archs {
        let Some(asset) = arch_assets.get(arch) else {
            println!("⚠️  no release asset for architecture '{arch}'; skipped");
            continue;
        };
        match scan_one_arch(
            source.as_ref(),
            token_for_source.as_deref(),
            &cfg,
            arch,
            asset,
            tmp.path(),
            &mut all_sonames,
        ) {
            Ok(scanned) => any_scanned = any_scanned || scanned,
            Err(e) => {
                eprintln!("  ✗ {arch}: {e:#}");
                any_failed = true;
            }
        }
    }

    if !any_scanned {
        if any_failed {
            bail!("every requested architecture failed to scan (see errors above)");
        }
        bail!("no ELF binaries found to scan across the requested architecture(s)");
    }

    let non_essential: Vec<&String> = all_sonames
        .iter()
        .filter(|s| !lx_lib::elfdeps::is_essential_libc_soname(s))
        .collect();
    if non_essential.is_empty() {
        println!("\nNo non-essential shared-library dependencies found.");
    } else if args.explain {
        print_why_depends(&cfg, &non_essential);
    } else {
        println!(
            "\n{} non-essential shared-library dependency(ies) found:",
            non_essential.len()
        );
        for lib in &non_essential {
            println!("  - {lib}");
        }
        if cfg.depends.trim().is_empty() {
            println!(
                "\npackage.yaml's `depends:` is currently empty -- review the above and add \
                 any that aren't already satisfied by a bare Debian install."
            );
        } else {
            println!("\npackage.yaml's current `depends:` is: {}", cfg.depends);
        }
    }

    Ok(())
}

/// Declared package names from a relation field (`depends:`,
/// `recommends:`, ...), stripped of version constraints (`libfoo (>= 1.0)`
/// -> `libfoo`) and alternatives (`a | b` -> `a`, `b`), lowercased for
/// case-insensitive matching against package-manager output.
pub fn declared_package_names(relation: &str) -> std::collections::BTreeSet<String> {
    relation
        .split(',')
        .flat_map(|clause| clause.split('|'))
        .map(|s| s.split('(').next().unwrap_or(s).trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// `lx why-depends` (nfpm/Nix closure parity, minus a full closure graph):
/// ties each non-essential shared-library need to the owning package
/// (best-effort via the host's local package manager) and flags whether
/// that owner is covered by package.yaml's `depends:`, so
/// declared-vs-actual runtime deps can be reviewed without walking the
/// whole ELF graph by hand.
fn print_why_depends(cfg: &PackageConfig, non_essential: &[&String]) {
    let declared = declared_package_names(&cfg.depends);
    println!(
        "\nwhy-depends report ({} shared librar{}):",
        non_essential.len(),
        if non_essential.len() == 1 { "y" } else { "ies" }
    );
    let mut undeclared = Vec::new();
    for lib in non_essential {
        match pkg_owner(lib) {
            Some(pkg) => {
                let pkg_lc = pkg.to_ascii_lowercase();
                if declared.contains(&pkg_lc) {
                    println!("  {lib:<32} -> {pkg:<24} [declared in depends:]");
                } else {
                    println!("  {lib:<32} -> {pkg:<24} [NOT in depends: -- consider adding]");
                    undeclared.push(pkg);
                }
            }
            None => {
                println!("  {lib:<32} -> ? (not owned by any locally installed package)");
            }
        }
    }
    if undeclared.is_empty() {
        println!(
            "\nEvery owned shared-library need is already covered by package.yaml's `depends:`."
        );
    } else {
        undeclared.sort();
        undeclared.dedup();
        println!(
            "\n{} package(s) not declared in `depends:`: {}",
            undeclared.len(),
            undeclared.join(", ")
        );
    }
}

/// Download, extract, and scan one architecture's asset. Returns `Ok(true)`
/// if at least one ELF binary was found and reported, `Ok(false)` if the
/// architecture resolved cleanly but had nothing to scan (e.g. a
/// `binary_path` that doesn't exist in this asset), and `Err` on a real
/// failure (download, unsupported/corrupt archive format, unreadable ELF)
/// -- left to the caller to report and move on to the next architecture
/// rather than aborting the whole scan.
fn scan_one_arch(
    source: &dyn crate::plugins::forge::ForgeSource,
    token: Option<&str>,
    cfg: &PackageConfig,
    arch: &str,
    asset: &Asset,
    tmp_dir: &Path,
    all_sonames: &mut std::collections::BTreeSet<String>,
) -> Result<bool> {
    let mut cfg = cfg.clone();
    if cfg.artifact_format.is_empty() {
        cfg.artifact_format = crate::discovery::guess_format(&asset.name).to_string();
    }

    println!("\n{arch}: {}", asset.name);
    // Own subdirectory per architecture (not a filename prefix) so
    // `raw`-format assets (e.g. AppImages) -- whose extracted name comes
    // from the downloaded file's own on-disk name, see `build::extract`'s
    // "raw" branch -- keep their real asset name instead of leaking an
    // internal disambiguation prefix into it.
    let asset_dir = tmp_dir.join(format!("{arch}-download"));
    std::fs::create_dir_all(&asset_dir)?;
    let asset_path = asset_dir.join(&asset.name);
    println!("  ↓ downloading (unverified -- inspected locally only, never installed)");
    {
        if asset.browser_download_url.is_empty() {
            bail!("asset '{}' has no download URL", asset.name);
        }
        let mut body = source
            .raw_get(&asset.browser_download_url, token)
            .context("asset download failed")?;
        let mut file = std::fs::File::create(&asset_path)?;
        std::io::copy(&mut body, &mut file)?;
    }

    let extract_dir = tmp_dir.join(format!("{arch}-scan-extract"));
    crate::build::extract(&asset_path, &extract_dir, &cfg.artifact_format)?;

    let binary_dir = if cfg.binary_path.is_empty() {
        extract_dir.clone()
    } else {
        extract_dir.join(&cfg.binary_path)
    };
    if !binary_dir.is_dir() {
        println!(
            "  ⚠️  binary_path '{}' not found in archive; skipped",
            cfg.binary_path
        );
        return Ok(false);
    }

    let elf_files = find_elf_files(&binary_dir)?;
    if elf_files.is_empty() {
        println!("  (no ELF binaries found)");
        return Ok(false);
    }

    let mut scanned_any = false;
    for elf_path in elf_files {
        scanned_any = true;
        let rel = elf_path.strip_prefix(&extract_dir).unwrap_or(&elf_path);
        let bytes = std::fs::read(&elf_path)
            .with_context(|| format!("failed to read '{}'", elf_path.display()))?;
        let libs = lx_lib::elfdeps::needed_libraries(&bytes)
            .with_context(|| format!("failed to scan '{}'", rel.display()))?;
        if libs.is_empty() {
            println!(
                "  {}: statically linked (no shared-library dependencies)",
                rel.display()
            );
            continue;
        }
        println!("  {}:", rel.display());
        for lib in &libs {
            all_sonames.insert(lib.clone());
            if lx_lib::elfdeps::is_essential_libc_soname(lib) {
                println!("    {lib}  (glibc/essential, usually omit from depends:)");
            } else if let Some(pkg) = pkg_owner(lib) {
                println!("    {lib}  -> {pkg} (via local package manager)");
            } else {
                println!("    {lib}");
            }
        }
    }
    Ok(scanned_any)
}

/// Recursively collect every ELF file under `dir` (handles both flat
/// release layouts and `bundle: true`'s nested `bin/`+`lib/` trees).
pub fn find_elf_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading '{}'", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            out.extend(find_elf_files(&path)?);
        } else if crate::build::is_elf(&path).unwrap_or(false) {
            out.push(path);
        }
    }
    Ok(out)
}

/// Scan a directory for ELF binaries and compute runtime `depends:` from
/// their `DT_NEEDED` sonames. Essential/libc sonames are skipped; remaining
/// sonames are resolved to versioned relations via the host's dpkg
/// `symbols`/`shlibs` databases when available, else to bare package names
/// via the host's package manager.
///
/// Returns `(depends_string, non_essential_sonames)` where:
/// - `depends_string` is ready to use in a PackageConfig
/// - `non_essential_sonames` is the raw set of scanned sonames (for
///   comparison against declared deps)
pub fn compute_depends_from_dir(
    dir: &std::path::Path,
    declared_depends: &str,
    musl: bool,
) -> (String, std::collections::BTreeSet<String>) {
    let elfs = find_elf_files(dir).unwrap_or_default();
    let scan = lx_lib::shlibdeps::scan_elfs(&elfs);

    // Prefer the dpkg symbols/shlibs databases (versioned, dpkg-shlibdeps
    // parity); fall back per-soname to the host package-manager lookup.
    let db = lx_lib::shlibdeps::ShlibsDb::host();
    let res = lx_lib::shlibdeps::resolve(&scan.needs, db, None, &std::collections::BTreeSet::new());
    let mut names = res.resolved_names;
    let mut pkgs = res.relations;
    for soname in &res.unresolved {
        if let Some(pkg) = pkg_owner(soname) {
            if names.insert(pkg.to_ascii_lowercase()) {
                pkgs.push(pkg);
            }
        }
    }
    pkgs.sort();

    // Non-essential sonames, for declared-vs-actual comparison.
    let non_essential = scan.sonames;

    if pkgs.is_empty() {
        if musl {
            return (String::new(), non_essential);
        }
        if !declared_depends.trim().is_empty() {
            return (declared_depends.trim().to_string(), non_essential);
        }
        return ("libc6".to_string(), non_essential);
    }

    (
        pkgs.into_iter().collect::<Vec<_>>().join(", "),
        non_essential,
    )
}

/// Compare scanned ELF sonames against declared deps and report gaps.
/// Returns lists of (missing, unnecessary) package names.
pub fn diff_deps(
    scanned_sonames: &std::collections::BTreeSet<String>,
    declared_depends: &str,
) -> (Vec<String>, Vec<String>) {
    let declared = declared_package_names(declared_depends);

    // Resolve sonames to package names for comparison.
    let mut scanned_pkgs = std::collections::BTreeSet::new();
    for soname in scanned_sonames {
        if let Some(pkg) = pkg_owner(soname) {
            let pkg = pkg.split(':').next().unwrap_or(&pkg).to_string();
            scanned_pkgs.insert(pkg.to_ascii_lowercase());
        }
    }

    let missing: Vec<String> = scanned_pkgs
        .iter()
        .filter(|p| !declared.contains(*p))
        .cloned()
        .collect();
    let unnecessary: Vec<String> = declared
        .iter()
        .filter(|p| !scanned_pkgs.contains(*p))
        .cloned()
        .collect();

    (missing, unnecessary)
}

/// Best-effort: the package that locally owns `soname`, auto-detecting the
/// host's package manager (dpkg, rpm, or pacman). `None` when no supported
/// package manager is on `PATH`, the library isn't installed on this host,
/// or the lookup otherwise fails -- this is informational only, not
/// authoritative for the target architecture/suite.
pub fn pkg_owner(soname: &str) -> Option<String> {
    // dpkg -S <soname> — Debian/Ubuntu
    if let Some(owner) = pkg_owner_deb(soname) {
        return Some(owner);
    }
    // rpm -q --whatprovides <soname> — Fedora/RHEL/openSUSE
    if let Some(owner) = pkg_owner_rpm(soname) {
        return Some(owner);
    }
    // pacman -Qo <resolved-path> — Arch/Manjaro
    pkg_owner_pacman(soname)
}

fn pkg_owner_deb(soname: &str) -> Option<String> {
    let out = std::process::Command::new("dpkg")
        .args(["-S", soname])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    let pkg = text.lines().next()?.split(':').next()?.trim();
    if pkg.is_empty() {
        None
    } else {
        // Strip any :arch qualifier dpkg -S may report.
        Some(pkg.split(':').next().unwrap_or(pkg).to_string())
    }
}

fn pkg_owner_rpm(soname: &str) -> Option<String> {
    let out = std::process::Command::new("rpm")
        .args(["-q", "--whatprovides", soname])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    let line = text.lines().next()?.trim();
    // "no package provides <soname>" means no match.
    if line.starts_with("no package provides") || line.is_empty() {
        return None;
    }
    // rpm -q --whatprovides returns full NEVRA; strip to just the name.
    // Epoch:name-version-release.arch → name
    let name = line
        .rsplit_once(':')
        .map(|(_, after)| after) // after last ':'
        .unwrap_or(line);
    // Strip version-release.arch suffix: "name-1.2.3-1.fc39.x86_64" → "name"
    let name = name.rsplit_once('-').map(|(n, _)| n).unwrap_or(name);
    // Handle epoch: "1:name" → "name"
    let name = name.split_once(':').map(|(_, n)| n).unwrap_or(name);
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn pkg_owner_pacman(soname: &str) -> Option<String> {
    // Resolve the soname to a filesystem path via ldconfig.
    let ldconfig = std::process::Command::new("ldconfig")
        .args(["-p"])
        .output()
        .ok()?;
    if !ldconfig.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&ldconfig.stdout);
    let path = text
        .lines()
        .find(|line: &&str| {
            // ldconfig -p format: "\tlibfoo.so.1 (libc6,x86-64) => /usr/lib/libfoo.so.1"
            line.contains(soname) && line.contains("=>")
        })
        .and_then(|line: &str| line.split("=>").last())
        .map(str::trim)
        .map(str::to_string)?;

    let out = std::process::Command::new("pacman")
        .args(["-Qo", &path])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    // pacman -Qo output: "/usr/lib/libfoo.so.1 is owned by package 1.2.3-1"
    let after = text.split("is owned by").last()?;
    let pkg = after.split_whitespace().next()?;
    if pkg.is_empty() {
        None
    } else {
        Some(pkg.to_string())
    }
}

/// Detected host package manager for soname-to-package resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PkgMgr {
    Dpkg,
    Rpm,
    Pacman,
}

/// Detect the host's package manager, if any.
pub fn detect_pkg_mgr() -> Option<PkgMgr> {
    if std::process::Command::new("dpkg")
        .arg("--version")
        .output()
        .is_ok()
    {
        return Some(PkgMgr::Dpkg);
    }
    if std::process::Command::new("rpm")
        .arg("--version")
        .output()
        .is_ok()
    {
        return Some(PkgMgr::Rpm);
    }
    if std::process::Command::new("pacman")
        .arg("--version")
        .output()
        .is_ok()
    {
        return Some(PkgMgr::Pacman);
    }
    None
}

/// Cross-platform: installed version of a package, or None.
pub fn pkg_installed_version(package: &str) -> Option<String> {
    match detect_pkg_mgr()? {
        PkgMgr::Dpkg => {
            let out = std::process::Command::new("dpkg-query")
                .args(["-W", "-f=${Version}", package])
                .output()
                .ok()?;
            if !out.status.success() {
                return None;
            }
            let v = String::from_utf8(out.stdout).ok()?.trim().to_string();
            if v.is_empty() {
                None
            } else {
                Some(v)
            }
        }
        PkgMgr::Rpm => {
            let out = std::process::Command::new("rpm")
                .args(["-q", "--queryformat", "%{VERSION}-%{RELEASE}", package])
                .output()
                .ok()?;
            if !out.status.success() {
                return None;
            }
            let text = String::from_utf8(out.stdout).ok()?;
            let line = text.trim().to_string();
            if line.starts_with("not installed") || line.is_empty() {
                None
            } else {
                Some(line)
            }
        }
        PkgMgr::Pacman => {
            let out = std::process::Command::new("pacman")
                .args(["-Qi", package])
                .output()
                .ok()?;
            if !out.status.success() {
                return None;
            }
            let text = String::from_utf8(out.stdout).ok()?;
            for line in text.lines() {
                if let Some(v) = line.strip_prefix("Version         : ") {
                    let v = v.trim().to_string();
                    if !v.is_empty() {
                        return Some(v);
                    }
                }
            }
            None
        }
    }
}

/// Cross-platform: runtime dependencies of an installed package.
/// Returns bare package names (version constraints stripped).
pub fn pkg_depends(package: &str) -> Vec<String> {
    match detect_pkg_mgr() {
        Some(PkgMgr::Dpkg) => {
            let Ok(out) = std::process::Command::new("dpkg-query")
                .args(["-W", "-f=${Depends}", package])
                .output()
            else {
                return Vec::new();
            };
            if !out.status.success() {
                return Vec::new();
            }
            let Ok(field) = String::from_utf8(out.stdout) else {
                return Vec::new();
            };
            crate::debs::parse_depends_field(&field)
        }
        Some(PkgMgr::Rpm) => {
            let Ok(out) = std::process::Command::new("rpm")
                .args(["-q", "--requires", package])
                .output()
            else {
                return Vec::new();
            };
            if !out.status.success() {
                return Vec::new();
            }
            let Ok(text) = String::from_utf8(out.stdout) else {
                return Vec::new();
            };
            text.lines()
                .map(str::trim)
                .filter(|l| {
                    !l.is_empty()
                        && !l.starts_with('/')
                        && !l.starts_with("rpmlib(")
                        && !l.starts_with("config(")
                })
                .map(|l| {
                    // Strip version constraint: "libfoo >= 1.0" → "libfoo"
                    l.split_whitespace().next().unwrap_or(l).to_string()
                })
                .collect()
        }
        Some(PkgMgr::Pacman) => {
            let Ok(out) = std::process::Command::new("pacman")
                .args(["-Qi", package])
                .output()
            else {
                return Vec::new();
            };
            if !out.status.success() {
                return Vec::new();
            }
            let Ok(text) = String::from_utf8(out.stdout) else {
                return Vec::new();
            };
            let mut in_deps = false;
            let mut deps = Vec::new();
            for line in text.lines() {
                if line.starts_with("Depends On     : ") {
                    let val = line.split_once(':').map(|(_, v)| v).unwrap_or("").trim();
                    if val == "None" || val.is_empty() {
                        return Vec::new();
                    }
                    // Depends On is space-separated on one line.
                    for dep in val.split_whitespace() {
                        // Strip version: "libfoo>=1.0" → "libfoo"
                        let name = dep
                            .split_once(['>', '<', '='])
                            .map(|(n, _)| n)
                            .unwrap_or(dep);
                        if !name.is_empty() {
                            deps.push(name.to_string());
                        }
                    }
                    in_deps = true;
                } else if in_deps && line.starts_with(' ') {
                    // Continuation line (rare but possible).
                    for dep in line.split_whitespace() {
                        let name = dep
                            .split_once(['>', '<', '='])
                            .map(|(n, _)| n)
                            .unwrap_or(dep);
                        if !name.is_empty() {
                            deps.push(name.to_string());
                        }
                    }
                } else if in_deps {
                    break;
                }
            }
            deps
        }
        None => Vec::new(),
    }
}

/// Cross-platform: list files owned by an installed package.
pub fn pkg_files(package: &str) -> Vec<String> {
    match detect_pkg_mgr() {
        Some(PkgMgr::Dpkg) => {
            let Ok(out) = std::process::Command::new("dpkg")
                .args(["-L", package])
                .output()
            else {
                return Vec::new();
            };
            if !out.status.success() {
                return Vec::new();
            }
            let Ok(text) = String::from_utf8(out.stdout) else {
                return Vec::new();
            };
            text.lines().map(str::trim).map(String::from).collect()
        }
        Some(PkgMgr::Rpm) => {
            let Ok(out) = std::process::Command::new("rpm")
                .args(["-ql", package])
                .output()
            else {
                return Vec::new();
            };
            if !out.status.success() {
                return Vec::new();
            }
            let Ok(text) = String::from_utf8(out.stdout) else {
                return Vec::new();
            };
            text.lines().map(str::trim).map(String::from).collect()
        }
        Some(PkgMgr::Pacman) => {
            let Ok(out) = std::process::Command::new("pacman")
                .args(["-Ql", package])
                .output()
            else {
                return Vec::new();
            };
            if !out.status.success() {
                return Vec::new();
            }
            let Ok(text) = String::from_utf8(out.stdout) else {
                return Vec::new();
            };
            // pacman -Ql output: "package /path/to/file"
            text.lines()
                .filter_map(|line| {
                    line.split_once(' ')
                        .map(|(_, path)| path.trim().to_string())
                })
                .collect()
        }
        None => Vec::new(),
    }
}
