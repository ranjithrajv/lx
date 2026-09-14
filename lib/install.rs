// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{anyhow, Context, Result};
use clap::Args;
use std::path::PathBuf;

use crate::consumer;
use crate::debs;
use crate::index::detect_host_format;
use crate::install_pkg;
use crate::manifest::{Manifest, PackageEntry};
use lx_lib::github::GitHubClient;

#[derive(Debug, Clone, Args)]
pub struct InstallArgs {
    /// Package name (e.g. "eza"), looked up as "<package>-debian" under the
    /// configured org (`LX_INDEX_ORG`, default `latest-debs`).
    pub package: String,

    /// Native package format to install: `deb`, `rpm`, or `arch`. Defaults
    /// to the host's own package manager.
    #[arg(long)]
    pub format: Option<String>,

    /// Version/tag to install (defaults to the latest release).
    #[arg(short = 'v', long)]
    pub version: Option<String>,

    /// Target architecture (defaults to the host's, per format).
    #[arg(long)]
    pub arch: Option<String>,

    /// Target distribution/suite (defaults to the host's, per format).
    #[arg(long)]
    pub distribution: Option<String>,

    /// Download the package into this directory instead of installing it.
    #[arg(long)]
    pub download_only: Option<PathBuf>,

    /// Skip checksum verification against the release's sidecar file (not
    /// recommended).
    #[arg(long)]
    pub no_verify: bool,

    /// Proceed when the release has no sidecar checksum to verify against,
    /// instead of failing the install. Most releases don't publish a
    /// checksum sidecar, so without this the default is to refuse to
    /// install an unverified package rather than silently warn and continue.
    #[arg(long)]
    pub allow_unverified: bool,

    /// Reinstall even if the host manager already reports this exact
    /// version installed.
    #[arg(long)]
    pub reinstall: bool,

    /// Skip the install confirmation prompt.
    #[arg(short = 'y', long)]
    pub yes: bool,
}

pub fn run(args: InstallArgs, token: Option<&str>) -> Result<()> {
    let format = match &args.format {
        Some(f) => consumer::parse_format(f)?,
        None => detect_host_format(),
    };
    let org = consumer::index_org();
    let repo = consumer::repo_name(&args.package);

    let client = GitHubClient::new(token.map(|s| s.to_string()))?;

    let release = match &args.version {
        Some(v) => match client.release_by_tag(&org, &repo, v) {
            Ok(r) => r,
            Err(e) => {
                debs::suggest_versions(&client, &org, &repo, v);
                return Err(e);
            }
        },
        None => client.latest_release(&org, &repo).with_context(|| {
            format!(
                "no releases found for '{org}/{repo}'. Is '{}' published under \
                 https://github.com/orgs/{org}/repositories ?",
                args.package
            )
        })?,
    };

    let arch = match &args.arch {
        Some(a) => a.clone(),
        None => install_pkg::detect_arch(format)?,
    };
    let dist = match &args.distribution {
        Some(d) => d.clone(),
        None => consumer::host_dist(format).unwrap_or_default(),
    };

    let resolved = consumer::resolve_asset(&release, &args.package, format, &arch, &dist)
        .ok_or_else(|| {
            anyhow!(
                "no {} asset for {arch}/{dist} in release '{}'. Available:\n  {}",
                format.name(),
                release.tag_name,
                release
                    .assets
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join("\n  ")
            )
        })?;
    if resolved.musl_fallback {
        println!("  (no {dist}-specific build; using musl-static binary — runs on any Linux)");
    }
    let asset = resolved.asset;
    let control_version = resolved.version;

    if args.download_only.is_none() && !args.reinstall {
        if let Some(installed) = consumer::installed_version(&args.package, format) {
            if installed == control_version {
                println!(
                    "{} is already at {control_version}; nothing to do (use --reinstall to force)",
                    args.package
                );
                return Ok(());
            }
        }
    }

    let target = if dist.is_empty() {
        format.name().to_string()
    } else {
        dist.clone()
    };
    println!(
        "Found {} ({}) for {arch}/{target}",
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

    consumer::install(&dest, &asset.name, format, args.yes)?;

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
            format: format.name().to_string(),
        },
    );
    manifest.save()?;

    Ok(())
}
