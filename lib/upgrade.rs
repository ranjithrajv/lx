// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{bail, Result};
use clap::Args;

use crate::consumer;
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

    /// When `--all` is set, auto-migrate distro packages flagged as outdated:
    /// install the lx-built version and remove the distro version. Without
    /// this flag, `--all` only prints migration targets (plan-only).
    #[arg(long, requires = "all")]
    pub auto_migrate: bool,

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
            if !manifest.contains(p) {
                bail!("'{p}' is not managed by lx (run `lx install {p}` first)");
            }
            vec![p.clone()]
        }
        None => manifest.names().map(str::to_string).collect(),
    };

    if targets.is_empty() && !args.all {
        println!("no lx-managed packages installed");
        return Ok(());
    }

    let targets: Vec<String> = if args.owned_only {
        let kept: Vec<String> = targets
            .into_iter()
            .filter(|p| {
                let format = manifest
                    .current(p)
                    .map(|e| consumer::format_or_host(&e.format))
                    .unwrap_or_else(crate::index::detect_host_format);
                let keep = consumer::installed_version(p, format).is_some();
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
/// With `args.auto_migrate`, they are migrated automatically.
fn check_distro_outdated(args: &UpgradeArgs, token: Option<&str>) -> Result<()> {
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
        let host = pkg.host_version.as_deref().unwrap_or("?");
        let newest = pkg.newest.as_deref().unwrap_or("?");
        println!(
            "    {:<20} host: {:<12} → newest: {}",
            pkg.name, host, newest
        );
    }

    if !args.auto_migrate {
        println!("\n  to take over management of a package, run `lx install <name>`");
        println!("  (or re-run with --auto-migrate to apply automatically)");
        return Ok(());
    }

    // Auto-migrate: install the lx-built version of each outdated package.
    println!("\n--auto-migrate: installing lx-built versions...\n");
    let mut migrated = 0;
    let mut failed = 0;

    for pkg in &outdated {
        if migrate_distro_package(&pkg.name, token)? {
            migrated += 1;
        } else {
            failed += 1;
        }
    }

    println!("\n  {} migrated, {} failed", migrated, failed);
    if failed > 0 {
        bail!("{} distro package(s) failed to migrate", failed);
    }
    Ok(())
}

/// Migrate a single distro-managed package to an lx-built one.
/// Returns true on success, false on failure.
fn migrate_distro_package(package: &str, token: Option<&str>) -> Result<bool> {
    println!("  ↓ migrating {package} to lx-built version");

    // Install via the lx install backend (downloads from latest-debs org).
    match crate::install::run(
        crate::install::InstallArgs {
            package: package.to_string(),
            format: None,
            version: None,
            arch: None,
            distribution: None,
            download_only: None,
            no_verify: false,
            allow_unverified: true,
            reinstall: true,
            yes: true,
            source: None,
        },
        token,
    ) {
        Ok(()) => {
            println!("    ✓ {package} migrated (now lx-managed)");
            Ok(true)
        }
        Err(e) => {
            eprintln!("    ✗ {package} migration failed: {e:#}");
            Ok(false)
        }
    }
}

/// Returns `Ok(true)` if the package was upgraded, `Ok(false)` if it was
/// already current (or `--dry-run` reported a pending upgrade).
fn upgrade_one(
    client: &GitHubClient,
    package: &str,
    entry: &PackageEntry,
    args: &UpgradeArgs,
) -> Result<bool> {
    let format = consumer::format_or_host(&entry.format);
    let org = consumer::index_org();
    let repo = consumer::repo_name(package);
    // The org is the primary source; a package it doesn't carry (e.g. one
    // installed via `lx index install --source aur`) falls back to the
    // enabled indexes instead of failing.
    let Ok(release) = client.latest_release(&org, &repo) else {
        return upgrade_one_from_index(package, entry, args);
    };
    let Some(resolved) =
        consumer::resolve_asset(&release, package, format, &entry.arch, &entry.distribution)
    else {
        return upgrade_one_from_index(package, entry, args);
    };
    let asset = resolved.asset;
    let candidate_version = resolved.version;

    let installed =
        consumer::installed_version(package, format).unwrap_or_else(|| entry.version.clone());
    if !consumer::is_newer(&installed, &candidate_version, format)? {
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
    consumer::install(&dest, &asset.name, format, args.yes)?;

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
            format: format.name().to_string(),
        },
    );
    manifest.save()?;

    Ok(true)
}

/// Upgrade a package the `latest-debs` org doesn't carry by re-resolving it
/// through the enabled indexes (`lx index`). Index sources advertise versions
/// as release tags; a source that only builds from a recipe has no version to
/// compare, so it is reported and skipped.
fn upgrade_one_from_index(package: &str, entry: &PackageEntry, args: &UpgradeArgs) -> Result<bool> {
    let format = consumer::format_or_host(&entry.format);
    let Some((name, candidate)) = crate::index::newest_active_candidate(package)? else {
        bail!(
            "'{package}' is not published under the {} org or any enabled index",
            consumer::index_org()
        );
    };
    let Some(candidate) = candidate else {
        println!("  ? {package}: '{name}' names no version (builds from source)");
        return Ok(false);
    };

    let installed =
        consumer::installed_version(package, format).unwrap_or_else(|| entry.version.clone());
    if !consumer::is_newer(&installed, &candidate, format)? {
        println!("  = {package} up to date ({installed})");
        return Ok(false);
    }
    println!("  ↑ {package}: {installed} -> {candidate} (from {name})");
    if args.dry_run {
        return Ok(false);
    }

    let opts = crate::index::InstallOpts {
        no_verify: args.no_verify,
        allow_unverified: args.allow_unverified,
        yes: args.yes,
        ..Default::default()
    };
    crate::index::install_from_active(package, Some(&name), opts)?;
    Ok(true)
}
