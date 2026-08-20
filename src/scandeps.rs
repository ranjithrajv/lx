use anyhow::{anyhow, bail, Context, Result};
use clap::Args;
use std::path::{Path, PathBuf};

use crate::config::PackageConfig;
use lpt_lib::github::GitHubClient;

#[derive(Debug, Clone, Args)]
pub struct ScanDepsArgs {
    /// Path to package.yaml, or a bare GitHub URL for a zero-config scan.
    #[arg(default_value = "package.yaml")]
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
    let mut cfg = match crate::build::parse_github_url(&args.config.to_string_lossy()) {
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
    };
    if let Some(v) = &args.version {
        cfg.version = v.clone();
    }

    let client = GitHubClient::with_cache(token.map(|s| s.to_string()), None)?;
    let (owner, repo) = crate::discovery::split_repo(&cfg.github_repo)?;

    let release = if !cfg.version.is_empty() {
        match client.release_by_tag(owner, repo, &cfg.version) {
            Ok(r) => r,
            Err(e) => {
                crate::build::suggest_versions(&client, owner, repo, &cfg.version);
                return Err(e);
            }
        }
    } else {
        client.latest_release(owner, repo)?
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
    let mut all_sonames: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    for arch in &target_archs {
        let Some(asset) = arch_assets.get(arch) else {
            println!("⚠️  no release asset for architecture '{arch}'; skipped");
            continue;
        };

        let mut cfg = cfg.clone();
        if cfg.artifact_format.is_empty() {
            cfg.artifact_format = crate::discovery::guess_format(&asset.name).to_string();
        }

        println!("\n{arch}: {}", asset.name);
        // Own subdirectory per architecture (not a filename prefix) so
        // `raw`-format assets (e.g. AppImages) -- whose extracted name
        // comes from the downloaded file's own on-disk name, see
        // `build::extract`'s "raw" branch -- keep their real asset name
        // instead of leaking an internal disambiguation prefix into it.
        let asset_dir = tmp.path().join(format!("{arch}-download"));
        std::fs::create_dir_all(&asset_dir)?;
        let asset_path = asset_dir.join(&asset.name);
        println!("  ↓ downloading (unverified -- inspected locally only, never installed)");
        crate::build::download(&client, asset, &asset_path)?;

        let extract_dir = tmp.path().join(format!("{arch}-scan-extract"));
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
            continue;
        }

        let elf_files = find_elf_files(&binary_dir)?;
        if elf_files.is_empty() {
            println!("  (no ELF binaries found)");
            continue;
        }

        for elf_path in elf_files {
            any_scanned = true;
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
    }

    if !any_scanned {
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

/// Recursively collect every ELF file under `dir` (handles both flat
/// release layouts and `bundle: true`'s nested `bin/`+`lib/` trees).
fn find_elf_files(dir: &Path) -> Result<Vec<PathBuf>> {
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
fn dpkg_owner(soname: &str) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_elf_files_walks_nested_dirs_and_skips_non_elf() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("bin")).unwrap();
        std::fs::write(dir.path().join("bin/tool"), b"\x7fELFrest-of-file").unwrap();
        std::fs::write(dir.path().join("README.md"), b"not an elf").unwrap();

        let found = find_elf_files(dir.path()).unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].ends_with("bin/tool"));
    }

    /// Regression test for a real bug found scanning a live `raw`-format
    /// asset (an AppImage): the download path used to disambiguate
    /// architectures via a filename prefix (`"{arch}-{asset_name}"`), but
    /// `build::extract`'s "raw" branch names the extracted file after the
    /// *downloaded file's own on-disk name* -- so the prefix leaked into
    /// what got displayed as the scanned binary's name. Downloads now go
    /// into a per-architecture subdirectory instead, keeping the asset's
    /// real name intact end to end.
    #[test]
    fn raw_format_extraction_preserves_the_real_asset_name() {
        let tmp = tempfile::tempdir().unwrap();
        let asset_dir = tmp.path().join("amd64-download");
        std::fs::create_dir_all(&asset_dir).unwrap();
        let asset_path = asset_dir.join("nvim-linux-x86_64.appimage");
        std::fs::write(&asset_path, b"\x7fELFfake-appimage-bytes").unwrap();

        let extract_dir = tmp.path().join("amd64-scan-extract");
        crate::build::extract(&asset_path, &extract_dir, "raw").unwrap();

        let found = find_elf_files(&extract_dir).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].file_name().unwrap().to_str().unwrap(),
            "nvim-linux-x86_64.appimage",
            "extracted raw asset must keep its real name, not an internal disambiguation prefix"
        );
    }

    #[test]
    fn dpkg_owner_is_none_when_dpkg_unavailable_or_no_match() {
        // This dev environment may or may not have `dpkg`; either way, a
        // nonsense soname must never resolve to a package.
        assert!(dpkg_owner("libtotally-made-up-soname.so.999").is_none());
    }
}
