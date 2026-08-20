use anyhow::{anyhow, Context, Result};
use clap::Args;
use std::path::PathBuf;

use crate::debs;
use crate::manifest::{Manifest, PackageEntry};
use lpt_lib::github::GitHubClient;

#[derive(Debug, Clone, Args)]
pub struct InstallArgs {
    /// Package name (e.g. "eza"), looked up as "<package>-debian" under
    /// the latest-debs GitHub org.
    pub package: String,

    /// Version/tag to install (defaults to the latest release).
    #[arg(short = 'v', long)]
    pub version: Option<String>,

    /// Target Debian architecture (defaults to `dpkg --print-architecture`).
    #[arg(long)]
    pub arch: Option<String>,

    /// Target Debian distribution/suite (defaults to the host's codename
    /// from /etc/os-release).
    #[arg(long)]
    pub distribution: Option<String>,

    /// Download the .deb into this directory instead of installing it.
    #[arg(long)]
    pub download_only: Option<PathBuf>,

    /// Skip checksum verification against the release's sidecar file (not
    /// recommended).
    #[arg(long)]
    pub no_verify: bool,

    /// Proceed when the release has no sidecar checksum to verify against,
    /// instead of failing the install. Most releases don't publish a
    /// checksum sidecar, so without this the default is to refuse to
    /// install an unverified .deb rather than silently warn and continue.
    #[arg(long)]
    pub allow_unverified: bool,

    /// Reinstall even if dpkg already reports this exact version installed.
    #[arg(long)]
    pub reinstall: bool,

    /// Skip the install confirmation prompt.
    #[arg(short = 'y', long)]
    pub yes: bool,
}

pub fn run(args: InstallArgs, token: Option<&str>) -> Result<()> {
    let client = GitHubClient::new(token.map(|s| s.to_string()))?;
    let repo = debs::repo_name(&args.package);

    let release = match &args.version {
        Some(v) => match client.release_by_tag(debs::LATEST_DEBS_ORG, &repo, v) {
            Ok(r) => r,
            Err(e) => {
                debs::suggest_versions(&client, &args.package, v);
                return Err(e);
            }
        },
        None => client
            .latest_release(debs::LATEST_DEBS_ORG, &repo)
            .with_context(|| {
                format!(
                    "no releases found for '{}/{repo}'. Is '{}' published under \
                     https://github.com/orgs/{}/repositories ?",
                    debs::LATEST_DEBS_ORG,
                    args.package,
                    debs::LATEST_DEBS_ORG
                )
            })?,
    };

    let arch = match &args.arch {
        Some(a) => a.clone(),
        None => debs::detect_dpkg_arch()?,
    };
    let dist = match &args.distribution {
        Some(d) => d.clone(),
        None => debs::detect_dist().ok_or_else(|| {
            anyhow!(
                "could not detect the host Debian distribution from /etc/os-release; \
                 pass --distribution explicitly"
            )
        })?,
    };

    let asset = debs::find_asset(&release, &args.package, &arch, &dist).ok_or_else(|| {
        anyhow!(
            "no .deb for {arch}/{dist} in release '{}'. Available:\n  {}",
            release.tag_name,
            release
                .assets
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join("\n  ")
        )
    })?;
    let control_version =
        debs::control_version(&asset.name, &args.package, &arch).ok_or_else(|| {
            anyhow!(
                "could not derive a Debian version from asset '{}'",
                asset.name
            )
        })?;

    if args.download_only.is_none() && !args.reinstall {
        if let Some(installed) = debs::dpkg_installed_version(&args.package) {
            if installed == control_version {
                println!(
                    "{} is already at {control_version}; nothing to do (use --reinstall to force)",
                    args.package
                );
                return Ok(());
            }
        }
    }

    println!(
        "Found {} ({}) for {arch}/{dist}",
        asset.name,
        debs::human_size(asset.size.unwrap_or(0))
    );

    let dest_dir = args
        .download_only
        .clone()
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&dest_dir)
        .with_context(|| format!("failed to create '{}'", dest_dir.display()))?;
    let dest = dest_dir.join(&asset.name);
    println!("  ↓ downloading {}", asset.name);
    debs::download(&client, asset, &dest)?;

    if !args.no_verify {
        debs::verify_sidecar_or_require_flag(&client, asset, &dest, args.allow_unverified)?;
    }

    if args.download_only.is_some() {
        println!("Downloaded to {}", dest.display());
        return Ok(());
    }

    debs::install_deb(&dest, args.yes)?;

    let mut manifest = Manifest::load()?;
    manifest.record(
        &args.package,
        PackageEntry {
            version: control_version,
            arch,
            distribution: dist,
            asset: asset.name.clone(),
            tag: release.tag_name.clone(),
            installed_at: debs::now_rfc3339(),
        },
    );
    manifest.save()?;

    Ok(())
}
