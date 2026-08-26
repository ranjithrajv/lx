use anyhow::{anyhow, bail, Context, Result};
use clap::Args;
use std::path::{Path, PathBuf};

use crate::config::PackageConfig;
use lpt_lib::github::Asset;

#[derive(Debug, Clone, Args)]
pub struct ScanDepsArgs {
    /// Path to package.yaml, or a bare GitHub URL for a zero-config scan.
    #[arg(default_value = lpt_lib::constants::DEFAULT_CONFIG_FILENAME)]
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
}

/// Downloads a release binary and reports its `DT_NEEDED` shared-library
/// dependencies (`lpt_lib::elfdeps`), to help verify or fill in
/// `package.yaml`'s `depends:` field. Read-only: unlike `build`/`install`,
/// nothing scanned here is ever installed or built into a package, so it
/// deliberately skips checksum verification -- the downloaded bytes are
/// only ever inspected locally, never trusted onto the system.
pub fn run(args: ScanDepsArgs, token: Option<&str>) -> Result<()> {
    let mut cfg = match crate::plugins::source::parse_any_url(&args.config.to_string_lossy()) {
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
        None => match crate::build::parse_github_url(&args.config.to_string_lossy()) {
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
        },
    };
    if let Some(v) = &args.version {
        cfg.version = v.clone();
    }

    let source_name = cfg.effective_source();
    let source = crate::plugins::source::get_source_plugin(&source_name).ok_or_else(|| {
        anyhow!(
            "unsupported source '{}' (expected one of: {})",
            source_name,
            crate::plugins::source::source_available_names().join(", ")
        )
    })?;
    crate::plugins::source::apply_source_host(source.as_ref(), &cfg);
    let token_for_source = crate::plugins::source::resolve_source_token(source.as_ref(), token);

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
        let auto = crate::discovery::config_from_release(&cfg.github_repo, &release)?;
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
        .filter(|s| !lpt_lib::elfdeps::is_essential_libc_soname(s))
        .collect();
    if non_essential.is_empty() {
        println!("\nNo non-essential shared-library dependencies found.");
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

/// Download, extract, and scan one architecture's asset. Returns `Ok(true)`
/// if at least one ELF binary was found and reported, `Ok(false)` if the
/// architecture resolved cleanly but had nothing to scan (e.g. a
/// `binary_path` that doesn't exist in this asset), and `Err` on a real
/// failure (download, unsupported/corrupt archive format, unreadable ELF)
/// -- left to the caller to report and move on to the next architecture
/// rather than aborting the whole scan.
fn scan_one_arch(
    source: &dyn crate::plugins::source::SourcePlugin,
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
        let libs = lpt_lib::elfdeps::needed_libraries(&bytes)
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
            if lpt_lib::elfdeps::is_essential_libc_soname(lib) {
                println!("    {lib}  (glibc/essential, usually omit from depends:)");
            } else if let Some(pkg) = dpkg_owner(lib) {
                println!("    {lib}  -> {pkg} (via local dpkg -S)");
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

/// Best-effort: the Debian package that locally owns `soname`, via
/// `dpkg -S`. `None` when `dpkg` isn't on `PATH`, the library isn't
/// installed on this host, or the lookup otherwise fails -- this is
/// informational only, not authoritative for the target architecture/suite.
pub fn dpkg_owner(soname: &str) -> Option<String> {
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
        Some(pkg.to_string())
    }
}
