// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{anyhow, Result};
use clap::Args;

use crate::config::{upstream_arch_names, PackageConfig};
use lx_lib::github::{Asset, Release};

#[derive(Debug, Clone, Args)]
pub struct DiscoverArgs {
    /// Repository in "owner/repo" form (for the selected source).
    pub repo: String,

    /// Version/tag to inspect (defaults to latest release).
    pub version: Option<String>,

    /// Source provider: github (default) or gitlab.
    #[arg(long)]
    pub source: Option<String>,

    /// Print the full auto-discovered package.yaml instead of a summary.
    #[arg(long)]
    pub full: bool,
}

/// A discovered asset for a given Debian architecture.
#[derive(Debug, Clone)]
pub struct ArchAsset {
    pub arch: String,
    pub asset: String,
}

/// Match release assets against every supported Debian architecture using
/// segment-run matching: filenames and aliases are split on `-`/`_`/`.`, and
/// an alias matches when its segment sequence appears as a contiguous run of
/// the filename's segments. Aliases are tried longest-first and each asset is
/// claimed at most once, so `x86_64` claims an amd64 asset before i386's
/// `x86` alias can, and `arm-unknown-linux-gnueabihf` is claimed by armhf.
///
/// OS filtering: assets that are clearly for Windows/macOS are only used as
/// a last resort, so a `.exe`/`.dmg` never shadows a Linux build.
///
/// When `musl` is true, assets containing `musl` in their name are preferred
/// over glibc variants (e.g. `*-linux-musl.tar.gz` wins over
/// `*-linux-gnu.tar.gz`), producing a binary with no glibc dependency that
/// runs on any Linux regardless of distro age.
pub fn match_assets(release: &Release) -> Vec<ArchAsset> {
    match_assets_with_musl(release, false)
}

/// Like [`match_assets`] but with explicit musl preference control.
pub fn match_assets_with_musl(release: &Release, musl: bool) -> Vec<ArchAsset> {
    let linux_assets: Vec<&Asset> = release
        .assets
        .iter()
        .filter(|a| {
            let lower = a.name.to_ascii_lowercase();
            !lower.contains("windows")
                && !lower.contains(".exe")
                && !lower.contains("darwin")
                && !lower.contains(".dmg")
                && !lower.contains(".msi")
                && !lower.contains("macos")
                && !lower.contains("apple")
        })
        .collect();

    let names = upstream_arch_names();
    // Longest (most specific) alias first so e.g. "x86_64" claims an amd64
    // asset before i386's generic "x86" alias can.
    let mut order: Vec<(&str, &[&str])> = names.iter().map(|(k, v)| (*k, *v)).collect();
    order.sort_by_key(|(_, aliases)| {
        std::cmp::Reverse(aliases.iter().map(|a| a.len()).max().unwrap_or(0))
    });

    let mut matched = Vec::new();
    let mut claimed: Vec<&str> = Vec::new();
    for (debian_arch, aliases) in order {
        // When musl is requested, prefer a musl-named asset for this arch if
        // one exists; fall back to any matching asset otherwise.
        let hit = linux_assets
            .iter()
            .find(|asset| {
                if claimed.contains(&asset.name.as_str()) {
                    return false;
                }
                if !asset_matches_arch(&asset.name, debian_arch, aliases) {
                    return false;
                }
                // First pass: prefer musl assets when requested.
                !musl || asset.name.to_ascii_lowercase().contains("musl")
            })
            .or_else(|| {
                if musl {
                    // No musl asset for this arch — fall back to any match so
                    // we still produce a package (it just won't be musl).
                    linux_assets.iter().find(|asset| {
                        if claimed.contains(&asset.name.as_str()) {
                            return false;
                        }
                        asset_matches_arch(&asset.name, debian_arch, aliases)
                    })
                } else {
                    None
                }
            });
        if let Some(asset) = hit {
            matched.push(ArchAsset {
                arch: debian_arch.to_string(),
                asset: asset.name.clone(),
            });
            claimed.push(asset.name.as_str());
        }
    }
    matched
}

/// Split a string into segments on `-`, `_`, and `.`.
fn segments(s: &str) -> Vec<&str> {
    s.split(['-', '_', '.']).filter(|t| !t.is_empty()).collect()
}

/// True if `alias`'s segments appear as a contiguous run inside the
/// filename's segments. armel vs armhf is disambiguated by checking for
/// hard-float markers first.
fn asset_matches_arch(filename: &str, debian_arch: &str, aliases: &[&str]) -> bool {
    let lower = filename.to_ascii_lowercase();
    let fseg = segments(&lower);

    // Hard-float arm binaries belong to armhf, not armel.
    if debian_arch == "armel"
        && fseg
            .iter()
            .any(|t| matches!(*t, "gnueabihf" | "armv7" | "armhf" | "eabihf"))
    {
        return false;
    }

    // i386's "x86" alias must not claim 64-bit assets.
    if debian_arch == "i386"
        && (fseg
            .windows(2)
            .any(|w| matches!(w, ["x86", "64"] | ["x", "64"]))
            || fseg.contains(&"amd64")
            || fseg.contains(&"x64"))
    {
        return false;
    }

    aliases.iter().any(|alias| {
        let aseg = segments(alias);
        !aseg.is_empty() && fseg.windows(aseg.len()).any(|w| w == aseg.as_slice())
    })
}

/// Guess the artifact format from a filename, via the artifact-format
/// plugin registry (so new formats are recognized without editing here).
pub fn guess_format(name: &str) -> &'static str {
    crate::plugins::artifact::detect_artifact_format(name)
}

/// Run the discover subcommand: fetch release metadata, match assets per
/// architecture, and report the resulting config.
pub fn run(args: DiscoverArgs, token: Option<&str>) -> Result<()> {
    let source_name = args
        .source
        .as_deref()
        .unwrap_or("github")
        .to_ascii_lowercase();
    let source = crate::plugins::forge::get_forge_source(&source_name).ok_or_else(|| {
        anyhow!(
            "unsupported source '{}' (expected one of: {})",
            source_name,
            crate::plugins::forge::forge_source_names().join(", ")
        )
    })?;

    let release = match &args.version {
        Some(v) => source.release_by_tag(&args.repo, v, token, None)?,
        None => source.latest_release(&args.repo, token, None)?,
    };

    println!("Release: {} ({})", release.tag_name, release.html_url);
    println!("Assets:  {} available", release.assets.len());

    let matched = match_assets(&release);
    if matched.is_empty() {
        return Err(anyhow!(
            "no assets matched any supported architecture. Available:\n  {}",
            release
                .assets
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join("\n  ")
        ));
    }

    if args.full {
        print_config(&args.repo, &source_name, &release, &matched);
    } else {
        println!("Matched architectures:");
        for m in &matched {
            println!("  {:<8} {}", m.arch, m.asset);
        }
        println!("\nUse --full to print the generated package.yaml.");
    }
    Ok(())
}

fn print_config(repo: &str, source: &str, release: &Release, matched: &[ArchAsset]) {
    print!("{}", render_config(repo, source, release, matched));
}

/// The starter `package.yaml` body for a discovered release, with a comment
/// header. Shared by `lx discover` (prints it) and `lx init --from` (writes
/// it to a file).
pub fn render_config(repo: &str, source: &str, release: &Release, matched: &[ArchAsset]) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# Auto-discovered by lx from {}\n",
        release.html_url
    ));
    out.push_str(&format!(
        "package_name: {}\n",
        repo.split('/').next_back().unwrap_or(repo)
    ));
    if source != "github" {
        out.push_str(&format!("source: {source}\n"));
    }
    out.push_str(&format!("github_repo: {repo}\n"));
    out.push_str(&format!(
        "artifact_format: {}\n",
        guess_format(matched.first().map(|m| m.asset.as_str()).unwrap_or(""))
    ));
    out.push_str("architectures:\n");
    for m in matched {
        out.push_str(&format!("  {}:\n", m.arch));
        out.push_str(&format!("    release_pattern: \"{}\"\n", m.asset));
    }
    out
}

pub fn split_repo(repo: &str) -> Result<(&str, &str)> {
    let (owner, rest) = repo
        .split_once('/')
        .ok_or_else(|| anyhow!("repo must be 'owner/repo', got '{repo}'"))?;
    if rest.is_empty() || rest.contains('/') {
        return Err(anyhow!("repo must be 'owner/repo', got '{repo}'"));
    }
    Ok((owner, rest))
}

/// Build a PackageConfig from a release and its matched assets. Used by
/// zero-config auto-discovery (like the action's --ad flag).
pub fn config_from_release(repo: &str, release: &Release) -> Result<PackageConfig> {
    config_from_release_with_musl(repo, release, false)
}

/// Like [`config_from_release`] but with explicit musl preference control.
pub fn config_from_release_with_musl(
    repo: &str,
    release: &Release,
    musl: bool,
) -> Result<PackageConfig> {
    let matched = match_assets_with_musl(release, musl);
    if matched.is_empty() {
        return Err(anyhow!("no assets matched any supported architecture"));
    }
    let mut cfg = PackageConfig {
        package_name: repo.split('/').next_back().unwrap_or(repo).to_string(),
        github_repo: repo.to_string(),
        artifact_format: guess_format(&matched[0].asset).to_string(),
        ..PackageConfig::default()
    };
    for m in matched {
        cfg.architectures.set_pattern(
            m.arch.clone(),
            crate::config::ArchConfig {
                release_pattern: m.asset,
            },
        );
    }
    Ok(cfg)
}
