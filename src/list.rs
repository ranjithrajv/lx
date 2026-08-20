use anyhow::Result;
use clap::Args;

use crate::debs;
use crate::manifest::Manifest;

#[derive(Debug, Clone, Args)]
pub struct ListArgs {}

/// List packages lpt has installed (tracked in its local manifest), cross-
/// checked against dpkg's own record of what's actually on disk.
pub fn run(_args: ListArgs) -> Result<()> {
    let manifest = Manifest::load()?;
    if manifest.packages.is_empty() {
        println!("no lpt-managed packages installed");
        return Ok(());
    }

    println!(
        "{:<20} {:<20} {:<8} {:<10} INSTALLED",
        "PACKAGE", "VERSION", "ARCH", "DIST"
    );
    for (name, entry) in &manifest.packages {
        let note = match debs::dpkg_installed_version(name) {
            Some(v) if v == entry.version => String::new(),
            Some(v) => format!(" (dpkg reports {v})"),
            None => " (missing from dpkg!)".to_string(),
        };
        println!(
            "{:<20} {:<20} {:<8} {:<10} {}{}",
            name, entry.version, entry.arch, entry.distribution, entry.installed_at, note
        );
    }
    Ok(())
}
