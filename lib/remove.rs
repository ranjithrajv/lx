// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{bail, Result};
use clap::Args;

use crate::consumer;
use crate::manifest::Manifest;

#[derive(Debug, Clone, Args)]
pub struct RemoveArgs {
    /// Package to remove.
    pub package: String,

    /// Purge configuration files too (`dpkg --purge`, `pacman -Rns`).
    #[arg(long)]
    pub purge: bool,

    /// Skip the removal confirmation prompt.
    #[arg(short = 'y', long)]
    pub yes: bool,
}

pub fn run(args: RemoveArgs) -> Result<()> {
    let mut manifest = Manifest::load()?;
    // Prefer the format lx installed it as; fall back to the host manager for
    // packages that aren't tracked (or predate the manifest's format field).
    let format = manifest
        .current(&args.package)
        .map(|e| consumer::format_or_host(&e.format))
        .unwrap_or_else(crate::index::detect_host_format);
    if consumer::installed_version(&args.package, format).is_none() {
        bail!("'{}' is not installed", args.package);
    }
    consumer::remove(&args.package, args.purge, args.yes, format)?;

    manifest.forget(&args.package);
    manifest.save()?;
    Ok(())
}
