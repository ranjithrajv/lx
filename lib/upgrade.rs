// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{anyhow, bail, Result};
use clap::Args;

use crate::debs;
use crate::manifest::{Manifest, PackageEntry};
use lx_lib::github::GitHubClient;

#[derive(Debug, Clone, Args)]
pub struct UpgradeArgs {
    /// Package to upgrade (omit to upgrade every lx-managed package).
    pub package: Option<String>,

    /// System-wide freshness check: also flag distro-managed packages that
    /// are behind upstream (per repology metadata) as migration targets.
    /// Each flagged package can be taken over by `lx install`.
    #[arg(long)]
    pub all: bool,

    /// Skip checksum verification against the release's sidecar file (not
    /// recommended).
    #[arg(long)]
    pub no_verify: bool,

    /// Proceed when the release has no sidecar checksum to verify against,
    /// instead of failing the upgrade. Most releases don't publish a
    /// checksum sidecar, so without this the default is to refuse to
    /// install an unverified .deb rather than silently warn and continue.
    #[arg(long)]
    pub allow_unverified: bool,

    /// Print what would be upgraded without downloading or installing.
    #[arg(long)]
    pub dry_run: bool,

    /// Skip the install confirmation prompt.
    #[arg(short = 'y', long)]
    pub yes: bool,

    /// Only upgrade packages currently installed per dpkg (deb-get
    /// `--dg-only` spirit: skip manifest entries removed outside lx).
    #[arg(long)]
    pub owned_only: bool,
}

pub fn run(args: UpgradeArgs, token: Option<&str>) -> Result<()> {
    let mut manifest = Manifest::load()?;

    let targets: Vec<String> = match &args.package {
        Some(p) => {
            if !manifest.packages.contains_key(p) {
                bail!("'{p}' is not managed by lx (run `lx install {p}` first)");
            }
            vec![p.clone()]
        }
        None => manifest.packages.keys().cloned().collect(),
    };

    if targets.is_empty() && !args.all {
        println!("no lx-managed packages installed");
        return Ok(());
    }

    let targets: Vec<String> = if args.owned_only {
        let kept: Vec<String> = targets
            .into_iter()
            .filter(|p| {
                let keep = debs::dpkg_installed_version(p).is_some();
                if !keep {
                    println!("skipping '{p}': not currently installed (--owned-only)");
                }
                keep
            })
            .collect();
        if kept.is_empty() && !args.all {
            println!("nothing currently installed to upgrade");
            return Ok(());
        }
        kept
    } else {
        targets
    };

    let client = GitHubClient::new(token.map(|s| s.to_string()))?;
    let mut upgraded = 0;
    let mut failed = 0;
    let mut up_to_date = 0;

    if !targets.is_empty() {
        println!("lx-managed packages:");
        for package in &targets {
            let entry = manifest.current(package).unwrap().clone();
            match upgrade_one(&client, package, &entry, &args) {
                Ok(true) => upgraded += 1,
                Ok(false) => up_to_date += 1,
                Err(e) => {
                    eprintln!("  ✗ {package}: {e:#}");
                    failed += 1;
                    continue;
                }
            }
        }
    } else {
        println!("no lx-managed packages to upgrade");
    }

    if !args.dry_run && upgraded > 0 {
        // Reload in case entries were updated by upgrade_one via a fresh
        // Manifest::load()/save() round-trip per package below.
        manifest = Manifest::load()?;
        manifest.save()?;
    }

    println!(
        "  {} upgraded, {} failed, {} up to date",
        upgraded, failed, up_to_date
    );

    // System-wide check: flag distro-managed packages that are behind upstream.
    if args.all {
        check_distro_outdated(&args, token)?;
    }

    if failed > 0 {
        bail!("{failed} package(s) failed to upgrade");
    }
    Ok(())
}

/// Check for distro-managed packages that are behind upstream (per repology).
/// These are flagged as migration targets — packages `lx install` could take over.
fn check_distro_outdated(_args: &UpgradeArgs, _token: Option<&str>) -> Result<()> {
    println!("\nsystem-wide freshness check (distro packages behind upstream):");

    let repology = crate::index::repology::RepologySource::new("repology");
    let outdated = repology.outdated_packages();

    if outdated.is_empty() {
        println!("  all tracked distro packages are up to date (or no repology data)");
        return Ok(());
    }

    println!(
        "  {} package(s) where distro lags upstream:\n",
        outdated.len()
    );
    for pkg in &outdated {
        let host = pkg
            .host_version
            .as_deref()
            .unwrap_or("?");
        let newest = pkg.newest.as_deref().unwrap_or("?");
        println!(
            "    {:<20} host: {:<12} → newest: {}",
            pkg.name, host, newest
        );
    }

    println!("\n  to take over management of a package, run `lx install <name>`");
    println!("  (this replaces the distro version with an lx-built one, tracked for upgrades)");

    Ok(())
}

/// Returns `Ok(true)` if the package was upgraded, `Ok(false)` if it was
/// already current (or `--dry-run` reported a pending upgrade).
fn upgrade_one(
    client: &GitHubClient,
    package: &str,
    entry: &PackageEntry,
    args: &UpgradeArgs,
) -> Result<bool> {
    let release = client.latest_release(debs::LATEST_DEBS_ORG, &debs::repo_name(package))?;
    let asset =
        debs::find_asset(&release, package, &entry.arch, &entry.distribution).ok_or_else(|| {
            anyhow!(
                "no .deb for {}/{} in release '{}'",
                entry.arch,
                entry.distribution,
                release.tag_name
            )
        })?;
    let candidate_version =
        debs::control_version(&asset.name, package, &entry.arch).ok_or_else(|| {
            anyhow!(
                "could not derive a Debian version from asset '{}'",
                asset.name
            )
        })?;

    let installed = debs::dpkg_installed_version(package).unwrap_or(entry.version.clone());
    if !debs::is_newer(&installed, &candidate_version)? {
        println!("  = {package} up to date ({installed})");
        return Ok(false);
    }

    println!("  ↑ {package}: {installed} -> {candidate_version}");
    if args.dry_run {
        return Ok(false);
    }

    let dest = std::env::temp_dir().join(&asset.name);
    println!("    ↓ downloading {}", asset.name);
    debs::download(client, asset, &dest)?;
    if !args.no_verify {
        debs::verify_sidecar_or_require_flag(client, asset, &dest, args.allow_unverified)?;
    }
    debs::install_deb(&dest, args.yes)?;

    let mut manifest = Manifest::load()?;
    manifest.record(
        package,
        PackageEntry {
            version: candidate_version,
            arch: entry.arch.clone(),
            distribution: entry.distribution.clone(),
            asset: asset.name.clone(),
            tag: release.tag_name.clone(),
            installed_at: debs::now_rfc3339(),
        },
    );
    manifest.save()?;

    Ok(true)
}
