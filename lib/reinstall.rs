// SPDX-License-Identifier: GPL-3.0-or-later

//! `lx reinstall` — reinstall the current version of an lx-managed package.

use anyhow::{bail, Result};
use clap::Args;

use crate::manifest::Manifest;

#[derive(Debug, Clone, Args)]
pub struct ReinstallArgs {
    /// Package to reinstall.
    pub package: String,

    /// Skip checksum verification (not recommended).
    #[arg(long)]
    pub no_verify: bool,

    /// Proceed without a sidecar checksum (not recommended).
    #[arg(long)]
    pub allow_unverified: bool,

    /// Skip the install confirmation prompt.
    #[arg(short = 'y', long)]
    pub yes: bool,
}

pub fn run(args: ReinstallArgs, token: Option<&str>) -> Result<()> {
    let manifest = Manifest::load()?;
    let entry = manifest.current(&args.package).ok_or_else(|| {
        anyhow::anyhow!(
            "'{}' is not managed by lx (run `lx install {}` first)",
            args.package,
            args.package
        )
    })?;
    if entry.version.trim().is_empty() {
        bail!(
            "no recorded version for '{}'; install it fresh instead",
            args.package
        );
    }
    println!("reinstalling {} {}", args.package, entry.version);
    crate::install::run(
        crate::install::InstallArgs {
            package: args.package,
            format: Some(entry.format.clone()),
            version: Some(entry.version.clone()),
            arch: Some(entry.arch.clone()),
            distribution: Some(entry.distribution.clone()),
            download_only: None,
            no_verify: args.no_verify,
            allow_unverified: args.allow_unverified,
            reinstall: true,
            yes: args.yes,
        },
        token,
    )
}
