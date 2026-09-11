use anyhow::{anyhow, Result};
use clap::Args;

use crate::debs;
use crate::manifest::{Manifest, PackageEntry};
use lx_lib::github::GitHubClient;

#[derive(Debug, Clone, Args)]
pub struct RollbackArgs {
    /// Package to roll back.
    pub package: String,

    /// How many generations back to roll to (1 = the generation
    /// immediately before the current one).
    #[arg(long, default_value_t = 1)]
    pub steps: usize,

    /// Skip checksum verification against the release's sidecar file (not
    /// recommended).
    #[arg(long)]
    pub no_verify: bool,

    /// Proceed when the release has no sidecar checksum to verify against.
    #[arg(long)]
    pub allow_unverified: bool,

    /// Skip the install confirmation prompt.
    #[arg(short = 'y', long)]
    pub yes: bool,
}

/// Reinstalls a prior generation of an `lx`-managed package from its
/// recorded (tag, asset) in `installed.json`'s history -- Nix profile
/// rollback, adapted for dpkg: `dpkg -i` the old .deb rather than
/// re-pointing a store symlink. Re-downloads from the release rather than
/// caching local .deb files, so it works even if the temp file from the
/// original install/upgrade was already cleaned up.
pub fn run(args: RollbackArgs, token: Option<&str>) -> Result<()> {
    let manifest = Manifest::load()?;
    let target = manifest
        .previous(&args.package, args.steps)
        .cloned()
        .ok_or_else(|| {
            anyhow!(
                "'{}' has no generation {} step(s) before current -- nothing to roll back to \
                 (run `lx list` to see what's tracked)",
                args.package,
                args.steps
            )
        })?;

    let client = GitHubClient::new(token.map(|s| s.to_string()))?;
    let repo = debs::repo_name(&args.package);
    let release = client.release_by_tag(debs::LATEST_DEBS_ORG, &repo, &target.tag)?;
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == target.asset)
        .ok_or_else(|| {
            anyhow!(
                "asset '{}' is no longer present in release '{}'",
                target.asset,
                target.tag
            )
        })?;

    println!(
        "Rolling back {} -> {} ({}/{})",
        args.package, target.version, target.arch, target.distribution
    );

    let dest = std::env::temp_dir().join(&asset.name);
    println!("  ↓ downloading {}", asset.name);
    debs::download(&client, asset, &dest)?;
    if !args.no_verify {
        debs::verify_sidecar_or_require_flag(&client, asset, &dest, args.allow_unverified)?;
    }
    debs::install_deb(&dest, args.yes)?;

    let mut manifest = Manifest::load()?;
    manifest.record(
        &args.package,
        PackageEntry {
            version: target.version.clone(),
            arch: target.arch.clone(),
            distribution: target.distribution.clone(),
            asset: asset.name.clone(),
            tag: target.tag.clone(),
            installed_at: debs::now_rfc3339(),
        },
    );
    manifest.save()?;

    println!("✓ rolled back {} to {}", args.package, target.version);
    Ok(())
}
