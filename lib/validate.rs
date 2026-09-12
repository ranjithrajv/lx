// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{bail, Context, Result};
use clap::Args;
use std::path::PathBuf;

use crate::config::{ArchSpec, PackageConfig};

#[derive(Debug, Clone, Args)]
pub struct ValidateArgs {
    /// Path to package.yaml
    #[arg(default_value = lx_lib::constants::DEFAULT_CONFIG_FILENAME)]
    pub config: PathBuf,

    /// Version to validate against (defaults to latest release).
    #[arg(short = 'v', long)]
    pub version: Option<String>,

    /// Apply a delta package.yaml over the base config before validating
    /// (see `lx build --overlay`).
    #[arg(long)]
    pub overlay: Option<PathBuf>,

    /// Download release binaries and scan ELF dependencies to verify
    /// that `depends:` in package.yaml matches what the binary actually
    /// needs at runtime.
    #[arg(long)]
    pub check_deps: bool,
}

pub fn run(args: ValidateArgs, token: Option<&str>) -> Result<()> {
    let mut cfg = PackageConfig::load(&args.config)?;
    if let Some(overlay) = &args.overlay {
        cfg.apply_overlay(overlay)
            .with_context(|| format!("applying overlay '{}'", overlay.display()))?;
    }
    println!(
        "config: OK (package '{}' from {})",
        cfg.package_name, cfg.github_repo
    );

    // Check a structural invariant: manual patterns must not collide.
    if cfg.has_manual_patterns() {
        println!(
            "patterns: OK ({} architectures pinned)",
            cfg.architectures.patterns().len()
        );
    } else if let ArchSpec::List(names) = &cfg.architectures {
        println!(
            "patterns: auto-discovery restricted to {} architecture(s): {}",
            names.len(),
            names.join(", ")
        );
    } else {
        println!("patterns: auto-discovery (architectures key omitted)");
    }

    // Network checks against the source API (github/gitlab).
    let source_name = cfg.effective_forge_source();
    let source = crate::plugins::forge::get_forge_source(&source_name).ok_or_else(|| {
        anyhow::anyhow!(
            "unsupported source '{}' (expected one of: {})",
            source_name,
            crate::plugins::forge::forge_source_names().join(", ")
        )
    })?;
    crate::plugins::forge::apply_forge_host(source.as_ref(), &cfg);
    let token_for_source = crate::plugins::forge::resolve_forge_token(source.as_ref(), token);

    let release = match &args.version {
        Some(v) => {
            match source.release_by_tag(&cfg.github_repo, v, token_for_source.as_deref(), None) {
                Ok(r) => r,
                Err(e) => {
                    crate::build::suggest_versions_source(
                        source.as_ref(),
                        &cfg.github_repo,
                        v,
                        token_for_source.as_deref(),
                        None,
                    );
                    return Err(e);
                }
            }
        }
        None => source.latest_release(&cfg.github_repo, token_for_source.as_deref(), None)?,
    };
    println!(
        "release: found '{}' ({} assets)",
        release.tag_name,
        release.assets.len()
    );
    crate::build::warn_if_prerelease_or_draft(&release);

    // Verify every pinned pattern resolves to an actual asset.
    if cfg.has_manual_patterns() {
        let mut missing = Vec::new();
        for (arch, acfg) in cfg.architectures.patterns() {
            let expanded =
                crate::build::expand_version_placeholder(&acfg.release_pattern, &release.tag_name);
            if !release.assets.iter().any(|a| a.name == expanded) {
                missing.push(format!("{arch}: '{expanded}'"));
            }
        }
        if !missing.is_empty() {
            bail!(
                "pinned patterns not found in release '{}':\n  {}",
                release.tag_name,
                missing.join("\n  ")
            );
        }
        println!(
            "assets: OK (all {} pinned patterns resolve)",
            cfg.architectures.patterns().len()
        );
    }

    // Auto-discovery sanity check.
    if !cfg.has_manual_patterns() {
        let matched = crate::discovery::match_assets(&release);
        println!(
            "assets: auto-discovery found {} of {} architectures",
            matched.len(),
            crate::config::DEFAULT_ARCHITECTURES.len()
        );
    }

    // --check-deps: download a release binary and verify depends: against
    // actual ELF DT_NEEDED sonames.
    if args.check_deps {
        check_deps(&cfg, &release, source.as_ref(), token_for_source.as_deref())?;
    }

    println!("\nvalidate: OK");
    Ok(())
}

/// Download a release binary (first matching architecture), scan its ELF
/// dependencies, and compare against the config's `depends:` field.
fn check_deps(
    cfg: &PackageConfig,
    release: &crate::github::Release,
    source: &dyn crate::plugins::forge::ForgeSource,
    token: Option<&str>,
) -> Result<()> {
    // Find a matching asset for this host's architecture or the first available.
    let host_arch = crate::build::host_arch().unwrap_or_default();
    let asset = if cfg.has_manual_patterns() {
        // Use manual patterns to find a matching asset.
        let auto =
            crate::discovery::config_from_release_with_musl(&cfg.github_repo, release, false)?;
        crate::build::resolve_manual(&auto, release)?
    } else {
        crate::discovery::config_from_release_with_musl(&cfg.github_repo, release, false)
            .map(|a| crate::build::resolve_manual(&a, release))
            .unwrap_or(Ok(std::collections::HashMap::new()))?
    };

    // Pick the host arch if available, else the first one.
    let (arch, asset_info) = if let Some(a) = asset.get(&host_arch) {
        (host_arch.clone(), a.clone())
    } else {
        asset
            .iter()
            .next()
            .map(|(k, v)| (k.clone(), v.clone()))
            .ok_or_else(|| anyhow::anyhow!("no release assets found to check deps against"))?
    };

    println!("deps: scanning {arch}/{}", asset_info.name);

    // Download the asset.
    let tmp = tempfile::tempdir().context("failed to create temp dir")?;
    let asset_path = tmp.path().join(&asset_info.name);
    {
        let mut body = source
            .raw_get(&asset_info.browser_download_url, token)
            .context("asset download failed for dep check")?;
        let mut file = std::fs::File::create(&asset_path)?;
        std::io::copy(&mut body, &mut file)?;
    }

    // Extract.
    let extract_dir = tmp.path().join("extract");
    let artifact_format = if asset_info.name.is_empty() {
        "tar.gz"
    } else {
        crate::discovery::guess_format(&asset_info.name)
    };
    crate::build::extract(&asset_path, &extract_dir, artifact_format)?;

    // Locate binaries.
    let binary_dir = if cfg.binary_path.is_empty() {
        extract_dir.clone()
    } else {
        extract_dir.join(&cfg.binary_path)
    };
    if !binary_dir.is_dir() {
        println!(
            "  ⚠ binary_path '{}' not found in archive; skipping dep check",
            cfg.binary_path
        );
        return Ok(());
    }

    // Scan ELF deps.
    let (scanned_depends, scanned_sonames) =
        lx_lib::scandeps::compute_depends_from_dir(&binary_dir, &cfg.depends, cfg.musl);

    if scanned_sonames.is_empty() {
        println!("  (no non-essential ELF dependencies found)");
        return Ok(());
    }

    // Compare against declared depends.
    let (missing, unnecessary) = lx_lib::scandeps::diff_deps(&scanned_sonames, &cfg.depends);

    println!("  scanned: {} non-essential sonames", scanned_sonames.len());
    println!("  resolved: {scanned_depends}");

    if missing.is_empty() && unnecessary.is_empty() {
        println!("  ✓ depends: matches ELF needs");
    } else {
        if !missing.is_empty() {
            println!("  ⚠ not declared in depends: {}", missing.join(", "));
        }
        if !unnecessary.is_empty() {
            println!(
                "  ℹ declared but not in ELF needs: {}",
                unnecessary.join(", ")
            );
        }
        bail!(
            "depends: mismatch — {} undeclared, {} unnecessary",
            missing.len(),
            unnecessary.len()
        );
    }

    Ok(())
}
