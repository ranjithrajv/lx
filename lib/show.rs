// SPDX-License-Identifier: GPL-3.0-or-later

//! `lx show` — deb-get `show` parity: everything known about one package.

use anyhow::{bail, Result};
use clap::Args;

use crate::debs;
use crate::manifest::Manifest;

#[derive(Debug, Clone, Args)]
pub struct ShowArgs {
    /// Package name (e.g. "eza").
    pub package: String,
}

pub fn run(args: ShowArgs) -> Result<()> {
    let manifest = Manifest::load().unwrap_or_default();
    let entries = manifest.packages.get(&args.package);
    let installed = debs::dpkg_installed_version(&args.package);

    if entries.is_none() && installed.is_none() {
        bail!(
            "'{}' is unknown: not managed by lx and not installed per dpkg",
            args.package
        );
    }

    println!("package: {}", args.package);
    match installed {
        Some(v) => println!("installed: {v} (dpkg)"),
        None => println!("installed: no"),
    }
    match entries {
        Some(hist) => {
            println!("managed: yes ({} record(s))", hist.len());
            for (i, e) in hist.iter().enumerate() {
                println!(
                    "  [{}] version={} tag={} arch={} suite={} asset={} at={}",
                    i, e.version, e.tag, e.arch, e.distribution, e.asset, e.installed_at
                );
            }
        }
        None => println!("managed: no"),
    }
    let deps = debs::dpkg_depends(&args.package);
    if !deps.is_empty() {
        println!("depends: {}", deps.join(", "));
    }
    println!("source: {}-debian", debs::repo_name(&args.package));
    Ok(())
}
