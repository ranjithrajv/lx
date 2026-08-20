use anyhow::{anyhow, Result};
use clap::Args;

use crate::config::{upstream_arch_names, PackageConfig};
use lpt_lib::github::{Asset, GitHubClient, Release};

#[derive(Debug, Clone, Args)]
pub struct DiscoverArgs {
    /// GitHub repository in "owner/repo" form.
    pub repo: String,

    /// Version/tag to inspect (defaults to latest release).
    pub version: Option<String>,

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
pub fn match_assets(release: &Release) -> Vec<ArchAsset> {
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
        let hit = linux_assets.iter().find(|asset| {
            if claimed.contains(&asset.name.as_str()) {
                return false;
            }
            asset_matches_arch(&asset.name, debian_arch, aliases)
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

/// Guess the artifact format from a filename.
pub fn guess_format(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        "tar.gz"
    } else if lower.ends_with(".zip") {
        "zip"
    } else {
        "raw"
    }
}

/// Run the discover subcommand: fetch release metadata, match assets per
/// architecture, and report the resulting config.
pub fn run(args: DiscoverArgs, token: Option<&str>) -> Result<()> {
    let client = GitHubClient::new(token.map(|s| s.to_string()))?;
    let (owner, repo) = split_repo(&args.repo)?;

    let release = match &args.version {
        Some(v) => client.release(owner, repo, v)?,
        None => client.latest_release(owner, repo)?,
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
        print_config(&args.repo, &release, &matched);
    } else {
        println!("Matched architectures:");
        for m in &matched {
            println!("  {:<8} {}", m.arch, m.asset);
        }
        println!("\nUse --full to print the generated package.yaml.");
    }
    Ok(())
}

fn print_config(repo: &str, release: &Release, matched: &[ArchAsset]) {
    println!(
        "# Auto-discovered by lpt discover from {}",
        release.html_url
    );
    println!(
        "package_name: {}",
        repo.split('/').next_back().unwrap_or(repo)
    );
    println!("github_repo: {repo}");
    println!(
        "artifact_format: {}",
        guess_format(&matched.first().unwrap().asset)
    );
    println!("architectures:");
    for m in matched {
        println!("  {}:", m.arch);
        println!("    release_pattern: \"{}\"", m.asset);
    }
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
    let matched = match_assets(release);
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

#[cfg(test)]
mod tests {
    use super::*;
    use lpt_lib::github::Asset;

    fn asset(name: &str) -> Asset {
        Asset {
            name: name.to_string(),
            size: None,
            browser_download_url: format!("https://github.com/x/y/releases/download/v1/{name}"),
        }
    }

    fn release(assets: Vec<Asset>) -> Release {
        Release {
            tag_name: "v1.0.0".into(),
            name: None,
            prerelease: false,
            draft: false,
            html_url: "https://github.com/x/y/releases".into(),
            assets,
        }
    }

    #[test]
    fn matches_x86_64_to_amd64() {
        let r = release(vec![
            asset("eza_x86_64-unknown-linux-gnu.tar.gz"),
            asset("eza_aarch64-unknown-linux-gnu.tar.gz"),
        ]);
        let m = match_assets(&r);
        assert!(m.iter().any(|a| a.arch == "amd64"));
        assert!(m.iter().any(|a| a.arch == "arm64"));
    }

    #[test]
    fn gnueabihf_is_armhf_not_armel() {
        let r = release(vec![asset("eza_arm-unknown-linux-gnueabihf.tar.gz")]);
        let m = match_assets(&r);
        assert!(m.iter().any(|a| a.arch == "armhf"));
        assert!(!m.iter().any(|a| a.arch == "armel"));
    }

    #[test]
    fn x86_64_does_not_match_i386() {
        let r = release(vec![asset("eza_x86_64-unknown-linux-gnu.tar.gz")]);
        let m = match_assets(&r);
        assert!(!m.iter().any(|a| a.arch == "i386"));
        assert!(m.iter().any(|a| a.arch == "amd64"));
    }

    #[test]
    fn windows_assets_are_ignored_when_linux_exists() {
        let r = release(vec![
            asset("eza_x86_64-pc-windows-gnu.tar.gz"),
            asset("eza_x86_64-unknown-linux-gnu.tar.gz"),
        ]);
        let m = match_assets(&r);
        assert!(m.iter().any(|a| a.arch == "amd64"));
        // must point at the linux asset
        let amd64 = m.iter().find(|a| a.arch == "amd64").unwrap();
        assert!(amd64.asset.contains("linux"));
    }

    #[test]
    fn x86_64_zip_does_not_become_i386() {
        let r = release(vec![
            asset("eza_x86_64-unknown-linux-gnu.tar.gz"),
            asset("eza_x86_64-unknown-linux-gnu.zip"),
        ]);
        let m = match_assets(&r);
        assert!(m.iter().any(|a| a.arch == "amd64"));
        assert!(!m.iter().any(|a| a.arch == "i386"));
    }

    #[test]
    fn real_i386_asset_still_matches() {
        let r = release(vec![asset("tool_i386-linux.tar.gz")]);
        let m = match_assets(&r);
        assert!(m.iter().any(|a| a.arch == "i386"));
    }

    #[test]
    fn no_match_is_empty() {
        let r = release(vec![asset("eza-windows.zip")]);
        assert!(match_assets(&r).is_empty());
    }

    #[test]
    fn guesses_formats() {
        assert_eq!(guess_format("foo.tar.gz"), "tar.gz");
        assert_eq!(guess_format("foo.tgz"), "tar.gz");
        assert_eq!(guess_format("foo.zip"), "zip");
        assert_eq!(guess_format("foo"), "raw");
    }
}
