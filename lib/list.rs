// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;
use clap::Args;
use std::collections::HashSet;

use crate::consumer;
use crate::manifest::Manifest;
use crate::scandeps;

#[derive(Debug, Clone, Args)]
pub struct ListArgs {
    /// Show each package's dependency closure (via the host package
    /// manager), not just its own version/arch/dist.
    #[arg(long)]
    pub tree: bool,
}

/// List packages lx has installed (tracked in its local manifest), cross-
/// checked against the host package manager's own record of what's actually
/// on disk.
pub fn run(args: ListArgs) -> Result<()> {
    let manifest = Manifest::load()?;
    if manifest.packages.is_empty() {
        println!("no lx-managed packages installed");
        return Ok(());
    }

    println!(
        "{:<20} {:<20} {:<8} {:<10} {:<8}INSTALLED",
        "PACKAGE", "VERSION", "ARCH", "DIST", "GENS"
    );
    for (name, gens) in &manifest.packages {
        let Some(entry) = gens.last() else { continue };
        let format = consumer::format_or_host(&entry.format);
        let note = match consumer::installed_version(name, format) {
            Some(v) if v == entry.version => String::new(),
            Some(v) => format!(" ({} reports {v})", format.name()),
            None => format!(" (missing from {}!)", format.name()),
        };
        println!(
            "{:<20} {:<20} {:<8} {:<10} {:<8}{}{}",
            name,
            entry.version,
            entry.arch,
            entry.distribution,
            gens.len(),
            entry.installed_at,
            note
        );
        if args.tree {
            let mut ancestors = HashSet::new();
            ancestors.insert(name.clone());
            print_deps(name, 1, &ancestors);
        }
    }
    Ok(())
}

/// Recursively prints `package`'s dependency closure, guarding against
/// cycles via `ancestors` (the chain of packages above this one in the
/// current branch).
fn print_deps(package: &str, depth: usize, ancestors: &HashSet<String>) {
    for dep in scandeps::pkg_depends(package) {
        let cyclic = ancestors.contains(&dep);
        println!(
            "{}└─ {}{}",
            "  ".repeat(depth),
            dep,
            if cyclic { " (cycle)" } else { "" }
        );
        if !cyclic {
            let mut next = ancestors.clone();
            next.insert(dep.clone());
            print_deps(&dep, depth + 1, &next);
        }
    }
}
