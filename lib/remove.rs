use anyhow::{bail, Result};
use clap::Args;

use crate::debs;
use crate::manifest::Manifest;

#[derive(Debug, Clone, Args)]
pub struct RemoveArgs {
    /// Package to remove.
    pub package: String,

    /// Purge configuration files too (`dpkg --purge`).
    #[arg(long)]
    pub purge: bool,

    /// Skip the removal confirmation prompt.
    #[arg(short = 'y', long)]
    pub yes: bool,
}

pub fn run(args: RemoveArgs) -> Result<()> {
    if debs::dpkg_installed_version(&args.package).is_none() {
        bail!("'{}' is not installed", args.package);
    }
    debs::remove_deb(&args.package, args.purge, args.yes)?;

    let mut manifest = Manifest::load()?;
    manifest.forget(&args.package);
    manifest.save()?;
    Ok(())
}
